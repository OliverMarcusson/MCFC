use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::RwLock;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::Diagnostic as LspDiagnostic;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};

use crate::analysis::{
    AnalysisResult, analyze_modules, analyze_source, function_at_offset, word_at_offset,
};
use crate::ast::Type;
use crate::diagnostics::{Diagnostic as McfcDiagnostic, TextRange};
use crate::language_catalog::{
    AGENT_EVENTS, ENTITY_METHOD_NAMES, VANILLA_EVENTS, capitalized, event_kind_for_type,
    event_type_name, internal_function_name, internal_method_name, property_names,
};
use crate::minecraft_ids::{MinecraftIdCategory, ids_for_category};
use crate::minecraft_nbt_schema::{self, NbtSchemaCategory, NbtSchemaNode};
use crate::project::{find_manifest_in_ancestors, load_manifest};
use crate::types::{RefKind, StructTypeDef};

#[derive(Debug, Clone)]
struct DocumentState {
    text: String,
    mode: DocumentMode,
}

#[derive(Debug, Clone)]
enum DocumentMode {
    Standalone { analysis: AnalysisResult },
    Project { manifest_path: PathBuf },
    Manifest,
}

#[derive(Debug, Clone)]
struct ProjectConfig {
    manifest_path: PathBuf,
    source_root: PathBuf,
    host_modules: crate::types::HostModules,
}

#[derive(Debug, Clone)]
struct ProjectFileSegment {
    source_start: usize,
    source_end: usize,
}

impl ProjectFileSegment {
    fn local_to_merged_offset(&self, offset: usize) -> usize {
        self.source_start + offset.min(self.len())
    }

    fn merged_to_local_range(&self, range: TextRange) -> Option<TextRange> {
        if range.start < self.source_start || range.start > self.source_end {
            return None;
        }

        Some(TextRange::new(
            range.start - self.source_start,
            range.end.min(self.source_end) - self.source_start,
        ))
    }

    fn len(&self) -> usize {
        self.source_end.saturating_sub(self.source_start)
    }
}

#[derive(Debug, Clone)]
struct ProjectSnapshot {
    manifest_path: PathBuf,
    source_root: PathBuf,
    merged_text: String,
    analysis: AnalysisResult,
    segments: HashMap<PathBuf, ProjectFileSegment>,
}

impl ProjectSnapshot {
    fn segment_for_path(&self, path: &Path) -> Option<&ProjectFileSegment> {
        self.segments.get(path)
    }
}

#[derive(Debug, Clone)]
struct DocumentContext {
    local_text: String,
    analysis_source: String,
    analysis: AnalysisResult,
    segment: Option<ProjectFileSegment>,
}

#[derive(Debug)]
pub struct Backend {
    client: Client,
    documents: Arc<RwLock<HashMap<Url, DocumentState>>>,
    projects: Arc<RwLock<HashMap<PathBuf, ProjectSnapshot>>>,
}

impl Backend {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            documents: Arc::new(RwLock::new(HashMap::new())),
            projects: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    async fn update_document(&self, uri: Url, text: String) {
        if is_manifest_uri(&uri) {
            self.documents.write().await.insert(
                uri.clone(),
                DocumentState {
                    text: text.clone(),
                    mode: DocumentMode::Manifest,
                },
            );
            let diagnostics = match toml::from_str::<crate::project::ProjectManifest>(&text) {
                Ok(_) => Vec::new(),
                Err(error) => vec![LspDiagnostic {
                    range: Range::default(),
                    severity: Some(DiagnosticSeverity::ERROR),
                    source: Some("mcfc".to_string()),
                    message: format!("invalid mcfc.toml: {}", error),
                    ..LspDiagnostic::default()
                }],
            };
            self.client
                .publish_diagnostics(uri, diagnostics, None)
                .await;
            return;
        }
        {
            self.documents.write().await.insert(
                uri.clone(),
                DocumentState {
                    text,
                    mode: DocumentMode::Standalone {
                        analysis: AnalysisResult {
                            diagnostics: Vec::new(),
                            program: None,
                            typed_program: None,
                            functions: Vec::new(),
                            locals: Vec::new(),
                        },
                    },
                },
            );
        }

        if let Some(config) = resolve_project_config_for_uri(&uri)
            && self.rebuild_project(config).await.is_ok()
        {
            return;
        }

        self.refresh_standalone_document(&uri).await;
    }

    async fn publish_diagnostics(&self, uri: Url, text: &str, analysis: &AnalysisResult) {
        let diagnostics = analysis
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic_to_lsp(text, diagnostic))
            .collect();
        self.client
            .publish_diagnostics(uri, diagnostics, None)
            .await;
    }

    async fn refresh_standalone_document(&self, uri: &Url) {
        let text = {
            let documents = self.documents.read().await;
            let Some(state) = documents.get(uri) else {
                return;
            };
            state.text.clone()
        };
        let analysis = analyze_source(&text);
        self.publish_diagnostics(uri.clone(), &text, &analysis)
            .await;
        if let Some(state) = self.documents.write().await.get_mut(uri) {
            state.mode = DocumentMode::Standalone { analysis };
        }
    }

    async fn rebuild_project(&self, config: ProjectConfig) -> Result<()> {
        let snapshot = self.build_project_snapshot(&config).await?;
        let manifest_path = snapshot.manifest_path.clone();

        {
            self.projects
                .write()
                .await
                .insert(manifest_path.clone(), snapshot.clone());
        }

        let open_docs: Vec<(Url, String)> = {
            let documents = self.documents.read().await;
            documents
                .iter()
                .filter_map(|(uri, state)| {
                    let path = uri.to_file_path().ok()?;
                    if is_project_source_file(&path, &snapshot.source_root) {
                        Some((uri.clone(), state.text.clone()))
                    } else {
                        None
                    }
                })
                .collect()
        };

        {
            let mut documents = self.documents.write().await;
            for (uri, _) in &open_docs {
                if let Some(state) = documents.get_mut(uri) {
                    state.mode = DocumentMode::Project {
                        manifest_path: manifest_path.clone(),
                    };
                }
            }
        }

        for (uri, text) in open_docs {
            let diagnostics = snapshot
                .segment_for_path(&path_from_url(&uri).unwrap_or_default())
                .map(|segment| project_diagnostics_for_segment(&text, segment, &snapshot.analysis))
                .unwrap_or_default();
            self.client
                .publish_diagnostics(uri, diagnostics, None)
                .await;
        }

        Ok(())
    }

    async fn build_project_snapshot(&self, config: &ProjectConfig) -> Result<ProjectSnapshot> {
        let open_documents = self.documents.read().await;
        let mut overrides = HashMap::new();
        for (uri, state) in open_documents.iter() {
            let Some(path) = path_from_url(uri) else {
                continue;
            };
            if is_project_source_file(&path, &config.source_root) {
                overrides.insert(path, state.text.clone());
            }
        }

        build_project_snapshot(config, &overrides)
            .map_err(tower_lsp::jsonrpc::Error::invalid_params)
    }

    async fn ensure_document_context(&self, uri: &Url) -> Option<DocumentContext> {
        if let Some(config) = resolve_project_config_for_uri(uri) {
            let manifest_path = config.manifest_path.clone();
            let has_snapshot = self.projects.read().await.contains_key(&manifest_path);
            if !has_snapshot && self.rebuild_project(config).await.is_err() {
                self.refresh_standalone_document(uri).await;
            }
        }

        let (local_text, mode) = {
            let documents = self.documents.read().await;
            let state = documents.get(uri)?;
            (state.text.clone(), state.mode.clone())
        };

        match mode {
            DocumentMode::Standalone { analysis } => Some(DocumentContext {
                local_text: local_text.clone(),
                analysis_source: local_text,
                analysis,
                segment: None,
            }),
            DocumentMode::Project { manifest_path } => {
                let path = path_from_url(uri)?;
                let snapshot = self.projects.read().await.get(&manifest_path)?.clone();
                let segment = snapshot.segment_for_path(&path)?.clone();
                Some(DocumentContext {
                    local_text,
                    analysis_source: snapshot.merged_text,
                    analysis: snapshot.analysis,
                    segment: Some(segment),
                })
            }
            DocumentMode::Manifest => None,
        }
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _params: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                hover_provider: Some(tower_lsp::lsp_types::HoverProviderCapability::Simple(true)),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec![
                        ".".to_string(),
                        "\"".to_string(),
                        "'".to_string(),
                        ":".to_string(),
                        "/".to_string(),
                    ]),
                    ..CompletionOptions::default()
                }),
                document_symbol_provider: Some(OneOf::Left(true)),
                definition_provider: Some(OneOf::Left(true)),
                references_provider: Some(OneOf::Left(true)),
                rename_provider: Some(OneOf::Left(true)),
                document_highlight_provider: Some(OneOf::Left(true)),
                folding_range_provider: Some(FoldingRangeProviderCapability::Simple(true)),
                selection_range_provider: Some(SelectionRangeProviderCapability::Simple(true)),
                document_formatting_provider: Some(OneOf::Left(true)),
                signature_help_provider: Some(SignatureHelpOptions {
                    trigger_characters: Some(vec!["(".to_string(), ",".to_string()]),
                    retrigger_characters: Some(vec![",".to_string()]),
                    ..SignatureHelpOptions::default()
                }),
                semantic_tokens_provider: Some(
                    SemanticTokensServerCapabilities::SemanticTokensOptions(
                        SemanticTokensOptions {
                            legend: SemanticTokensLegend {
                                token_types: vec![
                                    SemanticTokenType::FUNCTION,
                                    SemanticTokenType::STRUCT,
                                    SemanticTokenType::PARAMETER,
                                    SemanticTokenType::VARIABLE,
                                    SemanticTokenType::TYPE,
                                    SemanticTokenType::KEYWORD,
                                ],
                                token_modifiers: Vec::new(),
                            },
                            full: Some(SemanticTokensFullOptions::Bool(true)),
                            range: Some(false),
                            ..SemanticTokensOptions::default()
                        },
                    ),
                ),
                ..ServerCapabilities::default()
            },
            server_info: Some(tower_lsp::lsp_types::ServerInfo {
                name: "mcfc-lsp".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
        })
    }

    async fn initialized(&self, _params: InitializedParams) {}

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        self.update_document(params.text_document.uri, params.text_document.text)
            .await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let Some(change) = params.content_changes.into_iter().next() else {
            return;
        };
        self.update_document(params.text_document.uri, change.text)
            .await;
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        if let Some(text) = params.text {
            self.update_document(params.text_document.uri, text).await;
            return;
        }

        let uri = params.text_document.uri;
        if !self.documents.read().await.contains_key(&uri) {
            return;
        }

        if let Some(config) = resolve_project_config_for_uri(&uri)
            && self.rebuild_project(config).await.is_ok()
        {
            return;
        }

        self.refresh_standalone_document(&uri).await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        let config = resolve_project_config_for_uri(&uri);
        self.documents.write().await.remove(&uri);
        self.client
            .publish_diagnostics(uri.clone(), Vec::new(), None)
            .await;

        if let Some(config) = config {
            let has_open_project_files = {
                let documents = self.documents.read().await;
                documents.keys().any(|open_uri| {
                    path_from_url(open_uri)
                        .map(|path| is_project_source_file(&path, &config.source_root))
                        .unwrap_or(false)
                })
            };

            if has_open_project_files {
                let _ = self.rebuild_project(config).await;
            } else {
                self.projects.write().await.remove(&config.manifest_path);
            }
        }
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let Some(context) = self.ensure_document_context(uri).await else {
            return Ok(None);
        };
        let local_offset = position_to_offset(
            &context.local_text,
            params.text_document_position_params.position,
        );
        let offset = context
            .segment
            .as_ref()
            .map(|segment| segment.local_to_merged_offset(local_offset))
            .unwrap_or(local_offset);
        let Some((word, range)) = word_at_offset(&context.analysis_source, offset) else {
            return Ok(None);
        };
        let Some(contents) = hover_contents(&context.analysis, offset, &word) else {
            return Ok(None);
        };
        let local_range = context
            .segment
            .as_ref()
            .map(|segment| segment.merged_to_local_range(range))
            .unwrap_or(Some(range));
        let Some(local_range) = local_range else {
            return Ok(None);
        };

        Ok(Some(Hover {
            contents: HoverContents::Scalar(MarkedString::String(contents)),
            range: Some(range_from_text_range(&context.local_text, local_range)),
        }))
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        if is_manifest_uri(&params.text_document_position.text_document.uri) {
            return Ok(Some(CompletionResponse::Array(manifest_completion_items())));
        }
        let items = match self
            .ensure_document_context(&params.text_document_position.text_document.uri)
            .await
        {
            Some(context) => {
                let local_offset =
                    position_to_offset(&context.local_text, params.text_document_position.position);
                let offset = context
                    .segment
                    .as_ref()
                    .map(|segment| segment.local_to_merged_offset(local_offset))
                    .unwrap_or(local_offset);
                completion_items(&context.analysis_source, &context.analysis, offset)
            }
            None => static_completion_items(),
        };
        Ok(Some(CompletionResponse::Array(items)))
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        if is_manifest_uri(&params.text_document.uri) {
            return Ok(Some(DocumentSymbolResponse::Nested(
                manifest_document_symbols(
                    &self
                        .documents
                        .read()
                        .await
                        .get(&params.text_document.uri)
                        .map(|state| state.text.clone())
                        .unwrap_or_default(),
                ),
            )));
        }
        let Some(context) = self
            .ensure_document_context(&params.text_document.uri)
            .await
        else {
            return Ok(Some(DocumentSymbolResponse::Nested(Vec::new())));
        };
        let symbols = match context.segment.as_ref() {
            Some(segment) => {
                project_document_symbols(&context.local_text, &context.analysis, segment)
            }
            None => document_symbols_for_analysis(&context.local_text, &context.analysis),
        };

        Ok(Some(DocumentSymbolResponse::Nested(symbols)))
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let Some(context) = self.ensure_document_context(uri).await else {
            return Ok(None);
        };
        let offset = position_to_offset(
            &context.local_text,
            params.text_document_position_params.position,
        );
        let Some((word, _)) = word_at_offset(&context.local_text, offset) else {
            return Ok(None);
        };
        if let Some(range) = local_definition_range(&context.local_text, offset, &word) {
            return Ok(Some(GotoDefinitionResponse::Scalar(Location {
                uri: uri.clone(),
                range: range_from_text_range(&context.local_text, range),
            })));
        }
        if let Some(location) = self.project_function_definition(uri, &word).await {
            return Ok(Some(GotoDefinitionResponse::Scalar(location)));
        }
        Ok(
            function_definition_range(&context.local_text, &word).map(|range| {
                GotoDefinitionResponse::Scalar(Location {
                    uri: uri.clone(),
                    range: range_from_text_range(&context.local_text, range),
                })
            }),
        )
    }

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        let uri = &params.text_document_position.text_document.uri;
        let Some(context) = self.ensure_document_context(uri).await else {
            return Ok(None);
        };
        let offset =
            position_to_offset(&context.local_text, params.text_document_position.position);
        let Some((word, _)) = word_at_offset(&context.local_text, offset) else {
            return Ok(None);
        };
        Ok(Some(
            self.semantic_locations(uri, &context, offset, &word).await,
        ))
    }

    async fn document_highlight(
        &self,
        params: DocumentHighlightParams,
    ) -> Result<Option<Vec<DocumentHighlight>>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let Some(context) = self.ensure_document_context(uri).await else {
            return Ok(None);
        };
        let offset = position_to_offset(
            &context.local_text,
            params.text_document_position_params.position,
        );
        let Some((word, _)) = word_at_offset(&context.local_text, offset) else {
            return Ok(None);
        };
        Ok(Some(
            semantic_ranges(&context.local_text, offset, &word)
                .into_iter()
                .map(|range| DocumentHighlight {
                    range: range_from_text_range(&context.local_text, range),
                    kind: None,
                })
                .collect(),
        ))
    }

    async fn selection_range(
        &self,
        params: SelectionRangeParams,
    ) -> Result<Option<Vec<SelectionRange>>> {
        let Some(context) = self
            .ensure_document_context(&params.text_document.uri)
            .await
        else {
            return Ok(None);
        };
        Ok(Some(
            params
                .positions
                .into_iter()
                .map(|position| selection_at(&context.local_text, position))
                .collect(),
        ))
    }

    async fn folding_range(&self, params: FoldingRangeParams) -> Result<Option<Vec<FoldingRange>>> {
        let Some(context) = self
            .ensure_document_context(&params.text_document.uri)
            .await
        else {
            return Ok(None);
        };
        Ok(Some(folding_ranges(&context.local_text)))
    }

    async fn formatting(&self, params: DocumentFormattingParams) -> Result<Option<Vec<TextEdit>>> {
        let Some(context) = self
            .ensure_document_context(&params.text_document.uri)
            .await
        else {
            return Ok(None);
        };
        let formatted = format_mcfc(&context.local_text);
        if formatted == context.local_text {
            return Ok(Some(Vec::new()));
        }
        Ok(Some(vec![TextEdit {
            range: Range {
                start: Position::new(0, 0),
                end: offset_to_position(&context.local_text, context.local_text.len()),
            },
            new_text: formatted,
        }]))
    }

    async fn rename(&self, params: RenameParams) -> Result<Option<WorkspaceEdit>> {
        if !is_lsp_identifier(&params.new_name) {
            return Ok(None);
        }
        let uri = &params.text_document_position.text_document.uri;
        let Some(context) = self.ensure_document_context(uri).await else {
            return Ok(None);
        };
        let offset =
            position_to_offset(&context.local_text, params.text_document_position.position);
        let Some((word, _)) = word_at_offset(&context.local_text, offset) else {
            return Ok(None);
        };
        let locations = self.semantic_locations(uri, &context, offset, &word).await;
        if locations.is_empty() {
            return Ok(None);
        }
        let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();
        for location in locations {
            changes.entry(location.uri).or_default().push(TextEdit {
                range: location.range,
                new_text: params.new_name.clone(),
            });
        }
        Ok(Some(WorkspaceEdit {
            changes: Some(changes),
            ..WorkspaceEdit::default()
        }))
    }

    async fn signature_help(&self, params: SignatureHelpParams) -> Result<Option<SignatureHelp>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let Some(context) = self.ensure_document_context(uri).await else {
            return Ok(None);
        };
        let offset = position_to_offset(
            &context.local_text,
            params.text_document_position_params.position,
        );
        let Some(call) = call_context_before_offset(&context.local_text, offset) else {
            return Ok(None);
        };
        let Some(signature) = signature_for_call(&context.analysis, &call.name) else {
            return Ok(None);
        };
        Ok(Some(SignatureHelp {
            signatures: vec![SignatureInformation {
                label: signature.to_string(),
                documentation: None,
                parameters: None,
                active_parameter: None,
            }],
            active_signature: Some(0),
            active_parameter: Some(call.arg_index as u32),
        }))
    }

    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> Result<Option<SemanticTokensResult>> {
        let Some(context) = self
            .ensure_document_context(&params.text_document.uri)
            .await
        else {
            return Ok(None);
        };
        Ok(Some(SemanticTokensResult::Tokens(SemanticTokens {
            result_id: None,
            data: semantic_tokens(&context.local_text, &context.analysis),
        })))
    }
}

impl Backend {
    async fn project_function_definition(&self, uri: &Url, name: &str) -> Option<Location> {
        let config = resolve_project_config_for_uri(uri)?;
        let snapshot = self
            .projects
            .read()
            .await
            .get(&config.manifest_path)?
            .clone();
        let function = snapshot.analysis.functions.iter().find(|function| {
            is_named(&function.name, name) && !function.name.starts_with("__mcfc_")
        })?;
        snapshot.segments.iter().find_map(|(path, segment)| {
            let local = segment.merged_to_local_range(function.name_range)?;
            Some(Location {
                uri: Url::from_file_path(path).ok()?,
                range: range_from_text_range(&fs::read_to_string(path).ok()?, local),
            })
        })
    }

    async fn semantic_locations(
        &self,
        uri: &Url,
        context: &DocumentContext,
        offset: usize,
        word: &str,
    ) -> Vec<Location> {
        if local_definition_range(&context.local_text, offset, word).is_some() {
            return semantic_ranges(&context.local_text, offset, word)
                .into_iter()
                .map(|range| Location {
                    uri: uri.clone(),
                    range: range_from_text_range(&context.local_text, range),
                })
                .collect();
        }
        let Some(config) = resolve_project_config_for_uri(uri) else {
            return semantic_ranges(&context.local_text, offset, word)
                .into_iter()
                .map(|range| Location {
                    uri: uri.clone(),
                    range: range_from_text_range(&context.local_text, range),
                })
                .collect();
        };
        let Some(snapshot) = self
            .projects
            .read()
            .await
            .get(&config.manifest_path)
            .cloned()
        else {
            return Vec::new();
        };
        if !snapshot
            .analysis
            .functions
            .iter()
            .any(|function| is_named(&function.name, word) && !function.name.starts_with("__mcfc_"))
        {
            return Vec::new();
        }
        snapshot
            .segments
            .keys()
            .flat_map(|path| {
                let text = fs::read_to_string(path).unwrap_or_default();
                function_reference_ranges(&text, word)
                    .into_iter()
                    .map(move |range| Location {
                        uri: Url::from_file_path(path).unwrap_or_else(|_| uri.clone()),
                        range: range_from_text_range(&text, range),
                    })
            })
            .collect()
    }
}

fn is_lsp_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.chars().enumerate().all(|(index, ch)| {
            ch == '_' || ch.is_ascii_alphanumeric() && (index > 0 || ch.is_ascii_alphabetic())
        })
}

fn is_manifest_uri(uri: &Url) -> bool {
    uri.to_file_path()
        .ok()
        .and_then(|path| path.file_name().map(|name| name == "mcfc.toml"))
        .unwrap_or(false)
}

fn manifest_completion_items() -> Vec<CompletionItem> {
    [
        ("namespace", "namespace = \"my_pack\""),
        ("source_dir", "source_dir = \"src\""),
        ("asset_dir", "asset_dir = \"assets\""),
        ("out_dir", "out_dir = \"dist\""),
        ("load", "load = [\"main\"]"),
        ("tick", "tick = [\"tick\"]"),
        ("[helper]", "[helper]\nbackend = \"mcfd\""),
        ("[helper.agent]", "[helper.agent]\nenabled = true"),
        (
            "[helper.capabilities]",
            "[helper.capabilities]\ntime = true\nrand = true",
        ),
    ]
    .into_iter()
    .map(|(label, insert_text)| CompletionItem {
        label: label.to_string(),
        kind: Some(CompletionItemKind::PROPERTY),
        insert_text: Some(insert_text.to_string()),
        insert_text_format: Some(InsertTextFormat::SNIPPET),
        detail: Some("MCFC project manifest".to_string()),
        ..CompletionItem::default()
    })
    .collect()
}

#[allow(deprecated)]
fn manifest_document_symbols(source: &str) -> Vec<DocumentSymbol> {
    let mut symbols = Vec::new();
    let mut offset = 0usize;
    for line in source.lines() {
        let trimmed = line.trim();
        let name = if trimmed.starts_with('[') && trimmed.ends_with(']') {
            Some(trimmed.trim_matches(&['[', ']'][..]))
        } else {
            trimmed.split_once('=').map(|(key, _)| key.trim())
        };
        if let Some(name) = name.filter(|name| !name.is_empty()) {
            let start = offset + line.find(name).unwrap_or(0);
            let range = TextRange::new(start, start + name.len());
            symbols.push(DocumentSymbol {
                name: name.to_string(),
                detail: Some("mcfc.toml".to_string()),
                kind: SymbolKind::PROPERTY,
                tags: None,
                deprecated: None,
                range: range_from_text_range(source, range),
                selection_range: range_from_text_range(source, range),
                children: None,
            });
        }
        offset += line.len() + 1;
    }
    symbols
}

fn is_lsp_word_char(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

fn semantic_ranges(source: &str, offset: usize, word: &str) -> Vec<TextRange> {
    let Some(target) = word_at_offset(source, offset).map(|(_, range)| range) else {
        return Vec::new();
    };
    if previous_char(source, target.start) == Some('.') {
        return Vec::new();
    }
    if let Some(scope) = scope_at_offset(source, offset)
        && local_definition_in_scope(source, &scope, word).is_some()
    {
        return identifier_ranges(&source[scope.start..scope.end], word, false)
            .into_iter()
            .map(|range| TextRange::new(range.start + scope.start, range.end + scope.start))
            .collect();
    }
    if function_definition_range(source, word).is_some() {
        return function_reference_ranges(source, word);
    }
    Vec::new()
}

fn local_definition_range(source: &str, offset: usize, word: &str) -> Option<TextRange> {
    let scope = scope_at_offset(source, offset)?;
    local_definition_in_scope(source, &scope, word)
}

fn function_definition_range(source: &str, word: &str) -> Option<TextRange> {
    identifier_ranges(source, word, false)
        .into_iter()
        .find(|range| {
            let line_start = source[..range.start].rfind('\n').map_or(0, |i| i + 1);
            let line_end = source[range.end..]
                .find('\n')
                .map_or(source.len(), |i| range.end + i);
            let line = &source[line_start..line_end];
            is_scope_header(strip_line_comment(line).trim())
                && line.find('(').map(|open| line_start + open) == Some(range.end)
        })
}

fn function_reference_ranges(source: &str, word: &str) -> Vec<TextRange> {
    identifier_ranges(source, word, false)
        .into_iter()
        .filter(|range| {
            let rest = source[range.end..].trim_start();
            rest.starts_with('(')
        })
        .collect()
}

fn scope_at_offset(source: &str, offset: usize) -> Option<TextRange> {
    let mut line_start = 0usize;
    let mut active = None;
    for line in source.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if !line.starts_with(char::is_whitespace) && is_scope_header(trimmed) {
            if line_start <= offset {
                active = Some(line_start);
            } else {
                break;
            }
        }
        line_start += line.len();
    }
    let start = active?;
    let end = source[start..]
        .find('\n')
        .map(|_| {
            let mut cursor = start;
            for line in source[start..].split_inclusive('\n') {
                if cursor > start
                    && !line.starts_with(char::is_whitespace)
                    && is_scope_header(line.trim_start())
                {
                    return cursor;
                }
                cursor += line.len();
            }
            source.len()
        })
        .unwrap_or(source.len());
    Some(TextRange::new(start, end))
}

fn local_definition_in_scope(source: &str, scope: &TextRange, word: &str) -> Option<TextRange> {
    let body = &source[scope.start..scope.end];
    let header_end = body.find('\n').unwrap_or(body.len());
    if let Some(params) = parse_params(&body[..header_end])
        .into_iter()
        .find(|local| local.name == word)
    {
        let start = identifier_ranges(&body[..header_end], &params.name, false)
            .last()?
            .start
            + scope.start;
        return Some(TextRange::new(start, start + params.name.len()));
    }
    body.split_inclusive('\n')
        .scan(scope.start, |base, line| {
            let current = *base;
            *base += line.len();
            Some((current, line))
        })
        .find_map(|(base, line)| {
            let code = strip_line_comment(line).trim();
            let name = parse_let_binding(code)
                .map(|(name, _, _)| name)
                .or_else(|| parse_for_local(code).map(|local| local.name))?;
            (name == word).then(|| {
                let start = identifier_ranges(line, &name, false)
                    .first()
                    .map_or(0, |range| range.start);
                TextRange::new(base + start, base + start + name.len())
            })
        })
}

fn identifier_ranges(source: &str, word: &str, allow_members: bool) -> Vec<TextRange> {
    let bytes = source.as_bytes();
    let mut ranges = Vec::new();
    let mut index = 0usize;
    let mut quote = None;
    while index < bytes.len() {
        let byte = bytes[index];
        if let Some(delimiter) = quote {
            if byte == b'\\' {
                index += 2;
                continue;
            }
            if byte == delimiter {
                quote = None;
            }
            index += 1;
            continue;
        }
        if source[index..].starts_with("//") {
            index += source[index..].find('\n').unwrap_or(source.len() - index);
            continue;
        }
        if source[index..].starts_with("/*") {
            index += source[index..]
                .find("*/")
                .map_or(source.len() - index, |end| end + 2);
            continue;
        }
        if byte == b'"' {
            quote = Some(byte);
            index += 1;
            continue;
        }
        if source[index..].starts_with(word) {
            let end = index + word.len();
            let before = source[..index].chars().next_back();
            let after = source[end..].chars().next();
            if !before.map(is_lsp_word_char).unwrap_or(false)
                && !after.map(is_lsp_word_char).unwrap_or(false)
                && (allow_members || before != Some('.'))
            {
                ranges.push(TextRange::new(index, end));
            }
            index = end;
            continue;
        }
        index += 1;
    }
    ranges
}

fn selection_at(source: &str, position: Position) -> SelectionRange {
    let offset = position_to_offset(source, position);
    let word = word_at_offset(source, offset)
        .map(|(_, range)| range)
        .unwrap_or(TextRange::new(offset, offset));
    let line_start = source[..offset]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    let line_end = source[offset..]
        .find('\n')
        .map(|index| offset + index)
        .unwrap_or(source.len());
    SelectionRange {
        range: range_from_text_range(source, word),
        parent: Some(Box::new(SelectionRange {
            range: range_from_text_range(source, TextRange::new(line_start, line_end)),
            parent: None,
        })),
    }
}

fn folding_ranges(source: &str) -> Vec<FoldingRange> {
    let lines: Vec<&str> = source.lines().collect();
    let mut ranges = Vec::new();
    let mut stack: Vec<(usize, usize)> = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        while stack
            .last()
            .map(|(_, depth)| indent <= *depth)
            .unwrap_or(false)
        {
            let (start, _) = stack.pop().unwrap();
            if index > start + 1 {
                ranges.push(FoldingRange {
                    start_line: start as u32,
                    start_character: None,
                    end_line: (index - 1) as u32,
                    end_character: None,
                    kind: Some(FoldingRangeKind::Region),
                    collapsed_text: None,
                });
            }
        }
        if strip_line_comment(trimmed).trim_end().ends_with('{') {
            stack.push((index, indent));
        }
    }
    while let Some((start, _)) = stack.pop() {
        if lines.len() > start + 1 {
            ranges.push(FoldingRange {
                start_line: start as u32,
                start_character: None,
                end_line: (lines.len() - 1) as u32,
                end_character: None,
                kind: Some(FoldingRangeKind::Region),
                collapsed_text: None,
            });
        }
    }
    ranges
}

fn format_mcfc(source: &str) -> String {
    let mut output = String::new();
    for line in source.lines() {
        let trimmed_end = line.trim_end();
        if trimmed_end.is_empty() {
            output.push('\n');
            continue;
        }
        let indent = trimmed_end.len() - trimmed_end.trim_start().len();
        let level = indent / 4;
        output.push_str(&"    ".repeat(level));
        output.push_str(trimmed_end.trim_start());
        output.push('\n');
    }
    output
}

/// Matches `word` against a function's own name, ignoring its module path
/// (`util::double` matches `double`).
// ponytail: first match wins when two modules share a name; resolve through
// the cursor's module and imports if that ambiguity matters.
fn is_named(function: &str, word: &str) -> bool {
    function.rsplit("::").next() == Some(word)
}

fn signature_for_call(analysis: &AnalysisResult, name: &str) -> Option<String> {
    if let Some(function) = analysis
        .functions
        .iter()
        .find(|function| is_named(&function.name, name))
    {
        return Some(function.signature());
    }
    match name {
        "of" | "selector" => Some("Selector.of(value: String) -> Selector".to_string()),
        "getFirst" => Some("Selector.getFirst() -> Entity | List<T>.getFirst() -> T".to_string()),
        "findFirst" => Some("Selector.findFirst() -> Optional<Entity>".to_string()),
        "isValid" => Some("Entity.isValid() -> boolean".to_string()),
        "parseInt" => Some("Integer.parseInt(s: String) -> int".to_string()),
        "valueOf" => Some("String.valueOf(x) -> String".to_string()),
        "sqrt" => Some("Math.sqrt(x: float) -> float".to_string()),
        "pow" => Some("Math.pow(x: float, exponent: float) -> float".to_string()),
        "EntityData" => Some("new EntityData(id: String)".to_string()),
        "ItemStack" => Some("new ItemStack(id: String)".to_string()),
        "block" => Some(
            "Block.of(position: String) -> Block, Block.of(x: int, y: int, z: int) -> Block"
                .to_string(),
        ),
        "BlockData" => Some("new BlockData(id: String)".to_string()),
        "sleep" => Some("sleep(seconds: int) -> void".to_string()),
        "sleepTicks" => Some("sleepTicks(ticks: int) -> void".to_string()),
        "random" => Some(
            "random() -> int | random(max: int) -> int | random(min: int, max: int) -> int"
                .to_string(),
        ),
        "BossBar" => Some("new BossBar(id: String, name: String|Component)".to_string()),
        _ => None,
    }
}

fn semantic_tokens(source: &str, analysis: &AnalysisResult) -> Vec<SemanticToken> {
    let mut entries: Vec<(u32, u32, u32, u32)> = Vec::new();
    for function in &analysis.functions {
        let position = offset_to_position(source, function.name_range.start);
        entries.push((
            position.line,
            position.character,
            function.name.len() as u32,
            0,
        ));
    }
    if let Some(program) = &analysis.program {
        for struct_def in &program.structs {
            let position = offset_to_position(source, struct_def.span.range.start);
            entries.push((
                position.line,
                position.character,
                struct_def.name.len() as u32,
                1,
            ));
        }
    }
    entries.sort();
    let mut previous_line = 0u32;
    let mut previous_start = 0u32;
    entries
        .into_iter()
        .map(|(line, start, length, token_type)| {
            let delta_line = line - previous_line;
            let delta_start = if delta_line == 0 {
                start - previous_start
            } else {
                start
            };
            previous_line = line;
            previous_start = start;
            SemanticToken {
                delta_line,
                delta_start,
                length,
                token_type,
                token_modifiers_bitset: 0,
            }
        })
        .collect()
}

fn path_from_url(uri: &Url) -> Option<PathBuf> {
    uri.to_file_path().ok()
}

fn resolve_project_config_for_uri(uri: &Url) -> Option<ProjectConfig> {
    let path = path_from_url(uri)?;
    resolve_project_config_for_path(&path).ok().flatten()
}

fn resolve_project_config_for_path(
    path: &Path,
) -> std::result::Result<Option<ProjectConfig>, String> {
    if !is_mcf_file(path) {
        return Ok(None);
    }

    let Some(manifest_path) = find_manifest_in_ancestors(path)? else {
        return Ok(None);
    };
    let manifest = load_manifest(&manifest_path)?;
    let project_root = manifest_path.parent().ok_or_else(|| {
        format!(
            "manifest '{}' has no parent directory",
            manifest_path.display()
        )
    })?;
    let source_root = project_root.join(manifest.source_dir);
    if !is_project_source_file(path, &source_root) {
        return Ok(None);
    }

    Ok(Some(ProjectConfig {
        manifest_path,
        source_root,
        host_modules: crate::types::HostModules::from_helper(manifest.helper.as_ref()),
    }))
}

fn is_mcf_file(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .map(|value| value.eq_ignore_ascii_case("mcf"))
        .unwrap_or(false)
}

fn is_project_source_file(path: &Path, source_root: &Path) -> bool {
    is_mcf_file(path) && path.starts_with(source_root)
}

fn build_project_snapshot(
    config: &ProjectConfig,
    overrides: &HashMap<PathBuf, String>,
) -> std::result::Result<ProjectSnapshot, String> {
    let loaded = crate::modules::load(
        &config.source_root.join("main.mcf"),
        &crate::project::collect_source_files(&config.source_root)?,
        &|file: &Path| match overrides.get(file) {
            Some(source) => Ok(source.clone()),
            None => fs::read_to_string(file)
                .map_err(|error| format!("failed to read '{}': {}", file.display(), error)),
        },
    )?;
    let segments = loaded
        .modules
        .iter()
        .map(|module| {
            (
                module.file.clone(),
                ProjectFileSegment {
                    source_start: module.source_start,
                    source_end: module.source_end,
                },
            )
        })
        .collect();

    Ok(ProjectSnapshot {
        manifest_path: config.manifest_path.clone(),
        source_root: config.source_root.clone(),
        analysis: analyze_modules(&loaded.merged, &config.host_modules, &loaded.modules),
        merged_text: loaded.merged,
        segments,
    })
}

fn project_diagnostics_for_segment(
    local_text: &str,
    segment: &ProjectFileSegment,
    analysis: &AnalysisResult,
) -> Vec<LspDiagnostic> {
    analysis
        .diagnostics
        .iter()
        .filter_map(|diagnostic| {
            let range = segment.merged_to_local_range(diagnostic.span.range)?;
            Some(LspDiagnostic {
                range: range_from_text_range(local_text, range),
                severity: Some(DiagnosticSeverity::ERROR),
                source: Some("mcfc".to_string()),
                message: diagnostic.message.clone(),
                ..LspDiagnostic::default()
            })
        })
        .collect()
}

#[allow(deprecated)]
fn document_symbols_for_analysis(source: &str, analysis: &AnalysisResult) -> Vec<DocumentSymbol> {
    let mut symbols: Vec<DocumentSymbol> = analysis
        .program
        .as_ref()
        .map(|program| {
            program
                .structs
                .iter()
                .map(|struct_def| DocumentSymbol {
                    name: struct_def.name.clone(),
                    detail: Some(struct_signature_from_fields(
                        &struct_def.name,
                        &struct_def
                            .fields
                            .iter()
                            .map(|field| (field.name.clone(), field.ty.clone()))
                            .collect::<Vec<_>>(),
                    )),
                    kind: tower_lsp::lsp_types::SymbolKind::STRUCT,
                    tags: None,
                    deprecated: None,
                    range: range_from_text_range(source, struct_def.span.range),
                    selection_range: range_from_text_range(source, struct_def.span.range),
                    children: None,
                })
                .collect()
        })
        .unwrap_or_default();
    if let Some(program) = analysis.program.as_ref() {
        symbols.extend(program.enums.iter().map(|enum_def| {
            let range = range_from_text_range(source, enum_def.span.range);
            DocumentSymbol {
                name: enum_def.name.clone(),
                detail: Some(enum_signature(&enum_def.name, &enum_def.variants)),
                kind: tower_lsp::lsp_types::SymbolKind::ENUM,
                tags: None,
                deprecated: None,
                range,
                selection_range: range,
                children: None,
            }
        }));
    }
    symbols.extend(analysis.functions.iter().map(|function| {
        let (name, kind) = symbol_name_and_kind(source, function, function.name_range);
        DocumentSymbol {
            name,
            detail: Some(function.signature()),
            kind,
            tags: None,
            deprecated: None,
            range: range_from_text_range(source, function.range),
            selection_range: range_from_text_range(source, function.name_range),
            children: None,
        }
    }));
    symbols
}

#[allow(deprecated)]
fn project_document_symbols(
    local_text: &str,
    analysis: &AnalysisResult,
    segment: &ProjectFileSegment,
) -> Vec<DocumentSymbol> {
    let mut symbols = Vec::new();

    if let Some(program) = analysis.program.as_ref() {
        for struct_def in &program.structs {
            let Some(range) = segment.merged_to_local_range(struct_def.span.range) else {
                continue;
            };
            symbols.push(DocumentSymbol {
                name: struct_def.name.clone(),
                detail: Some(struct_signature_from_fields(
                    &struct_def.name,
                    &struct_def
                        .fields
                        .iter()
                        .map(|field| (field.name.clone(), field.ty.clone()))
                        .collect::<Vec<_>>(),
                )),
                kind: tower_lsp::lsp_types::SymbolKind::STRUCT,
                tags: None,
                deprecated: None,
                range: range_from_text_range(local_text, range),
                selection_range: range_from_text_range(local_text, range),
                children: None,
            });
        }
        for enum_def in &program.enums {
            let Some(range) = segment.merged_to_local_range(enum_def.span.range) else {
                continue;
            };
            let range = range_from_text_range(local_text, range);
            symbols.push(DocumentSymbol {
                name: enum_def.name.clone(),
                detail: Some(enum_signature(&enum_def.name, &enum_def.variants)),
                kind: tower_lsp::lsp_types::SymbolKind::ENUM,
                tags: None,
                deprecated: None,
                range,
                selection_range: range,
                children: None,
            });
        }
    }

    for function in &analysis.functions {
        let Some(range) = segment.merged_to_local_range(function.range) else {
            continue;
        };
        let Some(name_range) = segment.merged_to_local_range(function.name_range) else {
            continue;
        };
        let (name, kind) = symbol_name_and_kind(local_text, function, name_range);
        symbols.push(DocumentSymbol {
            name,
            detail: Some(function.signature()),
            kind,
            tags: None,
            deprecated: None,
            range: range_from_text_range(local_text, range),
            selection_range: range_from_text_range(local_text, name_range),
            children: None,
        });
    }

    symbols
}

/// Handlers are renamed to `__mcfc_*` hooks; outline them under their source name.
fn symbol_name_and_kind(
    source: &str,
    function: &crate::analysis::FunctionInfo,
    name_range: TextRange,
) -> (String, SymbolKind) {
    if !function.name.starts_with("__mcfc_") {
        return (function.name.clone(), SymbolKind::FUNCTION);
    }
    let name = source
        .get(name_range.start..name_range.end)
        .unwrap_or(&function.name)
        .to_string();
    let kind = if function.name.starts_with("__mcfc_event_")
        || function.name.starts_with("__mcfc_agent_event_")
    {
        SymbolKind::EVENT
    } else {
        SymbolKind::FUNCTION
    };
    (name, kind)
}

fn hover_contents(analysis: &AnalysisResult, offset: usize, word: &str) -> Option<String> {
    if let Some(struct_defs) = analysis
        .typed_program
        .as_ref()
        .map(|program| &program.struct_defs)
        && let Some(def) = struct_defs.get(word)
    {
        let signature = if let Some(variants) = &def.enum_variants {
            enum_signature(word, variants)
        } else {
            struct_signature(word, def)
        };
        return Some(format!("```mcfc\n{}\n```", signature));
    }

    if let Some(function) = analysis
        .functions
        .iter()
        .find(|function| is_named(&function.name, word))
    {
        return Some(format!("```mcfc\n{}\n```", function.signature()));
    }

    if let Some(function) = function_at_offset(analysis, offset)
        && let Some(local) = analysis
            .locals
            .iter()
            .find(|local| local.function == function.name && local.name == word)
    {
        return Some(format!(
            "```mcfc\n{} {}\n```",
            local.ty.as_str(),
            local.name
        ));
    }

    builtin_hover(word).map(str::to_string)
}

fn builtin_hover(word: &str) -> Option<&'static str> {
    match word {
        "record" => Some("```mcfc\nrecord Name(int field, String label) {}\n```"),
        "enum" => Some("```mcfc\nenum Mode { IDLE, RUNNING }\n```"),
        "PlayerState" => Some("```mcfc\n@PlayerState(\"Money\")\nint money;\n```"),
        "EntityState" => Some("```mcfc\n@EntityState\nString sendTitle;\n```"),
        "switch" => Some(
            "```mcfc\nswitch (mode) {\n    case Mode.IDLE -> ...;\n    default -> { ... }\n}\n```",
        ),
        "case" => Some("A `switch` arm: `case A, B -> ...`. Cases don't fall through."),
        "default" => Some("The fallback arm of a `switch` statement."),
        "mcf" => Some("```mcfc\nmcf(\"say $(expr)\");\n```"),
        "async" => Some("```mcfc\nasync {\n    ...\n}\n```"),
        "sleep" => Some("```mcfc\nsleep(seconds: int) -> void\n```"),
        "sleepTicks" => Some("```mcfc\nsleepTicks(ticks: int) -> void\n```"),
        "random" => Some(
            "```mcfc\nrandom() -> int\nrandom(max: int) -> int\nrandom(min: int, max: int) -> int\n```",
        ),
        "Selector" => Some("```mcfc\nSelector.of(value: String) -> Selector\n```"),
        "Log" => Some(
            "```mcfc
Log.debug(msg: String)
Log.info(msg: String)
Log.warn(msg: String)
Log.error(msg: String)
Log.dump(value)
Log.setLevel(level: String)
```
Shown to players tagged `mcfc.log`.",
        ),
        "Sidebar" => Some(
            "```mcfc\nSidebar.setTitle(text: String)\nSidebar.setLine(line: int, text: String)\nSidebar.removeLine(line: int)\nSidebar.clear()\n```\nThe sidebar every player sees. Line 0 is on top.",
        ),
        "getFirst" => Some("```mcfc\nSelector.getFirst() -> Entity\nList<T>.getFirst() -> T\n```"),
        "findFirst" => Some("```mcfc\nSelector.findFirst() -> Optional<Entity>\n```"),
        "isPresent" => Some("```mcfc\nOptional<T>.isPresent() -> boolean\n```"),
        "orElse" => Some("```mcfc\nOptional<T>.orElse(fallback: T) -> T\n```"),
        "get" => Some(
            "```mcfc\nList<T>.get(index: int) -> Optional<T>\nMap<String, T>.get(key: String) -> Optional<T>\nOptional<T>.get() -> T\n```",
        ),
        "isValid" => Some("```mcfc\nEntity.isValid() -> boolean\n```"),
        "parseInt" => Some("```mcfc\nInteger.parseInt(s: String) -> int\n```"),
        "valueOf" => Some("```mcfc\nString.valueOf(x) -> String\n```"),
        "sqrt" => Some("```mcfc\nMath.sqrt(x: float) -> float\n```"),
        "set" => Some("```mcfc\nList<T>.set(index: int, value: T) -> void\n```"),
        "put" => Some("```mcfc\nMap<String, T>.put(key: String, value: T) -> void\n```"),
        "getOrDefault" => {
            Some("```mcfc\nMap<String, T>.getOrDefault(key: String, fallback: T) -> T\n```")
        }
        "isEmpty" => Some(
            "```mcfc\nString.isEmpty() -> boolean\nList<T>.isEmpty() -> boolean\nMap<String, T>.isEmpty() -> boolean\nOptional<T>.isEmpty() -> boolean\n```",
        ),
        "hasData" => Some("```mcfc\nhasData(value: storage_path) -> boolean\n```"),
        "Block" => Some(
            "```mcfc\nBlock.of(position: String) -> Block\nBlock.of(x: int, y: int, z: int) -> Block\n```",
        ),
        "at" => Some(
            "```mcfc\nat(anchor: Entity, value: Selector|Entity|Block) -> Selector|Entity|Block\n\nat(anchor) {\n    ...\n}\n```",
        ),
        "as" => Some(
            "```mcfc\nas(anchor: Selector|Entity, value: Selector|Entity|Block) -> Selector|Entity|Block\n\nas(anchor) {\n    ...\n}\n```",
        ),
        "asNbt" => Some(
            "```mcfc\nEntityData.asNbt() -> Nbt\nBlockData.asNbt() -> Nbt\nItemStack.asNbt() -> Nbt\n```",
        ),
        "summon" => Some(
            "```mcfc\nsummon(entityId: String) -> Entity\nsummon(entityId: String, data: Nbt) -> Entity\nsummon(spec: EntityData) -> Entity\nblock.summon(entityId: String) -> Entity\nblock.summon(entityId: String, data: Nbt) -> Entity\nblock.summon(spec: EntityData) -> Entity\n```",
        ),
        "teleport" => Some("```mcfc\nentity.teleport(destination: Entity|Block) -> void\n```"),
        "damage" => Some("```mcfc\nentity.damage(amount: int) -> void\n```"),
        "heal" => Some("```mcfc\nentity.heal(amount: int) -> void\n```"),
        "setVelocity" => {
            Some("`Entity.setVelocity(x: float, y: float, z: float) -> void` (non-player entities)")
        }
        "getAttribute" => Some("`Entity.getAttribute(id: String|Attribute) -> float`"),
        "setAttribute" => Some("`Entity.setAttribute(id: String|Attribute, value: float) -> void`"),
        "setRotation" => Some("`Entity.setRotation(yaw: float, pitch: float) -> void`"),
        "addVelocity" => Some("`Entity.addVelocity(x: float, y: float, z: float) -> void`"),
        "lookAt" => Some("`Entity.lookAt(target: Entity|Block) -> void`"),
        "yawTo" => Some("`Entity.yawTo(target: Entity|Block) -> float`"),
        "setOwner" => Some("`Entity.setOwner(owner: Entity) -> void`"),
        "setInterpolationDuration" => {
            Some("`Display.setInterpolationDuration(ticks: int) -> void`")
        }
        "setInterpolationDelay" => Some("`Display.setInterpolationDelay(ticks: int) -> void`"),
        "setTeleportDuration" => {
            Some("`Display.setTeleportDuration(ticks: int) -> void` (0 to 59)")
        }
        "setTranslation" => Some("`Display.setTranslation(translation: Vec3) -> void`"),
        "setScale" => Some("`Display.setScale(scale: Vec3) -> void`"),
        "setLeftRotation" => {
            Some("`Display.setLeftRotation(angle: float, axis: Vec3) -> void` (radians)")
        }
        "animate" => Some(
            "`Display.animate(ticks: int, translation: Vec3, scale: Vec3) -> void`: interpolates to the new transform over `ticks`",
        ),
        "getOwner" => Some(
            "`Entity.getOwner() -> Optional<Entity>`: the setOwner link, or the vanilla owner of a tamed animal or projectile",
        ),
        "getTargetBlock" => Some(
            "`Entity.getTargetBlock(maxDistance: float) -> Optional<Block>`: first solid block along the view",
        ),
        "getTargetEntity" => Some(
            "`Entity.getTargetEntity(maxDistance: float) -> Optional<Entity>`: first entity along the view, stopped by blocks",
        ),
        "pitchTo" => Some("`Entity.pitchTo(target: Entity|Block) -> float`"),
        "setHealth" => {
            Some("`Entity.setHealth(points: float) -> void` (players finish on a later tick)")
        }
        "setFoodLevel" => Some("`Player.setFoodLevel(level: int) -> void` (converges over ticks)"),
        "setSidebarTitle" | "setSidebarLine" | "removeSidebarLine" | "clearSidebar" => Some(
            "`Player.setSidebarTitle(text)`, `setSidebarLine(line, text)`, `removeSidebarLine(line)`, `clearSidebar()`: this player's own sidebar through mcfd-agent; the shared `Sidebar` without it",
        ),
        "getCurrentInput" => Some(
            "`Player.getCurrentInput().isForward()` and the other movement key checks return boolean.",
        ),
        "give" => Some(
            "```mcfc\nentity.give(itemId: String, count: int) -> void\nentity.give(stack: ItemStack) -> void\n```",
        ),
        "clear" => Some("```mcfc\nentity.clear(itemId: String, count: int) -> void\n```"),
        "lootGive" => Some("```mcfc\nentity.lootGive(table: String) -> void\n```"),
        "lootInsert" => Some("```mcfc\nblock.lootInsert(table: String) -> void\n```"),
        "lootSpawn" => Some("```mcfc\nblock.lootSpawn(table: String) -> void\n```"),
        "spawnItem" => Some("```mcfc\nblock.spawnItem(stack: ItemStack) -> Entity\n```"),
        "sendMessage" => {
            Some("```mcfc\nentity.sendMessage(message: String|Component) -> void\n```")
        }
        "sendTitle" => Some("```mcfc\nentity.sendTitle(message: String|Component) -> void\n```"),
        "sendActionBar" => Some(
            "```mcfc\nentity.sendActionBar(message: String|Component, priority?: String) -> void\n```\nPriority is \"override\", \"notification\" (default), \"conditional\" or \"persistent\", as in Smithed Actionbar.",
        ),
        "debug" => Some("```mcfc\ndebug(message: String) -> void\n```"),
        "debugMarker" => Some(
            "```mcfc\nblock.debugMarker(label: String) -> void\nblock.debugMarker(label: String, markerBlock: String) -> void\n```",
        ),
        "debugEntity" => Some("```mcfc\nentity.debugEntity(label: String) -> void\n```"),
        "playSound" => {
            Some("```mcfc\nentity.playSound(sound: String, category: String) -> void\n```")
        }
        "stopSound" => {
            Some("```mcfc\nentity.stopSound(category: String, sound: String) -> void\n```")
        }
        "spawnParticle" => Some(
            "```mcfc\nblock.spawnParticle(name: String) -> void\nblock.spawnParticle(name: String, count: int) -> void\nblock.spawnParticle(name: String, count: int, viewers: Entity|Selector) -> void\n```",
        ),
        "setBlock" => Some("```mcfc\nblock.setBlock(blockId: String|BlockData) -> void\n```"),
        "is" => Some("```mcfc\nblock.is(blockId: String) -> boolean\n```"),
        "fill" => Some("```mcfc\nblock.fill(to: Block, blockId: String|BlockData) -> void\n```"),
        "EntityData" => Some(
            "```mcfc\nnew EntityData(id: String)\n- id: String (read-only)\n- nbt.*\n- asNbt() -> Nbt\n```",
        ),
        "Player" => Some(
            "```mcfc\nPlayer\nKnown-player entity reference. Supports entity methods plus player.inventory[index] and player.hotbar[index].\n\n(Player) entity\n```",
        ),
        "BlockData" => Some(
            "```mcfc\nnew BlockData(id: String)\n- id: String (read-only)\n- states.*\n- nbt.*\n- asNbt() -> Nbt\n```",
        ),
        "ItemStack" => Some(
            "```mcfc\nnew ItemStack(id: String)\n- id: String (read-only)\n- getCount()/setCount(int)\n- getName()/setName(String)\n- nbt.*\n- asNbt() -> Nbt\n```",
        ),
        "Component" => Some(
            "```mcfc\nnew Component() / new Component(text: String)\n- storage-backed text component builder\n- supports arbitrary .field / [index] writes for text component content, styling, events, and nested children\n```",
        ),
        "BossBar" => Some("```mcfc\nnew BossBar(id: String, name: String|Component)\n```"),
        "ItemSlot" => Some(
            "```mcfc\nItemSlot\n- isValid: boolean (read-only)\n- id: String (read-only)\n- count: int\n- nbt.*\n- clear() -> void\n```",
        ),
        "position" => Some("```mcfc\nentity.position -> Block\n```"),
        "state" => Some(
            "```mcfc\nentity.state.* -> MCFC-managed int/boolean scoreboard state for any Entity\nplayer.state.* -> MCFC-managed int/boolean scoreboard state for known players\n```",
        ),
        "size" | "length" => Some("```mcfc\nList<T>.size() -> int\nString.length() -> int\n```"),
        "add" => Some(
            "```mcfc\nList<T>.add(value: T) -> void\nList<T>.add(index: int, value: T) -> void\n```",
        ),
        "removeLast" => Some("```mcfc\nList<T>.removeLast() -> T\n```"),
        "containsKey" => Some("```mcfc\nMap<String, T>.containsKey(key: String) -> boolean\n```"),
        "remove" => Some(
            "```mcfc\nList<T>.remove(index: int) -> T\nMap<String, T>.remove(key: String) -> void\nBossBar.remove() -> void\n```",
        ),
        "effect" => {
            Some("```mcfc\nentity.effect(name: String, duration: int, amplifier: int) -> void\n```")
        }
        "addTag" => Some("```mcfc\nentity.addTag(name: String) -> void\n```"),
        "removeTag" => Some("```mcfc\nentity.removeTag(name: String) -> void\n```"),
        "hasTag" => Some("```mcfc\nentity.hasTag(name: String) -> boolean\n```"),
        _ => None,
    }
}

fn completion_items(source: &str, analysis: &AnalysisResult, offset: usize) -> Vec<CompletionItem> {
    if let Some(items) = minecraft_id_completion_items(source, offset) {
        return items;
    }
    if let Some(chain) = member_chain_before_cursor(source, offset) {
        return member_completion_items(source, analysis, offset, &chain);
    }
    if let Some(receiver) = inline_call_receiver_before_cursor(source, offset) {
        return completion_items_for_receiver(Some(receiver), analysis);
    }

    let containing_function =
        function_at_offset(analysis, offset).map(|function| function.name.as_str());
    let mut items = if is_declaration_completion_position(source, offset) {
        let mut items = static_completion_items();
        items.extend(struct_type_items(analysis));
        items
    } else {
        let mut items = expression_completion_items();
        items.extend(struct_type_items(analysis));
        for (label, detail, insert_text) in [
            (
                "switch ...",
                "Dispatch on an enum, integer or string value",
                "switch (${1:value}) {\n\tcase ${2:Mode.IDLE} -> $0\n\tdefault -> {}\n}",
            ),
            (
                "case ...",
                "Match a switch value",
                "case ${1:Mode.IDLE} -> $0",
            ),
            ("default ->", "Fallback switch arm", "default -> $0"),
        ] {
            items.push(snippet_item(
                label,
                CompletionItemKind::KEYWORD,
                detail,
                insert_text,
            ));
        }
        items
    };

    for function in analysis
        .functions
        .iter()
        .filter(|function| !function.name.starts_with("__mcfc_"))
    {
        items.push(CompletionItem {
            label: function.name.clone(),
            kind: Some(CompletionItemKind::FUNCTION),
            detail: Some(function.signature()),
            insert_text: Some(format!("{}($0)", function.name)),
            insert_text_format: Some(tower_lsp::lsp_types::InsertTextFormat::SNIPPET),
            ..CompletionItem::default()
        });
    }

    let syntactic_locals = syntactic_locals_at_offset(source, offset);
    let visible_syntactic_names: HashSet<_> = syntactic_locals
        .iter()
        .map(|local| local.name.as_str())
        .collect();
    let mut seen_locals = HashSet::new();
    if let Some(function_name) = containing_function {
        for local in analysis.locals.iter().filter(|local| {
            local.function == function_name
                && (visible_syntactic_names.is_empty()
                    || visible_syntactic_names.contains(local.name.as_str()))
        }) {
            seen_locals.insert(local.name.clone());
            items.push(CompletionItem {
                label: local.name.clone(),
                kind: Some(CompletionItemKind::VARIABLE),
                detail: Some(local.ty.as_str()),
                ..CompletionItem::default()
            });
        }
    }

    for local in syntactic_locals {
        if seen_locals.insert(local.name.clone()) {
            items.push(CompletionItem {
                label: local.name,
                kind: Some(CompletionItemKind::VARIABLE),
                detail: local.ty.map(|ty| ty.as_str()),
                ..CompletionItem::default()
            });
        }
    }

    items
}

fn is_declaration_completion_position(source: &str, offset: usize) -> bool {
    let line_start = source[..offset.min(source.len())]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    let before_cursor = &source[line_start..offset.min(source.len())];
    // Declarations are only legal at top level. A partially typed keyword is
    // still a declaration position, but indented code is always an expression.
    !before_cursor.starts_with(char::is_whitespace)
        && before_cursor.split_whitespace().next().is_none_or(|word| {
            word.starts_with('@') || matches!(word, "record" | "enum" | "public" | "import")
        })
}

fn expression_completion_items() -> Vec<CompletionItem> {
    static_completion_items()
        .into_iter()
        .filter(|item| {
            matches!(item.kind, Some(CompletionItemKind::FUNCTION)) && !item.label.starts_with('@')
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StringLiteralContext {
    quote_start: usize,
    content_range: TextRange,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CallContext {
    name: String,
    is_method: bool,
    arg_index: usize,
}

fn minecraft_id_completion_items(source: &str, offset: usize) -> Option<Vec<CompletionItem>> {
    let string_context = string_literal_context_at_offset(source, offset)?;

    if let Some(replace_range) = selector_entity_id_range(source, &string_context, offset) {
        return Some(minecraft_id_completion_items_for_range(
            source,
            MinecraftIdCategory::Entity,
            replace_range,
            offset,
        ));
    }

    let category = minecraft_id_category_at_offset(source, string_context.quote_start)?;
    Some(minecraft_id_completion_items_for_range(
        source,
        category,
        string_context.content_range,
        offset,
    ))
}

fn minecraft_id_completion_items_for_range(
    source: &str,
    category: MinecraftIdCategory,
    replace_range: TextRange,
    offset: usize,
) -> Vec<CompletionItem> {
    let prefix_end = offset.min(replace_range.end);
    let prefix = source[replace_range.start..prefix_end].to_ascii_lowercase();
    let filter_text_uses_suffix = !prefix.is_empty() && !prefix.contains(':');
    let edit_range = exact_range_from_offsets(source, replace_range.start, replace_range.end);
    let detail = minecraft_id_detail(category).to_string();

    ids_for_category(category)
        .iter()
        .copied()
        .filter(|id| minecraft_id_matches_prefix(id, &prefix))
        .map(|id| {
            let filter_text = if filter_text_uses_suffix {
                id.trim_start_matches("minecraft:").to_string()
            } else {
                id.to_string()
            };
            CompletionItem {
                label: id.to_string(),
                kind: Some(CompletionItemKind::CONSTANT),
                detail: Some(detail.clone()),
                filter_text: Some(filter_text),
                text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                    range: edit_range,
                    new_text: id.to_string(),
                })),
                ..CompletionItem::default()
            }
        })
        .collect()
}

fn selector_entity_id_range(
    source: &str,
    string_context: &StringLiteralContext,
    offset: usize,
) -> Option<TextRange> {
    let call = call_context_before_offset(source, string_context.quote_start)?;
    if call.name != "selector" || call.is_method || call.arg_index != 0 {
        return None;
    }

    let content_start = string_context.content_range.start;
    let content_end = string_context.content_range.end;
    let cursor = offset.min(content_end);
    let mut value_start = cursor;

    while value_start > content_start {
        let ch = previous_char(source, value_start)?;
        if !is_resource_location_char(ch) {
            break;
        }
        value_start -= ch.len_utf8();
    }

    let mut marker_start = value_start;
    if marker_start > content_start && previous_char(source, marker_start) == Some('!') {
        marker_start -= 1;
    }

    let type_marker = "type=";
    if marker_start < content_start + type_marker.len()
        || &source[marker_start - type_marker.len()..marker_start] != type_marker
    {
        return None;
    }

    let mut value_end = value_start;
    while value_end < content_end {
        let ch = source[value_end..].chars().next()?;
        if !is_resource_location_char(ch) {
            break;
        }
        value_end += ch.len_utf8();
    }

    if value_end < content_end {
        let ch = source[value_end..].chars().next()?;
        if !matches!(ch, ',' | ']' | ' ' | '\t' | '\n' | '\r') {
            return None;
        }
    }

    Some(TextRange::new(value_start, value_end))
}

fn is_resource_location_char(ch: char) -> bool {
    ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '_' | '-' | '.' | ':' | '/')
}

fn minecraft_id_matches_prefix(id: &str, prefix: &str) -> bool {
    prefix.is_empty()
        || id.starts_with(prefix)
        || id.trim_start_matches("minecraft:").starts_with(prefix)
}

fn minecraft_id_detail(category: MinecraftIdCategory) -> &'static str {
    match category {
        MinecraftIdCategory::Block => "Minecraft block id",
        MinecraftIdCategory::Item => "Minecraft item id",
        MinecraftIdCategory::Entity => "Minecraft entity id",
        MinecraftIdCategory::LootTable => "Minecraft loot table id",
        MinecraftIdCategory::Particle => "Minecraft particle id",
        MinecraftIdCategory::SoundEvent => "Minecraft sound id",
        MinecraftIdCategory::Effect => "Minecraft effect id",
    }
}

fn minecraft_id_category_at_offset(
    source: &str,
    quote_start: usize,
) -> Option<MinecraftIdCategory> {
    call_context_before_offset(source, quote_start)
        .and_then(|call| minecraft_id_category_for_call(&call))
        .or_else(|| minecraft_id_category_from_assignment(source, quote_start))
}

fn minecraft_id_category_for_call(call: &CallContext) -> Option<MinecraftIdCategory> {
    let name = if call.is_method {
        ENTITY_METHOD_NAMES
            .iter()
            .find(|(java, _)| *java == call.name)
            .map_or_else(
                || internal_method_name(&call.name, 1),
                |(_, internal)| *internal,
            )
    } else {
        internal_function_name(&call.name)
    };
    match (name, call.is_method, call.arg_index) {
        ("EntityData", false, 0) | ("summon", _, 0) => Some(MinecraftIdCategory::Entity),
        ("ItemStack", false, 0)
        | ("give", true, 0)
        | ("give", false, 1)
        | ("clear", true, 0)
        | ("clear", false, 1) => Some(MinecraftIdCategory::Item),
        ("BlockData", false, 0)
        | ("setblock", true, 0)
        | ("setblock", false, 1)
        | ("is", true, 0)
        | ("fill", true, 1)
        | ("fill", false, 2)
        | ("debug_marker", true, 1)
        | ("debug_marker", false, 2) => Some(MinecraftIdCategory::Block),
        ("loot_give", true, 0)
        | ("loot_give", false, 1)
        | ("loot_insert", true, 0)
        | ("loot_insert", false, 1)
        | ("loot_spawn", true, 0)
        | ("loot_spawn", false, 1) => Some(MinecraftIdCategory::LootTable),
        ("particle", true, 0) | ("particle", false, 0) => Some(MinecraftIdCategory::Particle),
        ("playsound", true, 0)
        | ("playsound", false, 0)
        | ("stopsound", true, 1)
        | ("stopsound", false, 2) => Some(MinecraftIdCategory::SoundEvent),
        ("effect", true, 0) => Some(MinecraftIdCategory::Effect),
        _ => None,
    }
}

fn minecraft_id_category_from_assignment(
    source: &str,
    quote_start: usize,
) -> Option<MinecraftIdCategory> {
    let line_start = source[..quote_start]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    let line = &source[line_start..quote_start];
    let equals = top_level_assignment_index(line)?;
    let lhs = line[..equals].trim_end();

    is_equipment_item_assignment(lhs).then_some(MinecraftIdCategory::Item)
}

fn is_equipment_item_assignment(lhs: &str) -> bool {
    lhs.ends_with(".item")
        && [".mainhand", ".offhand", ".head", ".chest", ".legs", ".feet"]
            .iter()
            .any(|segment| lhs.contains(segment))
}

fn top_level_assignment_index(line: &str) -> Option<usize> {
    let mut depth = 0usize;
    let mut in_string = None;
    let mut escaped = false;
    let mut last_equals = None;

    for (index, ch) in line.char_indices() {
        if let Some(delimiter) = in_string {
            if escaped {
                escaped = false;
                continue;
            }
            match ch {
                '\\' => escaped = true,
                value if value == delimiter => in_string = None,
                _ => {}
            }
            continue;
        }

        match ch {
            '"' | '\'' => in_string = Some(ch),
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            '=' if depth == 0 => {
                let previous = line[..index].chars().next_back();
                let next = line[index + 1..].chars().next();
                if previous != Some('=')
                    && previous != Some('!')
                    && previous != Some('<')
                    && previous != Some('>')
                    && next != Some('=')
                    && next != Some('>')
                {
                    last_equals = Some(index);
                }
            }
            _ => {}
        }
    }

    last_equals
}

fn call_context_before_offset(source: &str, target_offset: usize) -> Option<CallContext> {
    let open_paren = innermost_open_paren_before(source, target_offset)?;
    let (name, is_method) = call_name_before_paren(source, open_paren)?;
    Some(CallContext {
        name,
        is_method,
        arg_index: call_arg_index(source, open_paren, target_offset),
    })
}

fn innermost_open_paren_before(source: &str, target_offset: usize) -> Option<usize> {
    let mut stack: Vec<(char, usize)> = Vec::new();
    let mut in_string = None;
    let mut escaped = false;

    for (index, ch) in source.char_indices() {
        if index >= target_offset {
            break;
        }
        if let Some(delimiter) = in_string {
            if escaped {
                escaped = false;
                continue;
            }
            match ch {
                '\\' => escaped = true,
                value if value == delimiter => in_string = None,
                _ => {}
            }
            continue;
        }

        match ch {
            '"' | '\'' => in_string = Some(ch),
            '(' | '[' | '{' => stack.push((ch, index)),
            ')' => pop_matching_delimiter(&mut stack, '('),
            ']' => pop_matching_delimiter(&mut stack, '['),
            '}' => pop_matching_delimiter(&mut stack, '{'),
            _ => {}
        }
    }

    stack
        .iter()
        .rev()
        .find(|(delimiter, _)| *delimiter == '(')
        .map(|(_, index)| *index)
}

fn pop_matching_delimiter(stack: &mut Vec<(char, usize)>, expected: char) {
    if let Some(position) = stack
        .iter()
        .rposition(|(delimiter, _)| *delimiter == expected)
    {
        stack.remove(position);
    }
}

fn call_name_before_paren(source: &str, open_paren: usize) -> Option<(String, bool)> {
    let mut end = open_paren;
    while end > 0 {
        let ch = previous_char(source, end)?;
        if !ch.is_whitespace() {
            break;
        }
        end -= ch.len_utf8();
    }

    let mut start = end;
    while start > 0 {
        let ch = previous_char(source, start)?;
        if !is_member_word_char(ch) {
            break;
        }
        start -= ch.len_utf8();
    }
    if start == end {
        return None;
    }

    // `Selector.of(` and `Block.of(` are the builtin `selector(` and `block(` calls.
    if &source[start..end] == "of" {
        for (ty, builtin) in [("Selector.", "selector"), ("Block.", "block")] {
            if source[..start].ends_with(ty) {
                return Some((builtin.to_string(), false));
            }
        }
    }

    let mut cursor = start;
    while cursor > 0 {
        let ch = previous_char(source, cursor)?;
        if !ch.is_whitespace() {
            return Some((source[start..end].to_string(), ch == '.'));
        }
        cursor -= ch.len_utf8();
    }

    Some((source[start..end].to_string(), false))
}

fn call_arg_index(source: &str, open_paren: usize, target_offset: usize) -> usize {
    let mut arg_index = 0usize;
    let mut depth = 0usize;
    let mut in_string = None;
    let mut escaped = false;

    for ch in source[open_paren + 1..target_offset.min(source.len())].chars() {
        if let Some(delimiter) = in_string {
            if escaped {
                escaped = false;
                continue;
            }
            match ch {
                '\\' => escaped = true,
                value if value == delimiter => in_string = None,
                _ => {}
            }
            continue;
        }

        match ch {
            '"' | '\'' => in_string = Some(ch),
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => arg_index += 1,
            _ => {}
        }
    }

    arg_index
}

fn string_literal_context_at_offset(source: &str, offset: usize) -> Option<StringLiteralContext> {
    let mut in_string = None;
    let mut escaped = false;
    let offset = offset.min(source.len());

    for (index, ch) in source.char_indices() {
        if index >= offset {
            break;
        }
        if let Some((delimiter, _)) = in_string {
            if escaped {
                escaped = false;
                continue;
            }
            match ch {
                '\\' => escaped = true,
                value if value == delimiter => in_string = None,
                _ => {}
            }
            continue;
        }

        if ch == '"' || ch == '\'' {
            in_string = Some((ch, index));
            escaped = false;
        }
    }

    let (delimiter, quote_start) = in_string?;
    let content_start = quote_start + delimiter.len_utf8();
    let content_end = string_literal_end(source, quote_start, delimiter).unwrap_or(source.len());
    Some(StringLiteralContext {
        quote_start,
        content_range: TextRange::new(content_start, content_end),
    })
}

fn string_literal_end(source: &str, quote_start: usize, delimiter: char) -> Option<usize> {
    let start = quote_start + delimiter.len_utf8();
    let mut escaped = false;

    for (relative_index, ch) in source[start..].char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            value if value == delimiter => return Some(start + relative_index),
            _ => {}
        }
    }

    None
}

fn exact_range_from_offsets(source: &str, start: usize, end: usize) -> Range {
    Range {
        start: offset_to_position(source, start),
        end: offset_to_position(source, end),
    }
}

fn static_completion_items() -> Vec<CompletionItem> {
    let mut items = Vec::new();
    for keyword in [
        "var", "return", "if", "else", "switch", "case", "default", "while", "for", "break",
        "continue", "async", "assert", "new", "mc", "mcf", "true", "false", "record", "enum",
        "import", "public", "final", "static", "do", "yield",
    ] {
        items.push(CompletionItem {
            label: keyword.to_string(),
            kind: Some(CompletionItemKind::KEYWORD),
            ..CompletionItem::default()
        });
    }

    for (label, insert_text) in [
        ("int", "int"),
        ("float", "float"),
        ("boolean", "boolean"),
        ("Integer", "Integer"),
        ("Float", "Float"),
        ("Boolean", "Boolean"),
        ("String", "String"),
        ("Math", "Math"),
        ("List<>", "List<${1:Integer}>"),
        ("Map<>", "Map<String, ${1:Integer}>"),
        ("Optional<>", "Optional<${1:Integer}>"),
        ("Selector", "Selector"),
        ("Entity", "Entity"),
        ("Player", "Player"),
        ("Block", "Block"),
        ("EntityData", "EntityData"),
        ("BlockData", "BlockData"),
        ("ItemStack", "ItemStack"),
        ("Component", "Component"),
        ("ItemSlot", "ItemSlot"),
        ("BossBar", "BossBar"),
        ("Nbt", "Nbt"),
        ("void", "void"),
    ] {
        items.push(snippet_item(
            label,
            CompletionItemKind::TYPE_PARAMETER,
            "MCFC type",
            insert_text,
        ));
    }

    for (label, detail, insert_text) in [
        (
            "@Command",
            "Register a trigger command and optional agent root command",
            "@Command(\"${1:status}\")\nvoid ${2:status}() {\n\t$0\n}",
        ),
        (
            "@Every",
            "Repeat a task every positive number of ticks",
            "@Every(ticks = ${1:20})\nvoid ${2:tick}() {\n\t$0\n}",
        ),
        (
            "@Menu",
            "A command that is also a button in the pause-screen data pack menu",
            "@Menu(\"${1:Settings}\")
void ${2:settings}(Player player) {
	$0
}",
        ),
        (
            "@Test",
            "A test run by /function <namespace>:test",
            "@Test
void ${1:test}() {
	assert ${2:true};
}",
        ),
        (
            "record ...",
            "Define a record with typed fields",
            "record ${1:Name}(${2:int} ${3:field}) {}",
        ),
        (
            "enum ...",
            "Define named enum constants",
            "enum ${1:Mode} { ${2:IDLE}, ${3:RUNNING} }",
        ),
        (
            "switch ...",
            "Dispatch on an enum, integer or string value",
            "switch (${1:value}) {\n\tcase ${2:Mode.IDLE} -> $0\n\tdefault -> {}\n}",
        ),
        (
            "@PlayerState",
            "Declare player scoreboard state with a display name",
            "@PlayerState(\"${3:Money}\")\n${2:int} ${1:money};",
        ),
        (
            "@EntityState",
            "Declare typed persistent entity state",
            "@EntityState\n${2:String} ${1:title};",
        ),
        (
            "Selector.of",
            "Selector.of(value: String) -> Selector",
            "Selector.of(${1:\"@e\"})",
        ),
        ("sleep", "sleep(seconds: int) -> void", "sleep(${1:1})"),
        (
            "sleepTicks",
            "sleepTicks(ticks: int) -> void",
            "sleepTicks(${1:20})",
        ),
        ("random", "random() -> int", "random()"),
        (
            "randomWeighted",
            "randomWeighted(weights: List<Integer>) -> int",
            "randomWeighted(List.of(${1:3}, ${2:1}))",
        ),
        (
            "randomBinomial",
            "randomBinomial(n: int, p: float) -> int",
            "randomBinomial(${1:10}, ${2:0.5})",
        ),
        (
            "gamerule",
            "gamerule(name: String) -> int",
            "gamerule(${1:\"max_entity_cramming\"})",
        ),
        ("random(max)", "random(max: int) -> int", "random(${1:max})"),
        (
            "random(min, max)",
            "random(min: int, max: int) -> int",
            "random(${1:min}, ${2:max})",
        ),
        (
            "hasData",
            "hasData(value: storage_path) -> boolean",
            "hasData(${1:value})",
        ),
        (
            "Block.of",
            "Block.of(position: String) -> Block",
            "Block.of(${1:\"~ ~ ~\"})",
        ),
        (
            "at",
            "at(anchor: Entity, value: Selector|Entity|Block)",
            "at(${1:anchor}, ${2:value})",
        ),
        (
            "at(...) {}",
            "Run commands at an entity/block",
            "at(${1:anchor}) {\n\t$0\n}",
        ),
        (
            "as",
            "as(anchor: Selector|Entity, value: Selector|Entity|Block)",
            "as(${1:anchor}, ${2:value})",
        ),
        (
            "as(...) {}",
            "Run commands as an entity",
            "as(${1:anchor}) {\n\t$0\n}",
        ),
        (
            "summon",
            "summon(entityId: String|EntityData) -> Entity",
            "summon(${1:\"minecraft:pig\"})",
        ),
        (
            "async {}",
            "Spawn a non-blocking async block",
            "async {\n\t$0\n}",
        ),
        (
            "debug",
            "debug(message: String) -> void",
            "debug(${1:\"reached checkpoint\"})",
        ),
    ] {
        items.push(snippet_item(
            label,
            if label.starts_with('@') || label.ends_with("...") {
                CompletionItemKind::SNIPPET
            } else {
                CompletionItemKind::FUNCTION
            },
            detail,
            insert_text,
        ));
    }

    for event in VANILLA_EVENTS.iter().chain(AGENT_EVENTS) {
        let ty = event_type_name(event);
        items.push(CompletionItem {
            label: format!("@EventHandler {ty}"),
            kind: Some(CompletionItemKind::EVENT),
            detail: Some(if VANILLA_EVENTS.contains(event) {
                "Vanilla event handler".to_string()
            } else {
                "JVM-agent event handler".to_string()
            }),
            insert_text: Some(format!(
                "@EventHandler\nvoid ${{1:{}}}({ty} event) {{\n\t$0\n}}",
                handler_name(event)
            )),
            insert_text_format: Some(InsertTextFormat::SNIPPET),
            ..CompletionItem::default()
        });
    }
    for event in VANILLA_EVENTS.iter().chain(AGENT_EVENTS) {
        items.push(snippet_item(
            &event_type_name(event),
            CompletionItemKind::TYPE_PARAMETER,
            "MCFC event type",
            &event_type_name(event),
        ));
    }
    items
}

/// `player_join` -> `onPlayerJoin`.
fn handler_name(event: &str) -> String {
    let mut name = String::from("on");
    for word in event.split('_') {
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            name.push(first.to_ascii_uppercase());
            name.push_str(chars.as_str());
        }
    }
    name
}

fn snippet_item(
    label: &str,
    kind: CompletionItemKind,
    detail: &str,
    insert_text: &str,
) -> CompletionItem {
    CompletionItem {
        label: label.to_string(),
        kind: Some(kind),
        detail: Some(detail.to_string()),
        insert_text: Some(insert_text.to_string()),
        insert_text_format: Some(tower_lsp::lsp_types::InsertTextFormat::SNIPPET),
        ..CompletionItem::default()
    }
}

fn member_completion_items(
    source: &str,
    analysis: &AnalysisResult,
    offset: usize,
    chain: &[String],
) -> Vec<CompletionItem> {
    if let [name] = chain
        && let Some(items) = java_static_member_items(name)
    {
        return items;
    }
    if let [name] = chain
        && let Some(variants) = analysis
            .typed_program
            .as_ref()
            .and_then(|program| program.struct_defs.get(name))
            .and_then(|def| def.enum_variants.as_ref())
    {
        let mut items: Vec<_> = variants
            .iter()
            .map(|variant| CompletionItem {
                label: variant.clone(),
                kind: Some(CompletionItemKind::ENUM_MEMBER),
                detail: Some(format!("{name}.{variant}")),
                ..CompletionItem::default()
            })
            .collect();
        items.push(snippet_item(
            "values",
            CompletionItemKind::METHOD,
            &format!("{name}.values() -> List<{name}>"),
            "values()",
        ));
        return items;
    }
    if let Some(items) = agent_event_member_completion_items(source, analysis, offset, chain) {
        return items;
    }
    if let Some(items) = text_member_completion_items(source, analysis, offset, chain) {
        return items;
    }
    if let Some(items) = nbt_member_completion_items(source, analysis, offset, chain) {
        return items;
    }

    let receiver = resolve_receiver_kind(source, analysis, offset, chain)
        .or_else(|| inline_member_chain_receiver(source, analysis, offset, chain));
    if receiver.is_none() && chain.len() > 1 {
        Vec::new()
    } else {
        completion_items_for_receiver(receiver, analysis)
    }
}

fn java_static_member_items(name: &str) -> Option<Vec<CompletionItem>> {
    let methods: &[(&str, &str, &str)] = match name {
        "Math" => &[
            ("abs", "Math.abs(x) -> number", "abs(${1:x})"),
            ("min", "Math.min(a, b) -> number", "min(${1:a}, ${2:b})"),
            ("max", "Math.max(a, b) -> number", "max(${1:a}, ${2:b})"),
            (
                "clamp",
                "Math.clamp(x, low, high) -> number",
                "clamp(${1:x}, ${2:low}, ${3:high})",
            ),
            ("sqrt", "Math.sqrt(x) -> float", "sqrt(${1:x})"),
            (
                "pow",
                "Math.pow(x, exponent) -> float",
                "pow(${1:x}, ${2:exponent})",
            ),
            (
                "hypot",
                "Math.hypot(x, y) -> float",
                "hypot(${1:x}, ${2:y})",
            ),
            ("sin", "Math.sin(x) -> float", "sin(${1:x})"),
            ("cos", "Math.cos(x) -> float", "cos(${1:x})"),
            ("tan", "Math.tan(x) -> float", "tan(${1:x})"),
            ("floor", "Math.floor(x) -> float", "floor(${1:x})"),
            ("ceil", "Math.ceil(x) -> float", "ceil(${1:x})"),
            ("round", "Math.round(x) -> int", "round(${1:x})"),
            ("trunc", "Math.trunc(x) -> float", "trunc(${1:x})"),
            ("signum", "Math.signum(x) -> float", "signum(${1:x})"),
        ],
        "Integer" => &[
            (
                "parseInt",
                "Integer.parseInt(s: String) -> int",
                "parseInt(${1:s})",
            ),
            (
                "toString",
                "Integer.toString(x: int) -> String",
                "toString(${1:x})",
            ),
        ],
        "Float" => &[(
            "toString",
            "Float.toString(x: float) -> String",
            "toString(${1:x})",
        )],
        "String" => &[("valueOf", "String.valueOf(x) -> String", "valueOf(${1:x})")],
        _ => return None,
    };
    Some(
        methods
            .iter()
            .map(|(label, detail, insert_text)| {
                snippet_item(label, CompletionItemKind::METHOD, detail, insert_text)
            })
            .collect(),
    )
}

fn completion_items_for_receiver(
    receiver: Option<CompletionReceiver>,
    analysis: &AnalysisResult,
) -> Vec<CompletionItem> {
    match receiver {
        Some(CompletionReceiver::Array) => array_method_items(),
        Some(CompletionReceiver::Int) => vec![snippet_item(
            "toString",
            CompletionItemKind::METHOD,
            "int.toString() -> String",
            "toString()",
        )],
        Some(CompletionReceiver::Float) => float_method_items(),
        Some(CompletionReceiver::String) => string_method_items(),
        Some(CompletionReceiver::Dict) => dict_method_items(),
        Some(CompletionReceiver::Optional) => optional_method_items(),
        Some(CompletionReceiver::Selector) => selector_method_items(),
        Some(CompletionReceiver::GenericEntityRef) => generic_entity_root_items(),
        Some(CompletionReceiver::PlayerEntityRef) => player_entity_root_items(),
        Some(CompletionReceiver::PlayerInput) => [
            "Forward", "Backward", "Left", "Right", "Jump", "Sneak", "Sprint",
        ]
        .into_iter()
        .map(|direction| {
            let method = format!("is{direction}");
            snippet_item(
                &method,
                CompletionItemKind::METHOD,
                &format!("player.getCurrentInput().{method}() -> boolean"),
                &format!("{method}()"),
            )
        })
        .collect(),
        Some(CompletionReceiver::EntityDef) => entity_def_items(),
        Some(CompletionReceiver::ItemDef) => item_def_items(),
        Some(CompletionReceiver::TextDef) => text_def_items(),
        Some(CompletionReceiver::BlockDef) => block_def_items(),
        Some(CompletionReceiver::ItemSlot) => item_slot_items(),
        Some(CompletionReceiver::Bossbar) => bossbar_root_items(),
        Some(CompletionReceiver::EquipmentSlot) => equipment_slot_items(),
        Some(CompletionReceiver::Enum(_)) => vec![
            snippet_item(
                "name",
                CompletionItemKind::METHOD,
                "enum.name() -> String",
                "name()",
            ),
            snippet_item(
                "ordinal",
                CompletionItemKind::METHOD,
                "enum.ordinal() -> int",
                "ordinal()",
            ),
        ],
        Some(CompletionReceiver::Struct(name)) => {
            if analysis
                .typed_program
                .as_ref()
                .and_then(|program| program.struct_defs.get(&name))
                .and_then(|def| def.enum_variants.as_ref())
                .is_some()
            {
                vec![
                    snippet_item(
                        "name",
                        CompletionItemKind::METHOD,
                        "enum.name() -> String",
                        "name()",
                    ),
                    snippet_item(
                        "ordinal",
                        CompletionItemKind::METHOD,
                        "enum.ordinal() -> int",
                        "ordinal()",
                    ),
                ]
            } else {
                struct_field_items(analysis, &name)
            }
        }
        Some(
            CompletionReceiver::PlayerDynamicNamespace
            | CompletionReceiver::EntityTeam
            | CompletionReceiver::Nbt,
        ) => Vec::new(),
        Some(CompletionReceiver::BlockRef) => block_ref_items(),
        // Guessing every member API for an unresolved receiver makes the
        // completion list actively misleading while a document is incomplete.
        None => Vec::new(),
    }
}

fn text_member_completion_items(
    source: &str,
    analysis: &AnalysisResult,
    offset: usize,
    chain: &[String],
) -> Option<Vec<CompletionItem>> {
    let Some((base_name, rest)) = chain.split_first() else {
        return None;
    };

    let segments = if local_type_at_offset(source, analysis, offset, base_name)
        .is_some_and(|(ty, _)| ty == Type::TextDef)
    {
        rest
    } else if inline_member_chain_base_type(source, offset, chain)
        .is_some_and(|(ty, _)| ty == Type::TextDef)
    {
        chain
    } else {
        return None;
    };

    Some(text_completion_items_for_segments(segments))
}

fn text_completion_items_for_segments(segments: &[String]) -> Vec<CompletionItem> {
    let mut context = TextCompletionContext::Root;
    for segment in segments {
        context = match (context, segment.as_str()) {
            (TextCompletionContext::Root, "hover_event") => TextCompletionContext::HoverEvent,
            (TextCompletionContext::Root, "click_event") => TextCompletionContext::ClickEvent,
            (TextCompletionContext::Root, "score") => TextCompletionContext::Score,
            (TextCompletionContext::Root, "extra" | "with" | "separator") => {
                TextCompletionContext::Root
            }
            (
                TextCompletionContext::Root,
                "text" | "translate" | "keybind" | "selector" | "color" | "font" | "insertion"
                | "nbt" | "block" | "entity" | "storage",
            ) => TextCompletionContext::Scalar,
            (
                TextCompletionContext::Root,
                "bold" | "italic" | "underlined" | "strikethrough" | "obfuscated" | "interpret",
            ) => TextCompletionContext::Scalar,
            (TextCompletionContext::HoverEvent, "contents" | "value") => {
                TextCompletionContext::Root
            }
            (TextCompletionContext::HoverEvent, "action") => TextCompletionContext::Scalar,
            (TextCompletionContext::ClickEvent, "action" | "value") => {
                TextCompletionContext::Scalar
            }
            (TextCompletionContext::Score, "name" | "objective" | "value") => {
                TextCompletionContext::Scalar
            }
            (TextCompletionContext::Scalar, _) => return Vec::new(),
            (TextCompletionContext::Root, _) => TextCompletionContext::Root,
            _ => return Vec::new(),
        };
    }

    match context {
        TextCompletionContext::Root => text_def_items(),
        TextCompletionContext::HoverEvent => text_hover_event_items(),
        TextCompletionContext::ClickEvent => text_click_event_items(),
        TextCompletionContext::Score => text_score_items(),
        TextCompletionContext::Scalar => Vec::new(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextCompletionContext {
    Root,
    HoverEvent,
    ClickEvent,
    Score,
    Scalar,
}

fn nbt_member_completion_items(
    source: &str,
    analysis: &AnalysisResult,
    offset: usize,
    chain: &[String],
) -> Option<Vec<CompletionItem>> {
    let nbt_index = chain.iter().position(|segment| segment == "nbt")?;
    let origin = if nbt_index == 0 {
        inline_nbt_completion_origin(source, offset, chain)?
    } else {
        local_nbt_origin_at_offset(source, analysis, offset, &chain[0])?
    };

    let mut node = minecraft_nbt_schema::root_node(origin.category, origin.id.as_deref())?;
    for segment in &chain[nbt_index + 1..] {
        node = match minecraft_nbt_schema::child_node(node, segment) {
            Some(child) => child,
            None => return Some(Vec::new()),
        };
    }

    Some(nbt_schema_field_completion_items(source, offset, node))
}

fn inline_nbt_completion_origin(
    source: &str,
    offset: usize,
    chain: &[String],
) -> Option<NbtCompletionOrigin> {
    let base_expr = inline_member_chain_base_expr(source, offset, chain)?;
    infer_nbt_completion_origin(base_expr).or_else(|| {
        let (base_ty, _) = inline_member_chain_base_type(source, offset, chain)?;
        nbt_origin_for_type(&base_ty)
    })
}

fn nbt_schema_field_completion_items(
    source: &str,
    offset: usize,
    node: &NbtSchemaNode,
) -> Vec<CompletionItem> {
    let (replace_start, replace_end) = member_identifier_offsets(source, offset);
    let prefix_end = offset.min(replace_end);
    let prefix = source[replace_start..prefix_end].to_ascii_lowercase();
    let edit_range = exact_range_from_offsets(source, replace_start, replace_end);

    node.fields
        .iter()
        .filter(|field| prefix.is_empty() || field.name.to_ascii_lowercase().starts_with(&prefix))
        .map(|field| CompletionItem {
            label: field.name.to_string(),
            kind: Some(CompletionItemKind::FIELD),
            detail: Some(field.detail.to_string()),
            documentation: (!field.documentation.is_empty())
                .then(|| Documentation::String(field.documentation.to_string())),
            filter_text: Some(field.name.to_string()),
            text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                range: edit_range,
                new_text: field.name.to_string(),
            })),
            ..CompletionItem::default()
        })
        .collect()
}

#[allow(dead_code)]
fn broad_member_items() -> Vec<CompletionItem> {
    [
        array_method_items(),
        dict_method_items(),
        player_entity_root_items(),
        entity_def_items(),
        item_def_items(),
        text_def_items(),
        block_def_items(),
        item_slot_items(),
        block_ref_items(),
        bossbar_root_items(),
    ]
    .concat()
}

fn array_method_items() -> Vec<CompletionItem> {
    [
        ("size", "List<T>.size() -> int", "size()"),
        (
            "get",
            "List<T>.get(index: int) -> Optional<T>",
            "get(${1:index})",
        ),
        ("add", "List<T>.add(value: T) -> void", "add(${1:value})"),
        ("removeLast", "List<T>.removeLast() -> T", "removeLast()"),
        (
            "remove",
            "List<T>.remove(index: int) -> T",
            "remove(${1:index})",
        ),
        (
            "add",
            "List<T>.add(index: int, value: T) -> void",
            "add(${1:index}, ${2:value})",
        ),
        ("clear", "List<T>.clear() -> void", "clear()"),
        (
            "set",
            "List<T>.set(index: int, value: T) -> void",
            "set(${1:index}, ${2:value})",
        ),
        ("isEmpty", "List<T>.isEmpty() -> boolean", "isEmpty()"),
        ("getFirst", "List<T>.getFirst() -> T", "getFirst()"),
        ("getLast", "List<T>.getLast() -> T", "getLast()"),
        (
            "contains",
            "List<T>.contains(value: T) -> boolean",
            "contains(${1:value})",
        ),
        (
            "indexOf",
            "List<T>.indexOf(value: T) -> int",
            "indexOf(${1:value})",
        ),
        ("reverse", "List<T>.reverse() -> void", "reverse()"),
        ("sort", "List<Integer>.sort() -> void", "sort()"),
    ]
    .into_iter()
    .map(|(label, detail, insert_text)| {
        snippet_item(label, CompletionItemKind::METHOD, detail, insert_text)
    })
    .collect()
}

fn selector_method_items() -> Vec<CompletionItem> {
    let mut items = vec![
        snippet_item(
            "getFirst",
            CompletionItemKind::METHOD,
            "Selector.getFirst() -> Entity",
            "getFirst()",
        ),
        snippet_item(
            "findFirst",
            CompletionItemKind::METHOD,
            "Selector.findFirst() -> Optional<Entity>",
            "findFirst()",
        ),
    ];
    items.extend(generic_entity_root_items().into_iter().filter(|item| {
        matches!(
            item.label.as_str(),
            "sendMessage"
                | "sendTitle"
                | "sendActionBar"
                | "playSound"
                | "stopSound"
                | "teleport"
                | "damage"
                | "give"
                | "clear"
                | "lootGive"
        )
    }));
    items
}

fn string_method_items() -> Vec<CompletionItem> {
    [
        ("length", "String.length() -> int", "length()"),
        ("isEmpty", "String.isEmpty() -> boolean", "isEmpty()"),
        (
            "contains",
            "String.contains(part: String) -> boolean",
            "contains(${1:part})",
        ),
        (
            "equals",
            "String.equals(other: String) -> boolean",
            "equals(${1:other})",
        ),
        (
            "startsWith",
            "String.startsWith(prefix: String) -> boolean",
            "startsWith(${1:prefix})",
        ),
        (
            "endsWith",
            "String.endsWith(suffix: String) -> boolean",
            "endsWith(${1:suffix})",
        ),
        (
            "indexOf",
            "String.indexOf(part: String) -> int",
            "indexOf(${1:part})",
        ),
        (
            "charAt",
            "String.charAt(index: int) -> String",
            "charAt(${1:index})",
        ),
        (
            "substring",
            "String.substring(start: int, end: int) -> String",
            "substring(${1:start}, ${2:end})",
        ),
        (
            "replace",
            "String.replace(target: String, replacement: String) -> String",
            "replace(${1:target}, ${2:replacement})",
        ),
        (
            "split",
            "String.split(separator: String) -> List<String>",
            "split(${1:separator})",
        ),
        (
            "toUpperCase",
            "String.toUpperCase() -> String",
            "toUpperCase()",
        ),
        (
            "toLowerCase",
            "String.toLowerCase() -> String",
            "toLowerCase()",
        ),
        ("toString", "String.toString() -> String", "toString()"),
    ]
    .into_iter()
    .map(|(label, detail, insert_text)| {
        snippet_item(label, CompletionItemKind::METHOD, detail, insert_text)
    })
    .collect()
}

fn float_method_items() -> Vec<CompletionItem> {
    [("toString", "float.toString() -> String", "toString()")]
        .into_iter()
        .map(|(label, detail, insert_text)| {
            snippet_item(label, CompletionItemKind::METHOD, detail, insert_text)
        })
        .collect()
}

fn dict_method_items() -> Vec<CompletionItem> {
    [
        (
            "containsKey",
            "Map<String, T>.containsKey(key: String) -> boolean",
            "containsKey(${1:key})",
        ),
        (
            "get",
            "Map<String, T>.get(key: String) -> Optional<T>",
            "get(${1:key})",
        ),
        (
            "keySet",
            "Map<String, T>.keySet() -> List<String>",
            "keySet()",
        ),
        ("size", "Map<String, T>.size() -> int", "size()"),
        (
            "isEmpty",
            "Map<String, T>.isEmpty() -> boolean",
            "isEmpty()",
        ),
        (
            "put",
            "Map<String, T>.put(key: String, value: T) -> void",
            "put(${1:key}, ${2:value})",
        ),
        (
            "getOrDefault",
            "Map<String, T>.getOrDefault(key: String, fallback: T) -> T",
            "getOrDefault(${1:key}, ${2:fallback})",
        ),
        (
            "remove",
            "Map<String, T>.remove(key: String) -> void",
            "remove(${1:key})",
        ),
    ]
    .into_iter()
    .map(|(label, detail, insert_text)| {
        snippet_item(label, CompletionItemKind::METHOD, detail, insert_text)
    })
    .collect()
}

fn optional_method_items() -> Vec<CompletionItem> {
    [
        (
            "isPresent",
            "Optional<T>.isPresent() -> boolean",
            "isPresent()",
        ),
        (
            "orElse",
            "Optional<T>.orElse(fallback: T) -> T",
            "orElse(${1:fallback})",
        ),
        ("get", "Optional<T>.get() -> T", "get()"),
        ("isEmpty", "Optional<T>.isEmpty() -> boolean", "isEmpty()"),
    ]
    .into_iter()
    .map(|(label, detail, insert_text)| {
        snippet_item(label, CompletionItemKind::METHOD, detail, insert_text)
    })
    .collect()
}

fn generic_entity_root_items() -> Vec<CompletionItem> {
    [
        (
            "setVelocity",
            "entity.setVelocity(x: float, y: float, z: float) -> void",
            "setVelocity(${1:x}, ${2:y}, ${3:z})",
            CompletionItemKind::METHOD,
        ),
        (
            "getAttribute",
            "entity.getAttribute(id: String|Attribute) -> float",
            "getAttribute(${1:id})",
            CompletionItemKind::METHOD,
        ),
        (
            "setAttribute",
            "entity.setAttribute(id: String|Attribute, value: float) -> void",
            "setAttribute(${1:id}, ${2:value})",
            CompletionItemKind::METHOD,
        ),
        (
            "setHealth",
            "entity.setHealth(points: float) -> void",
            "setHealth(${1:points})",
            CompletionItemKind::METHOD,
        ),
        (
            "addVelocity",
            "entity.addVelocity(x: float, y: float, z: float) -> void",
            "addVelocity(${1:x}, ${2:y}, ${3:z})",
            CompletionItemKind::METHOD,
        ),
        (
            "setRotation",
            "entity.setRotation(yaw: float, pitch: float) -> void",
            "setRotation(${1:yaw}, ${2:pitch})",
            CompletionItemKind::METHOD,
        ),
        (
            "lookAt",
            "entity.lookAt(target: Entity|Block) -> void",
            "lookAt(${1:target})",
            CompletionItemKind::METHOD,
        ),
        (
            "yawTo",
            "entity.yawTo(target: Entity|Block) -> float",
            "yawTo(${1:target})",
            CompletionItemKind::METHOD,
        ),
        (
            "pitchTo",
            "entity.pitchTo(target: Entity|Block) -> float",
            "pitchTo(${1:target})",
            CompletionItemKind::METHOD,
        ),
        (
            "animate",
            "display.animate(ticks: int, translation: Vec3, scale: Vec3) -> void",
            "animate(${1:ticks}, ${2:translation}, ${3:scale})",
            CompletionItemKind::METHOD,
        ),
        (
            "setTranslation",
            "display.setTranslation(translation: Vec3) -> void",
            "setTranslation(${1:translation})",
            CompletionItemKind::METHOD,
        ),
        (
            "setScale",
            "display.setScale(scale: Vec3) -> void",
            "setScale(${1:scale})",
            CompletionItemKind::METHOD,
        ),
        (
            "setLeftRotation",
            "display.setLeftRotation(angle: float, axis: Vec3) -> void",
            "setLeftRotation(${1:angle}, ${2:axis})",
            CompletionItemKind::METHOD,
        ),
        (
            "setInterpolationDuration",
            "display.setInterpolationDuration(ticks: int) -> void",
            "setInterpolationDuration(${1:ticks})",
            CompletionItemKind::METHOD,
        ),
        (
            "setInterpolationDelay",
            "display.setInterpolationDelay(ticks: int) -> void",
            "setInterpolationDelay(${1:ticks})",
            CompletionItemKind::METHOD,
        ),
        (
            "setTeleportDuration",
            "display.setTeleportDuration(ticks: int) -> void",
            "setTeleportDuration(${1:ticks})",
            CompletionItemKind::METHOD,
        ),
        (
            "setOwner",
            "entity.setOwner(owner: Entity) -> void",
            "setOwner(${1:owner})",
            CompletionItemKind::METHOD,
        ),
        (
            "getOwner",
            "entity.getOwner() -> Optional<Entity>",
            "getOwner()",
            CompletionItemKind::METHOD,
        ),
        (
            "getTargetBlock",
            "entity.getTargetBlock(maxDistance: float) -> Optional<Block>",
            "getTargetBlock(${1:maxDistance})",
            CompletionItemKind::METHOD,
        ),
        (
            "getTargetEntity",
            "entity.getTargetEntity(maxDistance: float) -> Optional<Entity>",
            "getTargetEntity(${1:maxDistance})",
            CompletionItemKind::METHOD,
        ),
        (
            "getX",
            "entity.getX() -> float",
            "getX()",
            CompletionItemKind::METHOD,
        ),
        (
            "getY",
            "entity.getY() -> float",
            "getY()",
            CompletionItemKind::METHOD,
        ),
        (
            "getZ",
            "entity.getZ() -> float",
            "getZ()",
            CompletionItemKind::METHOD,
        ),
        (
            "getYaw",
            "entity.getYaw() -> float",
            "getYaw()",
            CompletionItemKind::METHOD,
        ),
        (
            "getPitch",
            "entity.getPitch() -> float",
            "getPitch()",
            CompletionItemKind::METHOD,
        ),
        (
            "getHealth",
            "entity.getHealth() -> float",
            "getHealth()",
            CompletionItemKind::METHOD,
        ),
        (
            "distanceTo",
            "entity.distanceTo(other: Entity) -> float",
            "distanceTo(${1:other})",
            CompletionItemKind::METHOD,
        ),
        (
            "getLookX",
            "entity.getLookX() -> float",
            "getLookX()",
            CompletionItemKind::METHOD,
        ),
        (
            "getLookY",
            "entity.getLookY() -> float",
            "getLookY()",
            CompletionItemKind::METHOD,
        ),
        (
            "getLookZ",
            "entity.getLookZ() -> float",
            "getLookZ()",
            CompletionItemKind::METHOD,
        ),
        (
            "teleport",
            "entity.teleport(destination: Entity|Block) -> void",
            "teleport(${1:destination})",
            CompletionItemKind::METHOD,
        ),
        (
            "damage",
            "entity.damage(amount: int) -> void",
            "damage(${1:amount})",
            CompletionItemKind::METHOD,
        ),
        (
            "heal",
            "entity.heal(amount: int) -> void",
            "heal(${1:amount})",
            CompletionItemKind::METHOD,
        ),
        (
            "give",
            "entity.give(item_id: String, count: int) -> void / entity.give(ItemStack) -> void",
            "give(${1:\"minecraft:stone\"}, ${2:1})",
            CompletionItemKind::METHOD,
        ),
        (
            "clear",
            "entity.clear(item_id: String, count: int) -> void",
            "clear(${1:\"minecraft:stone\"}, ${2:1})",
            CompletionItemKind::METHOD,
        ),
        (
            "lootGive",
            "entity.lootGive(table: String) -> void",
            "lootGive(${1:\"minecraft:chests/simple_dungeon\"})",
            CompletionItemKind::METHOD,
        ),
        (
            "sendMessage",
            "entity.sendMessage(message: String) -> void",
            "sendMessage(${1:\"hello\"})",
            CompletionItemKind::METHOD,
        ),
        (
            "sendTitle",
            "entity.sendTitle(message: String) -> void",
            "sendTitle(${1:\"hello\"})",
            CompletionItemKind::METHOD,
        ),
        (
            "sendActionBar",
            "entity.sendActionBar(message: String) -> void",
            "sendActionBar(${1:\"hello\"})",
            CompletionItemKind::METHOD,
        ),
        (
            "playSound",
            "entity.playSound(sound: String, category: String) -> void",
            "playSound(${1:\"minecraft:entity.experience_orb.pickup\"}, ${2:\"master\"})",
            CompletionItemKind::METHOD,
        ),
        (
            "stopSound",
            "entity.stopSound(category: String, sound: String) -> void",
            "stopSound(${1:\"master\"}, ${2:\"minecraft:entity.experience_orb.pickup\"})",
            CompletionItemKind::METHOD,
        ),
        (
            "debugEntity",
            "entity.debugEntity(label: String) -> void",
            "debugEntity(${1:\"target\"})",
            CompletionItemKind::METHOD,
        ),
        (
            "effect",
            "entity.effect(name: String, duration: int, amplifier: int) -> void",
            "effect(${1:name}, ${2:duration}, ${3:amplifier})",
            CompletionItemKind::METHOD,
        ),
        (
            "addTag",
            "entity.addTag(name: String) -> void",
            "addTag(${1:name})",
            CompletionItemKind::METHOD,
        ),
        (
            "removeTag",
            "entity.removeTag(name: String) -> void",
            "removeTag(${1:name})",
            CompletionItemKind::METHOD,
        ),
        (
            "hasTag",
            "entity.hasTag(name: String) -> boolean",
            "hasTag(${1:name})",
            CompletionItemKind::METHOD,
        ),
        (
            "team",
            "entity.team writable String",
            "team",
            CompletionItemKind::FIELD,
        ),
        (
            "position",
            "entity.position -> Block",
            "position",
            CompletionItemKind::FIELD,
        ),
        (
            "nbt",
            "entity.nbt.* read/write namespace",
            "nbt",
            CompletionItemKind::FIELD,
        ),
        (
            "state",
            "entity.state.* read/write namespace",
            "state",
            CompletionItemKind::FIELD,
        ),
        (
            "mainhand",
            "entity.mainhand.* writable namespace",
            "mainhand",
            CompletionItemKind::FIELD,
        ),
        (
            "offhand",
            "entity.offhand.* writable namespace",
            "offhand",
            CompletionItemKind::FIELD,
        ),
        (
            "head",
            "entity.head.* writable namespace",
            "head",
            CompletionItemKind::FIELD,
        ),
        (
            "chest",
            "entity.chest.* writable namespace",
            "chest",
            CompletionItemKind::FIELD,
        ),
        (
            "legs",
            "entity.legs.* writable namespace",
            "legs",
            CompletionItemKind::FIELD,
        ),
        (
            "feet",
            "entity.feet.* writable namespace",
            "feet",
            CompletionItemKind::FIELD,
        ),
    ]
    .into_iter()
    .map(|(label, detail, insert_text, kind)| snippet_item(label, kind, detail, insert_text))
    .collect()
}

fn player_entity_root_items() -> Vec<CompletionItem> {
    let mut items = generic_entity_root_items();
    items.retain(|item| item.label != "setVelocity");
    for item in &mut items {
        match item.label.as_str() {
            "nbt" => item.detail = Some("player.nbt.* read namespace".to_string()),
            "state" => item.detail = Some("player.state.* read/write namespace".to_string()),
            _ => {}
        }
    }
    items.extend(
        [
            ("tags", "player.tags.* read/write namespace", "tags"),
            (
                "inventory",
                "player.inventory[index] -> ItemSlot",
                "inventory",
            ),
            ("hotbar", "player.hotbar[index] -> ItemSlot", "hotbar"),
        ]
        .into_iter()
        .map(|(label, detail, insert_text)| {
            snippet_item(label, CompletionItemKind::FIELD, detail, insert_text)
        }),
    );
    items.extend(
        [
            (
                "setFoodLevel",
                "player.setFoodLevel(level: int) -> void",
                "setFoodLevel(${1:level})",
            ),
            (
                "setSidebarTitle",
                "player.setSidebarTitle(text: String) -> void",
                "setSidebarTitle(${1:\"Title\"})",
            ),
            (
                "setSidebarLine",
                "player.setSidebarLine(line: int, text: String) -> void",
                "setSidebarLine(${1:0}, ${2:\"text\"})",
            ),
            (
                "removeSidebarLine",
                "player.removeSidebarLine(line: int) -> void",
                "removeSidebarLine(${1:0})",
            ),
            (
                "clearSidebar",
                "player.clearSidebar() -> void",
                "clearSidebar()",
            ),
            (
                "getCurrentInput",
                "player.getCurrentInput() -> input view",
                "getCurrentInput()",
            ),
            (
                "getFoodLevel",
                "player.getFoodLevel() -> int",
                "getFoodLevel()",
            ),
            ("getLevel", "player.getLevel() -> int", "getLevel()"),
            (
                "getGameMode",
                "player.getGameMode() -> int",
                "getGameMode()",
            ),
            (
                "getSelectedSlot",
                "player.getSelectedSlot() -> int",
                "getSelectedSlot()",
            ),
            (
                "getDimension",
                "player.getDimension() -> String",
                "getDimension()",
            ),
        ]
        .into_iter()
        .map(|(label, detail, insert_text)| {
            snippet_item(label, CompletionItemKind::METHOD, detail, insert_text)
        }),
    );
    items
}

fn block_ref_items() -> Vec<CompletionItem> {
    let mut items = [
        (
            "lootInsert",
            "block.lootInsert(table: String) -> void",
            "lootInsert(${1:\"minecraft:chests/simple_dungeon\"})",
        ),
        (
            "lootSpawn",
            "block.lootSpawn(table: String) -> void",
            "lootSpawn(${1:\"minecraft:chests/simple_dungeon\"})",
        ),
        (
            "debugMarker",
            "block.debugMarker(label: String) -> void",
            "debugMarker(${1:\"checkpoint\"})",
        ),
        (
            "spawnParticle",
            "block.spawnParticle(name: String, count?: int, viewers?: Entity|Selector) -> void",
            "spawnParticle(${1:\"minecraft:flame\"})",
        ),
        (
            "setBlock",
            "block.setBlock(block_id: String|BlockData) -> void",
            "setBlock(${1:\"minecraft:stone\"})",
        ),
        (
            "fill",
            "block.fill(to: Block, block_id: String|BlockData) -> void",
            "fill(${1:Block.of(\"~1 ~1 ~1\")}, ${2:\"minecraft:stone\"})",
        ),
        (
            "summon",
            "block.summon(entityId: String|EntityData, data?: Nbt) -> Entity",
            "summon(${1:\"minecraft:pig\"})",
        ),
        (
            "spawnItem",
            "block.spawnItem(stack: ItemStack) -> Entity",
            "spawnItem(${1:new ItemStack(\"minecraft:apple\")})",
        ),
        (
            "getLightLevel",
            "block.getLightLevel() -> int",
            "getLightLevel()",
        ),
        ("getBiome", "block.getBiome() -> String", "getBiome()"),
        ("getType", "block.getType() -> String", "getType()"),
        (
            "getState",
            "block.getState(name: String) -> String",
            "getState(${1:\"facing\"})",
        ),
        (
            "copyTo",
            "block.copyTo(destination: Block) -> void",
            "copyTo(${1:Block.of(\"~ ~1 ~\")})",
        ),
        ("getX", "block.getX() -> int", "getX()"),
        ("getY", "block.getY() -> int", "getY()"),
        ("getZ", "block.getZ() -> int", "getZ()"),
        (
            "inBiome",
            "block.inBiome(getBiome: String) -> boolean",
            "inBiome(${1:\"minecraft:plains\"})",
        ),
        (
            "getEnvironment",
            "block.getEnvironment(attribute: String) -> float",
            "getEnvironment(${1:\"gameplay/sky_light_level\"})",
        ),
    ]
    .into_iter()
    .map(|(label, detail, insert_text)| {
        snippet_item(label, CompletionItemKind::METHOD, detail, insert_text)
    })
    .collect::<Vec<_>>();
    items.push(snippet_item(
        "nbt",
        CompletionItemKind::FIELD,
        "block.nbt.* read/write namespace",
        "nbt",
    ));
    items
}

fn entity_def_items() -> Vec<CompletionItem> {
    let mut items = vec![snippet_item(
        "asNbt",
        CompletionItemKind::METHOD,
        "EntityData.asNbt() -> Nbt",
        "asNbt()",
    )];
    items.extend(
        [
            ("id", "EntityData.id read-only String", "id"),
            ("nbt", "EntityData.nbt.* writable namespace", "nbt"),
            (
                "name",
                "EntityData.name -> EntityData.nbt.CustomName",
                "name",
            ),
            (
                "nameVisible",
                "EntityData.nameVisible -> EntityData.nbt.CustomNameVisible",
                "nameVisible",
            ),
            ("noAi", "EntityData.noAi -> EntityData.nbt.NoAI", "noAi"),
            (
                "silent",
                "EntityData.silent -> EntityData.nbt.Silent",
                "silent",
            ),
            (
                "glowing",
                "EntityData.glowing -> EntityData.nbt.Glowing",
                "glowing",
            ),
            ("tags", "EntityData.tags -> EntityData.nbt.Tags", "tags"),
        ]
        .into_iter()
        .map(|(label, detail, insert_text)| {
            snippet_item(label, CompletionItemKind::FIELD, detail, insert_text)
        }),
    );
    items.retain(|item| !property_names(&Type::EntityDef).contains(&item.label.as_str()));
    items.extend(property_accessor_items(&Type::EntityDef, "EntityData"));
    items
}

fn item_def_items() -> Vec<CompletionItem> {
    let mut items = vec![snippet_item(
        "asNbt",
        CompletionItemKind::METHOD,
        "ItemStack.asNbt() -> Nbt",
        "asNbt()",
    )];
    items.extend(
        [
            ("id", "ItemStack.id read-only String", "id"),
            ("count", "ItemStack.count writable int", "count"),
            ("nbt", "ItemStack.nbt.* writable namespace", "nbt"),
            (
                "name",
                "ItemStack.name -> ItemStack.nbt.display.Name",
                "name",
            ),
        ]
        .into_iter()
        .map(|(label, detail, insert_text)| {
            snippet_item(label, CompletionItemKind::FIELD, detail, insert_text)
        }),
    );
    items.retain(|item| !property_names(&Type::ItemDef).contains(&item.label.as_str()));
    items.extend(property_accessor_items(&Type::ItemDef, "ItemStack"));
    items
}

fn block_def_items() -> Vec<CompletionItem> {
    let mut items = vec![snippet_item(
        "asNbt",
        CompletionItemKind::METHOD,
        "BlockData.asNbt() -> Nbt",
        "asNbt()",
    )];
    items.extend(
        [
            ("id", "BlockData.id read-only String", "id"),
            ("states", "BlockData.states.* writable namespace", "states"),
            ("nbt", "BlockData.nbt.* writable namespace", "nbt"),
            ("name", "BlockData.name -> BlockData.nbt.CustomName", "name"),
            ("lock", "BlockData.lock -> BlockData.nbt.Lock", "lock"),
            (
                "lootTable",
                "BlockData.lootTable -> BlockData.nbt.LootTable",
                "lootTable",
            ),
            (
                "lootSeed",
                "BlockData.lootSeed -> BlockData.nbt.LootTableSeed",
                "lootSeed",
            ),
        ]
        .into_iter()
        .map(|(label, detail, insert_text)| {
            snippet_item(label, CompletionItemKind::FIELD, detail, insert_text)
        }),
    );
    items.retain(|item| !property_names(&Type::BlockDef).contains(&item.label.as_str()));
    items.extend(property_accessor_items(&Type::BlockDef, "BlockData"));
    items
}

fn property_accessor_items(ty: &Type, name: &str) -> Vec<CompletionItem> {
    property_names(ty)
        .iter()
        .flat_map(|property| {
            let suffix = capitalized(property);
            let getter = format!("get{suffix}");
            let setter = format!("set{suffix}");
            [
                snippet_item(
                    &getter,
                    CompletionItemKind::METHOD,
                    &format!("{name}.{getter}() reads {property}"),
                    &format!("{getter}()"),
                ),
                snippet_item(
                    &setter,
                    CompletionItemKind::METHOD,
                    &format!("{name}.{setter}(value) writes {property}"),
                    &format!("{setter}(${{1:value}})"),
                ),
            ]
        })
        .collect()
}

fn text_def_items() -> Vec<CompletionItem> {
    [
        ("text", "Component.text writable String"),
        ("translate", "Component.translate writable String"),
        ("keybind", "Component.keybind writable String"),
        ("selector", "Component.selector writable String"),
        ("color", "Component.color writable String"),
        ("font", "Component.font writable String"),
        ("insertion", "Component.insertion writable String"),
        ("bold", "Component.bold writable boolean"),
        ("italic", "Component.italic writable boolean"),
        ("underlined", "Component.underlined writable boolean"),
        ("strikethrough", "Component.strikethrough writable boolean"),
        ("obfuscated", "Component.obfuscated writable boolean"),
        ("extra", "Component.extra writable child component list"),
        (
            "hover_event",
            "Component.hover_event.* writable hover event fields",
        ),
        (
            "click_event",
            "Component.click_event.* writable click event fields",
        ),
        ("with", "Component.with writable translation argument list"),
        ("score", "Component.score.* writable score component fields"),
        ("separator", "Component.separator writable text component"),
        ("nbt", "Component.nbt writable source path String"),
        ("block", "Component.block writable source block String"),
        ("entity", "Component.entity writable source selector String"),
        (
            "storage",
            "Component.storage writable source storage String",
        ),
        ("interpret", "Component.interpret writable boolean"),
    ]
    .into_iter()
    .map(|(label, detail)| snippet_item(label, CompletionItemKind::FIELD, detail, label))
    .collect()
}

fn text_hover_event_items() -> Vec<CompletionItem> {
    [
        ("action", "Component.hover_event.action writable String"),
        (
            "value",
            "Component.hover_event.value writable legacy hover payload",
        ),
        (
            "contents",
            "Component.hover_event.contents writable nested hover payload",
        ),
    ]
    .into_iter()
    .map(|(label, detail)| snippet_item(label, CompletionItemKind::FIELD, detail, label))
    .collect()
}

fn text_click_event_items() -> Vec<CompletionItem> {
    [
        ("action", "Component.click_event.action writable String"),
        ("value", "Component.click_event.value writable String"),
    ]
    .into_iter()
    .map(|(label, detail)| snippet_item(label, CompletionItemKind::FIELD, detail, label))
    .collect()
}

fn text_score_items() -> Vec<CompletionItem> {
    [
        ("name", "Component.score.name writable String"),
        ("objective", "Component.score.objective writable String"),
        ("value", "Component.score.value writable String"),
    ]
    .into_iter()
    .map(|(label, detail)| snippet_item(label, CompletionItemKind::FIELD, detail, label))
    .collect()
}

fn item_slot_items() -> Vec<CompletionItem> {
    [
        (
            "clear",
            "ItemSlot.clear() -> void",
            "clear()",
            CompletionItemKind::METHOD,
        ),
        (
            "exists",
            "ItemSlot.exists read-only boolean",
            "exists",
            CompletionItemKind::FIELD,
        ),
        (
            "id",
            "ItemSlot.id read-only String",
            "id",
            CompletionItemKind::FIELD,
        ),
        (
            "count",
            "ItemSlot.count writable int",
            "count",
            CompletionItemKind::FIELD,
        ),
        (
            "nbt",
            "ItemSlot.nbt.* writable namespace",
            "nbt",
            CompletionItemKind::FIELD,
        ),
        (
            "name",
            "ItemSlot.name writable String",
            "name",
            CompletionItemKind::FIELD,
        ),
    ]
    .into_iter()
    .map(|(label, detail, insert_text, kind)| snippet_item(label, kind, detail, insert_text))
    .collect()
}

fn bossbar_root_items() -> Vec<CompletionItem> {
    let mut items: Vec<_> = [
        (
            "remove",
            "BossBar.remove() -> void",
            "remove()",
            CompletionItemKind::METHOD,
        ),
        (
            "name",
            "BossBar.name writable String",
            "name",
            CompletionItemKind::FIELD,
        ),
        (
            "value",
            "BossBar.value writable int",
            "value",
            CompletionItemKind::FIELD,
        ),
        (
            "max",
            "BossBar.max writable int",
            "max",
            CompletionItemKind::FIELD,
        ),
        (
            "visible",
            "BossBar.visible writable boolean",
            "visible",
            CompletionItemKind::FIELD,
        ),
        (
            "players",
            "BossBar.players writable entity target",
            "players",
            CompletionItemKind::FIELD,
        ),
    ]
    .into_iter()
    .map(|(label, detail, insert_text, kind)| snippet_item(label, kind, detail, insert_text))
    .collect();
    items.retain(|item| !property_names(&Type::Bossbar).contains(&item.label.as_str()));
    items.extend(property_accessor_items(&Type::Bossbar, "BossBar"));
    items
}

fn equipment_slot_items() -> Vec<CompletionItem> {
    ["name", "item", "count"]
        .into_iter()
        .map(|label| {
            snippet_item(
                label,
                CompletionItemKind::FIELD,
                "equipment slot field",
                label,
            )
        })
        .collect()
}

fn member_chain_before_cursor(source: &str, offset: usize) -> Option<Vec<String>> {
    let mut index = offset.min(source.len());
    index = move_back_over_word(source, index);
    if previous_char(source, index)? != '.' {
        return None;
    }
    index -= 1;

    let mut reversed = Vec::new();
    loop {
        let word_end = move_back_over_bracket_suffix(source, index);
        index = word_end;
        while index > 0 {
            let ch = previous_char(source, index)?;
            if !is_member_word_char(ch) {
                break;
            }
            index -= ch.len_utf8();
        }
        if index == word_end {
            break;
        }
        reversed.push(source[index..word_end].to_string());
        if previous_char(source, index) != Some('.') {
            break;
        }
        index -= 1;
    }

    if reversed.is_empty() {
        return None;
    }
    reversed.reverse();
    Some(reversed)
}

fn inline_call_receiver_before_cursor(source: &str, offset: usize) -> Option<CompletionReceiver> {
    let mut index = offset.min(source.len());
    index = move_back_over_word(source, index);
    if previous_char(source, index)? != '.' {
        return None;
    }
    index -= 1;

    let start = move_back_over_call_suffix(source, index);
    if start == index {
        return None;
    }

    let expr = source[start..index].trim();
    let ty = infer_expr_type(expr)?;
    receiver_for_terminal_type(&ty, RefKind::Unknown)
}

fn move_back_over_word(source: &str, mut index: usize) -> usize {
    while index > 0 {
        let Some(ch) = previous_char(source, index) else {
            break;
        };
        if !is_member_word_char(ch) {
            break;
        }
        index -= ch.len_utf8();
    }
    index
}

fn member_identifier_offsets(source: &str, offset: usize) -> (usize, usize) {
    let mut start = offset.min(source.len());
    while start > 0 {
        let Some(ch) = previous_char(source, start) else {
            break;
        };
        if !is_member_word_char(ch) {
            break;
        }
        start -= ch.len_utf8();
    }

    let mut end = offset.min(source.len());
    while end < source.len() {
        let Some(ch) = source[end..].chars().next() else {
            break;
        };
        if !is_member_word_char(ch) {
            break;
        }
        end += ch.len_utf8();
    }

    (start, end)
}

fn move_back_over_call_suffix(source: &str, mut index: usize) -> usize {
    if previous_char(source, index) != Some(')') {
        return index;
    }

    index -= 1;
    let mut depth = 1usize;
    while index > 0 {
        let Some(ch) = previous_char(source, index) else {
            break;
        };
        index -= ch.len_utf8();
        match ch {
            ')' => depth += 1,
            '(' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
    }

    let word_end = index;
    while index > 0 {
        let Some(ch) = previous_char(source, index) else {
            break;
        };
        if !is_member_word_char(ch) {
            break;
        }
        index -= ch.len_utf8();
    }

    if previous_char(source, index) == Some('.') {
        let prefix_end = index - 1;
        let prefix_start = move_back_over_call_suffix(source, prefix_end);
        if prefix_start < prefix_end {
            return prefix_start;
        }
    }
    if index == word_end {
        word_end
    } else if source[..index].ends_with("new ") {
        index - "new ".len()
    } else if &source[index..word_end] == "of"
        && let Some(ty) = ["Selector.", "Block.", "List.", "Map."]
            .into_iter()
            .find(|ty| source[..index].ends_with(ty))
    {
        index - ty.len()
    } else {
        index
    }
}

fn move_back_over_bracket_suffix(source: &str, mut index: usize) -> usize {
    while index > 0 && previous_char(source, index) == Some(']') {
        index -= 1;
        let mut depth = 1usize;
        while index > 0 {
            let Some(ch) = previous_char(source, index) else {
                break;
            };
            index -= ch.len_utf8();
            match ch {
                ']' => depth += 1,
                '[' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
        }
    }
    index
}

fn previous_char(source: &str, index: usize) -> Option<char> {
    if index == 0 {
        return None;
    }
    source[..index].chars().next_back()
}

fn is_member_word_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NbtCompletionOrigin {
    category: NbtSchemaCategory,
    id: Option<String>,
}

#[derive(Debug, Clone)]
struct CompletionLocal {
    name: String,
    ty: Option<Type>,
    nbt_origin: Option<NbtCompletionOrigin>,
}

fn local_type_at_offset(
    source: &str,
    analysis: &AnalysisResult,
    offset: usize,
    name: &str,
) -> Option<(Type, RefKind)> {
    if let Some(function) = function_at_offset(analysis, offset)
        && let Some(local) = analysis
            .locals
            .iter()
            .find(|local| local.function == function.name && local.name == name)
    {
        return Some((local.ty.clone(), local.ref_kind));
    }

    syntactic_locals_at_offset(source, offset)
        .into_iter()
        .rev()
        .find(|local| local.name == name)
        .and_then(|local| {
            local.ty.map(|ty| {
                let ref_kind = if ty == Type::PlayerRef {
                    RefKind::Player
                } else {
                    RefKind::Unknown
                };
                (ty, ref_kind)
            })
        })
}

fn local_nbt_origin_at_offset(
    source: &str,
    analysis: &AnalysisResult,
    offset: usize,
    name: &str,
) -> Option<NbtCompletionOrigin> {
    let syntactic = syntactic_locals_at_offset(source, offset);
    if let Some(local) = syntactic.into_iter().rev().find(|local| local.name == name) {
        if let Some(origin) = local.nbt_origin {
            return Some(origin);
        }
        if let Some(ty) = local.ty
            && let Some(origin) = nbt_origin_for_type(&ty)
        {
            return Some(origin);
        }
    }

    local_type_at_offset(source, analysis, offset, name)
        .and_then(|(ty, _)| nbt_origin_for_type(&ty))
}

fn nbt_origin_for_type(ty: &Type) -> Option<NbtCompletionOrigin> {
    let category = match ty {
        Type::EntityDef | Type::EntityRef | Type::PlayerRef => NbtSchemaCategory::Entity,
        Type::BlockDef | Type::BlockRef => NbtSchemaCategory::Block,
        Type::ItemDef | Type::ItemSlot => NbtSchemaCategory::Item,
        _ => return None,
    };
    Some(NbtCompletionOrigin { category, id: None })
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CompletionReceiver {
    Array,
    Int,
    Float,
    String,
    Dict,
    Optional,
    Selector,
    Struct(String),
    Enum(String),
    GenericEntityRef,
    PlayerEntityRef,
    PlayerInput,
    EntityDef,
    ItemDef,
    TextDef,
    BlockDef,
    ItemSlot,
    Bossbar,
    PlayerDynamicNamespace,
    EntityTeam,
    EquipmentSlot,
    BlockRef,
    Nbt,
}

fn resolve_receiver_kind(
    source: &str,
    analysis: &AnalysisResult,
    offset: usize,
    chain: &[String],
) -> Option<CompletionReceiver> {
    let Some((base_name, segments)) = chain.split_first() else {
        return None;
    };
    let (base_ty, base_ref_kind) = local_type_at_offset(source, analysis, offset, base_name)?;
    receiver_from_type(base_ty, base_ref_kind, segments, analysis)
}

fn inline_member_chain_receiver(
    source: &str,
    analysis: &AnalysisResult,
    offset: usize,
    chain: &[String],
) -> Option<CompletionReceiver> {
    let (base_ty, base_ref_kind) = inline_member_chain_base_type(source, offset, chain)?;
    receiver_from_type(base_ty, base_ref_kind, chain, analysis)
}

fn inline_member_chain_base_type(
    source: &str,
    offset: usize,
    chain: &[String],
) -> Option<(Type, RefKind)> {
    let ty = infer_expr_type(inline_member_chain_base_expr(source, offset, chain)?)?;
    let ref_kind = if ty == Type::PlayerRef {
        RefKind::Player
    } else {
        RefKind::Unknown
    };
    Some((ty, ref_kind))
}

fn inline_member_chain_base_expr<'a>(
    source: &'a str,
    offset: usize,
    chain: &[String],
) -> Option<&'a str> {
    let mut index = offset.min(source.len());
    index = move_back_over_word(source, index);
    if previous_char(source, index)? != '.' {
        return None;
    }
    index -= 1;

    for _ in chain.iter().rev() {
        let word_end = move_back_over_bracket_suffix(source, index);
        index = word_end;
        while index > 0 {
            let ch = previous_char(source, index)?;
            if !is_member_word_char(ch) {
                break;
            }
            index -= ch.len_utf8();
        }
        if index == word_end || previous_char(source, index)? != '.' {
            return None;
        }
        index -= 1;
    }

    let start = move_back_over_call_suffix(source, index);
    if start == index {
        return None;
    }

    Some(source[start..index].trim())
}

fn receiver_from_type(
    current: Type,
    current_ref_kind: RefKind,
    segments: &[String],
    analysis: &AnalysisResult,
) -> Option<CompletionReceiver> {
    if segments.is_empty() {
        return receiver_for_terminal_type(&current, current_ref_kind);
    }

    let (segment, rest) = segments.split_first()?;
    let current_is_player_ref = current == Type::PlayerRef;
    let next = match current {
        Type::Struct(name) => analysis
            .typed_program
            .as_ref()
            .and_then(|program| program.struct_defs.get(&name))
            .and_then(|def| def.fields.get(segment))
            .cloned()?,
        Type::EntityRef | Type::PlayerRef => match segment.as_str() {
            "mainhand" | "offhand" | "head" | "chest" | "legs" | "feet" => {
                return if rest.is_empty() {
                    Some(CompletionReceiver::EquipmentSlot)
                } else {
                    None
                };
            }
            "state" => {
                return if rest.is_empty() {
                    Some(CompletionReceiver::PlayerDynamicNamespace)
                } else {
                    None
                };
            }
            "nbt" => {
                return if rest.is_empty() {
                    Some(CompletionReceiver::Nbt)
                } else {
                    None
                };
            }
            "tags" => {
                if !current_is_player_ref && current_ref_kind != RefKind::Player {
                    return None;
                }
                return if rest.is_empty() {
                    Some(CompletionReceiver::PlayerDynamicNamespace)
                } else {
                    None
                };
            }
            "team" => {
                return if rest.is_empty() {
                    Some(CompletionReceiver::EntityTeam)
                } else {
                    None
                };
            }
            "inventory" | "hotbar" => {
                return if current_is_player_ref || current_ref_kind == RefKind::Player {
                    receiver_from_type(Type::ItemSlot, RefKind::Unknown, rest, analysis)
                } else {
                    None
                };
            }
            "getCurrentInput" if current_is_player_ref || current_ref_kind == RefKind::Player => {
                return if rest.is_empty() {
                    Some(CompletionReceiver::PlayerInput)
                } else {
                    None
                };
            }
            "position" => Type::BlockRef,
            "effect" | "add_tag" | "remove_tag" | "has_tag" | "teleport" | "damage" | "heal"
            | "give" | "clear" | "loot_give" | "tellraw" | "title" | "actionbar" | "playsound"
            | "stopsound" | "debug_entity" => return None,
            _ => return None,
        },
        Type::EntityDef => match segment.as_str() {
            "id" => Type::String,
            "nbt" => Type::Nbt,
            _ => return None,
        },
        Type::ItemDef => match segment.as_str() {
            "id" => Type::String,
            "count" => Type::Int,
            "nbt" => Type::Nbt,
            "name" => Type::String,
            "as_nbt" => return None,
            _ => return None,
        },
        Type::TextDef => Type::Nbt,
        Type::BlockDef => match segment.as_str() {
            "id" => Type::String,
            "states" | "nbt" => Type::Nbt,
            _ => return None,
        },
        Type::ItemSlot => match segment.as_str() {
            "exists" => Type::Bool,
            "id" | "name" => Type::String,
            "count" => Type::Int,
            "nbt" => Type::Nbt,
            "clear" => return None,
            _ => return None,
        },
        Type::BlockRef => match segment.as_str() {
            "nbt" => {
                return if rest.is_empty() {
                    Some(CompletionReceiver::Nbt)
                } else {
                    None
                };
            }
            "loot_insert" | "loot_spawn" | "debug_marker" | "particle" | "setblock" | "fill"
            | "summon" | "spawn_item" | "is" => return None,
            _ => return None,
        },
        Type::Bossbar => match segment.as_str() {
            "name" => Type::String,
            "value" | "max" => Type::Int,
            "visible" => Type::Bool,
            "players" => Type::EntitySet,
            "remove" => return None,
            _ => return None,
        },
        _ => return None,
    };

    receiver_from_type(next, RefKind::Unknown, rest, analysis)
}

fn receiver_for_terminal_type(ty: &Type, ref_kind: RefKind) -> Option<CompletionReceiver> {
    match ty {
        Type::Array(_) => Some(CompletionReceiver::Array),
        Type::Int => Some(CompletionReceiver::Int),
        Type::Float => Some(CompletionReceiver::Float),
        Type::String => Some(CompletionReceiver::String),
        Type::Dict(_) => Some(CompletionReceiver::Dict),
        Type::Optional(_) => Some(CompletionReceiver::Optional),
        Type::EntitySet => Some(CompletionReceiver::Selector),
        Type::Struct(name) => Some(CompletionReceiver::Struct(name.clone())),
        Type::Enum(name) => Some(CompletionReceiver::Enum(name.clone())),
        Type::EntityRef => Some(if ref_kind == RefKind::Player {
            CompletionReceiver::PlayerEntityRef
        } else {
            CompletionReceiver::GenericEntityRef
        }),
        Type::PlayerRef => Some(CompletionReceiver::PlayerEntityRef),
        Type::EntityDef => Some(CompletionReceiver::EntityDef),
        Type::ItemDef => Some(CompletionReceiver::ItemDef),
        Type::TextDef => Some(CompletionReceiver::TextDef),
        Type::BlockDef => Some(CompletionReceiver::BlockDef),
        Type::ItemSlot => Some(CompletionReceiver::ItemSlot),
        Type::Bossbar => Some(CompletionReceiver::Bossbar),
        Type::BlockRef => Some(CompletionReceiver::BlockRef),
        Type::Nbt => Some(CompletionReceiver::Nbt),
        _ => None,
    }
}

fn struct_type_items(analysis: &AnalysisResult) -> Vec<CompletionItem> {
    let Some(struct_defs) = analysis
        .typed_program
        .as_ref()
        .map(|program| &program.struct_defs)
    else {
        return Vec::new();
    };

    struct_defs
        .iter()
        .map(|(name, def)| match &def.enum_variants {
            Some(variants) => snippet_item(
                name,
                CompletionItemKind::ENUM,
                &enum_signature(name, variants),
                name,
            ),
            None => snippet_item(
                name,
                CompletionItemKind::STRUCT,
                &struct_signature(name, def),
                name,
            ),
        })
        .collect()
}

fn struct_field_items(analysis: &AnalysisResult, name: &str) -> Vec<CompletionItem> {
    let Some(struct_defs) = analysis
        .typed_program
        .as_ref()
        .map(|program| &program.struct_defs)
    else {
        return Vec::new();
    };
    let Some(def) = struct_defs.get(name) else {
        return Vec::new();
    };

    def.fields
        .iter()
        .map(|(field, ty)| CompletionItem {
            label: field.clone(),
            kind: Some(CompletionItemKind::METHOD),
            detail: Some(format!("{}() -> {}", field, ty.as_str())),
            insert_text: Some(format!("{}()", field)),
            ..CompletionItem::default()
        })
        .collect()
}

fn struct_signature(name: &str, def: &StructTypeDef) -> String {
    let fields = def
        .fields
        .iter()
        .map(|(field, ty)| format!("{} {}", ty.as_str(), field))
        .collect::<Vec<_>>()
        .join(", ");
    format!("record {}({}) {{}}", name, fields)
}

fn enum_signature(name: &str, variants: &[String]) -> String {
    format!("enum {name} {{ {} }}", variants.join(", "))
}

fn struct_signature_from_fields(name: &str, fields: &[(String, Type)]) -> String {
    let body = fields
        .iter()
        .map(|(field, ty)| format!("{} {}", ty.as_str(), field))
        .collect::<Vec<_>>()
        .join(", ");
    format!("record {}({}) {{}}", name, body)
}

fn syntactic_locals_at_offset(source: &str, offset: usize) -> Vec<CompletionLocal> {
    let prefix = &source[..offset.min(source.len())];
    // Each binding remembers the brace depth it lives at; closing that brace
    // drops it. Unfinished code is fine: an unclosed block just stays open.
    let mut locals: Vec<(CompletionLocal, usize)> = Vec::new();
    let mut depth = 0usize;

    for line in prefix.lines() {
        let code = strip_line_comment(line).trim();
        if depth == 0 && is_scope_header(code) {
            locals.clear();
            locals.extend(parse_params(code).into_iter().map(|local| (local, 1)));
        } else if depth > 0 {
            let visible = locals
                .iter()
                .map(|(local, _)| local.clone())
                .collect::<Vec<_>>();
            if let Some((name, declared, value)) = parse_let_binding(code) {
                let ty = declared.or_else(|| infer_expr_type(value)).or_else(|| {
                    visible
                        .iter()
                        .rev()
                        .find(|local| local.name == value)
                        .and_then(|local| local.ty.clone())
                });
                let nbt_origin = infer_nbt_completion_origin_from_value(value, &visible);
                upsert_scoped_completion_local(
                    &mut locals,
                    CompletionLocal {
                        name,
                        ty,
                        nbt_origin,
                    },
                    depth,
                );
            } else if let Some(local) = parse_for_local(code) {
                upsert_scoped_completion_local(&mut locals, local, depth + 1);
            } else if let Some(name) = assigned_local_name(code)
                && let Some((local, _)) = locals
                    .iter_mut()
                    .rev()
                    .find(|(local, _)| local.name == name)
            {
                local.nbt_origin = None;
            }
        }

        for brace in code_braces(code) {
            if brace == '{' {
                depth += 1;
            } else {
                depth = depth.saturating_sub(1);
                locals.retain(|(_, binding_depth)| *binding_depth <= depth);
            }
        }
    }

    locals.into_iter().map(|(local, _)| local).collect()
}

/// Cuts a `//` comment off a line, ignoring `//` inside string literals.
fn strip_line_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut in_string = false;
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' if in_string => index += 1,
            b'"' => in_string = !in_string,
            b'/' if !in_string && bytes.get(index + 1) == Some(&b'/') => return &line[..index],
            _ => {}
        }
        index += 1;
    }
    line
}

/// The `{` and `}` of a comment-free line, skipping string literals.
fn code_braces(code: &str) -> Vec<char> {
    let mut braces = Vec::new();
    let mut in_string = false;
    let mut escaped = false;
    for ch in code.chars() {
        if escaped {
            escaped = false;
        } else if in_string && ch == '\\' {
            escaped = true;
        } else if ch == '"' {
            in_string = !in_string;
        } else if !in_string && (ch == '{' || ch == '}') {
            braces.push(ch);
        }
    }
    braces
}

fn agent_event_member_completion_items(
    source: &str,
    analysis: &AnalysisResult,
    offset: usize,
    chain: &[String],
) -> Option<Vec<CompletionItem>> {
    let (parameter, payload_type) = agent_event_context(source, offset)?;
    if chain.first()? != &parameter {
        return None;
    }
    let mut ty = Type::Struct(payload_type);
    for segment in &chain[1..] {
        ty = match ty {
            Type::Struct(ref name) => analysis
                .typed_program
                .as_ref()?
                .struct_defs
                .get(name)?
                .fields
                .get(segment)?
                .clone(),
            Type::PlayerRef => return Some(player_entity_root_items()),
            _ => return Some(Vec::new()),
        };
    }
    match ty {
        Type::Struct(name) => Some(struct_field_items(analysis, &name)),
        Type::PlayerRef => Some(player_entity_root_items()),
        _ => Some(Vec::new()),
    }
}

/// Find the handler around the cursor when it is an `@EventHandler` with an
/// event parameter, e.g. `void onChat(ChatEvent event) {`.
fn agent_event_context(source: &str, offset: usize) -> Option<(String, String)> {
    let prefix = &source[..offset.min(source.len())];
    let mut lines = prefix.lines().rev();
    let header = lines.find(|line| {
        !line.starts_with(char::is_whitespace) && is_scope_header(strip_line_comment(line).trim())
    })?;
    let annotation = lines.map(str::trim).find(|line| !line.is_empty())?;
    if annotation != "@EventHandler" {
        return None;
    }
    let [(ty, name)] = <[_; 1]>::try_from(header_params(strip_line_comment(header).trim())).ok()?;
    event_kind_for_type(&ty).map(|_| (name, ty))
}

fn upsert_scoped_completion_local(
    locals: &mut Vec<(CompletionLocal, usize)>,
    local: CompletionLocal,
    indent: usize,
) {
    if let Some(index) = locals
        .iter()
        .position(|(existing, _)| existing.name == local.name)
    {
        locals.remove(index);
    }
    locals.push((local, indent));
}

/// A top-level function header: `[public] Type name(params) {`.
fn is_scope_header(line: &str) -> bool {
    let line = line.strip_prefix("public ").unwrap_or(line);
    let Some(open) = line.find('(') else {
        return false;
    };
    split_declaration(&line[..open]).is_some_and(|(ty, _)| {
        !matches!(
            ty,
            "record" | "enum" | "import" | "new" | "return" | "else" | "case"
        )
    })
}

/// Splits `Type name` (the type may be generic, `Map<String, int>`).
fn split_declaration(text: &str) -> Option<(&str, &str)> {
    let (ty, name) = text.trim().rsplit_once(char::is_whitespace)?;
    let ty = ty.trim();
    let valid_type = !ty.is_empty()
        && ty
            .chars()
            .all(|ch| is_member_word_char(ch) || matches!(ch, '<' | '>' | ',' | '.' | ' '));
    let valid_name = !name.is_empty()
        && !name.starts_with(|ch: char| ch.is_ascii_digit())
        && name.chars().all(is_member_word_char);
    (valid_type && valid_name).then_some((ty, name))
}

/// `(type, name)` for each parameter of a function header.
fn header_params(line: &str) -> Vec<(String, String)> {
    let (Some(open), Some(close)) = (line.find('('), line.rfind(')')) else {
        return Vec::new();
    };
    if close <= open {
        return Vec::new();
    }
    let mut params = Vec::new();
    let mut generic_depth = 0usize;
    let mut start = open + 1;
    for (index, ch) in line[..close].char_indices().skip_while(|(i, _)| *i <= open) {
        match ch {
            '<' => generic_depth += 1,
            '>' => generic_depth = generic_depth.saturating_sub(1),
            ',' if generic_depth == 0 => {
                params.extend(split_declaration(&line[start..index]));
                start = index + 1;
            }
            _ => {}
        }
    }
    params.extend(split_declaration(&line[start..close]));
    params
        .into_iter()
        .map(|(ty, name)| (ty.to_string(), name.to_string()))
        .collect()
}

fn parse_params(line: &str) -> Vec<CompletionLocal> {
    header_params(line)
        .into_iter()
        .map(|(ty, name)| CompletionLocal {
            name,
            ty: parse_type_name(&ty),
            nbt_origin: None,
        })
        .collect()
}

/// `var name = value;` or `Type name = value;` -> (name, declared type, value).
fn parse_let_binding(line: &str) -> Option<(String, Option<Type>, &str)> {
    let (lhs, value) = line.split_once('=')?;
    if value.starts_with('=') || lhs.contains('(') {
        return None;
    }
    let (ty, name) = split_declaration(lhs)?;
    if matches!(ty, "return" | "else" | "case") {
        return None;
    }
    let declared = if ty == "var" {
        None
    } else {
        parse_type_name(ty)
    };
    let value = value.trim().trim_end_matches(';').trim_end();
    Some((name.to_string(), declared, value))
}

/// `for (var x : values) {` or `for (int i = 0; ...) {`.
fn parse_for_local(line: &str) -> Option<CompletionLocal> {
    let rest = line.strip_prefix("for")?.trim_start().strip_prefix('(')?;
    if let Some((init, _)) = rest.split_once(';') {
        let (name, declared, value) = parse_let_binding(init)?;
        return Some(CompletionLocal {
            name,
            ty: declared.or_else(|| infer_expr_type(value)),
            nbt_origin: None,
        });
    }
    let (declaration, _) = rest.split_once(':')?;
    let (ty, name) = split_declaration(declaration)?;
    Some(CompletionLocal {
        name: name.to_string(),
        ty: parse_type_name(ty).or(Some(Type::EntityRef)),
        nbt_origin: None,
    })
}

fn assigned_local_name(line: &str) -> Option<&str> {
    let (lhs, value) = line.split_once('=')?;
    if value.starts_with('=') {
        return None;
    }
    let name = lhs.trim();
    if name.is_empty() || !name.chars().all(is_member_word_char) {
        return None;
    }
    Some(name)
}

fn parse_type_name(name: &str) -> Option<Type> {
    let name = name.trim();
    let generic = |prefix: &str| {
        name.strip_prefix(prefix)
            .and_then(|rest| rest.strip_suffix('>'))
            .map(str::trim)
    };
    if let Some(inner) = generic("List<") {
        return Some(Type::Array(Box::new(
            parse_type_name(inner).unwrap_or(Type::Nbt),
        )));
    }
    if let Some(inner) = generic("Optional<") {
        return Some(Type::Optional(Box::new(
            parse_type_name(inner).unwrap_or(Type::Nbt),
        )));
    }
    if let Some(inner) = generic("Map<") {
        let value = inner.split_once(',').map_or(inner, |(_, value)| value);
        return Some(Type::Dict(Box::new(
            parse_type_name(value).unwrap_or(Type::Nbt),
        )));
    }
    match name {
        "int" => Some(Type::Int),
        "float" => Some(Type::Float),
        "boolean" => Some(Type::Bool),
        "String" => Some(Type::String),
        "Selector" => Some(Type::EntitySet),
        "Entity" => Some(Type::EntityRef),
        "Player" => Some(Type::PlayerRef),
        "Block" => Some(Type::BlockRef),
        "EntityData" => Some(Type::EntityDef),
        "BlockData" => Some(Type::BlockDef),
        "ItemStack" => Some(Type::ItemDef),
        "Component" => Some(Type::TextDef),
        "ItemSlot" => Some(Type::ItemSlot),
        "BossBar" => Some(Type::Bossbar),
        "Nbt" => Some(Type::Nbt),
        "void" => Some(Type::Void),
        _ => None,
    }
}

fn infer_expr_type(value: &str) -> Option<Type> {
    let value = value.trim();
    let starts = |prefix: &str| value.starts_with(prefix);
    if !starts("(Player)")
        && let Some(inner) = value
            .strip_prefix('(')
            .and_then(|rest| rest.strip_suffix(')'))
    {
        infer_expr_type(inner)
    } else if starts("(Player)") {
        Some(Type::PlayerRef)
    } else if value.ends_with(".getFirst()") {
        Some(Type::EntityRef)
    } else if value.ends_with(".findFirst()") {
        Some(Type::Optional(Box::new(Type::EntityRef)))
    } else if starts("Selector.of(") {
        Some(Type::EntitySet)
    } else if starts("new EntityData(") {
        Some(Type::EntityDef)
    } else if starts("new ItemStack(") {
        Some(Type::ItemDef)
    } else if starts("new Component(") {
        Some(Type::TextDef)
    } else if starts("Block.of(") {
        Some(Type::BlockRef)
    } else if starts("new BlockData(") {
        Some(Type::BlockDef)
    } else if starts("new BossBar(") {
        Some(Type::Bossbar)
    } else if starts("\"") {
        Some(Type::String)
    } else if starts("true") || starts("false") {
        Some(Type::Bool)
    } else if value.chars().next().is_some_and(|ch| ch.is_ascii_digit())
        || value
            .strip_prefix('-')
            .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|ch| ch.is_ascii_digit()))
    {
        Some(Type::Int)
    } else if starts("List.of(") {
        Some(Type::Array(Box::new(Type::Nbt)))
    } else if starts("Map.of(") {
        Some(Type::Dict(Box::new(Type::Nbt)))
    } else {
        None
    }
}

fn infer_nbt_completion_origin_from_value(
    value: &str,
    locals: &[CompletionLocal],
) -> Option<NbtCompletionOrigin> {
    infer_nbt_completion_origin(value).or_else(|| {
        locals
            .iter()
            .rev()
            .find(|local| local.name == value)
            .and_then(|local| local.nbt_origin.clone())
    })
}

fn infer_nbt_completion_origin(value: &str) -> Option<NbtCompletionOrigin> {
    let value = value.trim();
    if let Some(origin) =
        infer_constructor_nbt_origin(value, "new EntityData", NbtSchemaCategory::Entity)
    {
        return Some(origin);
    }
    if let Some(origin) =
        infer_constructor_nbt_origin(value, "new BlockData", NbtSchemaCategory::Block)
    {
        return Some(origin);
    }
    if let Some(origin) =
        infer_constructor_nbt_origin(value, "new ItemStack", NbtSchemaCategory::Item)
    {
        return Some(origin);
    }
    value
        .strip_suffix(".asNbt()")
        .and_then(infer_nbt_completion_origin)
}

fn infer_constructor_nbt_origin(
    value: &str,
    constructor: &str,
    category: NbtSchemaCategory,
) -> Option<NbtCompletionOrigin> {
    value.strip_prefix(&format!("{constructor}("))?;
    Some(NbtCompletionOrigin {
        category,
        id: parse_leading_string_literal(
            value
                .rsplit_once(')')
                .map(|(prefix, _)| prefix)
                .unwrap_or(value)
                .trim_start_matches(&format!("{constructor}(")),
        ),
    })
}

fn parse_leading_string_literal(value: &str) -> Option<String> {
    let mut chars = value.trim_start().chars();
    let quote = chars.next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }

    let mut escaped = false;
    let mut parsed = String::new();
    for ch in chars {
        if escaped {
            parsed.push(ch);
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            value if value == quote => return Some(parsed),
            _ => parsed.push(ch),
        }
    }

    None
}

fn diagnostic_to_lsp(source: &str, diagnostic: &McfcDiagnostic) -> LspDiagnostic {
    LspDiagnostic {
        range: range_from_text_range(source, diagnostic.span.range),
        severity: Some(DiagnosticSeverity::ERROR),
        source: Some("mcfc".to_string()),
        message: diagnostic.message.clone(),
        ..LspDiagnostic::default()
    }
}

pub fn range_from_text_range(source: &str, range: TextRange) -> Range {
    let start = range.start.min(source.len());
    let mut end = range.end.min(source.len());
    if start == end
        && end < source.len()
        && let Some(ch) = source[end..].chars().next()
    {
        end += ch.len_utf8();
    }
    Range {
        start: offset_to_position(source, start),
        end: offset_to_position(source, end),
    }
}

pub fn offset_to_position(source: &str, offset: usize) -> Position {
    let offset = offset.min(source.len());
    let mut line = 0u32;
    let mut character = 0u32;

    for (index, ch) in source.char_indices() {
        if index >= offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            character = 0;
        } else {
            character += ch.len_utf16() as u32;
        }
    }

    Position { line, character }
}

pub fn position_to_offset(source: &str, position: Position) -> usize {
    let mut line = 0u32;
    let mut character = 0u32;

    for (index, ch) in source.char_indices() {
        if line == position.line && character >= position.character {
            return index;
        }
        if ch == '\n' {
            if line == position.line {
                return index;
            }
            line += 1;
            character = 0;
        } else if line == position.line {
            let next_character = character + ch.len_utf16() as u32;
            if next_character > position.character {
                return index;
            }
            character = next_character;
        }
    }

    source.len()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use tower_lsp::lsp_types::{CompletionTextEdit, Position};

    use super::{
        ProjectConfig, build_project_snapshot, builtin_hover, completion_items, infer_expr_type,
        offset_to_position, position_to_offset, project_diagnostics_for_segment,
        project_document_symbols, range_from_text_range, resolve_project_config_for_path,
        semantic_ranges,
    };
    use crate::analysis::analyze_source;
    use crate::diagnostics::TextRange;

    fn temp_path() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("mcfc-lsp-tests-{}", unique));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn write_file(path: &PathBuf, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn converts_offsets_to_utf16_positions() {
        let source = "a\nå😀b";

        assert_eq!(offset_to_position(source, 0), Position::new(0, 0));
        assert_eq!(offset_to_position(source, 2), Position::new(1, 0));
        assert_eq!(
            offset_to_position(source, source.find("b").unwrap()),
            Position::new(1, 3)
        );
        assert_eq!(
            position_to_offset(source, Position::new(1, 3)),
            source.find("b").unwrap()
        );
    }

    #[test]
    fn widens_zero_width_ranges() {
        let source = "åb";
        let range = range_from_text_range(source, TextRange::new(0, 0));

        assert_eq!(range.start, Position::new(0, 0));
        assert_eq!(range.end, Position::new(0, 1));
    }

    #[test]
    fn completes_static_and_analysis_items() {
        let source = r#"
int helper(int x) {
    return x;
}
void main() {
    var value = helper(1);
    value = value + 1;
}
"#;
        let analysis = analyze_source(source);
        let items = completion_items(source, &analysis, source.find("helper(1)").unwrap());
        assert!(!items.iter().any(|item| item.label == "fn"));
        assert!(items.iter().any(|item| item.label == "helper"));
        assert!(items.iter().any(|item| item.label == "value"));

        let method_items = completion_items("value.", &analysis, 6);
        assert!(method_items.is_empty());
    }

    #[test]
    fn completes_syntactic_locals_when_source_is_incomplete() {
        let source = r#"
void main(String kind) {
    var me = Selector.of("@a").getFirst();
    var amount = 1;
    me.team.;
}
"#;
        let analysis = analyze_source(source);
        assert!(!analysis.diagnostics.is_empty());

        let local_items = completion_items(source, &analysis, source.find("me.team.").unwrap());
        assert!(local_items.iter().any(|item| item.label == "kind"));
        assert!(local_items.iter().any(|item| item.label == "me"));
        assert!(local_items.iter().any(|item| item.label == "amount"));

        let team_items = completion_items(source, &analysis, source.find("me.team.").unwrap() + 8);
        assert!(team_items.is_empty());
    }

    #[test]
    fn completes_locals_and_members_inside_lowered_event_declarations() {
        let source = r#"
@EventHandler
void onPlayerDeath(PlayerDeathEvent event) {
    Player player = event.player();
    player.;
}
"#;
        let analysis = analyze_source(source);
        let local_items = completion_items(source, &analysis, source.find("player.").unwrap());
        assert!(local_items.iter().any(|item| item.label == "player"));
        assert!(
            !local_items
                .iter()
                .any(|item| item.label == "@EventHandler PlayerJoinEvent")
        );

        let member_items = completion_items(
            source,
            &analysis,
            source.find("player.").unwrap() + "player.".len(),
        );
        assert!(member_items.iter().any(|item| item.label == "sendMessage"));
        assert!(!member_items.iter().any(|item| item.label == "add"));
    }

    #[test]
    fn semantic_ranges_exclude_strings_comments_members_and_other_scopes() {
        let source = r#"
void first() {
    var player = Selector.of("@s").getFirst();
    player.sendMessage("player");  // player
}
void second() {
    var player = Selector.of("@p").getFirst();
    player.sendMessage("ok");
}
"#;
        let offset = source.find("player.sendMessage").unwrap();
        let ranges = semantic_ranges(source, offset, "player");
        assert_eq!(
            ranges.len(),
            2,
            "definition and use in the first scope only"
        );
    }

    #[test]
    fn completion_does_not_leak_dedented_branch_locals() {
        let source = r#"
void main() {
    var outer = 1;
    if (true) {
        var branch_only = 2;
    }
    outer;
}
"#;
        let analysis = analyze_source(source);
        let items = completion_items(source, &analysis, source.rfind("outer").unwrap());
        assert!(items.iter().any(|item| item.label == "outer"));
        assert!(!items.iter().any(|item| item.label == "branch_only"));
    }

    #[test]
    fn narrows_member_completions_by_receiver_type() {
        let source = r#"
void main() {
    var values = List.of(1, 2, 3);
    var me = Selector.of("@a").getFirst();
    values.;
    me.;
}
"#;
        let analysis = analyze_source(source);
        let values_items = completion_items(source, &analysis, source.find("values.").unwrap() + 7);
        assert!(values_items.iter().any(|item| item.label == "add"));
        assert!(values_items.iter().any(|item| item.label == "remove"));
        assert!(values_items.iter().any(|item| item.label == "remove"));
        assert!(!values_items.iter().any(|item| item.label == "team"));

        let me_items = completion_items(source, &analysis, source.find("me.").unwrap() + 3);
        assert!(me_items.iter().any(|item| item.label == "team"));
        assert!(!me_items.iter().any(|item| item.label == "add"));
    }

    #[test]
    fn completes_struct_types_and_fields() {
        let source = r#"
record Profile(int duration, String label) {}
record Action(Profile profile, String kind) {}
void main(Action action) {
    var next = action.profile();
    var duration = next.duration();
    var kind = action.kind();
}
"#;
        let analysis = analyze_source(source);
        assert!(
            analysis.typed_program.is_some(),
            "{:?}",
            analysis.diagnostics
        );

        let action_items = completion_items(
            source,
            &analysis,
            source.find("action.profile").unwrap() + "action.".len(),
        );
        assert!(action_items.iter().any(
            |item| item.label == "profile" && item.insert_text.as_deref() == Some("profile()")
        ));

        let top_level_items =
            completion_items(source, &analysis, source.find("void main").unwrap());
        assert!(top_level_items.iter().any(|item| item.label == "Action"));
        assert!(top_level_items.iter().any(|item| item.label == "Profile"));

        let action_items = completion_items(
            source,
            &analysis,
            source.find("action.kind").unwrap() + "action.".len(),
        );
        assert!(action_items.iter().any(|item| item.label == "profile"));
        assert!(action_items.iter().any(|item| item.label == "kind"));

        let next_items = completion_items(
            source,
            &analysis,
            source.find("next.duration").unwrap() + "next.".len(),
        );
        assert!(next_items.iter().any(|item| item.label == "duration"));
        assert!(next_items.iter().any(|item| item.label == "label"));
    }

    #[test]
    fn completes_nested_player_member_paths() {
        let source = r#"
void main() {
    var me = (Player) Selector.of("@a").getFirst();
    var asserted = (Player) Selector.of("@e[limit=1]").getFirst();
    me.mainhand.;
    me.inventory[0].;
    me.inventory[-1].;
    me.hotbar[0].;
    me.hotbar[-1].;
    asserted.;
    mcf("say $(me.mainhand.)");
}
"#;
        let analysis = analyze_source(source);

        let mainhand_items =
            completion_items(source, &analysis, source.find("me.mainhand.").unwrap() + 12);
        assert!(mainhand_items.iter().any(|item| item.label == "name"));
        assert!(mainhand_items.iter().any(|item| item.label == "count"));

        let placeholder_items = completion_items(
            source,
            &analysis,
            source.find("me.mainhand.)").unwrap() + "me.mainhand.".len(),
        );
        assert!(placeholder_items.iter().any(|item| item.label == "item"));

        let inventory_items = completion_items(
            source,
            &analysis,
            source.find("me.inventory[0].").unwrap() + 16,
        );
        assert!(inventory_items.iter().any(|item| item.label == "exists"));
        assert!(inventory_items.iter().any(|item| item.label == "clear"));

        let inventory_negative_items = completion_items(
            source,
            &analysis,
            source.find("me.inventory[-1].").unwrap() + 17,
        );
        assert!(
            inventory_negative_items
                .iter()
                .any(|item| item.label == "exists")
        );
        assert!(
            inventory_negative_items
                .iter()
                .any(|item| item.label == "clear")
        );

        let hotbar_items = completion_items(
            source,
            &analysis,
            source.find("me.hotbar[0].").unwrap() + 13,
        );
        assert!(hotbar_items.iter().any(|item| item.label == "id"));
        assert!(hotbar_items.iter().any(|item| item.label == "count"));

        let hotbar_negative_items = completion_items(
            source,
            &analysis,
            source.find("me.hotbar[-1].").unwrap() + 14,
        );
        assert!(hotbar_negative_items.iter().any(|item| item.label == "id"));
        assert!(
            hotbar_negative_items
                .iter()
                .any(|item| item.label == "count")
        );

        let asserted_items =
            completion_items(source, &analysis, source.find("asserted.").unwrap() + 9);
        assert!(asserted_items.iter().any(|item| item.label == "inventory"));
        assert!(asserted_items.iter().any(|item| item.label == "hotbar"));
    }

    #[test]
    fn infers_negative_integer_literals_for_lsp_type_heuristics() {
        assert_eq!(infer_expr_type("-1"), Some(crate::ast::Type::Int));
        assert_eq!(infer_expr_type("  -42  "), Some(crate::ast::Type::Int));
    }

    #[test]
    fn completes_gameplay_builtins_and_generic_entity_members() {
        let source = r#"
void main() {
    var pig = Selector.of("@e[type=pig,limit=1]").getFirst();
    pig.;
}
"#;
        let analysis = analyze_source(source);
        let top_level_items =
            completion_items(source, &analysis, source.find("void main").unwrap());
        assert!(top_level_items.iter().any(|item| item.label == "sleep"));
        assert!(top_level_items.iter().any(|item| item.label == "random"));
        assert!(
            top_level_items
                .iter()
                .any(|item| item.label == "random(min, max)")
        );
        assert!(top_level_items.iter().any(|item| item.label == "summon"));
        assert!(top_level_items.iter().any(|item| item.label == "async {}"));
        assert!(
            top_level_items
                .iter()
                .any(|item| item.label == "sleepTicks")
        );
        assert!(top_level_items.iter().any(|item| item.label == "debug"));
        assert!(
            !top_level_items
                .iter()
                .any(|item| item.label == "sendMessage")
        );

        let pig_items = completion_items(source, &analysis, source.find("pig.").unwrap() + 4);
        assert!(pig_items.iter().any(|item| item.label == "teleport"));
        assert!(pig_items.iter().any(|item| item.label == "sendMessage"));
        assert!(pig_items.iter().any(|item| item.label == "position"));
        assert!(pig_items.iter().any(|item| item.label == "addTag"));
        assert!(pig_items.iter().any(|item| item.label == "removeTag"));
        assert!(pig_items.iter().any(|item| item.label == "hasTag"));
        assert!(pig_items.iter().any(|item| item.label == "offhand"));
        assert!(pig_items.iter().any(|item| item.label == "team"));
        assert!(pig_items.iter().any(|item| item.label == "state"));
        assert!(pig_items.iter().any(|item| item.label == "nbt"));

        let sleep_hover = builtin_hover("sleep").expect("sleep hover");
        assert!(sleep_hover.contains("sleep(seconds: int) -> void"));
        let random_hover = builtin_hover("random").expect("random hover");
        assert!(random_hover.contains("random(min: int, max: int) -> int"));
        let state_hover = builtin_hover("state").expect("state hover");
        assert!(state_hover.contains("entity.state.*"));
        assert!(state_hover.contains("player.state.*"));
    }

    #[test]
    fn completes_state_namespace_consistently_for_generic_entities_and_players() {
        let source = r#"
void main() {
    var pig = Selector.of("@e[type=pig,limit=1]").getFirst();
    var player = (Player) Selector.of("@a[limit=1]").getFirst();
    pig.state.;
    player.state.;
    Selector.of("@e[type=pig,limit=1]").getFirst().state.;
    ((Player) Selector.of("@a[limit=1]").getFirst()).state.;
    pig.position.foo.;
    Selector.of("@e[type=pig,limit=1]").getFirst().position.foo.;
}
"#;
        let analysis = analyze_source(source);

        let pig_state_items = completion_items(
            source,
            &analysis,
            source.find("pig.state.").unwrap() + "pig.state.".len(),
        );
        assert!(pig_state_items.is_empty());

        let player_state_items = completion_items(
            source,
            &analysis,
            source.find("player.state.").unwrap() + "player.state.".len(),
        );
        assert!(player_state_items.is_empty());

        let inline_entity_state_items = completion_items(
            source,
            &analysis,
            source
                .find("Selector.of(\"@e[type=pig,limit=1]\").getFirst().state.")
                .unwrap()
                + "Selector.of(\"@e[type=pig,limit=1]\").getFirst().state.".len(),
        );
        assert!(inline_entity_state_items.is_empty());

        let inline_player_state_items = completion_items(
            source,
            &analysis,
            source
                .find("(Player) Selector.of(\"@a[limit=1]\").getFirst()).state.")
                .unwrap()
                + "(Player) Selector.of(\"@a[limit=1]\").getFirst()).state.".len(),
        );
        assert!(inline_player_state_items.is_empty());

        let invalid_nested_items = completion_items(
            source,
            &analysis,
            source.find("pig.position.foo.").unwrap() + "pig.position.foo.".len(),
        );
        assert!(invalid_nested_items.is_empty());

        let invalid_inline_nested_items = completion_items(
            source,
            &analysis,
            source
                .find("Selector.of(\"@e[type=pig,limit=1]\").getFirst().position.foo.")
                .unwrap()
                + "Selector.of(\"@e[type=pig,limit=1]\").getFirst().position.foo.".len(),
        );
        assert!(invalid_inline_nested_items.is_empty());
    }

    #[test]
    fn completes_builder_members_and_hover_signatures() {
        let source = r#"
void main() {
    var pig = new EntityData("minecraft:pig");
    var chest = new BlockData("minecraft:chest");
    var stack = new ItemStack("minecraft:apple");
    var msg = new Component("Hello");
    pig.;
    chest.;
    stack.;
    msg.;
    new ItemStack("minecraft:apple").;
    new Component("Hello").;
    Block.of("~ ~ ~").;
}
"#;
        let analysis = analyze_source(source);
        let top_level_items =
            completion_items(source, &analysis, source.find("void main").unwrap());
        assert!(
            top_level_items
                .iter()
                .any(|item| item.label == "EntityData")
        );
        assert!(top_level_items.iter().any(|item| item.label == "BlockData"));
        assert!(top_level_items.iter().any(|item| item.label == "ItemStack"));

        let pig_items = completion_items(source, &analysis, source.find("pig.").unwrap() + 4);
        assert!(pig_items.iter().any(|item| item.label == "asNbt"));
        assert!(pig_items.iter().any(|item| item.label == "getName"));
        assert!(pig_items.iter().any(|item| item.label == "setName"));
        assert!(pig_items.iter().any(|item| item.label == "nbt"));
        assert!(pig_items.iter().any(|item| item.label == "setNoAi"));

        let chest_items = completion_items(source, &analysis, source.find("chest.").unwrap() + 6);
        assert!(chest_items.iter().any(|item| item.label == "asNbt"));
        assert!(chest_items.iter().any(|item| item.label == "states"));
        assert!(chest_items.iter().any(|item| item.label == "setLock"));
        assert!(chest_items.iter().any(|item| item.label == "getName"));

        let stack_items = completion_items(source, &analysis, source.find("stack.").unwrap() + 6);
        assert!(stack_items.iter().any(|item| item.label == "asNbt"));
        assert!(stack_items.iter().any(|item| item.label == "setCount"));
        assert!(stack_items.iter().any(|item| item.label == "getName"));

        let msg_items = completion_items(source, &analysis, source.find("msg.").unwrap() + 4);
        assert!(msg_items.iter().any(|item| item.label == "color"));
        assert!(msg_items.iter().any(|item| item.label == "hover_event"));
        assert!(msg_items.iter().any(|item| item.label == "score"));

        let inline_item_items = completion_items(
            source,
            &analysis,
            source.find("ItemStack(\"minecraft:apple\").").unwrap()
                + "ItemStack(\"minecraft:apple\").".len(),
        );
        assert!(inline_item_items.iter().any(|item| item.label == "asNbt"));
        assert!(
            inline_item_items
                .iter()
                .any(|item| item.label == "setCount")
        );

        let inline_text_items = completion_items(
            source,
            &analysis,
            source.find("Component(\"Hello\").").unwrap() + "Component(\"Hello\").".len(),
        );
        assert!(inline_text_items.iter().any(|item| item.label == "text"));
        assert!(
            inline_text_items
                .iter()
                .any(|item| item.label == "click_event")
        );

        let inline_block_items = completion_items(
            source,
            &analysis,
            source.find("Block.of(\"~ ~ ~\").").unwrap() + "Block.of(\"~ ~ ~\").".len(),
        );
        assert!(inline_block_items.iter().any(|item| item.label == "summon"));
        assert!(
            inline_block_items
                .iter()
                .any(|item| item.label == "spawnItem")
        );
        assert!(inline_block_items.iter().any(|item| item.label == "nbt"));

        let summon_hover = builtin_hover("summon").expect("summon hover");
        assert!(summon_hover.contains("summon(spec: EntityData) -> Entity"));
        let as_nbt_hover = builtin_hover("asNbt").expect("asNbt hover");
        assert!(as_nbt_hover.contains("EntityData.asNbt() -> Nbt"));
        assert!(as_nbt_hover.contains("ItemStack.asNbt() -> Nbt"));
        let item_hover = builtin_hover("ItemStack").expect("ItemStack hover");
        assert!(item_hover.contains("new ItemStack(id: String)"));
        let item_slot_hover = builtin_hover("ItemSlot").expect("ItemSlot hover");
        assert!(item_slot_hover.contains("clear() -> void"));
        let player_hover = builtin_hover("Player").expect("Player hover");
        assert!(player_hover.contains("(Player) entity"));
        let give_hover = builtin_hover("give").expect("give hover");
        assert!(give_hover.contains("entity.give(stack: ItemStack) -> void"));
        let spawn_item_hover = builtin_hover("spawnItem").expect("spawnItem hover");
        assert!(spawn_item_hover.contains("block.spawnItem(stack: ItemStack) -> Entity"));
        let component_hover = builtin_hover("Component").expect("Component hover");
        assert!(component_hover.contains("storage-backed text component builder"));
    }

    #[test]
    fn completes_text_def_nested_members() {
        let source = r#"
void main() {
    var msg = new Component("Hello");
    msg.hover_event.;
    msg.click_event.;
    msg.score.;
    msg.extra[0].;
    new Component("Hello").hover_event.;
}
"#;
        let analysis = analyze_source(source);

        let hover_items = completion_items(
            source,
            &analysis,
            source.find("msg.hover_event.").unwrap() + "msg.hover_event.".len(),
        );
        assert!(hover_items.iter().any(|item| item.label == "action"));
        assert!(hover_items.iter().any(|item| item.label == "contents"));

        let click_items = completion_items(
            source,
            &analysis,
            source.find("msg.click_event.").unwrap() + "msg.click_event.".len(),
        );
        assert!(click_items.iter().any(|item| item.label == "action"));
        assert!(click_items.iter().any(|item| item.label == "value"));

        let score_items = completion_items(
            source,
            &analysis,
            source.find("msg.score.").unwrap() + "msg.score.".len(),
        );
        assert!(score_items.iter().any(|item| item.label == "name"));
        assert!(score_items.iter().any(|item| item.label == "objective"));

        let extra_items = completion_items(
            source,
            &analysis,
            source.find("msg.extra[0].").unwrap() + "msg.extra[0].".len(),
        );
        assert!(extra_items.iter().any(|item| item.label == "color"));
        assert!(extra_items.iter().any(|item| item.label == "hover_event"));

        let inline_hover_items = completion_items(
            source,
            &analysis,
            source.find("Component(\"Hello\").hover_event.").unwrap()
                + "Component(\"Hello\").hover_event.".len(),
        );
        assert!(inline_hover_items.iter().any(|item| item.label == "action"));
    }

    #[test]
    fn completes_schema_backed_nbt_fields_for_inline_and_local_builders() {
        let source = r#"
void main() {
    new EntityData("minecraft:mannequin").nbt.;
    new EntityData("minecraft:mannequin").nbt.profile.;
    var mannequin = new EntityData("minecraft:mannequin");
    var alias = mannequin;
    alias.nbt.profile.;
    new BlockData("minecraft:player_head").nbt.;
    new ItemStack("minecraft:player_head").nbt.;
}
"#;
        let analysis = analyze_source(source);

        let entity_root_items = completion_items(
            source,
            &analysis,
            source
                .find("EntityData(\"minecraft:mannequin\").nbt.")
                .unwrap()
                + "EntityData(\"minecraft:mannequin\").nbt.".len(),
        );
        assert!(
            entity_root_items
                .iter()
                .any(|item| item.label == "CustomName")
        );
        assert!(entity_root_items.iter().any(|item| item.label == "profile"));

        let entity_nested_items = completion_items(
            source,
            &analysis,
            source
                .find("EntityData(\"minecraft:mannequin\").nbt.profile.")
                .unwrap()
                + "EntityData(\"minecraft:mannequin\").nbt.profile.".len(),
        );
        assert!(entity_nested_items.iter().any(|item| item.label == "name"));
        assert!(entity_nested_items.iter().any(|item| item.label == "model"));

        let alias_items = completion_items(
            source,
            &analysis,
            source.find("alias.nbt.profile.").unwrap() + "alias.nbt.profile.".len(),
        );
        assert!(alias_items.iter().any(|item| item.label == "id"));
        assert!(alias_items.iter().any(|item| item.label == "properties"));

        let block_items = completion_items(
            source,
            &analysis,
            source
                .find("BlockData(\"minecraft:player_head\").nbt.")
                .unwrap()
                + "BlockData(\"minecraft:player_head\").nbt.".len(),
        );
        assert!(block_items.iter().any(|item| item.label == "profile"));
        assert!(block_items.iter().any(|item| item.label == "custom_name"));

        let item_items = completion_items(
            source,
            &analysis,
            source
                .find("ItemStack(\"minecraft:player_head\").nbt.")
                .unwrap()
                + "ItemStack(\"minecraft:player_head\").nbt.".len(),
        );
        assert!(item_items.iter().any(|item| item.label == "display"));
        assert!(item_items.iter().any(|item| item.label == "SkullOwner"));
    }

    #[test]
    fn completes_schema_backed_nbt_fields_for_runtime_refs() {
        let source = r#"
void main() {
    var pig = Selector.of("@e[type=pig,limit=1]").getFirst();
    var player = (Player) Selector.of("@a[limit=1]").getFirst();
    var chest = Block.of("~ ~ ~");
    pig.nbt.;
    Selector.of("@e[type=pig,limit=1]").getFirst().nbt.;
    player.nbt.;
    chest.nbt.;
    Block.of("~ ~ ~").nbt.;
}
"#;
        let analysis = analyze_source(source);

        let pig_items = completion_items(
            source,
            &analysis,
            source.find("pig.nbt.").unwrap() + "pig.nbt.".len(),
        );
        assert!(pig_items.iter().any(|item| item.label == "CustomName"));

        let inline_entity_items = completion_items(
            source,
            &analysis,
            source
                .find("Selector.of(\"@e[type=pig,limit=1]\").getFirst().nbt.")
                .unwrap()
                + "Selector.of(\"@e[type=pig,limit=1]\").getFirst().nbt.".len(),
        );
        assert!(
            inline_entity_items
                .iter()
                .any(|item| item.label == "CustomName")
        );

        let player_items = completion_items(
            source,
            &analysis,
            source.find("player.nbt.").unwrap() + "player.nbt.".len(),
        );
        assert!(player_items.iter().any(|item| item.label == "Air"));

        let chest_items = completion_items(
            source,
            &analysis,
            source.find("chest.nbt.").unwrap() + "chest.nbt.".len(),
        );
        assert!(chest_items.iter().any(|item| item.label == "lock"));

        let inline_block_items = completion_items(
            source,
            &analysis,
            source.find("Block.of(\"~ ~ ~\").nbt.").unwrap() + "Block.of(\"~ ~ ~\").nbt.".len(),
        );
        assert!(inline_block_items.iter().any(|item| item.label == "lock"));
    }

    #[test]
    fn completes_full_upstream_nbt_for_additional_exact_ids() {
        let source = r#"
void main() {
    new EntityData("minecraft:armor_stand").nbt.;
    new ItemStack("minecraft:diamond_sword").nbt.;
}
"#;
        let analysis = analyze_source(source);

        let armor_stand_items = completion_items(
            source,
            &analysis,
            source
                .find("EntityData(\"minecraft:armor_stand\").nbt.")
                .unwrap()
                + "EntityData(\"minecraft:armor_stand\").nbt.".len(),
        );
        assert!(
            armor_stand_items
                .iter()
                .any(|item| item.label == "equipment")
        );
        assert!(
            armor_stand_items
                .iter()
                .any(|item| item.label == "ShowArms")
        );

        let sword_items = completion_items(
            source,
            &analysis,
            source
                .find("ItemStack(\"minecraft:diamond_sword\").nbt.")
                .unwrap()
                + "ItemStack(\"minecraft:diamond_sword\").nbt.".len(),
        );
        assert!(sword_items.iter().any(|item| item.label == "Damage"));
        assert!(sword_items.iter().any(|item| item.label == "Enchantments"));
    }

    #[test]
    fn falls_back_to_default_nbt_schema_for_dynamic_builder_ids() {
        let source = r#"
void main() {
    var id = "minecraft:unknown";
    var entity_value = new EntityData(id);
    var block_value = new BlockData(id);
    var item_value = new ItemStack(id);
    entity_value.nbt.;
    block_value.nbt.;
    item_value.nbt.;
}
"#;
        let analysis = analyze_source(source);

        let entity_items = completion_items(
            source,
            &analysis,
            source.find("entity_value.nbt.").unwrap() + "entity_value.nbt.".len(),
        );
        assert!(entity_items.iter().any(|item| item.label == "CustomName"));
        assert!(!entity_items.iter().any(|item| item.label == "profile"));

        let block_items = completion_items(
            source,
            &analysis,
            source.find("block_value.nbt.").unwrap() + "block_value.nbt.".len(),
        );
        assert!(block_items.iter().any(|item| item.label == "lock"));

        let item_items = completion_items(
            source,
            &analysis,
            source.find("item_value.nbt.").unwrap() + "item_value.nbt.".len(),
        );
        assert!(item_items.iter().any(|item| item.label == "display"));
    }

    #[test]
    fn replaces_partial_nbt_field_names() {
        let source = r#"
void main() {
    new EntityData("minecraft:mannequin").nbt.pro;
}
"#;
        let analysis = analyze_source(source);
        let items = completion_items(
            source,
            &analysis,
            source.find(".nbt.pro").unwrap() + ".nbt.pro".len(),
        );
        let profile = items
            .iter()
            .find(|item| item.label == "profile")
            .expect("profile completion");
        let Some(CompletionTextEdit::Edit(edit)) = profile.text_edit.clone() else {
            panic!("profile completion should use a text edit");
        };
        assert_eq!(edit.new_text, "profile");
        assert_eq!(
            edit.range.start,
            offset_to_position(source, source.find("pro").unwrap())
        );
        assert_eq!(
            edit.range.end,
            offset_to_position(source, source.find("pro").unwrap() + "pro".len())
        );
    }

    #[test]
    fn completes_contextual_minecraft_ids_inside_string_arguments() {
        let source = r#"
void main() {
    var player = (Player) Selector.of("@a").getFirst();
    new EntityData("pig");
    new ItemStack("diamond_swo");
    new ItemStack("music_disc_boun");
    Block.of("~ ~ ~").setBlock("gold_bloc");
    player.playSound("entity.experience_orb.picku", "master");
    player.playSound("block.sulfur_spike.brea", "master");
    Block.of("~ ~ ~").spawnParticle("happy_villag");
    Block.of("~ ~ ~").spawnParticle("geyser_poo");
    player.effect("glowin", 3, 0);
    player.give("stick", 1);
    player.position.lootSpawn("chests/simple_dungeo");
}
"#;
        let analysis = analyze_source(source);

        let entity_items = completion_items(
            source,
            &analysis,
            source.find("EntityData(\"pig").unwrap() + "EntityData(\"pig".len(),
        );
        assert!(
            entity_items
                .iter()
                .any(|item| item.label == "minecraft:pig")
        );

        let item_items = completion_items(
            source,
            &analysis,
            source.find("ItemStack(\"diamond_swo").unwrap() + "ItemStack(\"diamond_swo".len(),
        );
        assert!(
            item_items
                .iter()
                .any(|item| item.label == "minecraft:diamond_sword")
        );
        let new_item_items = completion_items(
            source,
            &analysis,
            source.find("ItemStack(\"music_disc_boun").unwrap()
                + "ItemStack(\"music_disc_boun".len(),
        );
        assert!(
            new_item_items
                .iter()
                .any(|item| item.label == "minecraft:music_disc_bounce")
        );

        let block_items = completion_items(
            source,
            &analysis,
            source.find("setBlock(\"gold_bloc").unwrap() + "setBlock(\"gold_bloc".len(),
        );
        assert!(
            block_items
                .iter()
                .any(|item| item.label == "minecraft:gold_block")
        );

        let sound_items = completion_items(
            source,
            &analysis,
            source
                .find("playSound(\"entity.experience_orb.picku")
                .unwrap()
                + "playSound(\"entity.experience_orb.picku".len(),
        );
        assert!(
            sound_items
                .iter()
                .any(|item| item.label == "minecraft:entity.experience_orb.pickup")
        );
        let new_sound_items = completion_items(
            source,
            &analysis,
            source.find("playSound(\"block.sulfur_spike.brea").unwrap()
                + "playSound(\"block.sulfur_spike.brea".len(),
        );
        assert!(
            new_sound_items
                .iter()
                .any(|item| item.label == "minecraft:block.sulfur_spike.break")
        );

        let spawn_particle_items = completion_items(
            source,
            &analysis,
            source.find("spawnParticle(\"happy_villag").unwrap()
                + "spawnParticle(\"happy_villag".len(),
        );
        assert!(
            spawn_particle_items
                .iter()
                .any(|item| item.label == "minecraft:happy_villager")
        );
        let new_spawn_particle_items = completion_items(
            source,
            &analysis,
            source.find("spawnParticle(\"geyser_poo").unwrap() + "spawnParticle(\"geyser_poo".len(),
        );
        assert!(
            new_spawn_particle_items
                .iter()
                .any(|item| item.label == "minecraft:geyser_poof")
        );

        let effect_items = completion_items(
            source,
            &analysis,
            source.find("effect(\"glowin").unwrap() + "effect(\"glowin".len(),
        );
        assert!(
            effect_items
                .iter()
                .any(|item| item.label == "minecraft:glowing")
        );

        let give_items = completion_items(
            source,
            &analysis,
            source.find("give(\"stick").unwrap() + "give(\"stick".len(),
        );
        assert!(
            give_items
                .iter()
                .any(|item| item.label == "minecraft:stick")
        );

        let loot_items = completion_items(
            source,
            &analysis,
            source.find("lootSpawn(\"chests/simple_dungeo").unwrap()
                + "lootSpawn(\"chests/simple_dungeo".len(),
        );
        assert!(
            loot_items
                .iter()
                .any(|item| item.label == "minecraft:chests/simple_dungeon")
        );
    }

    #[test]
    fn completes_minecraft_ids_for_unterminated_strings_and_item_assignments() {
        let assignment_source = r#"
void main() {
    var player = (Player) Selector.of("@a").getFirst();
    player.mainhand.item = "carrot_on_a_stic
"#;
        let assignment_analysis = analyze_source(assignment_source);
        assert!(!assignment_analysis.diagnostics.is_empty());

        let equipment_items = completion_items(
            assignment_source,
            &assignment_analysis,
            assignment_source.find("\"carrot_on_a_stic").unwrap() + "\"carrot_on_a_stic".len(),
        );
        assert!(
            equipment_items
                .iter()
                .any(|item| item.label == "minecraft:carrot_on_a_stick")
        );

        let entity_source = r#"
void main() {
    new EntityData("chicke
"#;
        let entity_analysis = analyze_source(entity_source);
        assert!(!entity_analysis.diagnostics.is_empty());

        let entity_items = completion_items(
            entity_source,
            &entity_analysis,
            entity_source.rfind("\"chicke").unwrap() + "\"chicke".len(),
        );
        assert!(
            entity_items
                .iter()
                .any(|item| item.label == "minecraft:chicken")
        );
    }

    #[test]
    fn completes_selector_entity_ids_and_top_level_debug_marker_block_ids() {
        let source = r#"
void main() {
    var matching = Selector.of("@e[type=chicke,limit=1]");
    var negated = Selector.of("@e[type=!zomb,limit=1]");
    debugMarker(Block.of("~ ~ ~"), "marker", "gold_bloc");
}
"#;
        let analysis = analyze_source(source);

        let selector_items = completion_items(
            source,
            &analysis,
            source.find("type=chicke").unwrap() + "type=chicke".len(),
        );
        assert!(
            selector_items
                .iter()
                .any(|item| item.label == "minecraft:chicken")
        );

        let negated_items = completion_items(
            source,
            &analysis,
            source.find("type=!zomb").unwrap() + "type=!zomb".len(),
        );
        let zombie = negated_items
            .iter()
            .find(|item| item.label == "minecraft:zombie")
            .expect("minecraft:zombie completion");
        let Some(CompletionTextEdit::Edit(edit)) = zombie.text_edit.clone() else {
            panic!("minecraft:zombie completion should use a text edit");
        };
        assert_eq!(edit.new_text, "minecraft:zombie");
        assert_eq!(
            edit.range.start,
            offset_to_position(source, source.find("zomb").unwrap())
        );
        assert_eq!(
            edit.range.end,
            offset_to_position(source, source.find("zomb").unwrap() + "zomb".len())
        );

        let debug_marker_items = completion_items(
            source,
            &analysis,
            source.find("\"gold_bloc").unwrap() + "\"gold_bloc".len(),
        );
        assert!(
            debug_marker_items
                .iter()
                .any(|item| item.label == "minecraft:gold_block")
        );
    }

    #[test]
    fn does_not_offer_item_id_completions_for_read_only_item_slot_ids() {
        let source = r#"
void main() {
    var player = (Player) Selector.of("@a").getFirst();
    player.hotbar[0].id = "stick";
}
"#;
        let analysis = analyze_source(source);
        let items = completion_items(
            source,
            &analysis,
            source.find("\"stick").unwrap() + "\"stick".len(),
        );
        assert!(!items.iter().any(|item| item.label == "minecraft:stick"));
    }

    #[test]
    fn minecraft_id_completions_replace_the_full_string_contents() {
        let source = "void main() {\n    new EntityData(\"pig\");\n}\n";
        let analysis = analyze_source(source);
        let items = completion_items(
            source,
            &analysis,
            source.find("EntityData(\"pig").unwrap() + "EntityData(\"pig".len(),
        );
        let pig = items
            .iter()
            .find(|item| item.label == "minecraft:pig")
            .expect("minecraft:pig completion");
        let Some(CompletionTextEdit::Edit(edit)) = pig.text_edit.clone() else {
            panic!("minecraft:pig completion should use a text edit");
        };
        assert_eq!(edit.new_text, "minecraft:pig");
        assert_eq!(
            edit.range.start,
            offset_to_position(source, source.find("pig").unwrap())
        );
        assert_eq!(
            edit.range.end,
            offset_to_position(source, source.find("pig").unwrap() + "pig".len())
        );
    }

    #[test]
    fn resolves_project_config_only_for_files_under_source_dir() {
        let base = temp_path();
        let project = base.join("project");
        let src_dir = project.join("src").join("nested");
        let asset_dir = project.join("assets");
        fs::create_dir_all(&src_dir).unwrap();
        fs::create_dir_all(&asset_dir).unwrap();
        write_file(
            &project.join("sample.mcfc.toml"),
            "namespace = \"sample\"\nsource_dir = \"src\"\nasset_dir = \"assets\"\n",
        );
        let source_file = src_dir.join("main.mcf");
        let asset_file = asset_dir.join("ignored.mcf");
        write_file(&source_file, "void main() {\n}\n");
        write_file(&asset_file, "void ignored() {\n}\n");

        let source_config = resolve_project_config_for_path(&source_file)
            .unwrap()
            .expect("source file should resolve to project");
        assert_eq!(
            source_config.manifest_path,
            project.join("sample.mcfc.toml")
        );
        assert_eq!(source_config.source_root, project.join("src"));

        assert!(
            resolve_project_config_for_path(&asset_file)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn maps_project_ranges_between_merged_and_local_offsets() {
        let base = temp_path();
        let project = base.join("project");
        let src_dir = project.join("src");
        fs::create_dir_all(&src_dir).unwrap();
        write_file(
            &project.join("sample.mcfc.toml"),
            "namespace = \"sample\"\nsource_dir = \"src\"\n",
        );
        let first = src_dir.join("main.mcf");
        let second = src_dir.join("beta.mcf");
        write_file(&first, "public void alpha() {\n}\n");
        write_file(
            &second,
            "import main.alpha;\nvoid beta() {\n    alpha();\n}",
        );

        let snapshot = build_project_snapshot(
            &ProjectConfig {
                manifest_path: project.join("sample.mcfc.toml"),
                source_root: src_dir.clone(),
                host_modules: crate::types::HostModules::for_editor(),
            },
            &HashMap::new(),
        )
        .expect("snapshot should build");
        let segment = snapshot
            .segment_for_path(&second)
            .expect("second file should have segment");
        let local_call_offset = fs::read_to_string(&second).unwrap().find("alpha").unwrap();
        let merged_call_offset = segment.local_to_merged_offset(local_call_offset);
        let range = TextRange::new(merged_call_offset, merged_call_offset + "alpha".len());
        let local_range = segment
            .merged_to_local_range(range)
            .expect("merged range should map back");

        assert_eq!(local_range.start, local_call_offset);
        assert_eq!(local_range.end, local_call_offset + "alpha".len());
    }

    #[test]
    fn multi_file_project_supports_cross_file_diagnostics_hover_completion_and_symbols() {
        let base = temp_path();
        let project = base.join("project");
        let src_dir = project.join("src");
        fs::create_dir_all(&src_dir).unwrap();
        write_file(
            &project.join("sample.mcfc.toml"),
            "namespace = \"sample\"\nsource_dir = \"src\"\n",
        );
        let helper = src_dir.join("helper.mcf");
        let main = src_dir.join("main.mcf");
        write_file(
            &helper,
            r#"
public record Action(String kind) {}
public void helper() {
    return;
}
"#,
        );
        let main_source = r#"import helper.helper;
import helper.Action;

void main() {
    helper();
}
"#;
        write_file(&main, main_source);

        let snapshot = build_project_snapshot(
            &ProjectConfig {
                manifest_path: project.join("sample.mcfc.toml"),
                source_root: src_dir.clone(),
                host_modules: crate::types::HostModules::for_editor(),
            },
            &HashMap::new(),
        )
        .expect("snapshot should build");
        let main_segment = snapshot
            .segment_for_path(&main)
            .expect("main file should have segment");
        let main_text = fs::read_to_string(&main).unwrap();
        let local_call_offset = main_text.find("    helper()").unwrap() + 4;
        let merged_call_offset = main_segment.local_to_merged_offset(local_call_offset);
        let (word, _) = crate::analysis::word_at_offset(&snapshot.merged_text, merged_call_offset)
            .expect("word at helper call");
        let hover = super::hover_contents(&snapshot.analysis, merged_call_offset, &word)
            .expect("hover should resolve cross-file function");
        assert!(hover.contains("void helper::helper()"));

        let top_level_items = completion_items(
            &snapshot.merged_text,
            &snapshot.analysis,
            merged_call_offset,
        );
        assert!(
            top_level_items
                .iter()
                .any(|item| item.label == "helper::helper")
        );
        assert!(
            top_level_items
                .iter()
                .any(|item| item.label == "helper::Action")
        );

        let main_symbols = project_document_symbols(&main_text, &snapshot.analysis, main_segment);
        assert!(main_symbols.iter().any(|symbol| symbol.name == "main"));
        assert!(
            !main_symbols
                .iter()
                .any(|symbol| symbol.name.ends_with("helper"))
        );

        let mut overrides = HashMap::new();
        overrides.insert(helper.clone(), String::new());
        let broken_snapshot = build_project_snapshot(
            &ProjectConfig {
                manifest_path: project.join("sample.mcfc.toml"),
                source_root: src_dir,
                host_modules: crate::types::HostModules::for_editor(),
            },
            &overrides,
        )
        .expect("snapshot with override should build");
        let broken_main_segment = broken_snapshot
            .segment_for_path(&main)
            .expect("main file should still have segment");
        let diagnostics = project_diagnostics_for_segment(
            &main_text,
            broken_main_segment,
            &broken_snapshot.analysis,
        );
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("cannot find 'helper'"))
        );
    }

    #[test]
    fn completes_java_static_and_selector_calls() {
        let source = "void main() {\n    var sel = Selector.of(\"@e\");\n    sel.;\n}";
        let analysis = analyze_source(source);
        let selector = completion_items(source, &analysis, source.find("sel.").unwrap() + 4);
        assert!(selector.iter().any(|item| item.label == "getFirst"));
        assert!(selector.iter().any(|item| item.label == "findFirst"));
        assert!(!selector.iter().any(|item| item.label == "single"));

        let math = completion_items("Math.", &analyze_source("Math."), 5);
        assert!(math.iter().any(|item| item.label == "sqrt"));
        let integer = completion_items("Integer.", &analyze_source("Integer."), 8);
        assert!(integer.iter().any(|item| item.label == "parseInt"));
        let string = completion_items("String.", &analyze_source("String."), 7);
        assert!(string.iter().any(|item| item.label == "valueOf"));
    }

    #[test]
    fn completes_results_of_conditional_and_switch_expressions() {
        let source = r#"
enum Stage { NEW, DONE }
void main() {
    var label = true ? "yes" : "no";
    var code = switch (Stage.NEW) {
        case NEW -> "new";
        case DONE -> "done";
    };
    var stage = Stage.NEW;
    debug(label.toString());
    debug(code.toString());
    debug(stage.name());
}
"#;
        let analysis = analyze_source(source);
        assert!(
            analysis.typed_program.is_some(),
            "{:?}",
            analysis.diagnostics
        );
        for name in ["label.", "code."] {
            let offset = source.find(name).unwrap() + name.len();
            let items = completion_items(source, &analysis, offset);
            assert!(
                items.iter().any(|item| item.label == "startsWith"),
                "{name}"
            );
        }
        let stage = completion_items(
            source,
            &analysis,
            source.find("stage.name").unwrap() + "stage.".len(),
        );
        assert!(stage.iter().any(|item| item.label == "ordinal"));
    }
}
