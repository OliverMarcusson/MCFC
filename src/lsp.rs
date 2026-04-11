use std::collections::{HashMap, HashSet};
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

#[derive(Debug, Clone)]
struct DocumentState {
    text: String,
    analysis: AnalysisResult,
}

#[derive(Debug)]
pub struct Backend {
    client: Client,
    documents: Arc<RwLock<HashMap<Url, DocumentState>>>,
}

impl Backend {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            documents: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    async fn update_document(&self, uri: Url, text: String) {
        let analysis = analyze_source(&text);
        self.publish_diagnostics(uri.clone(), &text, &analysis)
            .await;
        self.documents
            .write()
            .await
            .insert(uri, DocumentState { text, analysis });
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

        let Some(state) = self
            .documents
            .read()
            .await
            .get(&params.text_document.uri)
            .cloned()
        else {
            return;
        };
        self.publish_diagnostics(params.text_document.uri, &state.text, &state.analysis)
            .await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.documents
            .write()
            .await
            .remove(&params.text_document.uri);
        self.client
            .publish_diagnostics(params.text_document.uri, Vec::new(), None)
            .await;
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let documents = self.documents.read().await;
        let Some(state) = documents.get(&params.text_document_position_params.text_document.uri)
        else {
            return Ok(None);
        };
        let offset = position_to_offset(&state.text, params.text_document_position_params.position);
        let Some((word, range)) = word_at_offset(&state.text, offset) else {
            return Ok(None);
        };
        let Some(contents) = hover_contents(&state.analysis, offset, &word) else {
            return Ok(None);
        };

        Ok(Some(Hover {
            contents: HoverContents::Scalar(MarkedString::String(contents)),
            range: Some(range_from_text_range(&state.text, range)),
        }))
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let documents = self.documents.read().await;
        let state = documents.get(&params.text_document_position.text_document.uri);
        let items = match state {
            Some(state) => completion_items(
                &state.text,
                &state.analysis,
                position_to_offset(&state.text, params.text_document_position.position),
            ),
            None => static_completion_items(false, None),
        };
        Ok(Some(CompletionResponse::Array(items)))
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        let documents = self.documents.read().await;
        let Some(state) = documents.get(&params.text_document.uri) else {
            return Ok(Some(DocumentSymbolResponse::Nested(Vec::new())));
        };

        #[allow(deprecated)]
        let symbols = state
            .analysis
            .functions
            .iter()
            .map(|function| DocumentSymbol {
                name: function.name.clone(),
                detail: Some(function.signature()),
                kind: tower_lsp::lsp_types::SymbolKind::FUNCTION,
                tags: None,
                deprecated: None,
                range: range_from_text_range(&state.text, function.range),
                selection_range: range_from_text_range(&state.text, function.name_range),
                children: None,
            })
            .collect();

        Ok(Some(DocumentSymbolResponse::Nested(symbols)))
    }
}

fn hover_contents(analysis: &AnalysisResult, offset: usize, word: &str) -> Option<String> {
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
        "selector" => Some("```mcfc\nselector(value: string) -> entity_set\n```"),
        "single" => Some("```mcfc\nsingle(value: entity_set) -> entity_ref\n```"),
        "exists" => Some("```mcfc\nexists(value: entity_ref) -> bool\n```"),
        "block" => Some("```mcfc\nblock(position: string) -> block_ref\n```"),
        "at" => Some(
            "```mcfc\nat(anchor: entity_ref, value: entity_set|entity_ref|block_ref) -> entity_set|entity_ref|block_ref\n```",
        ),
        "int" => Some("```mcfc\nint(value: nbt) -> int\n```"),
        "bool" => Some("```mcfc\nbool(value: nbt) -> bool\n```"),
        "string" => Some("```mcfc\nstring(value: nbt) -> string\n```"),
        "len" => Some("```mcfc\narray<T>.len() -> int\n```"),
        "push" => Some("```mcfc\narray<T>.push(value: T) -> void\n```"),
        "pop" => Some("```mcfc\narray<T>.pop() -> T\n```"),
        "has" => Some("```mcfc\ndict<T>.has(key: string) -> bool\n```"),
        "remove" => Some("```mcfc\ndict<T>.remove(key: string) -> void\n```"),
        "effect" => {
            Some("```mcfc\nplayer.effect(name: string, duration: int, amplifier: int) -> void\n```")
        }
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
            ("has", "dict<T>.has(key: string) -> bool", "has(${1:key})"),
            (
                "remove",
                "dict<T>.remove(key: string) -> void",
                "remove(${1:key})",
            ),
            (
                "effect",
                "player.effect(name: string, duration: int, amplifier: int) -> void",
                "effect(${1:name}, ${2:duration}, ${3:amplifier})",
            ),
            ("nbt", "player.nbt.* read namespace", "nbt"),
            ("state", "player.state.* read/write namespace", "state"),
            ("tags", "player.tags.* read/write namespace", "tags"),
            ("team", "player.team writable string", "team"),
            (
                "mainhand",
                "player.mainhand.* writable namespace",
                "mainhand",
            ),
        ]
        .into_iter()
        .map(|(label, detail, insert_text)| {
            snippet_item(label, CompletionItemKind::METHOD, detail, insert_text)
        })
        .collect();
    }

    let mut items = Vec::new();
    for keyword in [
        "fn", "let", "return", "end", "if", "else", "while", "for", "in", "break", "continue",
        "mc", "mcf", "true", "false", "and", "or", "not", "@book",
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
            "block",
            "block(position: string) -> block_ref",
            "block(${1:\"~ ~ ~\"})",
        ),
        (
            "at",
            "at(anchor: entity_ref, value: entity_set|entity_ref|block_ref)",
            "at(${1:anchor}, ${2:value})",
        ),
        ("int", "int(value: nbt) -> int", "int(${1:value})"),
        ("bool", "bool(value: nbt) -> bool", "bool(${1:value})"),
        (
            "string",
            "string(value: nbt) -> string",
            "string(${1:value})",
        ),
    ] {
        items.push(snippet_item(
            label,
            CompletionItemKind::FUNCTION,
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
    let Some(base_name) = chain.first() else {
        return Vec::new();
    };

    if chain.len() > 1 {
        return nested_member_completion_items(chain);
    }

    match local_type_at_offset(source, analysis, offset, base_name) {
        Some(Type::Array(_)) => array_method_items(),
        Some(Type::Dict(_)) => dict_method_items(),
        Some(Type::EntityRef) => player_root_items(),
        Some(Type::BlockRef | Type::Nbt) => Vec::new(),
        Some(_) => Vec::new(),
        None => broad_member_items(),
    }
}

fn nested_member_completion_items(chain: &[String]) -> Vec<CompletionItem> {
    match chain.get(1).map(String::as_str) {
        Some("mainhand") => ["name", "item", "count"]
            .into_iter()
            .map(|label| {
                snippet_item(
                    label,
                    CompletionItemKind::FIELD,
                    "player.mainhand field",
                    label,
                )
            })
            .collect(),
        Some("team" | "state" | "tags" | "nbt") => Vec::new(),
        _ => Vec::new(),
    }
}

fn broad_member_items() -> Vec<CompletionItem> {
    [
        array_method_items(),
        dict_method_items(),
        player_root_items(),
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

fn player_root_items() -> Vec<CompletionItem> {
    [
        (
            "effect",
            "player.effect(name: string, duration: int, amplifier: int) -> void",
            "effect(${1:name}, ${2:duration}, ${3:amplifier})",
            CompletionItemKind::METHOD,
        ),
        (
            "nbt",
            "player.nbt.* read namespace",
            "nbt",
            CompletionItemKind::FIELD,
        ),
        (
            "state",
            "player.state.* read/write namespace",
            "state",
            CompletionItemKind::FIELD,
        ),
        (
            "tags",
            "player.tags.* read/write namespace",
            "tags",
            CompletionItemKind::FIELD,
        ),
        (
            "team",
            "player.team writable string",
            "team",
            CompletionItemKind::FIELD,
        ),
        (
            "mainhand",
            "player.mainhand.* writable namespace",
            "mainhand",
            CompletionItemKind::FIELD,
        ),
    ]
    .into_iter()
    .map(|(label, detail, insert_text, kind)| snippet_item(label, kind, detail, insert_text))
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
) -> Option<Type> {
    if let Some(function) = function_at_offset(analysis, offset) {
        if let Some(local) = analysis
            .locals
            .iter()
            .find(|local| local.function == function.name && local.name == name)
        {
            return Some(local.ty.clone());
        }
    }

    syntactic_locals_at_offset(source, offset)
        .into_iter()
        .find(|local| local.name == name)
        .and_then(|local| local.ty)
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
    (line.starts_with("if ") || line.starts_with("while ") || line.starts_with("for "))
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
    use tower_lsp::lsp_types::Position;

    use super::{completion_items, offset_to_position, position_to_offset, range_from_text_range};
    use crate::analysis::analyze_source;
    use crate::diagnostics::TextRange;

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
        assert!(!values_items.iter().any(|item| item.label == "team"));

        let me_items = completion_items(source, &analysis, source.find("me.").unwrap() + 3);
        assert!(me_items.iter().any(|item| item.label == "team"));
        assert!(!me_items.iter().any(|item| item.label == "push"));
    }
}
