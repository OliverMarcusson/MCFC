use crate::ast::{Function, Program, Stmt, StmtKind, Type};
use crate::diagnostics::{Diagnostic, Diagnostics, TextRange};
use crate::lexer::{Token, TokenKind, lex};
use crate::parser;
use crate::types::{self, RefKind, TypedProgram};

#[derive(Debug, Clone)]
pub struct AnalysisResult {
    pub diagnostics: Vec<Diagnostic>,
    pub program: Option<Program>,
    pub typed_program: Option<TypedProgram>,
    pub functions: Vec<FunctionInfo>,
    pub locals: Vec<LocalInfo>,
    /// Maps parser/type-checker spans back to the text the editor displays.
    pub source_map: SourceMap,
}

/// A byte-offset map between public MCFC source and the internal, lowered
/// syntax consumed by the parser.  All LSP-facing spans must use original
/// offsets; only the parser and type checker see normalized offsets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceMap {
    original_to_normalized: Vec<usize>,
    normalized_to_original: Vec<usize>,
}

impl SourceMap {
    pub fn identity(source: &str) -> Self {
        let offsets: Vec<usize> = (0..=source.len()).collect();
        Self {
            original_to_normalized: offsets.clone(),
            normalized_to_original: offsets,
        }
    }

    pub fn from_sources(original: &str, normalized: &str) -> Self {
        if original == normalized {
            return Self::identity(original);
        }
        Self {
            original_to_normalized: build_offset_map(original, normalized),
            normalized_to_original: build_offset_map(normalized, original),
        }
    }

    pub fn to_original_offset(&self, offset: usize) -> usize {
        self.normalized_to_original[offset.min(self.normalized_to_original.len().saturating_sub(1))]
    }

    pub fn to_original_range(&self, range: TextRange) -> TextRange {
        TextRange::new(
            self.to_original_offset(range.start),
            self.to_original_offset(range.end),
        )
    }
}

/// Align byte offsets using common prefix/suffix anchors. Lowering only
/// changes individual declaration lines, so this keeps editor positions exact
/// for bodies and identifiers without exposing generated function names.
fn build_offset_map(from: &str, to: &str) -> Vec<usize> {
    let mut map = vec![0; from.len() + 1];
    let mut from_base = 0usize;
    let mut to_base = 0usize;
    for (from_line, to_line) in from.split_inclusive('\n').zip(to.split_inclusive('\n')) {
        map_line_offsets(from_line, to_line, from_base, to_base, &mut map);
        from_base += from_line.len();
        to_base += to_line.len();
    }
    // Both normalizers preserve line count.  This fallback also makes a
    // malformed/incomplete final line safe.
    for index in from_base..=from.len() {
        map[index] = to_base + (index - from_base).min(to.len().saturating_sub(to_base));
    }
    map
}

fn map_line_offsets(from: &str, to: &str, from_base: usize, to_base: usize, map: &mut [usize]) {
    let prefix = from
        .bytes()
        .zip(to.bytes())
        .take_while(|(a, b)| a == b)
        .count();
    let mut suffix = 0usize;
    while suffix < from.len().saturating_sub(prefix)
        && suffix < to.len().saturating_sub(prefix)
        && from.as_bytes()[from.len() - 1 - suffix] == to.as_bytes()[to.len() - 1 - suffix]
    {
        suffix += 1;
    }
    let from_changed_end = from.len() - suffix;
    let to_changed_end = to.len() - suffix;
    for index in 0..=prefix {
        map[from_base + index] = to_base + index;
    }
    for index in prefix..=from_changed_end {
        let relative = index - prefix;
        map[from_base + index] = to_base + prefix + relative.min(to_changed_end - prefix);
    }
    for index in from_changed_end..=from.len() {
        map[from_base + index] = to_base + to_changed_end + (index - from_changed_end);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionInfo {
    pub name: String,
    pub params: Vec<(String, Type)>,
    pub return_type: Type,
    pub range: TextRange,
    pub name_range: TextRange,
}

impl FunctionInfo {
    pub fn signature(&self) -> String {
        let params = self
            .params
            .iter()
            .map(|(name, ty)| format!("{}: {}", name, ty.as_str()))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "fn {}({}) -> {}",
            self.name,
            params,
            self.return_type.as_str()
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalInfo {
    pub function: String,
    pub name: String,
    pub ty: Type,
    pub ref_kind: RefKind,
}

pub fn analyze_source(source: &str) -> AnalysisResult {
    analyze_source_with_host_modules(source, &types::HostModules::for_editor())
}

/// Analyze source with the helper capability set resolved from its project
/// manifest. The compiler and LSP share this entry point so a valid `kv.get`
/// or agent payload is never rejected only inside the editor.
pub fn analyze_source_with_host_modules(
    source: &str,
    host_modules: &types::HostModules,
) -> AnalysisResult {
    // The editor consumes the public MCFC syntax, including Bukkit-style
    // declarations which are lowered before the compact parser sees them.
    let normalized = crate::compiler::normalize_bukkit_declarations_source(source);
    let source_map = SourceMap::from_sources(source, &normalized);
    match parser::parse(&normalized) {
        Ok(program) => {
            let functions = collect_functions(&normalized, &program, &source_map);
            match types::type_check(&program, host_modules) {
                Ok(typed_program) => {
                    let locals = collect_locals(&typed_program);
                    AnalysisResult {
                        diagnostics: Vec::new(),
                        program: Some(program),
                        typed_program: Some(typed_program),
                        functions,
                        locals,
                        source_map,
                    }
                }
                Err(diagnostics) => AnalysisResult {
                    diagnostics: diagnostics.0,
                    program: Some(program),
                    typed_program: None,
                    functions,
                    locals: Vec::new(),
                    source_map,
                },
            }
        }
        Err(diagnostics) => AnalysisResult {
            diagnostics: diagnostics.0,
            program: None,
            typed_program: None,
            functions: Vec::new(),
            locals: Vec::new(),
            source_map,
        },
    }
}

fn collect_locals(typed_program: &TypedProgram) -> Vec<LocalInfo> {
    typed_program
        .functions
        .iter()
        .flat_map(|function| {
            function
                .locals
                .iter()
                .map(|(name, ty)| LocalInfo {
                    function: function.name.clone(),
                    name: name.clone(),
                    ty: ty.clone(),
                    ref_kind: function
                        .local_ref_kinds
                        .get(name)
                        .copied()
                        .unwrap_or(RefKind::Unknown),
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn collect_functions(source: &str, program: &Program, source_map: &SourceMap) -> Vec<FunctionInfo> {
    let Ok(tokens) = lex(source) else {
        return fallback_functions(program, source_map);
    };

    let mut cursor = 0usize;
    program
        .functions
        .iter()
        .map(|function| {
            let info = collect_function_info(function, &tokens, &mut cursor, source_map);
            cursor = info.next_cursor;
            info.function
        })
        .collect()
}

fn fallback_functions(program: &Program, source_map: &SourceMap) -> Vec<FunctionInfo> {
    program
        .functions
        .iter()
        .map(|function| FunctionInfo {
            name: function.name.clone(),
            params: function
                .params
                .iter()
                .map(|param| (param.name.clone(), param.ty.clone()))
                .collect(),
            return_type: function.return_type.clone(),
            range: source_map.to_original_range(function.span.range),
            name_range: source_map.to_original_range(function.span.range),
        })
        .collect()
}

struct CollectedFunction {
    function: FunctionInfo,
    next_cursor: usize,
}

fn collect_function_info(
    function: &Function,
    tokens: &[Token],
    cursor: &mut usize,
    source_map: &SourceMap,
) -> CollectedFunction {
    let mut fn_index = *cursor;
    while fn_index < tokens.len() {
        if matches!(tokens[fn_index].kind, TokenKind::Fn)
            && matches!(
                tokens.get(fn_index + 1).map(|token| &token.kind),
                Some(TokenKind::Identifier(name)) if name == &function.name
            )
        {
            break;
        }
        fn_index += 1;
    }

    let start = tokens
        .get(fn_index)
        .map(|token| token.range.start)
        .unwrap_or(function.span.range.start);
    let name_range = tokens
        .get(fn_index + 1)
        .map(|token| token.range)
        .unwrap_or(function.span.range);

    let mut end = tokens
        .get(fn_index)
        .map(|token| token.range.end)
        .unwrap_or(function.span.range.end);
    let mut index = fn_index + 1;
    while index < tokens.len() && !matches!(tokens[index].kind, TokenKind::Indent | TokenKind::Eof)
    {
        index += 1;
    }
    let mut block_depth = 0usize;
    let mut last_body_end = end;
    while index < tokens.len() {
        match tokens[index].kind {
            TokenKind::Indent => block_depth += 1,
            TokenKind::Dedent => {
                block_depth = block_depth.saturating_sub(1);
                if block_depth == 0 {
                    end = last_body_end;
                    index += 1;
                    break;
                }
            }
            TokenKind::Eof => break,
            _ => last_body_end = tokens[index].range.end,
        }
        index += 1;
    }

    CollectedFunction {
        function: FunctionInfo {
            name: function.name.clone(),
            params: function
                .params
                .iter()
                .map(|param| (param.name.clone(), param.ty.clone()))
                .collect(),
            return_type: function.return_type.clone(),
            range: source_map.to_original_range(TextRange::new(start, end)),
            name_range: source_map.to_original_range(name_range),
        },
        next_cursor: index,
    }
}

pub fn function_at_offset<'a>(
    analysis: &'a AnalysisResult,
    offset: usize,
) -> Option<&'a FunctionInfo> {
    analysis
        .functions
        .iter()
        .find(|function| function.range.start <= offset && offset <= function.range.end)
}

pub fn word_at_offset(source: &str, offset: usize) -> Option<(String, TextRange)> {
    if source.is_empty() {
        return None;
    }
    let offset = offset.min(source.len());
    let mut start = offset;
    while start > 0 {
        let ch = source[..start].chars().next_back()?;
        if !is_word_char(ch) {
            break;
        }
        start -= ch.len_utf8();
    }
    let mut end = offset;
    while end < source.len() {
        let ch = source[end..].chars().next()?;
        if !is_word_char(ch) {
            break;
        }
        end += ch.len_utf8();
    }
    if start == end {
        return None;
    }
    Some((source[start..end].to_string(), TextRange::new(start, end)))
}

fn is_word_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

pub fn collect_statement_let_names(statements: &[Stmt], names: &mut Vec<String>) {
    for statement in statements {
        match &statement.kind {
            StmtKind::Let { name, .. } => names.push(name.clone()),
            StmtKind::If {
                then_body,
                else_body,
                ..
            } => {
                collect_statement_let_names(then_body, names);
                collect_statement_let_names(else_body, names);
            }
            StmtKind::While { body, .. } | StmtKind::For { body, .. } => {
                collect_statement_let_names(body, names);
            }
            StmtKind::Match {
                arms, else_body, ..
            } => {
                for arm in arms {
                    collect_statement_let_names(&arm.body, names);
                }
                collect_statement_let_names(else_body, names);
            }
            _ => {}
        }
    }
}

impl From<Diagnostics> for AnalysisResult {
    fn from(diagnostics: Diagnostics) -> Self {
        Self {
            diagnostics: diagnostics.0,
            program: None,
            typed_program: None,
            functions: Vec::new(),
            locals: Vec::new(),
            source_map: SourceMap::identity(""),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{analyze_source, collect_statement_let_names, function_at_offset, word_at_offset};
    use crate::ast::{Expr, ExprKind, MatchArm, Stmt, StmtKind};
    use crate::diagnostics::Span;

    #[test]
    fn reports_parser_diagnostics() {
        let analysis = analyze_source("fn main() -> void\n    let x =\nend\n");

        assert!(
            analysis
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("expected expression"))
        );
        assert!(analysis.typed_program.is_none());
    }

    #[test]
    fn reports_type_diagnostics_and_keeps_symbols() {
        let analysis = analyze_source(
            r#"
fn main() -> void:
    missing()
"#,
        );

        assert!(
            analysis
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("unknown function"))
        );
        assert_eq!(analysis.functions.len(), 1);
    }

    #[test]
    fn collects_functions_and_locals_for_valid_source() {
        let source = r#"
fn launch(level: int) -> void:
    let amount = level
    return
"#;

        let analysis = analyze_source(source);

        assert!(analysis.diagnostics.is_empty());
        assert_eq!(analysis.functions[0].name, "launch");
        assert!(
            analysis
                .locals
                .iter()
                .any(|local| local.name == "amount" && local.ty.as_str() == "int")
        );
        let offset = source.find("amount").unwrap();
        assert_eq!(
            function_at_offset(&analysis, offset).unwrap().name,
            "launch"
        );
    }

    #[test]
    fn analyzes_bukkit_style_declarations_with_the_compiler_frontend() {
        let analysis = analyze_source(
            r#"data player.coins: int = 0
event chat(event: chat_event):
    event.player.send_message(event.message)
command status:
    let player = single(selector("@s"))
    player.send_message("ok")
"#,
        );

        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );
    }

    #[test]
    fn finds_word_at_utf8_offset() {
        let source = "mc \"å\"\nlet value = 1\n";
        let offset = source.find("value").unwrap() + 2;
        let (word, range) = word_at_offset(source, offset).unwrap();

        assert_eq!(word, "value");
        assert_eq!(&source[range.start..range.end], "value");
    }
    #[test]
    fn collects_match_arm_let_names() {
        let span = Span::new(1, 1);
        let statements = vec![Stmt {
            span: span.clone(),
            kind: StmtKind::Match {
                value: Expr {
                    kind: ExprKind::String("idle".to_string()),
                    span: span.clone(),
                },
                arms: vec![MatchArm {
                    pattern: "idle".to_string(),
                    body: vec![Stmt {
                        span: span.clone(),
                        kind: StmtKind::Let {
                            name: "inner".to_string(),
                            value: Expr {
                                kind: ExprKind::Int(1),
                                span: span.clone(),
                            },
                        },
                    }],
                }],
                else_body: vec![Stmt {
                    span: span.clone(),
                    kind: StmtKind::Let {
                        name: "fallback".to_string(),
                        value: Expr {
                            kind: ExprKind::Int(2),
                            span: span.clone(),
                        },
                    },
                }],
            },
        }];
        let mut names = Vec::new();

        collect_statement_let_names(&statements, &mut names);

        assert!(names.contains(&"inner".to_string()));
        assert!(names.contains(&"fallback".to_string()));
    }
}
