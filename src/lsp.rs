use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::RwLock;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, CompletionOptions, CompletionParams, CompletionResponse,
    Diagnostic as LspDiagnostic, DiagnosticSeverity, DidChangeTextDocumentParams,
    DidCloseTextDocumentParams, DidOpenTextDocumentParams, DidSaveTextDocumentParams,
    DocumentSymbol, DocumentSymbolParams, DocumentSymbolResponse, Hover, HoverContents,
    HoverParams, InitializeParams, InitializeResult, InitializedParams, MarkedString, OneOf,
    Position, Range, ServerCapabilities, TextDocumentSyncCapability, TextDocumentSyncKind, Url,
};
use tower_lsp::{Client, LanguageServer};

use crate::analysis::{AnalysisResult, analyze_source, function_at_offset, word_at_offset};
use crate::ast::Type;
use crate::diagnostics::{Diagnostic as McfcDiagnostic, TextRange};
use crate::project::{collect_source_files, find_manifest_in_ancestors, load_manifest};
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
}

#[derive(Debug, Clone)]
struct ProjectConfig {
    manifest_path: PathBuf,
    source_root: PathBuf,
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
        if range.start < self.source_start || range.end > self.source_end {
            return None;
        }

        Some(TextRange::new(
            range.start - self.source_start,
            range.end - self.source_start,
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

        if let Some(config) = resolve_project_config_for_uri(&uri) {
            if self.rebuild_project(config).await.is_ok() {
                return;
            }
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
            .map_err(|error| tower_lsp::jsonrpc::Error::invalid_params(error))
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
                    trigger_characters: Some(vec![".".to_string()]),
                    ..CompletionOptions::default()
                }),
                document_symbol_provider: Some(OneOf::Left(true)),
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

        if let Some(config) = resolve_project_config_for_uri(&uri) {
            if self.rebuild_project(config).await.is_ok() {
                return;
            }
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
            None => static_completion_items(false, None),
        };
        Ok(Some(CompletionResponse::Array(items)))
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
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
    let files = collect_source_files(&config.source_root)?;
    let mut merged_text = String::new();
    let mut segments = HashMap::new();

    for file in files {
        let source = match overrides.get(&file) {
            Some(source) => source.clone(),
            None => fs::read_to_string(&file)
                .map_err(|error| format!("failed to read '{}': {}", file.display(), error))?,
        };
        merged_text.push_str(&format!("# source: {}\n", file.display()));
        let source_start = merged_text.len();
        merged_text.push_str(&source);
        if !source.ends_with('\n') {
            merged_text.push('\n');
        }
        let source_end = merged_text.len();
        merged_text.push('\n');

        segments.insert(
            file.clone(),
            ProjectFileSegment {
                source_start,
                source_end,
            },
        );
    }

    if segments.is_empty() {
        return Err(format!(
            "no '.mcf' files found under '{}'",
            config.source_root.display()
        ));
    }

    Ok(ProjectSnapshot {
        manifest_path: config.manifest_path.clone(),
        source_root: config.source_root.clone(),
        analysis: analyze_source(&merged_text),
        merged_text,
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
    symbols.extend(analysis.functions.iter().map(|function| DocumentSymbol {
        name: function.name.clone(),
        detail: Some(function.signature()),
        kind: tower_lsp::lsp_types::SymbolKind::FUNCTION,
        tags: None,
        deprecated: None,
        range: range_from_text_range(source, function.range),
        selection_range: range_from_text_range(source, function.name_range),
        children: None,
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
    }

    for function in &analysis.functions {
        let Some(range) = segment.merged_to_local_range(function.range) else {
            continue;
        };
        let Some(name_range) = segment.merged_to_local_range(function.name_range) else {
            continue;
        };
        symbols.push(DocumentSymbol {
            name: function.name.clone(),
            detail: Some(function.signature()),
            kind: tower_lsp::lsp_types::SymbolKind::FUNCTION,
            tags: None,
            deprecated: None,
            range: range_from_text_range(local_text, range),
            selection_range: range_from_text_range(local_text, name_range),
            children: None,
        });
    }

    symbols
}

fn hover_contents(analysis: &AnalysisResult, offset: usize, word: &str) -> Option<String> {
    if let Some(struct_defs) = analysis
        .typed_program
        .as_ref()
        .map(|program| &program.struct_defs)
    {
        if let Some(def) = struct_defs.get(word) {
            return Some(format!("```mcfc\n{}\n```", struct_signature(word, def)));
        }
    }

    if let Some(function) = analysis
        .functions
        .iter()
        .find(|function| function.name == word)
    {
        let mut hover = format!("```mcfc\n{}\n```", function.signature());
        if function.book_exposed {
            hover.push_str("\n@book command");
        }
        return Some(hover);
    }

    if let Some(function) = function_at_offset(analysis, offset) {
        if let Some(local) = analysis
            .locals
            .iter()
            .find(|local| local.function == function.name && local.name == word)
        {
            return Some(format!(
                "```mcfc\n{}: {}\n```",
                local.name,
                local.ty.as_str()
            ));
        }
    }

    builtin_hover(word).map(str::to_string)
}

fn builtin_hover(word: &str) -> Option<&'static str> {
    match word {
        "struct" => Some("```mcfc\nstruct Name:\n    field: Type\nend\n```"),
        "match" => Some(
            "```mcfc\nmatch value:\n    \"pattern\" =>\n        ...\n    else =>\n        ...\nend\n```",
        ),
        "mcf" => Some("```mcfc\nmcf \"say $(expr)\"\n```"),
        "selector" => Some("```mcfc\nselector(value: string) -> entity_set\n```"),
        "single" => Some("```mcfc\nsingle(value: entity_set) -> entity_ref\n```"),
        "exists" => Some("```mcfc\nexists(value: entity_ref) -> bool\n```"),
        "has_data" => Some("```mcfc\nhas_data(value: storage_path) -> bool\n```"),
        "block" => Some("```mcfc\nblock(position: string) -> block_ref\n```"),
        "at" => Some(
            "```mcfc\nat(anchor: entity_ref, value: entity_set|entity_ref|block_ref) -> entity_set|entity_ref|block_ref\n\nat(anchor):\n    ...\nend\n```",
        ),
        "as" => Some(
            "```mcfc\nas(anchor: entity_set|entity_ref, value: entity_set|entity_ref|block_ref) -> entity_set|entity_ref|block_ref\n\nas(anchor):\n    ...\nend\n```",
        ),
        "int" => Some("```mcfc\nint(value: nbt) -> int\n```"),
        "bool" => Some("```mcfc\nbool(value: nbt) -> bool\n```"),
        "string" => Some("```mcfc\nstring(value: nbt) -> string\n```"),
        "summon" => Some(
            "```mcfc\nsummon(entity_id: string) -> entity_ref\nsummon(entity_id: string, data: nbt) -> entity_ref\n```",
        ),
        "teleport" => Some(
            "```mcfc\nteleport(target: entity_ref|entity_set, destination: entity_ref|block_ref) -> void\n```",
        ),
        "damage" => {
            Some("```mcfc\ndamage(target: entity_ref|entity_set, amount: int) -> void\n```")
        }
        "heal" => Some("```mcfc\nheal(target: entity_ref, amount: int) -> void\n```"),
        "give" => Some(
            "```mcfc\ngive(target: entity_ref|entity_set, item_id: string, count: int) -> void\n```",
        ),
        "clear" => Some(
            "```mcfc\nclear(target: entity_ref|entity_set, item_id: string, count: int) -> void\n```",
        ),
        "loot_give" => {
            Some("```mcfc\nloot_give(target: entity_ref|entity_set, table: string) -> void\n```")
        }
        "loot_insert" => {
            Some("```mcfc\nloot_insert(container: block_ref, table: string) -> void\n```")
        }
        "loot_spawn" => {
            Some("```mcfc\nloot_spawn(position: block_ref, table: string) -> void\n```")
        }
        "tellraw" => {
            Some("```mcfc\ntellraw(target: entity_ref|entity_set, message: string) -> void\n```")
        }
        "title" => {
            Some("```mcfc\ntitle(target: entity_ref|entity_set, message: string) -> void\n```")
        }
        "actionbar" => {
            Some("```mcfc\nactionbar(target: entity_ref|entity_set, message: string) -> void\n```")
        }
        "debug" => Some("```mcfc\ndebug(message: string) -> void\n```"),
        "debug_marker" => Some(
            "```mcfc\ndebug_marker(position: block_ref, label: string) -> void\ndebug_marker(position: block_ref, label: string, marker_block: string) -> void\n```",
        ),
        "debug_entity" => {
            Some("```mcfc\ndebug_entity(target: entity_ref|entity_set, label: string) -> void\n```")
        }
        "bossbar_add" => Some("```mcfc\nbossbar_add(id: string, name: string) -> void\n```"),
        "bossbar_remove" => Some("```mcfc\nbossbar_remove(id: string) -> void\n```"),
        "bossbar_name" => Some("```mcfc\nbossbar_name(id: string, name: string) -> void\n```"),
        "bossbar_value" => Some("```mcfc\nbossbar_value(id: string, value: int) -> void\n```"),
        "bossbar_max" => Some("```mcfc\nbossbar_max(id: string, max: int) -> void\n```"),
        "bossbar_visible" => {
            Some("```mcfc\nbossbar_visible(id: string, visible: bool) -> void\n```")
        }
        "bossbar_players" => Some(
            "```mcfc\nbossbar_players(id: string, targets: entity_ref|entity_set) -> void\n```",
        ),
        "playsound" => Some(
            "```mcfc\nplaysound(sound: string, category: string, target: entity_ref|entity_set) -> void\n```",
        ),
        "stopsound" => Some(
            "```mcfc\nstopsound(target: entity_ref|entity_set, category: string, sound: string) -> void\n```",
        ),
        "particle" => Some(
            "```mcfc\nparticle(name: string, position: block_ref) -> void\nparticle(name: string, position: block_ref, count: int) -> void\nparticle(name: string, position: block_ref, count: int, viewers: entity_ref|entity_set) -> void\n```",
        ),
        "setblock" => Some("```mcfc\nsetblock(position: block_ref, block_id: string) -> void\n```"),
        "fill" => {
            Some("```mcfc\nfill(from: block_ref, to: block_ref, block_id: string) -> void\n```")
        }
        "len" => Some("```mcfc\narray<T>.len() -> int\n```"),
        "push" => Some("```mcfc\narray<T>.push(value: T) -> void\n```"),
        "pop" => Some("```mcfc\narray<T>.pop() -> T\n```"),
        "remove_at" => Some("```mcfc\narray<T>.remove_at(index: int) -> T\n```"),
        "has" => Some("```mcfc\ndict<T>.has(key: string) -> bool\n```"),
        "remove" => Some("```mcfc\ndict<T>.remove(key: string) -> void\n```"),
        "effect" => {
            Some("```mcfc\nentity.effect(name: string, duration: int, amplifier: int) -> void\n```")
        }
        "add_tag" => Some("```mcfc\nentity.add_tag(name: string) -> void\n```"),
        "remove_tag" => Some("```mcfc\nentity.remove_tag(name: string) -> void\n```"),
        "has_tag" => Some("```mcfc\nentity.has_tag(name: string) -> bool\n```"),
        _ => None,
    }
}

fn completion_items(source: &str, analysis: &AnalysisResult, offset: usize) -> Vec<CompletionItem> {
    if let Some(chain) = member_chain_before_cursor(source, offset) {
        return member_completion_items(source, analysis, offset, &chain);
    }

    let containing_function =
        function_at_offset(analysis, offset).map(|function| function.name.as_str());
    let mut items = static_completion_items(false, containing_function);
    items.extend(struct_type_items(analysis));

    for function in &analysis.functions {
        items.push(CompletionItem {
            label: function.name.clone(),
            kind: Some(CompletionItemKind::FUNCTION),
            detail: Some(function.signature()),
            insert_text: Some(format!("{}($0)", function.name)),
            insert_text_format: Some(tower_lsp::lsp_types::InsertTextFormat::SNIPPET),
            ..CompletionItem::default()
        });
    }

    let mut seen_locals = HashSet::new();
    if let Some(function_name) = containing_function {
        for local in analysis
            .locals
            .iter()
            .filter(|local| local.function == function_name)
        {
            seen_locals.insert(local.name.clone());
            items.push(CompletionItem {
                label: local.name.clone(),
                kind: Some(CompletionItemKind::VARIABLE),
                detail: Some(local.ty.as_str()),
                ..CompletionItem::default()
            });
        }
    }

    for local in syntactic_locals_at_offset(source, offset) {
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

fn static_completion_items(
    after_dot: bool,
    _containing_function: Option<&str>,
) -> Vec<CompletionItem> {
    if after_dot {
        return [
            ("len", "array<T>.len() -> int", "len()"),
            (
                "push",
                "array<T>.push(value: T) -> void",
                "push(${1:value})",
            ),
            ("pop", "array<T>.pop() -> T", "pop()"),
            (
                "remove_at",
                "array<T>.remove_at(index: int) -> T",
                "remove_at(${1:index})",
            ),
            ("has", "dict<T>.has(key: string) -> bool", "has(${1:key})"),
            (
                "remove",
                "dict<T>.remove(key: string) -> void",
                "remove(${1:key})",
            ),
            (
                "effect",
                "entity.effect(name: string, duration: int, amplifier: int) -> void",
                "effect(${1:name}, ${2:duration}, ${3:amplifier})",
            ),
            (
                "add_tag",
                "entity.add_tag(name: string) -> void",
                "add_tag(${1:name})",
            ),
            (
                "remove_tag",
                "entity.remove_tag(name: string) -> void",
                "remove_tag(${1:name})",
            ),
            (
                "has_tag",
                "entity.has_tag(name: string) -> bool",
                "has_tag(${1:name})",
            ),
            ("nbt", "player.nbt.* read namespace", "nbt"),
            ("state", "player.state.* read/write namespace", "state"),
            ("tags", "player.tags.* read/write namespace", "tags"),
            ("team", "entity.team writable string", "team"),
            (
                "mainhand",
                "entity.mainhand.* writable namespace",
                "mainhand",
            ),
            ("offhand", "entity.offhand.* writable namespace", "offhand"),
            ("head", "entity.head.* writable namespace", "head"),
            ("chest", "entity.chest.* writable namespace", "chest"),
            ("legs", "entity.legs.* writable namespace", "legs"),
            ("feet", "entity.feet.* writable namespace", "feet"),
        ]
        .into_iter()
        .map(|(label, detail, insert_text)| {
            snippet_item(label, CompletionItemKind::METHOD, detail, insert_text)
        })
        .collect();
    }

    let mut items = Vec::new();
    for keyword in [
        "fn", "struct", "let", "return", "end", "if", "match", "else", "while", "for", "in",
        "break", "continue", "mc", "mcf", "true", "false", "and", "or", "not", "@book",
    ] {
        items.push(CompletionItem {
            label: keyword.to_string(),
            kind: Some(CompletionItemKind::KEYWORD),
            ..CompletionItem::default()
        });
    }

    for (label, insert_text) in [
        ("int", "int"),
        ("bool", "bool"),
        ("string", "string"),
        ("array<>", "array<${1:int}>"),
        ("dict<>", "dict<${1:int}>"),
        ("entity_set", "entity_set"),
        ("entity_ref", "entity_ref"),
        ("block_ref", "block_ref"),
        ("nbt", "nbt"),
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
            "struct … end",
            "Define a named struct with typed fields",
            "struct ${1:Name}:\n\t${2:field}: ${3:int}\nend",
        ),
        (
            "match … end",
            "Dispatch on a string value",
            "match ${1:value}:\n\t\"${2:pattern}\" =>\n\t\t$0\n\telse =>\n\t\t\nend",
        ),
        (
            "selector",
            "selector(value: string) -> entity_set",
            "selector(${1:\"@e\"})",
        ),
        (
            "single",
            "single(value: entity_set) -> entity_ref",
            "single(${1:value})",
        ),
        (
            "exists",
            "exists(value: entity_ref) -> bool",
            "exists(${1:value})",
        ),
        (
            "has_data",
            "has_data(value: storage_path) -> bool",
            "has_data(${1:value})",
        ),
        (
            "block",
            "block(position: string) -> block_ref",
            "block(${1:\"~ ~ ~\"})",
        ),
        (
            "at",
            "at(anchor: entity_ref, value: entity_set|entity_ref|block_ref)",
            "at(${1:anchor}, ${2:value})",
        ),
        (
            "at(…): … end",
            "at(anchor): … end — run commands at an entity/block",
            "at(${1:anchor}):\n\t$0\nend",
        ),
        (
            "as",
            "as(anchor: entity_set|entity_ref, value: entity_set|entity_ref|block_ref)",
            "as(${1:anchor}, ${2:value})",
        ),
        (
            "as(…): … end",
            "as(anchor): … end — run commands as an entity",
            "as(${1:anchor}):\n\t$0\nend",
        ),
        ("int", "int(value: nbt) -> int", "int(${1:value})"),
        ("bool", "bool(value: nbt) -> bool", "bool(${1:value})"),
        (
            "string",
            "string(value: nbt) -> string",
            "string(${1:value})",
        ),
        (
            "summon",
            "summon(entity_id: string) -> entity_ref",
            "summon(${1:\"minecraft:pig\"})",
        ),
        (
            "teleport",
            "teleport(target: entity_ref|entity_set, destination: entity_ref|block_ref) -> void",
            "teleport(${1:target}, ${2:destination})",
        ),
        (
            "damage",
            "damage(target: entity_ref|entity_set, amount: int) -> void",
            "damage(${1:target}, ${2:amount})",
        ),
        (
            "heal",
            "heal(target: entity_ref, amount: int) -> void",
            "heal(${1:target}, ${2:amount})",
        ),
        (
            "give",
            "give(target: entity_ref|entity_set, item_id: string, count: int) -> void",
            "give(${1:target}, ${2:\"minecraft:stone\"}, ${3:1})",
        ),
        (
            "clear",
            "clear(target: entity_ref|entity_set, item_id: string, count: int) -> void",
            "clear(${1:target}, ${2:\"minecraft:stone\"}, ${3:1})",
        ),
        (
            "loot_give",
            "loot_give(target: entity_ref|entity_set, table: string) -> void",
            "loot_give(${1:target}, ${2:\"minecraft:chests/simple_dungeon\"})",
        ),
        (
            "loot_insert",
            "loot_insert(container: block_ref, table: string) -> void",
            "loot_insert(${1:container}, ${2:\"minecraft:chests/simple_dungeon\"})",
        ),
        (
            "loot_spawn",
            "loot_spawn(position: block_ref, table: string) -> void",
            "loot_spawn(${1:position}, ${2:\"minecraft:chests/simple_dungeon\"})",
        ),
        (
            "tellraw",
            "tellraw(target: entity_ref|entity_set, message: string) -> void",
            "tellraw(${1:target}, ${2:\"hello\"})",
        ),
        (
            "title",
            "title(target: entity_ref|entity_set, message: string) -> void",
            "title(${1:target}, ${2:\"hello\"})",
        ),
        (
            "actionbar",
            "actionbar(target: entity_ref|entity_set, message: string) -> void",
            "actionbar(${1:target}, ${2:\"hello\"})",
        ),
        (
            "debug",
            "debug(message: string) -> void",
            "debug(${1:\"reached checkpoint\"})",
        ),
        (
            "debug_marker",
            "debug_marker(position: block_ref, label: string) -> void",
            "debug_marker(${1:block(\"~ ~ ~\")}, ${2:\"checkpoint\"})",
        ),
        (
            "debug_entity",
            "debug_entity(target: entity_ref|entity_set, label: string) -> void",
            "debug_entity(${1:target}, ${2:\"target\"})",
        ),
        (
            "bossbar_add",
            "bossbar_add(id: string, name: string) -> void",
            "bossbar_add(${1:\"mcfc:boss\"}, ${2:\"Boss\"})",
        ),
        (
            "bossbar_remove",
            "bossbar_remove(id: string) -> void",
            "bossbar_remove(${1:\"mcfc:boss\"})",
        ),
        (
            "bossbar_name",
            "bossbar_name(id: string, name: string) -> void",
            "bossbar_name(${1:\"mcfc:boss\"}, ${2:\"Boss\"})",
        ),
        (
            "bossbar_value",
            "bossbar_value(id: string, value: int) -> void",
            "bossbar_value(${1:\"mcfc:boss\"}, ${2:10})",
        ),
        (
            "bossbar_max",
            "bossbar_max(id: string, max: int) -> void",
            "bossbar_max(${1:\"mcfc:boss\"}, ${2:20})",
        ),
        (
            "bossbar_visible",
            "bossbar_visible(id: string, visible: bool) -> void",
            "bossbar_visible(${1:\"mcfc:boss\"}, ${2:true})",
        ),
        (
            "bossbar_players",
            "bossbar_players(id: string, targets: entity_ref|entity_set) -> void",
            "bossbar_players(${1:\"mcfc:boss\"}, ${2:target})",
        ),
        (
            "playsound",
            "playsound(sound: string, category: string, target: entity_ref|entity_set) -> void",
            "playsound(${1:\"minecraft:entity.experience_orb.pickup\"}, ${2:\"master\"}, ${3:target})",
        ),
        (
            "stopsound",
            "stopsound(target: entity_ref|entity_set, category: string, sound: string) -> void",
            "stopsound(${1:target}, ${2:\"master\"}, ${3:\"minecraft:entity.experience_orb.pickup\"})",
        ),
        (
            "particle",
            "particle(name: string, position: block_ref) -> void",
            "particle(${1:\"minecraft:flame\"}, ${2:block(\"~ ~ ~\")})",
        ),
        (
            "setblock",
            "setblock(position: block_ref, block_id: string) -> void",
            "setblock(${1:block(\"~ ~ ~\")}, ${2:\"minecraft:stone\"})",
        ),
        (
            "fill",
            "fill(from: block_ref, to: block_ref, block_id: string) -> void",
            "fill(${1:block(\"~ ~ ~\")}, ${2:block(\"~1 ~1 ~1\")}, ${3:\"minecraft:stone\"})",
        ),
    ] {
        items.push(snippet_item(
            label,
            if label.contains("…") {
                CompletionItemKind::SNIPPET
            } else {
                CompletionItemKind::FUNCTION
            },
            detail,
            insert_text,
        ));
    }

    items
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
    match resolve_receiver_kind(source, analysis, offset, chain) {
        Some(CompletionReceiver::Array) => array_method_items(),
        Some(CompletionReceiver::Dict) => dict_method_items(),
        Some(CompletionReceiver::GenericEntityRef) => generic_entity_root_items(),
        Some(CompletionReceiver::PlayerEntityRef) => player_entity_root_items(),
        Some(CompletionReceiver::EquipmentSlot) => equipment_slot_items(),
        Some(CompletionReceiver::Struct(name)) => struct_field_items(analysis, &name),
        Some(
            CompletionReceiver::PlayerDynamicNamespace
            | CompletionReceiver::EntityTeam
            | CompletionReceiver::BlockRef
            | CompletionReceiver::Nbt,
        ) => Vec::new(),
        None => broad_member_items(),
    }
}

fn broad_member_items() -> Vec<CompletionItem> {
    [
        array_method_items(),
        dict_method_items(),
        player_entity_root_items(),
    ]
    .concat()
}

fn array_method_items() -> Vec<CompletionItem> {
    [
        ("len", "array<T>.len() -> int", "len()"),
        (
            "push",
            "array<T>.push(value: T) -> void",
            "push(${1:value})",
        ),
        ("pop", "array<T>.pop() -> T", "pop()"),
        (
            "remove_at",
            "array<T>.remove_at(index: int) -> T",
            "remove_at(${1:index})",
        ),
    ]
    .into_iter()
    .map(|(label, detail, insert_text)| {
        snippet_item(label, CompletionItemKind::METHOD, detail, insert_text)
    })
    .collect()
}

fn dict_method_items() -> Vec<CompletionItem> {
    [
        ("has", "dict<T>.has(key: string) -> bool", "has(${1:key})"),
        (
            "remove",
            "dict<T>.remove(key: string) -> void",
            "remove(${1:key})",
        ),
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
            "effect",
            "entity.effect(name: string, duration: int, amplifier: int) -> void",
            "effect(${1:name}, ${2:duration}, ${3:amplifier})",
            CompletionItemKind::METHOD,
        ),
        (
            "add_tag",
            "entity.add_tag(name: string) -> void",
            "add_tag(${1:name})",
            CompletionItemKind::METHOD,
        ),
        (
            "remove_tag",
            "entity.remove_tag(name: string) -> void",
            "remove_tag(${1:name})",
            CompletionItemKind::METHOD,
        ),
        (
            "has_tag",
            "entity.has_tag(name: string) -> bool",
            "has_tag(${1:name})",
            CompletionItemKind::METHOD,
        ),
        (
            "team",
            "entity.team writable string",
            "team",
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
    items.extend(
        [
            ("nbt", "player.nbt.* read namespace", "nbt"),
            ("state", "player.state.* read/write namespace", "state"),
            ("tags", "player.tags.* read/write namespace", "tags"),
        ]
        .into_iter()
        .map(|(label, detail, insert_text)| {
            snippet_item(label, CompletionItemKind::FIELD, detail, insert_text)
        }),
    );
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
        let word_end = index;
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

fn previous_char(source: &str, index: usize) -> Option<char> {
    if index == 0 {
        return None;
    }
    source[..index].chars().next_back()
}

fn is_member_word_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

#[derive(Debug, Clone)]
struct CompletionLocal {
    name: String,
    ty: Option<Type>,
}

fn local_type_at_offset(
    source: &str,
    analysis: &AnalysisResult,
    offset: usize,
    name: &str,
) -> Option<(Type, RefKind)> {
    if let Some(function) = function_at_offset(analysis, offset) {
        if let Some(local) = analysis
            .locals
            .iter()
            .find(|local| local.function == function.name && local.name == name)
        {
            return Some((local.ty.clone(), local.ref_kind));
        }
    }

    syntactic_locals_at_offset(source, offset)
        .into_iter()
        .find(|local| local.name == name)
        .and_then(|local| local.ty.map(|ty| (ty, RefKind::Unknown)))
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CompletionReceiver {
    Array,
    Dict,
    Struct(String),
    GenericEntityRef,
    PlayerEntityRef,
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
    let next = match current {
        Type::Struct(name) => analysis
            .typed_program
            .as_ref()
            .and_then(|program| program.struct_defs.get(&name))
            .and_then(|def| def.fields.get(segment))
            .cloned()?,
        Type::EntityRef => match segment.as_str() {
            "mainhand" | "offhand" | "head" | "chest" | "legs" | "feet" => {
                return if rest.is_empty() {
                    Some(CompletionReceiver::EquipmentSlot)
                } else {
                    None
                };
            }
            "state" | "tags" | "nbt" => {
                if current_ref_kind != RefKind::Player {
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
            "effect" | "add_tag" | "remove_tag" | "has_tag" => return None,
            _ => return None,
        },
        _ => return None,
    };

    receiver_from_type(next, RefKind::Unknown, rest, analysis)
}

fn receiver_for_terminal_type(ty: &Type, ref_kind: RefKind) -> Option<CompletionReceiver> {
    match ty {
        Type::Array(_) => Some(CompletionReceiver::Array),
        Type::Dict(_) => Some(CompletionReceiver::Dict),
        Type::Struct(name) => Some(CompletionReceiver::Struct(name.clone())),
        Type::EntityRef => Some(if ref_kind == RefKind::Player {
            CompletionReceiver::PlayerEntityRef
        } else {
            CompletionReceiver::GenericEntityRef
        }),
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
        .map(|(name, def)| {
            snippet_item(
                name,
                CompletionItemKind::STRUCT,
                &struct_signature(name, def),
                name,
            )
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
            kind: Some(CompletionItemKind::FIELD),
            detail: Some(ty.as_str()),
            ..CompletionItem::default()
        })
        .collect()
}

fn struct_signature(name: &str, def: &StructTypeDef) -> String {
    let fields = def
        .fields
        .iter()
        .map(|(field, ty)| format!("    {}: {}", field, ty.as_str()))
        .collect::<Vec<_>>()
        .join("\n");
    if fields.is_empty() {
        format!("struct {}:\nend", name)
    } else {
        format!("struct {}:\n{}\nend", name, fields)
    }
}

fn struct_signature_from_fields(name: &str, fields: &[(String, Type)]) -> String {
    let body = fields
        .iter()
        .map(|(field, ty)| format!("    {}: {}", field, ty.as_str()))
        .collect::<Vec<_>>()
        .join("\n");
    if body.is_empty() {
        format!("struct {}:\nend", name)
    } else {
        format!("struct {}:\n{}\nend", name, body)
    }
}

fn syntactic_locals_at_offset(source: &str, offset: usize) -> Vec<CompletionLocal> {
    let prefix = &source[..offset.min(source.len())];
    let mut locals = Vec::new();
    let mut active = false;
    let mut depth = 0usize;

    for line in prefix.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("fn ") {
            locals.clear();
            active = true;
            depth = 1;
            locals.extend(parse_params(trimmed));
            continue;
        }

        if !active {
            continue;
        }

        if let Some(local) = parse_let(trimmed) {
            locals.push(local);
        } else if let Some(local) = parse_for_local(trimmed) {
            locals.push(local);
        }

        if opens_block(trimmed) {
            depth += 1;
        }
        if trimmed == "end" || trimmed.starts_with("end ") || trimmed.starts_with("end#") {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                active = false;
            }
        }
    }

    locals
}

fn parse_params(line: &str) -> Vec<CompletionLocal> {
    let Some(open) = line.find('(') else {
        return Vec::new();
    };
    let Some(close) = line[open + 1..].find(')').map(|close| open + 1 + close) else {
        return Vec::new();
    };

    line[open + 1..close]
        .split(',')
        .filter_map(|part| {
            let (name, ty) = part.split_once(':')?;
            Some(CompletionLocal {
                name: name.trim().to_string(),
                ty: parse_type_name(ty.trim()),
            })
        })
        .filter(|local| !local.name.is_empty())
        .collect()
}

fn parse_let(line: &str) -> Option<CompletionLocal> {
    let rest = line.strip_prefix("let ")?;
    let (name, value) = rest.split_once('=')?;
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    Some(CompletionLocal {
        name: name.to_string(),
        ty: infer_expr_type(value.trim()),
    })
}

fn parse_for_local(line: &str) -> Option<CompletionLocal> {
    let rest = line.strip_prefix("for ")?;
    let (name, source) = rest.split_once(" in ")?;
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let ty = if source.contains("..") {
        Some(Type::Int)
    } else {
        Some(Type::EntityRef)
    };
    Some(CompletionLocal {
        name: name.to_string(),
        ty,
    })
}

fn parse_type_name(name: &str) -> Option<Type> {
    match name {
        "int" => Some(Type::Int),
        "bool" => Some(Type::Bool),
        "string" => Some(Type::String),
        "entity_set" => Some(Type::EntitySet),
        "entity_ref" => Some(Type::EntityRef),
        "block_ref" => Some(Type::BlockRef),
        "nbt" => Some(Type::Nbt),
        "void" => Some(Type::Void),
        _ => None,
    }
}

fn infer_expr_type(value: &str) -> Option<Type> {
    if value.starts_with("single(") {
        Some(Type::EntityRef)
    } else if value.starts_with("selector(") {
        Some(Type::EntitySet)
    } else if value.starts_with("block(") {
        Some(Type::BlockRef)
    } else if value.starts_with('"') || value.starts_with('\'') {
        Some(Type::String)
    } else if value.starts_with("true") || value.starts_with("false") {
        Some(Type::Bool)
    } else if value.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
        Some(Type::Int)
    } else if value.starts_with('[') {
        Some(Type::Array(Box::new(Type::Nbt)))
    } else if value.starts_with('{') {
        Some(Type::Dict(Box::new(Type::Nbt)))
    } else {
        None
    }
}

fn opens_block(line: &str) -> bool {
    (line.starts_with("if ")
        || line.starts_with("while ")
        || line.starts_with("for ")
        || line.starts_with("match ")
        || line.starts_with("as(")
        || line.starts_with("at("))
        && line.contains(':')
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
    if start == end && end < source.len() {
        if let Some(ch) = source[end..].chars().next() {
            end += ch.len_utf8();
        }
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

    use tower_lsp::lsp_types::Position;

    use super::{
        ProjectConfig, build_project_snapshot, completion_items, project_diagnostics_for_segment,
        project_document_symbols, range_from_text_range, resolve_project_config_for_path,
    };
    use super::{offset_to_position, position_to_offset};
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
fn helper(x: int) -> int
    return x
end

fn main() -> void
    let value = helper(1)
    value = value + 1
end
"#;
        let analysis = analyze_source(source);
        let items = completion_items(source, &analysis, source.find("helper(1)").unwrap());
        assert!(items.iter().any(|item| item.label == "fn"));
        assert!(items.iter().any(|item| item.label == "helper"));
        assert!(items.iter().any(|item| item.label == "value"));

        let method_items = completion_items("value.", &analysis, 6);
        assert!(method_items.iter().any(|item| item.label == "len"));
        assert!(!method_items.iter().any(|item| item.label == "helper"));
    }

    #[test]
    fn completes_syntactic_locals_when_source_is_incomplete() {
        let source = r#"
fn main(kind: string) -> void
    let me = single(selector("@a"))
    let amount = 1
    me.team.
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
    fn narrows_member_completions_by_receiver_type() {
        let source = r#"
fn main() -> void
    let values = [1, 2, 3]
    let me = single(selector("@a"))
    values.
    me.
"#;
        let analysis = analyze_source(source);
        let values_items = completion_items(source, &analysis, source.find("values.").unwrap() + 7);
        assert!(values_items.iter().any(|item| item.label == "push"));
        assert!(values_items.iter().any(|item| item.label == "remove_at"));
        assert!(!values_items.iter().any(|item| item.label == "team"));

        let me_items = completion_items(source, &analysis, source.find("me.").unwrap() + 3);
        assert!(me_items.iter().any(|item| item.label == "team"));
        assert!(!me_items.iter().any(|item| item.label == "push"));
    }

    #[test]
    fn completes_struct_types_and_fields() {
        let source = r#"
struct Profile:
    duration: int
    label: string
end

struct Action:
    profile: Profile
    kind: string
end

fn main(action: Action) -> void
    let next = action.profile
    let duration = next.duration
    let kind = action.kind
end
"#;
        let analysis = analyze_source(source);
        assert!(
            analysis.typed_program.is_some(),
            "{:?}",
            analysis.diagnostics
        );

        let top_level_items = completion_items(source, &analysis, source.find("fn main").unwrap());
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
fn main() -> void
    let me = single(selector("@a"))
    me.mainhand.
    mcf "say $(me.mainhand.)"
end
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
    }

    #[test]
    fn completes_gameplay_builtins_and_generic_entity_members() {
        let source = r#"
fn main() -> void
    let pig = single(selector("@e[type=pig,limit=1]"))
    pig.
end
"#;
        let analysis = analyze_source(source);
        let top_level_items = completion_items(source, &analysis, source.find("fn main").unwrap());
        assert!(top_level_items.iter().any(|item| item.label == "summon"));
        assert!(top_level_items.iter().any(|item| item.label == "tellraw"));
        assert!(top_level_items.iter().any(|item| item.label == "debug"));
        assert!(
            top_level_items
                .iter()
                .any(|item| item.label == "debug_marker")
        );
        assert!(
            top_level_items
                .iter()
                .any(|item| item.label == "debug_entity")
        );
        assert!(top_level_items.iter().any(|item| item.label == "fill"));

        let pig_items = completion_items(source, &analysis, source.find("pig.").unwrap() + 4);
        assert!(pig_items.iter().any(|item| item.label == "add_tag"));
        assert!(pig_items.iter().any(|item| item.label == "remove_tag"));
        assert!(pig_items.iter().any(|item| item.label == "has_tag"));
        assert!(pig_items.iter().any(|item| item.label == "offhand"));
        assert!(pig_items.iter().any(|item| item.label == "team"));
        assert!(!pig_items.iter().any(|item| item.label == "state"));
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
        write_file(&source_file, "fn main() -> void\nend\n");
        write_file(&asset_file, "fn ignored() -> void\nend\n");

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
        let first = src_dir.join("alpha.mcf");
        let second = src_dir.join("beta.mcf");
        write_file(&first, "fn alpha() -> void\nend\n");
        write_file(&second, "fn beta() -> void\n    alpha()\nend");

        let snapshot = build_project_snapshot(
            &ProjectConfig {
                manifest_path: project.join("sample.mcfc.toml"),
                source_root: src_dir.clone(),
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
        fs::create_dir_all(src_dir.join("lib")).unwrap();
        write_file(
            &project.join("sample.mcfc.toml"),
            "namespace = \"sample\"\nsource_dir = \"src\"\n",
        );
        let helper = src_dir.join("lib").join("helper.mcf");
        let main = src_dir.join("main.mcf");
        write_file(
            &helper,
            r#"
struct Action:
    kind: string
end

fn helper() -> void
    return
end
"#,
        );
        let main_source = r#"
fn main() -> void
    helper()
end
"#;
        write_file(&main, main_source);

        let snapshot = build_project_snapshot(
            &ProjectConfig {
                manifest_path: project.join("sample.mcfc.toml"),
                source_root: src_dir.clone(),
            },
            &HashMap::new(),
        )
        .expect("snapshot should build");
        let main_segment = snapshot
            .segment_for_path(&main)
            .expect("main file should have segment");
        let main_text = fs::read_to_string(&main).unwrap();
        let local_call_offset = main_text.find("helper").unwrap();
        let merged_call_offset = main_segment.local_to_merged_offset(local_call_offset);
        let (word, _) = crate::analysis::word_at_offset(&snapshot.merged_text, merged_call_offset)
            .expect("word at helper call");
        let hover = super::hover_contents(&snapshot.analysis, merged_call_offset, &word)
            .expect("hover should resolve cross-file function");
        assert!(hover.contains("fn helper() -> void"));

        let top_level_items = completion_items(
            &snapshot.merged_text,
            &snapshot.analysis,
            merged_call_offset,
        );
        assert!(top_level_items.iter().any(|item| item.label == "helper"));
        assert!(top_level_items.iter().any(|item| item.label == "Action"));

        let main_symbols = project_document_symbols(&main_text, &snapshot.analysis, main_segment);
        assert!(main_symbols.iter().any(|symbol| symbol.name == "main"));
        assert!(!main_symbols.iter().any(|symbol| symbol.name == "helper"));

        let mut overrides = HashMap::new();
        overrides.insert(helper.clone(), String::new());
        let broken_snapshot = build_project_snapshot(
            &ProjectConfig {
                manifest_path: project.join("sample.mcfc.toml"),
                source_root: src_dir,
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
                .any(|diagnostic| diagnostic.message.contains("unknown function"))
        );
    }
}
