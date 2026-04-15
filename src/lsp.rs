use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::RwLock;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, CompletionOptions, CompletionParams, CompletionResponse,
    CompletionTextEdit, Diagnostic as LspDiagnostic, DiagnosticSeverity,
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    DidSaveTextDocumentParams, DocumentSymbol, DocumentSymbolParams, DocumentSymbolResponse,
    Documentation, Hover, HoverContents, HoverParams, InitializeParams, InitializeResult,
    InitializedParams, MarkedString, OneOf, Position, Range, ServerCapabilities,
    TextDocumentSyncCapability, TextDocumentSyncKind, TextEdit, Url,
};
use tower_lsp::{Client, LanguageServer};

use crate::analysis::{AnalysisResult, analyze_source, function_at_offset, word_at_offset};
use crate::ast::Type;
use crate::diagnostics::{Diagnostic as McfcDiagnostic, TextRange};
use crate::minecraft_ids::{MinecraftIdCategory, ids_for_category};
use crate::minecraft_nbt_schema::{self, NbtSchemaCategory, NbtSchemaNode};
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
        return Some(format!("```mcfc\n{}\n```", function.signature()));
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
        "struct" => Some("```mcfc\nstruct Name:\n    field: Type\n```"),
        "player_state" => Some("```mcfc\nplayer_state money: int = \"Money\"\n```"),
        "match" => Some(
            "```mcfc\nmatch value:\n    \"pattern\" =>\n        ...\n    else =>\n        ...\nend\n```",
        ),
        "mcf" => Some("```mcfc\nmcf \"say $(expr)\"\n```"),
        "async" => Some("```mcfc\nasync:\n    ...\n```"),
        "sleep" => Some("```mcfc\nsleep(seconds: int) -> void\n```"),
        "sleep_ticks" => Some("```mcfc\nsleep_ticks(ticks: int) -> void\n```"),
        "random" => Some(
            "```mcfc\nrandom() -> int\nrandom(max: int) -> int\nrandom(min: int, max: int) -> int\n```",
        ),
        "selector" => Some("```mcfc\nselector(value: string) -> entity_set\n```"),
        "single" => Some("```mcfc\nsingle(value: entity_set) -> entity_ref\n```"),
        "exists" => Some("```mcfc\nexists(value: entity_ref) -> bool\n```"),
        "has_data" => Some("```mcfc\nhas_data(value: storage_path) -> bool\n```"),
        "entity" => Some("```mcfc\nentity(id: string) -> entity_def\n```"),
        "item" => Some("```mcfc\nitem(id: string) -> item_def\n```"),
        "text" => Some("```mcfc\ntext() -> text_def\ntext(value: string) -> text_def\n```"),
        "block" => Some("```mcfc\nblock(position: string) -> block_ref\n```"),
        "block_type" => Some("```mcfc\nblock_type(id: string) -> block_def\n```"),
        "at" => Some(
            "```mcfc\nat(anchor: entity_ref, value: entity_set|entity_ref|block_ref) -> entity_set|entity_ref|block_ref\n\nat(anchor):\n    ...\nend\n```",
        ),
        "as" => Some(
            "```mcfc\nas(anchor: entity_set|entity_ref, value: entity_set|entity_ref|block_ref) -> entity_set|entity_ref|block_ref\n\nas(anchor):\n    ...\nend\n```",
        ),
        "int" => Some("```mcfc\nint(value: nbt) -> int\n```"),
        "bool" => Some("```mcfc\nbool(value: nbt) -> bool\n```"),
        "string" => Some("```mcfc\nstring(value: nbt) -> string\n```"),
        "as_nbt" => Some(
            "```mcfc\nentity_def.as_nbt() -> nbt\nblock_def.as_nbt() -> nbt\nitem_def.as_nbt() -> nbt\n```",
        ),
        "summon" => Some(
            "```mcfc\nsummon(entity_id: string) -> entity_ref\nsummon(entity_id: string, data: nbt) -> entity_ref\nsummon(spec: entity_def) -> entity_ref\nblock.summon(entity_id: string) -> entity_ref\nblock.summon(entity_id: string, data: nbt) -> entity_ref\nblock.summon(spec: entity_def) -> entity_ref\n```",
        ),
        "bossbar" => Some("```mcfc\nbossbar(id: string, name: string|text_def) -> bossbar\n```"),
        "teleport" => {
            Some("```mcfc\nentity.teleport(destination: entity_ref|block_ref) -> void\n```")
        }
        "damage" => Some("```mcfc\nentity.damage(amount: int) -> void\n```"),
        "heal" => Some("```mcfc\nentity.heal(amount: int) -> void\n```"),
        "give" => Some(
            "```mcfc\nentity.give(item_id: string, count: int) -> void\nentity.give(stack: item_def) -> void\n```",
        ),
        "clear" => Some("```mcfc\nentity.clear(item_id: string, count: int) -> void\n```"),
        "loot_give" => Some("```mcfc\nentity.loot_give(table: string) -> void\n```"),
        "loot_insert" => Some("```mcfc\nblock.loot_insert(table: string) -> void\n```"),
        "loot_spawn" => Some("```mcfc\nblock.loot_spawn(table: string) -> void\n```"),
        "spawn_item" => Some("```mcfc\nblock.spawn_item(stack: item_def) -> entity_ref\n```"),
        "tellraw" => Some("```mcfc\nentity.tellraw(message: string|text_def) -> void\n```"),
        "title" => Some("```mcfc\nentity.title(message: string|text_def) -> void\n```"),
        "actionbar" => Some("```mcfc\nentity.actionbar(message: string|text_def) -> void\n```"),
        "debug" => Some("```mcfc\ndebug(message: string) -> void\n```"),
        "debug_marker" => Some(
            "```mcfc\nblock.debug_marker(label: string) -> void\nblock.debug_marker(label: string, marker_block: string) -> void\n```",
        ),
        "debug_entity" => Some("```mcfc\nentity.debug_entity(label: string) -> void\n```"),
        "bossbar_add" => Some("```mcfc\nDeprecated. Use `let bb = bossbar(id, name)`.\n```"),
        "bossbar_remove" => Some("```mcfc\nDeprecated. Use `bb.remove()`.\n```"),
        "bossbar_name" => Some("```mcfc\nDeprecated. Use `bb.name = name`.\n```"),
        "bossbar_value" => Some("```mcfc\nDeprecated. Use `bb.value = value`.\n```"),
        "bossbar_max" => Some("```mcfc\nDeprecated. Use `bb.max = max`.\n```"),
        "bossbar_visible" => Some("```mcfc\nDeprecated. Use `bb.visible = visible`.\n```"),
        "bossbar_players" => Some("```mcfc\nDeprecated. Use `bb.players = targets`.\n```"),
        "playsound" => {
            Some("```mcfc\nentity.playsound(sound: string, category: string) -> void\n```")
        }
        "stopsound" => {
            Some("```mcfc\nentity.stopsound(category: string, sound: string) -> void\n```")
        }
        "particle" => Some(
            "```mcfc\nblock.particle(name: string) -> void\nblock.particle(name: string, count: int) -> void\nblock.particle(name: string, count: int, viewers: entity_ref|entity_set) -> void\n```",
        ),
        "setblock" => Some("```mcfc\nblock.setblock(block_id: string|block_def) -> void\n```"),
        "is" => Some("```mcfc\nblock.is(block_id: string) -> bool\n```"),
        "fill" => {
            Some("```mcfc\nblock.fill(to: block_ref, block_id: string|block_def) -> void\n```")
        }
        "entity_def" => {
            Some("```mcfc\nentity_def\n- id: string (read-only)\n- nbt.*\n- as_nbt() -> nbt\n```")
        }
        "player_ref" => Some(
            "```mcfc\nplayer_ref\nKnown-player entity reference. Supports entity methods plus player.inventory[index] and player.hotbar[index].\n\nplayer_ref(entity: entity_ref) -> player_ref\n```",
        ),
        "block_def" => Some(
            "```mcfc\nblock_def\n- id: string (read-only)\n- states.*\n- nbt.*\n- as_nbt() -> nbt\n```",
        ),
        "item_def" => Some(
            "```mcfc\nitem_def\n- id: string (read-only)\n- count: int\n- nbt.*\n- as_nbt() -> nbt\n```",
        ),
        "text_def" => Some(
            "```mcfc\ntext_def\n- storage-backed text component builder\n- supports arbitrary .field / [index] writes for text component content, styling, events, and nested children\n```",
        ),
        "item_slot" => Some(
            "```mcfc\nitem_slot\n- exists: bool (read-only)\n- id: string (read-only)\n- count: int\n- nbt.*\n- clear() -> void\n```",
        ),
        "position" => Some("```mcfc\nentity.position -> block_ref\n```"),
        "state" => Some(
            "```mcfc\nentity.state.* -> MCFC-managed int/bool scoreboard state for any entity_ref\nplayer.state.* -> MCFC-managed int/bool scoreboard state for known players\n```",
        ),
        "len" => Some("```mcfc\narray<T>.len() -> int\n```"),
        "push" => Some("```mcfc\narray<T>.push(value: T) -> void\n```"),
        "pop" => Some("```mcfc\narray<T>.pop() -> T\n```"),
        "remove_at" => Some("```mcfc\narray<T>.remove_at(index: int) -> T\n```"),
        "has" => Some("```mcfc\ndict<T>.has(key: string) -> bool\n```"),
        "remove" => Some(
            "```mcfc\narray<T>.remove(index: int) -> T\ndict<T>.remove(key: string) -> void\nbossbar.remove() -> void\n```",
        ),
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
    match (call.name.as_str(), call.is_method, call.arg_index) {
        ("entity", false, 0) | ("summon", _, 0) => Some(MinecraftIdCategory::Entity),
        ("item", false, 0)
        | ("give", true, 0)
        | ("give", false, 1)
        | ("clear", true, 0)
        | ("clear", false, 1) => Some(MinecraftIdCategory::Item),
        ("block_type", false, 0)
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
                "remove",
                "array<T>.remove(index: int) -> T",
                "remove(${1:index})",
            ),
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
            ("teleport", "entity.teleport(destination) -> void", "teleport(${1:destination})"),
            ("damage", "entity.damage(amount: int) -> void", "damage(${1:amount})"),
            ("heal", "entity.heal(amount: int) -> void", "heal(${1:amount})"),
            ("give", "entity.give(item_id: string, count: int) -> void / entity.give(item_def) -> void", "give(${1:\"minecraft:stone\"}, ${2:1})"),
            ("clear", "entity.clear(item_id: string, count: int) -> void", "clear(${1:\"minecraft:stone\"}, ${2:1})"),
            ("loot_give", "entity.loot_give(table: string) -> void", "loot_give(${1:\"minecraft:chests/simple_dungeon\"})"),
            ("tellraw", "entity.tellraw(message: string) -> void", "tellraw(${1:\"hello\"})"),
            ("title", "entity.title(message: string) -> void", "title(${1:\"hello\"})"),
            ("actionbar", "entity.actionbar(message: string) -> void", "actionbar(${1:\"hello\"})"),
            ("playsound", "entity.playsound(sound: string, category: string) -> void", "playsound(${1:\"minecraft:entity.experience_orb.pickup\"}, ${2:\"master\"})"),
            ("stopsound", "entity.stopsound(category: string, sound: string) -> void", "stopsound(${1:\"master\"}, ${2:\"minecraft:entity.experience_orb.pickup\"})"),
            ("debug_entity", "entity.debug_entity(label: string) -> void", "debug_entity(${1:\"target\"})"),
            ("loot_insert", "block.loot_insert(table: string) -> void", "loot_insert(${1:\"minecraft:chests/simple_dungeon\"})"),
            ("loot_spawn", "block.loot_spawn(table: string) -> void", "loot_spawn(${1:\"minecraft:chests/simple_dungeon\"})"),
            ("debug_marker", "block.debug_marker(label: string) -> void", "debug_marker(${1:\"checkpoint\"})"),
            ("particle", "block.particle(name: string, count?: int, viewers?: entity_ref|entity_set) -> void", "particle(${1:\"minecraft:flame\"})"),
            ("setblock", "block.setblock(block_id: string|block_def) -> void", "setblock(${1:\"minecraft:stone\"})"),
            ("is", "block.is(block_id: string) -> bool", "is(${1:\"minecraft:air\"})"),
            ("fill", "block.fill(to: block_ref, block_id: string|block_def) -> void", "fill(${1:block(\"~1 ~1 ~1\")}, ${2:\"minecraft:stone\"})"),
            ("summon", "block.summon(entity_id: string|entity_def, data?: nbt) -> entity_ref", "summon(${1:\"minecraft:pig\"})"),
            ("spawn_item", "block.spawn_item(stack: item_def) -> entity_ref", "spawn_item(${1:item(\"minecraft:apple\")})"),
            ("name", "bossbar.name writable string", "name"),
            ("value", "bossbar.value writable int", "value"),
            ("max", "bossbar.max writable int", "max"),
            ("visible", "bossbar.visible writable bool", "visible"),
            ("players", "bossbar.players writable entity target", "players"),
            ("position", "entity.position -> block_ref", "position"),
            (
                "nbt",
                "entity.nbt.* / block.nbt.* runtime namespace",
                "nbt",
            ),
            ("state", "entity.state.* / player.state.* read/write namespace", "state"),
            ("tags", "player.tags.* read/write namespace", "tags"),
            ("inventory", "player.inventory[index] -> item_slot", "inventory"),
            ("hotbar", "player.hotbar[index] -> item_slot", "hotbar"),
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
        "fn",
        "struct",
        "player_state",
        "let",
        "return",
        "if",
        "match",
        "else",
        "while",
        "for",
        "in",
        "break",
        "continue",
        "async",
        "mc",
        "mcf",
        "true",
        "false",
        "and",
        "or",
        "not",
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
        ("player_ref", "player_ref"),
        ("block_ref", "block_ref"),
        ("entity_def", "entity_def"),
        ("block_def", "block_def"),
        ("item_def", "item_def"),
        ("text_def", "text_def"),
        ("item_slot", "item_slot"),
        ("bossbar", "bossbar"),
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
            "struct ...",
            "Define a named struct with typed fields",
            "struct ${1:Name}:\n\t${2:field}: ${3:int}",
        ),
        (
            "match ...",
            "Dispatch on a string value",
            "match ${1:value}:\n\t\"${2:pattern}\" => $0\n\telse => ",
        ),
        (
            "player_state",
            "Declare player scoreboard state display metadata",
            "player_state ${1:money}: ${2:int} = ${3:\"Money\"}",
        ),
        (
            "selector",
            "selector(value: string) -> entity_set",
            "selector(${1:\"@e\"})",
        ),
        (
            "entity",
            "entity(id: string) -> entity_def",
            "entity(${1:\"minecraft:pig\"})",
        ),
        (
            "item",
            "item(id: string) -> item_def",
            "item(${1:\"minecraft:apple\"})",
        ),
        (
            "text",
            "text() -> text_def / text(value: string) -> text_def",
            "text(${1:\"hello\"})",
        ),
        ("sleep", "sleep(seconds: int) -> void", "sleep(${1:1})"),
        (
            "sleep_ticks",
            "sleep_ticks(ticks: int) -> void",
            "sleep_ticks(${1:20})",
        ),
        ("random", "random() -> int", "random()"),
        ("random(max)", "random(max: int) -> int", "random(${1:max})"),
        (
            "random(min, max)",
            "random(min: int, max: int) -> int",
            "random(${1:min}, ${2:max})",
        ),
        (
            "single",
            "single(value: entity_set) -> entity_ref",
            "single(${1:value})",
        ),
        (
            "player_ref",
            "player_ref(entity: entity_ref) -> player_ref",
            "player_ref(${1:entity})",
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
            "block_type",
            "block_type(id: string) -> block_def",
            "block_type(${1:\"minecraft:chest\"})",
        ),
        (
            "at",
            "at(anchor: entity_ref, value: entity_set|entity_ref|block_ref)",
            "at(${1:anchor}, ${2:value})",
        ),
        (
            "at(...):",
            "Run commands at an entity/block",
            "at(${1:anchor}):\n\t$0",
        ),
        (
            "as",
            "as(anchor: entity_set|entity_ref, value: entity_set|entity_ref|block_ref)",
            "as(${1:anchor}, ${2:value})",
        ),
        (
            "as(...):",
            "Run commands as an entity",
            "as(${1:anchor}):\n\t$0",
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
            "summon(entity_id: string|entity_def) -> entity_ref",
            "summon(${1:\"minecraft:pig\"})",
        ),
        (
            "bossbar",
            "bossbar(id: string, name: string|text_def) -> bossbar",
            "bossbar(${1:\"mcfc:boss\"}, ${2:\"Boss\"})",
        ),
        ("async:", "Spawn a non-blocking async block", "async:\n\t$0"),
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
            "give(target: entity_ref|entity_set, item_id: string, count: int) -> void / entity.give(item_def)",
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
            "setblock(position: block_ref, block_id: string|block_def) -> void",
            "setblock(${1:block(\"~ ~ ~\")}, ${2:\"minecraft:stone\"})",
        ),
        (
            "fill",
            "fill(from: block_ref, to: block_ref, block_id: string|block_def) -> void",
            "fill(${1:block(\"~ ~ ~\")}, ${2:block(\"~1 ~1 ~1\")}, ${3:\"minecraft:stone\"})",
        ),
    ] {
        if is_removed_legacy_completion(label) {
            continue;
        }
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

fn is_removed_legacy_completion(label: &str) -> bool {
    matches!(
        label,
        "teleport"
            | "damage"
            | "heal"
            | "give"
            | "clear"
            | "loot_give"
            | "loot_insert"
            | "loot_spawn"
            | "tellraw"
            | "title"
            | "actionbar"
            | "debug_marker"
            | "debug_entity"
            | "bossbar_add"
            | "bossbar_remove"
            | "bossbar_name"
            | "bossbar_value"
            | "bossbar_max"
            | "bossbar_visible"
            | "bossbar_players"
            | "playsound"
            | "stopsound"
            | "particle"
            | "setblock"
            | "fill"
    )
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

fn completion_items_for_receiver(
    receiver: Option<CompletionReceiver>,
    analysis: &AnalysisResult,
) -> Vec<CompletionItem> {
    match receiver {
        Some(CompletionReceiver::Array) => array_method_items(),
        Some(CompletionReceiver::Dict) => dict_method_items(),
        Some(CompletionReceiver::GenericEntityRef) => generic_entity_root_items(),
        Some(CompletionReceiver::PlayerEntityRef) => player_entity_root_items(),
        Some(CompletionReceiver::EntityDef) => entity_def_items(),
        Some(CompletionReceiver::ItemDef) => item_def_items(),
        Some(CompletionReceiver::TextDef) => text_def_items(),
        Some(CompletionReceiver::BlockDef) => block_def_items(),
        Some(CompletionReceiver::ItemSlot) => item_slot_items(),
        Some(CompletionReceiver::Bossbar) => bossbar_root_items(),
        Some(CompletionReceiver::EquipmentSlot) => equipment_slot_items(),
        Some(CompletionReceiver::Struct(name)) => struct_field_items(analysis, &name),
        Some(
            CompletionReceiver::PlayerDynamicNamespace
            | CompletionReceiver::EntityTeam
            | CompletionReceiver::Nbt,
        ) => Vec::new(),
        Some(CompletionReceiver::BlockRef) => block_ref_items(),
        None => broad_member_items(),
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
        ("len", "array<T>.len() -> int", "len()"),
        (
            "push",
            "array<T>.push(value: T) -> void",
            "push(${1:value})",
        ),
        ("pop", "array<T>.pop() -> T", "pop()"),
        (
            "remove",
            "array<T>.remove(index: int) -> T",
            "remove(${1:index})",
        ),
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
            "teleport",
            "entity.teleport(destination: entity_ref|block_ref) -> void",
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
            "entity.give(item_id: string, count: int) -> void / entity.give(item_def) -> void",
            "give(${1:\"minecraft:stone\"}, ${2:1})",
            CompletionItemKind::METHOD,
        ),
        (
            "clear",
            "entity.clear(item_id: string, count: int) -> void",
            "clear(${1:\"minecraft:stone\"}, ${2:1})",
            CompletionItemKind::METHOD,
        ),
        (
            "loot_give",
            "entity.loot_give(table: string) -> void",
            "loot_give(${1:\"minecraft:chests/simple_dungeon\"})",
            CompletionItemKind::METHOD,
        ),
        (
            "tellraw",
            "entity.tellraw(message: string) -> void",
            "tellraw(${1:\"hello\"})",
            CompletionItemKind::METHOD,
        ),
        (
            "title",
            "entity.title(message: string) -> void",
            "title(${1:\"hello\"})",
            CompletionItemKind::METHOD,
        ),
        (
            "actionbar",
            "entity.actionbar(message: string) -> void",
            "actionbar(${1:\"hello\"})",
            CompletionItemKind::METHOD,
        ),
        (
            "playsound",
            "entity.playsound(sound: string, category: string) -> void",
            "playsound(${1:\"minecraft:entity.experience_orb.pickup\"}, ${2:\"master\"})",
            CompletionItemKind::METHOD,
        ),
        (
            "stopsound",
            "entity.stopsound(category: string, sound: string) -> void",
            "stopsound(${1:\"master\"}, ${2:\"minecraft:entity.experience_orb.pickup\"})",
            CompletionItemKind::METHOD,
        ),
        (
            "debug_entity",
            "entity.debug_entity(label: string) -> void",
            "debug_entity(${1:\"target\"})",
            CompletionItemKind::METHOD,
        ),
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
            "position",
            "entity.position -> block_ref",
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
                "player.inventory[index] -> item_slot",
                "inventory",
            ),
            ("hotbar", "player.hotbar[index] -> item_slot", "hotbar"),
        ]
        .into_iter()
        .map(|(label, detail, insert_text)| {
            snippet_item(label, CompletionItemKind::FIELD, detail, insert_text)
        }),
    );
    items
}

fn block_ref_items() -> Vec<CompletionItem> {
    let mut items = [
        (
            "loot_insert",
            "block.loot_insert(table: string) -> void",
            "loot_insert(${1:\"minecraft:chests/simple_dungeon\"})",
        ),
        (
            "loot_spawn",
            "block.loot_spawn(table: string) -> void",
            "loot_spawn(${1:\"minecraft:chests/simple_dungeon\"})",
        ),
        (
            "debug_marker",
            "block.debug_marker(label: string) -> void",
            "debug_marker(${1:\"checkpoint\"})",
        ),
        (
            "particle",
            "block.particle(name: string, count?: int, viewers?: entity_ref|entity_set) -> void",
            "particle(${1:\"minecraft:flame\"})",
        ),
        (
            "setblock",
            "block.setblock(block_id: string|block_def) -> void",
            "setblock(${1:\"minecraft:stone\"})",
        ),
        (
            "fill",
            "block.fill(to: block_ref, block_id: string|block_def) -> void",
            "fill(${1:block(\"~1 ~1 ~1\")}, ${2:\"minecraft:stone\"})",
        ),
        (
            "summon",
            "block.summon(entity_id: string|entity_def, data?: nbt) -> entity_ref",
            "summon(${1:\"minecraft:pig\"})",
        ),
        (
            "spawn_item",
            "block.spawn_item(stack: item_def) -> entity_ref",
            "spawn_item(${1:item(\"minecraft:apple\")})",
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
        "as_nbt",
        CompletionItemKind::METHOD,
        "entity_def.as_nbt() -> nbt",
        "as_nbt()",
    )];
    items.extend(
        [
            ("id", "entity_def.id read-only string", "id"),
            ("nbt", "entity_def.nbt.* writable namespace", "nbt"),
            (
                "name",
                "entity_def.name -> entity_def.nbt.CustomName",
                "name",
            ),
            (
                "name_visible",
                "entity_def.name_visible -> entity_def.nbt.CustomNameVisible",
                "name_visible",
            ),
            ("no_ai", "entity_def.no_ai -> entity_def.nbt.NoAI", "no_ai"),
            (
                "silent",
                "entity_def.silent -> entity_def.nbt.Silent",
                "silent",
            ),
            (
                "glowing",
                "entity_def.glowing -> entity_def.nbt.Glowing",
                "glowing",
            ),
            ("tags", "entity_def.tags -> entity_def.nbt.Tags", "tags"),
        ]
        .into_iter()
        .map(|(label, detail, insert_text)| {
            snippet_item(label, CompletionItemKind::FIELD, detail, insert_text)
        }),
    );
    items
}

fn item_def_items() -> Vec<CompletionItem> {
    let mut items = vec![snippet_item(
        "as_nbt",
        CompletionItemKind::METHOD,
        "item_def.as_nbt() -> nbt",
        "as_nbt()",
    )];
    items.extend(
        [
            ("id", "item_def.id read-only string", "id"),
            ("count", "item_def.count writable int", "count"),
            ("nbt", "item_def.nbt.* writable namespace", "nbt"),
            ("name", "item_def.name -> item_def.nbt.display.Name", "name"),
        ]
        .into_iter()
        .map(|(label, detail, insert_text)| {
            snippet_item(label, CompletionItemKind::FIELD, detail, insert_text)
        }),
    );
    items
}

fn block_def_items() -> Vec<CompletionItem> {
    let mut items = vec![snippet_item(
        "as_nbt",
        CompletionItemKind::METHOD,
        "block_def.as_nbt() -> nbt",
        "as_nbt()",
    )];
    items.extend(
        [
            ("id", "block_def.id read-only string", "id"),
            ("states", "block_def.states.* writable namespace", "states"),
            ("nbt", "block_def.nbt.* writable namespace", "nbt"),
            ("name", "block_def.name -> block_def.nbt.CustomName", "name"),
            ("lock", "block_def.lock -> block_def.nbt.Lock", "lock"),
            (
                "loot_table",
                "block_def.loot_table -> block_def.nbt.LootTable",
                "loot_table",
            ),
            (
                "loot_seed",
                "block_def.loot_seed -> block_def.nbt.LootTableSeed",
                "loot_seed",
            ),
        ]
        .into_iter()
        .map(|(label, detail, insert_text)| {
            snippet_item(label, CompletionItemKind::FIELD, detail, insert_text)
        }),
    );
    items
}

fn text_def_items() -> Vec<CompletionItem> {
    [
        ("text", "text_def.text writable string"),
        ("translate", "text_def.translate writable string"),
        ("keybind", "text_def.keybind writable string"),
        ("selector", "text_def.selector writable string"),
        ("color", "text_def.color writable string"),
        ("font", "text_def.font writable string"),
        ("insertion", "text_def.insertion writable string"),
        ("bold", "text_def.bold writable bool"),
        ("italic", "text_def.italic writable bool"),
        ("underlined", "text_def.underlined writable bool"),
        ("strikethrough", "text_def.strikethrough writable bool"),
        ("obfuscated", "text_def.obfuscated writable bool"),
        ("extra", "text_def.extra writable child component list"),
        (
            "hover_event",
            "text_def.hover_event.* writable hover event fields",
        ),
        (
            "click_event",
            "text_def.click_event.* writable click event fields",
        ),
        ("with", "text_def.with writable translation argument list"),
        ("score", "text_def.score.* writable score component fields"),
        ("separator", "text_def.separator writable text component"),
        ("nbt", "text_def.nbt writable source path string"),
        ("block", "text_def.block writable source block string"),
        ("entity", "text_def.entity writable source selector string"),
        ("storage", "text_def.storage writable source storage string"),
        ("interpret", "text_def.interpret writable bool"),
    ]
    .into_iter()
    .map(|(label, detail)| snippet_item(label, CompletionItemKind::FIELD, detail, label))
    .collect()
}

fn text_hover_event_items() -> Vec<CompletionItem> {
    [
        ("action", "text_def.hover_event.action writable string"),
        (
            "value",
            "text_def.hover_event.value writable legacy hover payload",
        ),
        (
            "contents",
            "text_def.hover_event.contents writable nested hover payload",
        ),
    ]
    .into_iter()
    .map(|(label, detail)| snippet_item(label, CompletionItemKind::FIELD, detail, label))
    .collect()
}

fn text_click_event_items() -> Vec<CompletionItem> {
    [
        ("action", "text_def.click_event.action writable string"),
        ("value", "text_def.click_event.value writable string"),
    ]
    .into_iter()
    .map(|(label, detail)| snippet_item(label, CompletionItemKind::FIELD, detail, label))
    .collect()
}

fn text_score_items() -> Vec<CompletionItem> {
    [
        ("name", "text_def.score.name writable string"),
        ("objective", "text_def.score.objective writable string"),
        ("value", "text_def.score.value writable string"),
    ]
    .into_iter()
    .map(|(label, detail)| snippet_item(label, CompletionItemKind::FIELD, detail, label))
    .collect()
}

fn item_slot_items() -> Vec<CompletionItem> {
    [
        (
            "clear",
            "item_slot.clear() -> void",
            "clear()",
            CompletionItemKind::METHOD,
        ),
        (
            "exists",
            "item_slot.exists read-only bool",
            "exists",
            CompletionItemKind::FIELD,
        ),
        (
            "id",
            "item_slot.id read-only string",
            "id",
            CompletionItemKind::FIELD,
        ),
        (
            "count",
            "item_slot.count writable int",
            "count",
            CompletionItemKind::FIELD,
        ),
        (
            "nbt",
            "item_slot.nbt.* writable namespace",
            "nbt",
            CompletionItemKind::FIELD,
        ),
        (
            "name",
            "item_slot.name writable string",
            "name",
            CompletionItemKind::FIELD,
        ),
    ]
    .into_iter()
    .map(|(label, detail, insert_text, kind)| snippet_item(label, kind, detail, insert_text))
    .collect()
}

fn bossbar_root_items() -> Vec<CompletionItem> {
    [
        (
            "remove",
            "bossbar.remove() -> void",
            "remove()",
            CompletionItemKind::METHOD,
        ),
        (
            "name",
            "bossbar.name writable string",
            "name",
            CompletionItemKind::FIELD,
        ),
        (
            "value",
            "bossbar.value writable int",
            "value",
            CompletionItemKind::FIELD,
        ),
        (
            "max",
            "bossbar.max writable int",
            "max",
            CompletionItemKind::FIELD,
        ),
        (
            "visible",
            "bossbar.visible writable bool",
            "visible",
            CompletionItemKind::FIELD,
        ),
        (
            "players",
            "bossbar.players writable entity target",
            "players",
            CompletionItemKind::FIELD,
        ),
    ]
    .into_iter()
    .map(|(label, detail, insert_text, kind)| snippet_item(label, kind, detail, insert_text))
    .collect()
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

    if index == word_end { word_end } else { index }
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
        if let Some(ty) = local.ty {
            if let Some(origin) = nbt_origin_for_type(&ty) {
                return Some(origin);
            }
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
    Dict,
    Struct(String),
    GenericEntityRef,
    PlayerEntityRef,
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
        Type::Dict(_) => Some(CompletionReceiver::Dict),
        Type::Struct(name) => Some(CompletionReceiver::Struct(name.clone())),
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

        if let Some((name, value)) = parse_let_binding(trimmed) {
            let ty = infer_expr_type(value).or_else(|| {
                locals
                    .iter()
                    .rev()
                    .find(|local| local.name == value)
                    .and_then(|local| local.ty.clone())
            });
            let nbt_origin = infer_nbt_completion_origin_from_value(value, &locals);
            upsert_completion_local(
                &mut locals,
                CompletionLocal {
                    name,
                    ty,
                    nbt_origin,
                },
            );
        } else if let Some(local) = parse_for_local(trimmed) {
            upsert_completion_local(&mut locals, local);
        } else if let Some(name) = assigned_local_name(trimmed) {
            if let Some(local) = locals.iter_mut().rev().find(|local| local.name == name) {
                local.nbt_origin = None;
            }
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

fn upsert_completion_local(locals: &mut Vec<CompletionLocal>, local: CompletionLocal) {
    if let Some(index) = locals
        .iter()
        .position(|existing| existing.name == local.name)
    {
        locals.remove(index);
    }
    locals.push(local);
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
                nbt_origin: None,
            })
        })
        .filter(|local| !local.name.is_empty())
        .collect()
}

fn parse_let_binding(line: &str) -> Option<(String, &str)> {
    let rest = line.strip_prefix("let ")?;
    let (name, value) = rest.split_once('=')?;
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    Some((name.to_string(), value.trim()))
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
        nbt_origin: None,
    })
}

fn assigned_local_name(line: &str) -> Option<&str> {
    if line.starts_with("let ") || !line.contains('=') {
        return None;
    }
    let (lhs, _) = line.split_once('=')?;
    let name = lhs.trim();
    if name.is_empty()
        || name.contains('.')
        || name.contains('[')
        || !name.chars().all(is_member_word_char)
    {
        return None;
    }
    Some(name)
}

fn parse_type_name(name: &str) -> Option<Type> {
    match name {
        "int" => Some(Type::Int),
        "bool" => Some(Type::Bool),
        "string" => Some(Type::String),
        "entity_set" => Some(Type::EntitySet),
        "entity_ref" => Some(Type::EntityRef),
        "player_ref" => Some(Type::PlayerRef),
        "block_ref" => Some(Type::BlockRef),
        "entity_def" => Some(Type::EntityDef),
        "block_def" => Some(Type::BlockDef),
        "item_def" => Some(Type::ItemDef),
        "text_def" => Some(Type::TextDef),
        "item_slot" => Some(Type::ItemSlot),
        "bossbar" => Some(Type::Bossbar),
        "nbt" => Some(Type::Nbt),
        "void" => Some(Type::Void),
        _ => None,
    }
}

fn infer_expr_type(value: &str) -> Option<Type> {
    if value.starts_with("single(") {
        Some(Type::EntityRef)
    } else if value.starts_with("player_ref(") {
        Some(Type::PlayerRef)
    } else if value.starts_with("selector(") {
        Some(Type::EntitySet)
    } else if value.starts_with("entity(") {
        Some(Type::EntityDef)
    } else if value.starts_with("item(") {
        Some(Type::ItemDef)
    } else if value.starts_with("text(") {
        Some(Type::TextDef)
    } else if value.starts_with("block(") {
        Some(Type::BlockRef)
    } else if value.starts_with("block_type(") {
        Some(Type::BlockDef)
    } else if value.starts_with("bossbar(") {
        Some(Type::Bossbar)
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
    if let Some(origin) = infer_constructor_nbt_origin(value, "entity", NbtSchemaCategory::Entity) {
        return Some(origin);
    }
    if let Some(origin) =
        infer_constructor_nbt_origin(value, "block_type", NbtSchemaCategory::Block)
    {
        return Some(origin);
    }
    if let Some(origin) = infer_constructor_nbt_origin(value, "item", NbtSchemaCategory::Item) {
        return Some(origin);
    }
    value
        .strip_suffix(".as_nbt()")
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

    use tower_lsp::lsp_types::{CompletionTextEdit, Position};

    use super::{
        ProjectConfig, build_project_snapshot, builtin_hover, completion_items, offset_to_position,
        position_to_offset, project_diagnostics_for_segment, project_document_symbols,
        range_from_text_range, resolve_project_config_for_path,
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
fn helper(x: int) -> int:
    return x
fn main() -> void:
    let value = helper(1)
    value = value + 1
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
fn main(kind: string) -> void:
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
fn main() -> void:
    let values = [1, 2, 3]
    let me = single(selector("@a"))
    values.
    me.
"#;
        let analysis = analyze_source(source);
        let values_items = completion_items(source, &analysis, source.find("values.").unwrap() + 7);
        assert!(values_items.iter().any(|item| item.label == "push"));
        assert!(values_items.iter().any(|item| item.label == "remove"));
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
struct Action:
    profile: Profile
    kind: string
fn main(action: Action) -> void:
    let next = action.profile
    let duration = next.duration
    let kind = action.kind
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
fn main() -> void:
    let me = player_ref(single(selector("@a")))
    let asserted = player_ref(single(selector("@e[limit=1]")))
    me.mainhand.
    me.inventory[0].
    me.hotbar[0].
    asserted.
    mcf "say $(me.mainhand.)"
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

        let hotbar_items = completion_items(
            source,
            &analysis,
            source.find("me.hotbar[0].").unwrap() + 13,
        );
        assert!(hotbar_items.iter().any(|item| item.label == "id"));
        assert!(hotbar_items.iter().any(|item| item.label == "count"));

        let asserted_items =
            completion_items(source, &analysis, source.find("asserted.").unwrap() + 9);
        assert!(asserted_items.iter().any(|item| item.label == "inventory"));
        assert!(asserted_items.iter().any(|item| item.label == "hotbar"));
    }

    #[test]
    fn completes_gameplay_builtins_and_generic_entity_members() {
        let source = r#"
fn main() -> void:
    let pig = single(selector("@e[type=pig,limit=1]"))
    pig.
"#;
        let analysis = analyze_source(source);
        let top_level_items = completion_items(source, &analysis, source.find("fn main").unwrap());
        assert!(top_level_items.iter().any(|item| item.label == "sleep"));
        assert!(top_level_items.iter().any(|item| item.label == "random"));
        assert!(
            top_level_items
                .iter()
                .any(|item| item.label == "random(min, max)")
        );
        assert!(top_level_items.iter().any(|item| item.label == "summon"));
        assert!(top_level_items.iter().any(|item| item.label == "bossbar"));
        assert!(top_level_items.iter().any(|item| item.label == "async:"));
        assert!(
            top_level_items
                .iter()
                .any(|item| item.label == "sleep_ticks")
        );
        assert!(top_level_items.iter().any(|item| item.label == "debug"));
        assert!(!top_level_items.iter().any(|item| item.label == "tellraw"));

        let pig_items = completion_items(source, &analysis, source.find("pig.").unwrap() + 4);
        assert!(pig_items.iter().any(|item| item.label == "teleport"));
        assert!(pig_items.iter().any(|item| item.label == "tellraw"));
        assert!(pig_items.iter().any(|item| item.label == "position"));
        assert!(pig_items.iter().any(|item| item.label == "add_tag"));
        assert!(pig_items.iter().any(|item| item.label == "remove_tag"));
        assert!(pig_items.iter().any(|item| item.label == "has_tag"));
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
fn main() -> void:
    let pig = single(selector("@e[type=pig,limit=1]"))
    let player = player_ref(single(selector("@a[limit=1]")))
    pig.state.
    player.state.
    single(selector("@e[type=pig,limit=1]")).state.
    player_ref(single(selector("@a[limit=1]"))).state.
    pig.position.foo.
    single(selector("@e[type=pig,limit=1]")).position.foo.
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
                .find("single(selector(\"@e[type=pig,limit=1]\")).state.")
                .unwrap()
                + "single(selector(\"@e[type=pig,limit=1]\")).state.".len(),
        );
        assert!(inline_entity_state_items.is_empty());

        let inline_player_state_items = completion_items(
            source,
            &analysis,
            source
                .find("player_ref(single(selector(\"@a[limit=1]\"))).state.")
                .unwrap()
                + "player_ref(single(selector(\"@a[limit=1]\"))).state.".len(),
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
                .find("single(selector(\"@e[type=pig,limit=1]\")).position.foo.")
                .unwrap()
                + "single(selector(\"@e[type=pig,limit=1]\")).position.foo.".len(),
        );
        assert!(invalid_inline_nested_items.is_empty());
    }

    #[test]
    fn completes_builder_members_and_hover_signatures() {
        let source = r#"
fn main() -> void:
    let pig = entity("minecraft:pig")
    let chest = block_type("minecraft:chest")
    let stack = item("minecraft:apple")
    let msg = text("Hello")
    pig.
    chest.
    stack.
    msg.
    item("minecraft:apple").
    text("Hello").
    block("~ ~ ~").
"#;
        let analysis = analyze_source(source);
        let top_level_items = completion_items(source, &analysis, source.find("fn main").unwrap());
        assert!(top_level_items.iter().any(|item| item.label == "entity"));
        assert!(
            top_level_items
                .iter()
                .any(|item| item.label == "block_type")
        );
        assert!(top_level_items.iter().any(|item| item.label == "item"));

        let pig_items = completion_items(source, &analysis, source.find("pig.").unwrap() + 4);
        assert!(pig_items.iter().any(|item| item.label == "as_nbt"));
        assert!(pig_items.iter().any(|item| item.label == "name"));
        assert!(pig_items.iter().any(|item| item.label == "nbt"));
        assert!(pig_items.iter().any(|item| item.label == "no_ai"));

        let chest_items = completion_items(source, &analysis, source.find("chest.").unwrap() + 6);
        assert!(chest_items.iter().any(|item| item.label == "as_nbt"));
        assert!(chest_items.iter().any(|item| item.label == "states"));
        assert!(chest_items.iter().any(|item| item.label == "lock"));
        assert!(chest_items.iter().any(|item| item.label == "name"));

        let stack_items = completion_items(source, &analysis, source.find("stack.").unwrap() + 6);
        assert!(stack_items.iter().any(|item| item.label == "as_nbt"));
        assert!(stack_items.iter().any(|item| item.label == "count"));
        assert!(stack_items.iter().any(|item| item.label == "name"));

        let msg_items = completion_items(source, &analysis, source.find("msg.").unwrap() + 4);
        assert!(msg_items.iter().any(|item| item.label == "color"));
        assert!(msg_items.iter().any(|item| item.label == "hover_event"));
        assert!(msg_items.iter().any(|item| item.label == "score"));

        let inline_item_items = completion_items(
            source,
            &analysis,
            source.find("item(\"minecraft:apple\").").unwrap() + "item(\"minecraft:apple\").".len(),
        );
        assert!(inline_item_items.iter().any(|item| item.label == "as_nbt"));
        assert!(inline_item_items.iter().any(|item| item.label == "count"));

        let inline_text_items = completion_items(
            source,
            &analysis,
            source.find("text(\"Hello\").").unwrap() + "text(\"Hello\").".len(),
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
            source.find("block(\"~ ~ ~\").").unwrap() + "block(\"~ ~ ~\").".len(),
        );
        assert!(inline_block_items.iter().any(|item| item.label == "summon"));
        assert!(
            inline_block_items
                .iter()
                .any(|item| item.label == "spawn_item")
        );
        assert!(inline_block_items.iter().any(|item| item.label == "nbt"));

        let summon_hover = builtin_hover("summon").expect("summon hover");
        assert!(summon_hover.contains("summon(spec: entity_def) -> entity_ref"));
        let as_nbt_hover = builtin_hover("as_nbt").expect("as_nbt hover");
        assert!(as_nbt_hover.contains("entity_def.as_nbt() -> nbt"));
        assert!(as_nbt_hover.contains("item_def.as_nbt() -> nbt"));
        let item_hover = builtin_hover("item").expect("item hover");
        assert!(item_hover.contains("item(id: string) -> item_def"));
        let item_def_hover = builtin_hover("item_def").expect("item_def hover");
        assert!(item_def_hover.contains("item_def"));
        let item_slot_hover = builtin_hover("item_slot").expect("item_slot hover");
        assert!(item_slot_hover.contains("clear() -> void"));
        let player_ref_hover = builtin_hover("player_ref").expect("player_ref hover");
        assert!(player_ref_hover.contains("player_ref(entity: entity_ref) -> player_ref"));
        let give_hover = builtin_hover("give").expect("give hover");
        assert!(give_hover.contains("entity.give(stack: item_def) -> void"));
        let spawn_item_hover = builtin_hover("spawn_item").expect("spawn_item hover");
        assert!(spawn_item_hover.contains("block.spawn_item(stack: item_def) -> entity_ref"));
        let text_def_hover = builtin_hover("text_def").expect("text_def hover");
        assert!(text_def_hover.contains("storage-backed text component builder"));
    }

    #[test]
    fn completes_text_def_nested_members() {
        let source = r#"
fn main() -> void:
    let msg = text("Hello")
    msg.hover_event.
    msg.click_event.
    msg.score.
    msg.extra[0].
    text("Hello").hover_event.
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
            source.find("text(\"Hello\").hover_event.").unwrap()
                + "text(\"Hello\").hover_event.".len(),
        );
        assert!(inline_hover_items.iter().any(|item| item.label == "action"));
    }

    #[test]
    fn completes_schema_backed_nbt_fields_for_inline_and_local_builders() {
        let source = r#"
fn main() -> void:
    entity("minecraft:mannequin").nbt.
    entity("minecraft:mannequin").nbt.profile.
    let mannequin = entity("minecraft:mannequin")
    let alias = mannequin
    alias.nbt.profile.
    block_type("minecraft:player_head").nbt.
    item("minecraft:player_head").nbt.
"#;
        let analysis = analyze_source(source);

        let entity_root_items = completion_items(
            source,
            &analysis,
            source.find("entity(\"minecraft:mannequin\").nbt.").unwrap()
                + "entity(\"minecraft:mannequin\").nbt.".len(),
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
                .find("entity(\"minecraft:mannequin\").nbt.profile.")
                .unwrap()
                + "entity(\"minecraft:mannequin\").nbt.profile.".len(),
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
                .find("block_type(\"minecraft:player_head\").nbt.")
                .unwrap()
                + "block_type(\"minecraft:player_head\").nbt.".len(),
        );
        assert!(block_items.iter().any(|item| item.label == "profile"));
        assert!(block_items.iter().any(|item| item.label == "custom_name"));

        let item_items = completion_items(
            source,
            &analysis,
            source.find("item(\"minecraft:player_head\").nbt.").unwrap()
                + "item(\"minecraft:player_head\").nbt.".len(),
        );
        assert!(item_items.iter().any(|item| item.label == "display"));
        assert!(item_items.iter().any(|item| item.label == "SkullOwner"));
    }

    #[test]
    fn completes_schema_backed_nbt_fields_for_runtime_refs() {
        let source = r#"
fn main() -> void:
    let pig = single(selector("@e[type=pig,limit=1]"))
    let player = player_ref(single(selector("@a[limit=1]")))
    let chest = block("~ ~ ~")
    pig.nbt.
    single(selector("@e[type=pig,limit=1]")).nbt.
    player.nbt.
    chest.nbt.
    block("~ ~ ~").nbt.
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
                .find("single(selector(\"@e[type=pig,limit=1]\")).nbt.")
                .unwrap()
                + "single(selector(\"@e[type=pig,limit=1]\")).nbt.".len(),
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
            source.find("block(\"~ ~ ~\").nbt.").unwrap() + "block(\"~ ~ ~\").nbt.".len(),
        );
        assert!(inline_block_items.iter().any(|item| item.label == "lock"));
    }

    #[test]
    fn completes_full_upstream_nbt_for_additional_exact_ids() {
        let source = r#"
fn main() -> void:
    entity("minecraft:armor_stand").nbt.
    item("minecraft:diamond_sword").nbt.
"#;
        let analysis = analyze_source(source);

        let armor_stand_items = completion_items(
            source,
            &analysis,
            source
                .find("entity(\"minecraft:armor_stand\").nbt.")
                .unwrap()
                + "entity(\"minecraft:armor_stand\").nbt.".len(),
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
                .find("item(\"minecraft:diamond_sword\").nbt.")
                .unwrap()
                + "item(\"minecraft:diamond_sword\").nbt.".len(),
        );
        assert!(sword_items.iter().any(|item| item.label == "Damage"));
        assert!(sword_items.iter().any(|item| item.label == "Enchantments"));
    }

    #[test]
    fn falls_back_to_default_nbt_schema_for_dynamic_builder_ids() {
        let source = r#"
fn main() -> void:
    let id = "minecraft:unknown"
    let entity_value = entity(id)
    let block_value = block_type(id)
    let item_value = item(id)
    entity_value.nbt.
    block_value.nbt.
    item_value.nbt.
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
fn main() -> void:
    entity("minecraft:mannequin").nbt.pro
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
fn main() -> void:
    let player = player_ref(single(selector("@a")))
    entity("pig")
    item("diamond_swo")
    block("~ ~ ~").setblock("gold_bloc")
    player.playsound("entity.experience_orb.picku", "master")
    block("~ ~ ~").particle("happy_villag")
    player.effect("glowin", 3, 0)
    player.give("stick", 1)
    player.position.loot_spawn("chests/simple_dungeo")
"#;
        let analysis = analyze_source(source);

        let entity_items = completion_items(
            source,
            &analysis,
            source.find("entity(\"pig").unwrap() + "entity(\"pig".len(),
        );
        assert!(
            entity_items
                .iter()
                .any(|item| item.label == "minecraft:pig")
        );

        let item_items = completion_items(
            source,
            &analysis,
            source.find("item(\"diamond_swo").unwrap() + "item(\"diamond_swo".len(),
        );
        assert!(
            item_items
                .iter()
                .any(|item| item.label == "minecraft:diamond_sword")
        );

        let block_items = completion_items(
            source,
            &analysis,
            source.find("setblock(\"gold_bloc").unwrap() + "setblock(\"gold_bloc".len(),
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
                .find("playsound(\"entity.experience_orb.picku")
                .unwrap()
                + "playsound(\"entity.experience_orb.picku".len(),
        );
        assert!(
            sound_items
                .iter()
                .any(|item| item.label == "minecraft:entity.experience_orb.pickup")
        );

        let particle_items = completion_items(
            source,
            &analysis,
            source.find("particle(\"happy_villag").unwrap() + "particle(\"happy_villag".len(),
        );
        assert!(
            particle_items
                .iter()
                .any(|item| item.label == "minecraft:happy_villager")
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
            source.find("loot_spawn(\"chests/simple_dungeo").unwrap()
                + "loot_spawn(\"chests/simple_dungeo".len(),
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
fn main() -> void:
    let player = player_ref(single(selector("@a")))
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
fn main() -> void:
    entity("chicke
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
fn main() -> void:
    let matching = selector("@e[type=chicke,limit=1]")
    let negated = selector("@e[type=!zomb,limit=1]")
    debug_marker(block("~ ~ ~"), "marker", "gold_bloc")
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
fn main() -> void:
    let player = player_ref(single(selector("@a")))
    player.hotbar[0].id = "stick"
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
        let source = "fn main() -> void:\n    entity(\"pig\")\n";
        let analysis = analyze_source(source);
        let items = completion_items(
            source,
            &analysis,
            source.find("entity(\"pig").unwrap() + "entity(\"pig".len(),
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
fn helper() -> void:
    return
"#,
        );
        let main_source = r#"
fn main() -> void:
    helper()
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
