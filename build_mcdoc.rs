use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Read;

use flate2::read::GzDecoder;
use tar::Archive;

pub const TARGET_MINECRAFT_VERSION: &str = "26.1.2";
pub const TARGET_VANILLA_MCDOC_REF: &str = "6ef5413a6b0dcd4cbf448aedeebead491221c5cb";

const DEFAULT_ENTITY_SYMBOL: &str = "::java::world::entity::mob::MobBase";
const DEFAULT_BLOCK_SYMBOL: &str = "::java::world::block::container::ContainerBase";
const DEFAULT_ITEM_SYMBOL: &str = "::java::world::item::ItemBase";

#[derive(Debug, Clone)]
pub struct NbtSchemaSnapshot {
    pub entity: BTreeMap<String, SchemaNode>,
    pub block: BTreeMap<String, SchemaNode>,
    pub item: BTreeMap<String, SchemaNode>,
}

#[derive(Debug, Clone)]
pub struct SchemaNode {
    pub detail: String,
    pub documentation: String,
    pub fields: Vec<SchemaField>,
}

#[derive(Debug, Clone)]
pub struct SchemaField {
    pub name: String,
    pub detail: String,
    pub documentation: String,
    pub node: Option<SchemaNode>,
}

pub fn build_nbt_schema_snapshot_from_tarball(
    tarball: &[u8],
    block_ids: &[String],
    item_ids: &[String],
    entity_ids: &[String],
) -> Result<NbtSchemaSnapshot, String> {
    let files = read_mcdoc_files_from_tarball(tarball)?;
    let parsed = ParsedRepo::parse(&files)?;
    let reducer = SchemaReducer::new(parsed, Version::parse(TARGET_MINECRAFT_VERSION)?);

    let mut entity = BTreeMap::new();
    let mut block = BTreeMap::new();
    let mut item = BTreeMap::new();

    if let Some(node) = reducer.schema_for_symbol(DEFAULT_ENTITY_SYMBOL) {
        entity.insert("__default__".to_string(), node);
    }
    if let Some(node) = reducer.schema_for_symbol(DEFAULT_BLOCK_SYMBOL) {
        block.insert("__default__".to_string(), node);
    }
    if let Some(node) = reducer.schema_for_symbol(DEFAULT_ITEM_SYMBOL) {
        item.insert("__default__".to_string(), node);
    }

    for id in entity_ids {
        if let Some(node) = reducer.schema_for_dispatch("minecraft:entity", id) {
            entity.insert(format!("minecraft:{id}"), node);
        }
    }
    for id in block_ids {
        if let Some(node) = reducer.schema_for_dispatch("minecraft:block", id) {
            block.insert(format!("minecraft:{id}"), node);
        }
    }
    for id in item_ids {
        if let Some(node) = reducer.schema_for_dispatch("minecraft:item", id) {
            item.insert(format!("minecraft:{id}"), node);
        }
    }

    Ok(NbtSchemaSnapshot {
        entity,
        block,
        item,
    })
}

fn read_mcdoc_files_from_tarball(tarball: &[u8]) -> Result<BTreeMap<String, String>, String> {
    let mut archive = Archive::new(GzDecoder::new(tarball));
    let mut files = BTreeMap::new();
    for entry in archive.entries().map_err(|error| error.to_string())? {
        let mut entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path().map_err(|error| error.to_string())?;
        let path = path.to_string_lossy().into_owned();
        if !path.ends_with(".mcdoc") {
            continue;
        }
        let Some(relative) = path.split_once('/') else {
            continue;
        };
        let include = relative.1.starts_with("java/world/") || relative.1 == "java/util/avatar.mcdoc";
        if !include {
            continue;
        }
        let mut content = String::new();
        entry.read_to_string(&mut content)
            .map_err(|error| error.to_string())?;
        files.insert(relative.1.to_string(), content);
    }
    if files.is_empty() {
        return Err("vanilla-mcdoc tarball did not contain any .mcdoc files".to_string());
    }
    Ok(files)
}

#[derive(Debug, Clone, Default)]
struct Metadata {
    documentation: String,
    since: Option<Version>,
    until: Option<Version>,
    canonical: bool,
}

impl Metadata {
    fn is_active(&self, version: &Version) -> bool {
        if let Some(since) = &self.since {
            if version < since {
                return false;
            }
        }
        if let Some(until) = &self.until {
            if version >= until {
                return false;
            }
        }
        true
    }

}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Version(Vec<u32>);

impl Version {
    fn parse(input: &str) -> Result<Self, String> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err("version must not be empty".to_string());
        }
        let mut parts = Vec::new();
        for part in trimmed.split('.') {
            let numeric = part
                .parse::<u32>()
                .map_err(|_| format!("invalid version component '{part}' in '{input}'"))?;
            parts.push(numeric);
        }
        Ok(Self(parts))
    }
}

#[derive(Debug, Clone)]
struct DefinitionContext {
    module_path: String,
    uses: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
struct SymbolDef {
    meta: Metadata,
    context: DefinitionContext,
    kind: SymbolKind,
}

#[derive(Debug, Clone)]
enum SymbolKind {
    Struct(StructDef),
    Alias(TypeExpr),
    Enum { underlying: String },
}

#[derive(Debug, Clone)]
struct DispatchDef {
    meta: Metadata,
    context: DefinitionContext,
    target: TypeExpr,
}

#[derive(Debug, Clone)]
struct StructDef {
    name: Option<String>,
    entries: Vec<StructEntry>,
}

#[derive(Debug, Clone)]
enum StructEntry {
    Field {
        meta: Metadata,
        name: String,
        ty: TypeExpr,
    },
    Spread {
        meta: Metadata,
        ty: TypeExpr,
    },
    Dynamic {
        meta: Metadata,
        _value_ty: TypeExpr,
    },
}

#[derive(Debug, Clone)]
enum TypeExpr {
    Primitive(String),
    Struct(StructDef),
    Reference(String),
    DispatchRef(DispatchRef),
    List(Box<TypeExpr>),
    Union(Vec<UnionVariant>),
    Enum(String),
    Unknown(()),
}

#[derive(Debug, Clone)]
struct DispatchRef {
    dispatcher: String,
    key: DispatchRefKey,
}

#[derive(Debug, Clone)]
enum DispatchRefKey {
    Literal(Vec<String>),
    Dynamic(()),
}

#[derive(Debug, Clone)]
struct UnionVariant {
    meta: Metadata,
    ty: TypeExpr,
}

#[derive(Debug)]
struct ParsedRepo {
    symbols: BTreeMap<String, SymbolDef>,
    dispatches: BTreeMap<String, BTreeMap<String, Vec<DispatchDef>>>,
}

impl ParsedRepo {
    fn parse(files: &BTreeMap<String, String>) -> Result<Self, String> {
        let mut repo = ParsedRepo {
            symbols: BTreeMap::new(),
            dispatches: BTreeMap::new(),
        };
        for (path, source) in files {
            let module_path = module_path_for_file(path);
            let mut parser = FileParser::new(path, source, &module_path, &mut repo);
            parser.parse()?;
        }
        Ok(repo)
    }

    fn insert_symbol(&mut self, path: String, def: SymbolDef) {
        self.symbols.insert(path, def);
    }

    fn insert_dispatch(&mut self, dispatcher: String, key: String, def: DispatchDef) {
        self.dispatches
            .entry(dispatcher)
            .or_default()
            .entry(key)
            .or_default()
            .push(def);
    }
}

fn module_path_for_file(path: &str) -> String {
    let without_extension = path.trim_end_matches(".mcdoc");
    let mut segments = without_extension.split('/').collect::<Vec<_>>();
    if segments.last() == Some(&"mod") {
        segments.pop();
    }
    if segments.is_empty() {
        return "::".to_string();
    }
    format!("::{}", segments.join("::"))
}

#[derive(Debug, Clone)]
struct Token {
    kind: TokenKind,
}

#[derive(Debug, Clone)]
enum TokenKind {
    Word(String),
    String(String),
    Doc(String),
    AttrStart,
    Spread,
    OptionalColon,
    Symbol(char),
}

struct Lexer<'a> {
    input: &'a str,
    index: usize,
}

impl<'a> Lexer<'a> {
    fn new(input: &'a str) -> Self {
        Self { input, index: 0 }
    }

    fn tokenize(mut self) -> Result<Vec<Token>, String> {
        let mut tokens = Vec::new();
        while let Some(ch) = self.peek_char() {
            if ch.is_whitespace() {
                self.index += ch.len_utf8();
                continue;
            }
            if self.starts_with("///") {
                self.index += 3;
                let start = self.index;
                while let Some(ch) = self.peek_char() {
                    if ch == '\n' {
                        break;
                    }
                    self.index += ch.len_utf8();
                }
                tokens.push(Token {
                    kind: TokenKind::Doc(self.input[start..self.index].trim().to_string()),
                });
                continue;
            }
            if self.starts_with("//") {
                while let Some(ch) = self.peek_char() {
                    self.index += ch.len_utf8();
                    if ch == '\n' {
                        break;
                    }
                }
                continue;
            }
            if self.starts_with("#[") {
                self.index += 2;
                tokens.push(Token {
                    kind: TokenKind::AttrStart,
                });
                continue;
            }
            if self.starts_with("...") {
                self.index += 3;
                tokens.push(Token {
                    kind: TokenKind::Spread,
                });
                continue;
            }
            if self.starts_with("?:") {
                self.index += 2;
                tokens.push(Token {
                    kind: TokenKind::OptionalColon,
                });
                continue;
            }
            if matches!(ch, '{' | '}' | '(' | ')' | '[' | ']' | '<' | '>' | ',' | '=' | '|' | '@')
                || (ch == ':' && !self.starts_with("::"))
            {
                self.index += ch.len_utf8();
                tokens.push(Token {
                    kind: TokenKind::Symbol(ch),
                });
                continue;
            }
            if ch == '"' || ch == '\'' {
                tokens.push(Token {
                    kind: TokenKind::String(self.read_string(ch)?),
                });
                continue;
            }
            tokens.push(Token {
                kind: TokenKind::Word(self.read_word()),
            });
        }
        Ok(tokens)
    }

    fn read_string(&mut self, delimiter: char) -> Result<String, String> {
        self.index += delimiter.len_utf8();
        let mut parsed = String::new();
        let mut escaped = false;
        while let Some(ch) = self.peek_char() {
            self.index += ch.len_utf8();
            if escaped {
                parsed.push(ch);
                escaped = false;
                continue;
            }
            match ch {
                '\\' => escaped = true,
                value if value == delimiter => return Ok(parsed),
                _ => parsed.push(ch),
            }
        }
        Err("unterminated string literal in mcdoc".to_string())
    }

    fn read_word(&mut self) -> String {
        let start = self.index;
        while let Some(ch) = self.peek_char() {
            if ch.is_whitespace()
                || matches!(ch, '{' | '}' | '(' | ')' | '[' | ']' | '<' | '>' | ',' | '=' | '|' | '@')
                || (ch == ':' && !self.starts_with("::") && !self.colon_belongs_to_word())
                || (ch == '?' && self.starts_with("?:"))
                || (ch == '#' && self.starts_with("#["))
            {
                break;
            }
            self.index += ch.len_utf8();
        }
        self.input[start..self.index].to_string()
    }

    fn colon_belongs_to_word(&self) -> bool {
        let mut chars = self.input[self.index..].chars();
        let _ = chars.next();
        let Some(next) = chars.next() else {
            return false;
        };
        next.is_ascii_alphanumeric() || next == '_' || next == '%' || next == ':'
    }

    fn starts_with(&self, value: &str) -> bool {
        self.input[self.index..].starts_with(value)
    }

    fn peek_char(&self) -> Option<char> {
        self.input[self.index..].chars().next()
    }
}

struct FileParser<'a> {
    path: &'a str,
    tokens: Vec<Token>,
    index: usize,
    module_path: String,
    uses: BTreeMap<String, String>,
    repo: &'a mut ParsedRepo,
}

impl<'a> FileParser<'a> {
    fn new(path: &'a str, source: &'a str, module_path: &str, repo: &'a mut ParsedRepo) -> Self {
        let tokens = Lexer::new(source)
            .tokenize()
            .unwrap_or_else(|error| panic!("failed to tokenize {path}: {error}"));
        Self {
            path,
            tokens,
            index: 0,
            module_path: module_path.to_string(),
            uses: BTreeMap::new(),
            repo,
        }
    }

    fn parse(&mut self) -> Result<(), String> {
        while self.index < self.tokens.len() {
            let meta = self.parse_metadata()?;
            if self.eat_word("use") {
                let path = self.expect_word()?;
                let absolute = self.resolve_path(&path);
                let alias = absolute
                    .split("::")
                    .filter(|segment| !segment.is_empty())
                    .last()
                    .ok_or_else(|| format!("invalid use path '{absolute}' in {}", self.path))?;
                self.uses.insert(alias.to_string(), absolute);
                continue;
            }
            if self.eat_word("struct") {
                let name = self.expect_word()?;
                let def = self.parse_struct_after_name(Some(name.clone()), meta.clone())?;
                let path = self.symbol_path(&name);
                self.repo.insert_symbol(
                    path,
                    SymbolDef {
                        meta,
                        context: self.context(),
                        kind: SymbolKind::Struct(def),
                    },
                );
                continue;
            }
            if self.eat_word("type") {
                let name = self.expect_word()?;
                self.skip_generic_args();
                self.expect_symbol('=')?;
                let expr = self.parse_type_expr()?;
                let path = self.symbol_path(&name);
                self.repo.insert_symbol(
                    path,
                    SymbolDef {
                        meta,
                        context: self.context(),
                        kind: SymbolKind::Alias(expr),
                    },
                );
                continue;
            }
            if self.eat_word("enum") {
                let underlying = self.parse_enum_underlying()?;
                let name = self.expect_word()?;
                self.skip_block()?;
                let path = self.symbol_path(&name);
                self.repo.insert_symbol(
                    path,
                    SymbolDef {
                        meta,
                        context: self.context(),
                        kind: SymbolKind::Enum { underlying },
                    },
                );
                continue;
            }
            if self.eat_word("dispatch") {
                let dispatcher = self.expect_word()?;
                let keys = self.parse_dispatch_keys_definition()?;
                self.expect_word_exact("to")?;
                let target = self.parse_type_expr()?;
                for key in keys {
                    self.repo.insert_dispatch(
                        dispatcher.clone(),
                        key,
                        DispatchDef {
                            meta: meta.clone(),
                            context: self.context(),
                            target: target.clone(),
                        },
                    );
                }
                continue;
            }

            self.index += 1;
        }
        Ok(())
    }

    fn context(&self) -> DefinitionContext {
        DefinitionContext {
            module_path: self.module_path.clone(),
            uses: self.uses.clone(),
        }
    }

    fn symbol_path(&self, name: &str) -> String {
        if self.module_path == "::" {
            format!("::{name}")
        } else {
            format!("{}::{name}", self.module_path)
        }
    }

    fn parse_metadata(&mut self) -> Result<Metadata, String> {
        let mut meta = Metadata::default();
        let mut docs = Vec::new();
        loop {
            match self.peek() {
                Some(TokenKind::Doc(doc)) => {
                    docs.push(doc.clone());
                    self.index += 1;
                }
                Some(TokenKind::AttrStart) => {
                    self.index += 1;
                    let attribute = self.expect_word()?;
                    let value = if self.eat_symbol('=') {
                        match self.next() {
                            Some(TokenKind::String(value)) | Some(TokenKind::Word(value)) => {
                                Some(value)
                            }
                            other => {
                                return Err(format!(
                                    "unexpected attribute value token {:?} in {}",
                                    other, self.path
                                ))
                            }
                        }
                    } else {
                        None
                    };
                    self.skip_until_matching_bracket()?;
                    match attribute.as_str() {
                        "since" => {
                            if let Some(value) = value {
                                meta.since = Some(Version::parse(&value)?);
                            }
                        }
                        "until" => {
                            if let Some(value) = value {
                                meta.until = Some(Version::parse(&value)?);
                            }
                        }
                        "canonical" => meta.canonical = true,
                        _ => {}
                    }
                }
                _ => break,
            }
        }
        meta.documentation = docs.join("\n");
        Ok(meta)
    }

    fn parse_struct_after_name(
        &mut self,
        name: Option<String>,
        meta: Metadata,
    ) -> Result<StructDef, String> {
        let mut entries = Vec::new();
        if self.eat_word("extends") {
            let ty = self.parse_type_expr()?;
            entries.push(StructEntry::Spread { meta, ty });
        }
        self.expect_symbol('{')?;
        while !self.eat_symbol('}') {
            while self.eat_symbol(',') {}
            let entry_meta = self.parse_metadata()?;
            while self.eat_symbol(',') {}
            if self.eat_symbol('}') {
                break;
            }
            if self.eat_spread() {
                let ty = self.parse_type_expr()?;
                entries.push(StructEntry::Spread {
                    meta: entry_meta,
                    ty,
                });
                while self.eat_symbol(',') {}
                continue;
            }
            if self.eat_symbol('[') {
                self.skip_balanced('[', ']')?;
                if !self.eat_optional_colon() {
                    self.expect_symbol(':')?;
                }
                let value_ty = self.parse_type_expr()?;
                entries.push(StructEntry::Dynamic {
                    meta: entry_meta,
                    _value_ty: value_ty,
                });
                while self.eat_symbol(',') {}
                continue;
            }
            let name = self.expect_word()?;
            if !self.eat_optional_colon() {
                self.expect_symbol(':')?;
            }
            let ty = self.parse_type_expr()?;
            entries.push(StructEntry::Field {
                meta: entry_meta,
                name,
                ty,
            });
            while self.eat_symbol(',') {}
        }
        Ok(StructDef { name, entries })
    }

    fn parse_type_expr(&mut self) -> Result<TypeExpr, String> {
        let mut variants = Vec::new();
        loop {
            let meta = self.parse_metadata()?;
            if matches!(self.peek(), Some(TokenKind::Symbol(')')) | Some(TokenKind::Symbol('}')) | Some(TokenKind::Symbol(']'))) {
                break;
            }
            let ty = self.parse_type_atom()?;
            self.skip_annotations()?;
            variants.push(UnionVariant { meta, ty });
            if !self.eat_symbol('|') {
                break;
            }
            if matches!(self.peek(), Some(TokenKind::Symbol(')')) | Some(TokenKind::Symbol('}')) | Some(TokenKind::Symbol(']'))) {
                break;
            }
        }
        if variants.len() == 1 {
            Ok(variants.pop().unwrap().ty)
        } else {
            Ok(TypeExpr::Union(variants))
        }
    }

    fn parse_type_atom(&mut self) -> Result<TypeExpr, String> {
        if self.eat_word("struct") {
            if matches!(self.peek(), Some(TokenKind::Word(_))) {
                let name = self.expect_word()?;
                let def = self.parse_struct_after_name(Some(name.clone()), Metadata::default())?;
                let path = self.symbol_path(&name);
                self.repo.insert_symbol(
                    path.clone(),
                    SymbolDef {
                        meta: Metadata::default(),
                        context: self.context(),
                        kind: SymbolKind::Struct(def),
                    },
                );
                return Ok(TypeExpr::Reference(path));
            }
            let def = self.parse_struct_after_name(None, Metadata::default())?;
            return Ok(TypeExpr::Struct(def));
        }
        if self.eat_word("enum") {
            let underlying = self.parse_enum_underlying()?;
            if matches!(self.peek(), Some(TokenKind::Word(_))) && self.peek_next_is_symbol('{') {
                let name = self.expect_word()?;
                self.skip_block()?;
                let path = self.symbol_path(&name);
                self.repo.insert_symbol(
                    path.clone(),
                    SymbolDef {
                        meta: Metadata::default(),
                        context: self.context(),
                        kind: SymbolKind::Enum {
                            underlying: underlying.clone(),
                        },
                    },
                );
                return Ok(TypeExpr::Reference(path));
            }
            self.skip_block()?;
            return Ok(TypeExpr::Enum(underlying));
        }
        if self.eat_symbol('(') {
            let inner = self.parse_type_expr()?;
            self.expect_symbol(')')?;
            return Ok(inner);
        }
        if self.eat_symbol('[') {
            let item = self.parse_type_expr()?;
            self.expect_symbol(']')?;
            return Ok(TypeExpr::List(Box::new(item)));
        }
        let raw = match self.next() {
            Some(TokenKind::Word(value)) => value,
            Some(TokenKind::String(_value)) => return Ok(TypeExpr::Unknown(())),
            other => {
                return Err(format!(
                    "unexpected token {:?} while parsing type in {}",
                    other, self.path
                ))
            }
        };

        self.skip_generic_args();

        let mut base = match raw.as_str() {
            "any" | "boolean" | "string" | "int" | "short" | "long" | "byte" | "float"
            | "double" => TypeExpr::Primitive(raw.clone()),
            _ => TypeExpr::Reference(raw.clone()),
        };

        loop {
            if !self.eat_symbol('[') {
                break;
            }
            if self.eat_symbol(']') {
                base = TypeExpr::List(Box::new(base));
                continue;
            }
            let key = if self.eat_symbol('[') {
                let dynamic = self.collect_until_closing(']')?;
                self.expect_symbol(']')?;
                self.expect_symbol(']')?;
                let _ = dynamic;
                DispatchRefKey::Dynamic(())
            } else {
                let mut keys = Vec::new();
                loop {
                    match self.next() {
                        Some(TokenKind::Word(value)) | Some(TokenKind::String(value)) => {
                            keys.push(value)
                        }
                        other => {
                            return Err(format!(
                                "unexpected dispatch key token {:?} in {}",
                                other, self.path
                            ))
                        }
                    }
                    if self.eat_symbol(',') {
                        continue;
                    }
                    self.expect_symbol(']')?;
                    break;
                }
                DispatchRefKey::Literal(keys)
            };
            return Ok(TypeExpr::DispatchRef(DispatchRef {
                dispatcher: raw,
                key,
            }));
        }

        Ok(base)
    }

    fn parse_dispatch_keys_definition(&mut self) -> Result<Vec<String>, String> {
        self.expect_symbol('[')?;
        let mut keys = Vec::new();
        loop {
            while self.eat_symbol(',') {}
            if self.eat_symbol(']') {
                break;
            }
            match self.next() {
                Some(TokenKind::Word(value)) | Some(TokenKind::String(value)) => keys.push(value),
                other => {
                    return Err(format!(
                        "unexpected dispatch key token {:?} in {}",
                        other, self.path
                    ))
                }
            }
            if self.eat_symbol(',') {
                continue;
            }
            self.expect_symbol(']')?;
            break;
        }
        Ok(keys)
    }

    fn parse_enum_underlying(&mut self) -> Result<String, String> {
        self.expect_symbol('(')?;
        let underlying = self.expect_word()?;
        self.expect_symbol(')')?;
        Ok(underlying)
    }

    fn skip_annotations(&mut self) -> Result<(), String> {
        while self.eat_symbol('@') {
            let mut depth = 0usize;
            while let Some(token) = self.peek() {
                match token {
                    TokenKind::Symbol('(') | TokenKind::Symbol('[') | TokenKind::Symbol('{') => {
                        depth += 1;
                        self.index += 1;
                    }
                    TokenKind::Symbol(')') | TokenKind::Symbol(']') | TokenKind::Symbol('}') => {
                        if depth == 0 {
                            break;
                        }
                        depth = depth.saturating_sub(1);
                        self.index += 1;
                    }
                    TokenKind::Symbol('|') | TokenKind::Symbol(',') if depth == 0 => break,
                    TokenKind::Doc(_) | TokenKind::AttrStart if depth == 0 => break,
                    _ => self.index += 1,
                }
            }
        }
        Ok(())
    }

    fn skip_generic_args(&mut self) {
        if !self.eat_symbol('<') {
            return;
        }
        let mut depth = 1usize;
        while let Some(token) = self.next() {
            match token {
                TokenKind::Symbol('<') => depth += 1,
                TokenKind::Symbol('>') => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
        }
    }

    fn skip_block(&mut self) -> Result<(), String> {
        self.expect_symbol('{')?;
        self.skip_balanced('{', '}')
    }

    fn skip_balanced(&mut self, open: char, close: char) -> Result<(), String> {
        let mut depth = 1usize;
        while let Some(token) = self.next() {
            match token {
                TokenKind::Symbol(ch) if ch == open => depth += 1,
                TokenKind::AttrStart if open == '[' && close == ']' => depth += 1,
                TokenKind::Symbol(ch) if ch == close => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return Ok(());
                    }
                }
                _ => {}
            }
        }
        Err(format!("unterminated balanced block in {}", self.path))
    }

    fn collect_until_closing(&mut self, close: char) -> Result<String, String> {
        let mut pieces = Vec::new();
        while let Some(token) = self.next() {
            match token {
                TokenKind::Symbol(ch) if ch == close => {
                    self.index = self.index.saturating_sub(1);
                    return Ok(pieces.join(""));
                }
                TokenKind::Word(value) | TokenKind::String(value) => pieces.push(value),
                TokenKind::Symbol(ch) => pieces.push(ch.to_string()),
                TokenKind::Spread => pieces.push("...".to_string()),
                TokenKind::OptionalColon => pieces.push("?:".to_string()),
                TokenKind::AttrStart => pieces.push("#[".to_string()),
                TokenKind::Doc(doc) => pieces.push(doc),
            }
        }
        Err(format!("unterminated dispatch key in {}", self.path))
    }

    fn skip_until_matching_bracket(&mut self) -> Result<(), String> {
        let mut depth = 1usize;
        while let Some(token) = self.next() {
            match token {
                TokenKind::AttrStart | TokenKind::Symbol('[') => depth += 1,
                TokenKind::Symbol(']') => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return Ok(());
                    }
                }
                _ => {}
            }
        }
        Err(format!("unterminated attribute in {}", self.path))
    }

    fn resolve_path(&self, raw: &str) -> String {
        if raw.starts_with("::") {
            return raw.to_string();
        }
        if raw.starts_with("super::") {
            let mut module = self
                .module_path
                .split("::")
                .filter(|segment| !segment.is_empty())
                .collect::<Vec<_>>();
            let mut remainder = raw;
            while let Some(next) = remainder.strip_prefix("super::") {
                let _ = module.pop();
                remainder = next;
            }
            if remainder.is_empty() {
                return format!("::{}", module.join("::"));
            }
            if module.is_empty() {
                return format!("::{remainder}");
            }
            return format!("::{}::{remainder}", module.join("::"));
        }
        if let Some(absolute) = self.uses.get(raw) {
            return absolute.clone();
        }
        if let Some((head, tail)) = raw.split_once("::") {
            if let Some(absolute) = self.uses.get(head) {
                return format!("{}::{tail}", absolute);
            }
        }
        if self.module_path == "::" {
            format!("::{raw}")
        } else {
            format!("{}::{raw}", self.module_path)
        }
    }

    fn peek(&self) -> Option<&TokenKind> {
        self.tokens.get(self.index).map(|token| &token.kind)
    }

    fn peek_next(&self) -> Option<&TokenKind> {
        self.tokens.get(self.index + 1).map(|token| &token.kind)
    }

    fn peek_next_is_symbol(&self, symbol: char) -> bool {
        matches!(self.peek_next(), Some(TokenKind::Symbol(value)) if *value == symbol)
    }

    fn next(&mut self) -> Option<TokenKind> {
        let token = self.tokens.get(self.index).cloned();
        self.index += usize::from(token.is_some());
        token.map(|token| token.kind)
    }

    fn eat_word(&mut self, expected: &str) -> bool {
        match self.peek() {
            Some(TokenKind::Word(word)) if word == expected => {
                self.index += 1;
                true
            }
            _ => false,
        }
    }

    fn expect_word_exact(&mut self, expected: &str) -> Result<(), String> {
        if self.eat_word(expected) {
            Ok(())
        } else {
            Err(format!("expected '{expected}' in {}", self.path))
        }
    }

    fn expect_word(&mut self) -> Result<String, String> {
        match self.next() {
            Some(TokenKind::Word(word)) => Ok(word),
            other => Err(format!(
                "expected word token in {}, got {:?}; next tokens: {:?}",
                self.path,
                other,
                self.tokens
                    .iter()
                    .skip(self.index.saturating_sub(1))
                    .take(8)
                    .map(|token| &token.kind)
                    .collect::<Vec<_>>()
            )),
        }
    }

    fn eat_symbol(&mut self, expected: char) -> bool {
        match self.peek() {
            Some(TokenKind::Symbol(value)) if *value == expected => {
                self.index += 1;
                true
            }
            _ => false,
        }
    }

    fn expect_symbol(&mut self, expected: char) -> Result<(), String> {
        if self.eat_symbol(expected) {
            Ok(())
        } else {
            Err(format!(
                "expected symbol '{expected}' in {}; next tokens: {:?}",
                self.path,
                self.tokens
                    .iter()
                    .skip(self.index)
                    .take(8)
                    .map(|token| &token.kind)
                    .collect::<Vec<_>>()
            ))
        }
    }

    fn eat_optional_colon(&mut self) -> bool {
        matches!(self.next_if(|kind| matches!(kind, TokenKind::OptionalColon)), Some(_))
    }

    fn eat_spread(&mut self) -> bool {
        matches!(self.next_if(|kind| matches!(kind, TokenKind::Spread)), Some(_))
    }

    fn next_if(&mut self, predicate: impl FnOnce(&TokenKind) -> bool) -> Option<TokenKind> {
        let should_take = self.peek().is_some_and(predicate);
        if should_take {
            self.next()
        } else {
            None
        }
    }
}

struct SchemaReducer {
    parsed: ParsedRepo,
    version: Version,
    symbol_cache: HashMap<String, Option<SchemaNode>>,
    dispatch_cache: HashMap<(String, String), Option<SchemaNode>>,
    resolving_symbols: HashSet<String>,
    resolving_dispatches: HashSet<(String, String)>,
}

impl SchemaReducer {
    fn new(parsed: ParsedRepo, version: Version) -> Self {
        Self {
            parsed,
            version,
            symbol_cache: HashMap::new(),
            dispatch_cache: HashMap::new(),
            resolving_symbols: HashSet::new(),
            resolving_dispatches: HashSet::new(),
        }
    }

    fn schema_for_dispatch(&self, dispatcher: &str, key_with_namespace: &str) -> Option<SchemaNode> {
        let key = key_with_namespace
            .strip_prefix("minecraft:")
            .unwrap_or(key_with_namespace);
        let mut reducer = self.clone_for_mutation();
        reducer.reduce_dispatch(dispatcher, key)
    }

    fn schema_for_symbol(&self, symbol: &str) -> Option<SchemaNode> {
        let mut reducer = self.clone_for_mutation();
        reducer.reduce_symbol(symbol)
    }

    fn clone_for_mutation(&self) -> Self {
        Self {
            parsed: ParsedRepo {
                symbols: self.parsed.symbols.clone(),
                dispatches: self.parsed.dispatches.clone(),
            },
            version: self.version.clone(),
            symbol_cache: HashMap::new(),
            dispatch_cache: HashMap::new(),
            resolving_symbols: HashSet::new(),
            resolving_dispatches: HashSet::new(),
        }
    }

    fn reduce_dispatch(&mut self, dispatcher: &str, key: &str) -> Option<SchemaNode> {
        let cache_key = (dispatcher.to_string(), key.to_string());
        if let Some(cached) = self.dispatch_cache.get(&cache_key) {
            return cached.clone();
        }
        if !self.resolving_dispatches.insert(cache_key.clone()) {
            return None;
        }
        let defs = self
            .parsed
            .dispatches
            .get(dispatcher)
            .and_then(|entries| entries.get(key))
            .cloned();
        let reduced = defs.and_then(|defs| {
            let mut nodes = Vec::new();
            for def in defs {
                if !def.meta.is_active(&self.version) {
                    continue;
                }
                if let Some(node) = self.reduce_type(&def.target, &def.context) {
                    nodes.push((def.meta.clone(), node));
                }
            }
            merge_schema_nodes(nodes)
        });
        self.resolving_dispatches.remove(&cache_key);
        self.dispatch_cache.insert(cache_key, reduced.clone());
        reduced
    }

    fn reduce_symbol(&mut self, symbol: &str) -> Option<SchemaNode> {
        if let Some(cached) = self.symbol_cache.get(symbol) {
            return cached.clone();
        }
        if !self.resolving_symbols.insert(symbol.to_string()) {
            return None;
        }
        let def = self.parsed.symbols.get(symbol).cloned();
        let reduced = def.and_then(|def| {
            if !def.meta.is_active(&self.version) {
                return None;
            }
            match &def.kind {
                SymbolKind::Struct(definition) => {
                    Some(self.reduce_struct(definition, &def.context, &def.meta))
                }
                SymbolKind::Alias(expr) => self.reduce_type(expr, &def.context).map(|mut node| {
                    if node.documentation.is_empty() {
                        node.documentation = def.meta.documentation.clone();
                    }
                    node
                }),
                SymbolKind::Enum { underlying } => Some(SchemaNode {
                    detail: underlying.clone(),
                    documentation: def.meta.documentation.clone(),
                    fields: Vec::new(),
                }),
            }
        });
        self.resolving_symbols.remove(symbol);
        self.symbol_cache.insert(symbol.to_string(), reduced.clone());
        reduced
    }

    fn reduce_type(&mut self, ty: &TypeExpr, context: &DefinitionContext) -> Option<SchemaNode> {
        match ty {
            TypeExpr::Struct(definition) => Some(self.reduce_struct(definition, context, &Metadata::default())),
            TypeExpr::Reference(raw) => self.reduce_symbol(&self.resolve_reference(raw, context)),
            TypeExpr::DispatchRef(reference) => match &reference.key {
                DispatchRefKey::Literal(keys) => {
                    let mut nodes = Vec::new();
                    for key in keys {
                        if let Some(node) = self.reduce_dispatch(&reference.dispatcher, key) {
                            nodes.push((Metadata::default(), node));
                        }
                    }
                    merge_schema_nodes(nodes)
                }
                DispatchRefKey::Dynamic(_) => None,
            },
            TypeExpr::Union(variants) => {
                let mut nodes = Vec::new();
                for variant in variants {
                    if !variant.meta.is_active(&self.version) {
                        continue;
                    }
                    if let Some(node) = self.reduce_type(&variant.ty, context) {
                        nodes.push((variant.meta.clone(), node));
                    }
                }
                merge_schema_nodes(nodes)
            }
            _ => None,
        }
    }

    fn reduce_struct(
        &mut self,
        definition: &StructDef,
        context: &DefinitionContext,
        meta: &Metadata,
    ) -> SchemaNode {
        let mut fields = Vec::new();
        for entry in &definition.entries {
            match entry {
                StructEntry::Field {
                    meta: entry_meta,
                    name,
                    ty,
                } if entry_meta.is_active(&self.version) => {
                    push_or_replace_field(
                        &mut fields,
                        SchemaField {
                            name: name.clone(),
                            detail: self.detail_for_type(ty, context),
                            documentation: entry_meta.documentation.clone(),
                            node: self.nested_node_for_type(ty, context),
                        },
                    );
                }
                StructEntry::Spread {
                    meta: entry_meta,
                    ty,
                } if entry_meta.is_active(&self.version) => {
                    if let Some(node) = self.reduce_type(ty, context) {
                        for field in node.fields {
                            push_or_replace_field(&mut fields, field);
                        }
                    }
                }
                StructEntry::Dynamic { meta: entry_meta, .. } if entry_meta.is_active(&self.version) => {}
                _ => {}
            }
        }
        SchemaNode {
            detail: definition
                .name
                .clone()
                .unwrap_or_else(|| "compound".to_string()),
            documentation: meta.documentation.clone(),
            fields,
        }
    }

    fn nested_node_for_type(
        &mut self,
        ty: &TypeExpr,
        context: &DefinitionContext,
    ) -> Option<SchemaNode> {
        match ty {
            TypeExpr::Struct(_) | TypeExpr::Reference(_) | TypeExpr::DispatchRef(_) | TypeExpr::Union(_) => {
                self.reduce_type(ty, context)
            }
            _ => None,
        }
    }

    fn detail_for_type(&mut self, ty: &TypeExpr, context: &DefinitionContext) -> String {
        match ty {
            TypeExpr::Primitive(name) => name.clone(),
            TypeExpr::Struct(_) => "compound".to_string(),
            TypeExpr::Reference(raw) => {
                let resolved = self.resolve_reference(raw, context);
                if let Some(symbol) = self.parsed.symbols.get(&resolved) {
                    match &symbol.kind {
                        SymbolKind::Enum { underlying } => underlying.clone(),
                        SymbolKind::Struct(_) | SymbolKind::Alias(_) => {
                            if self.reduce_symbol(&resolved).is_some() {
                                "compound".to_string()
                            } else {
                                tail_name(&resolved)
                            }
                        }
                    }
                } else {
                    tail_name(raw)
                }
            }
            TypeExpr::DispatchRef(reference) => match &reference.key {
                DispatchRefKey::Literal(keys) => {
                    if keys.iter().any(|key| self.reduce_dispatch(&reference.dispatcher, key).is_some()) {
                        "compound".to_string()
                    } else {
                        tail_name(&reference.dispatcher)
                    }
                }
                DispatchRefKey::Dynamic(_) => tail_name(&reference.dispatcher),
            },
            TypeExpr::List(item) => format!("list<{}>", self.detail_for_type(item, context)),
            TypeExpr::Union(variants) => {
                if variants.iter().any(|variant| {
                    variant.meta.is_active(&self.version)
                        && self.nested_node_for_type(&variant.ty, context).is_some()
                }) {
                    "compound".to_string()
                } else {
                    variants
                        .iter()
                        .find(|variant| variant.meta.is_active(&self.version))
                        .map(|variant| self.detail_for_type(&variant.ty, context))
                        .unwrap_or_else(|| "nbt".to_string())
                }
            }
            TypeExpr::Enum(underlying) => underlying.clone(),
            TypeExpr::Unknown(_) => "nbt".to_string(),
        }
    }

    fn resolve_reference(&self, raw: &str, context: &DefinitionContext) -> String {
        if raw.starts_with("::") {
            return raw.to_string();
        }
        if raw.starts_with("super::") {
            let mut module = context
                .module_path
                .split("::")
                .filter(|segment| !segment.is_empty())
                .collect::<Vec<_>>();
            let mut remainder = raw;
            while let Some(next) = remainder.strip_prefix("super::") {
                let _ = module.pop();
                remainder = next;
            }
            return if module.is_empty() {
                format!("::{remainder}")
            } else {
                format!("::{}::{remainder}", module.join("::"))
            };
        }
        if let Some(absolute) = context.uses.get(raw) {
            return absolute.clone();
        }
        if let Some((head, tail)) = raw.split_once("::") {
            if let Some(absolute) = context.uses.get(head) {
                return format!("{}::{tail}", absolute);
            }
        }
        if context.module_path == "::" {
            format!("::{raw}")
        } else {
            format!("{}::{raw}", context.module_path)
        }
    }
}

fn merge_schema_nodes(nodes: Vec<(Metadata, SchemaNode)>) -> Option<SchemaNode> {
    let mut iter = nodes.into_iter();
    let (meta, mut merged) = iter.next()?;
    if !meta.documentation.is_empty() && merged.documentation.is_empty() {
        merged.documentation = meta.documentation;
    }
    for (meta, node) in iter {
        if merged.documentation.is_empty() && !meta.documentation.is_empty() {
            merged.documentation = meta.documentation;
        }
        for field in node.fields {
            push_or_replace_field(&mut merged.fields, field);
        }
    }
    Some(merged)
}

fn push_or_replace_field(fields: &mut Vec<SchemaField>, field: SchemaField) {
    if let Some(index) = fields.iter().position(|existing| existing.name == field.name) {
        fields[index] = merge_fields(fields[index].clone(), field);
    } else {
        fields.push(field);
    }
}

fn merge_fields(existing: SchemaField, incoming: SchemaField) -> SchemaField {
    SchemaField {
        name: existing.name,
        detail: if existing.detail == "nbt" {
            incoming.detail
        } else {
            existing.detail
        },
        documentation: if existing.documentation.is_empty() {
            incoming.documentation
        } else {
            existing.documentation
        },
        node: match (existing.node, incoming.node) {
            (Some(left), Some(right)) => merge_schema_nodes(vec![
                (Metadata::default(), left),
                (Metadata::default(), right),
            ]),
            (Some(left), None) => Some(left),
            (None, Some(right)) => Some(right),
            (None, None) => None,
        },
    }
}

fn tail_name(path: &str) -> String {
    path.split("::")
        .filter(|segment| !segment.is_empty())
        .last()
        .unwrap_or(path)
        .to_string()
}
