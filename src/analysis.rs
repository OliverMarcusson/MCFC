use crate::ast::{Function, Program, Stmt, StmtKind, Type};
use crate::diagnostics::{Diagnostic, Diagnostics, TextRange};
use crate::lexer::{Token, TokenKind, lex};
use crate::parser;
use crate::types::{self, TypedProgram};

#[derive(Debug, Clone)]
pub struct AnalysisResult {
    pub diagnostics: Vec<Diagnostic>,
    pub program: Option<Program>,
    pub typed_program: Option<TypedProgram>,
    pub functions: Vec<FunctionInfo>,
    pub locals: Vec<LocalInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionInfo {
    pub name: String,
    pub params: Vec<(String, Type)>,
    pub return_type: Type,
    pub book_exposed: bool,
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
}

pub fn analyze_source(source: &str) -> AnalysisResult {
    match parser::parse(source) {
        Ok(program) => {
            let functions = collect_functions(source, &program);
            match types::type_check(&program) {
                Ok(typed_program) => {
                    let locals = collect_locals(&typed_program);
                    AnalysisResult {
                        diagnostics: Vec::new(),
                        program: Some(program),
                        typed_program: Some(typed_program),
                        functions,
                        locals,
                    }
                }
                Err(diagnostics) => AnalysisResult {
                    diagnostics: diagnostics.0,
                    program: Some(program),
                    typed_program: None,
                    functions,
                    locals: Vec::new(),
                },
            }
        }
        Err(diagnostics) => AnalysisResult {
            diagnostics: diagnostics.0,
            program: None,
            typed_program: None,
            functions: Vec::new(),
            locals: Vec::new(),
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
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn collect_functions(source: &str, program: &Program) -> Vec<FunctionInfo> {
    let Ok(tokens) = lex(source) else {
        return fallback_functions(program);
    };

    let mut cursor = 0usize;
    program
        .functions
        .iter()
        .map(|function| {
            let info = collect_function_info(function, &tokens, &mut cursor);
            cursor = info.next_cursor;
            info.function
        })
        .collect()
}

fn fallback_functions(program: &Program) -> Vec<FunctionInfo> {
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
            book_exposed: function.book_exposed,
            range: function.span.range,
            name_range: function.span.range,
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

    let start = if fn_index > 0 && matches!(tokens[fn_index - 1].kind, TokenKind::BookAnnotation) {
        tokens[fn_index - 1].range.start
    } else {
        tokens
            .get(fn_index)
            .map(|token| token.range.start)
            .unwrap_or(function.span.range.start)
    };
    let name_range = tokens
        .get(fn_index + 1)
        .map(|token| token.range)
        .unwrap_or(function.span.range);

    let mut end = tokens
        .get(fn_index)
        .map(|token| token.range.end)
        .unwrap_or(function.span.range.end);
    let mut index = fn_index + 1;
    let mut block_depth = 0usize;
    while index < tokens.len() {
        match tokens[index].kind {
            TokenKind::If if !previous_significant_is_else(tokens, index) => block_depth += 1,
            TokenKind::While | TokenKind::For => block_depth += 1,
            TokenKind::End if block_depth == 0 => {
                end = tokens[index].range.end;
                index += 1;
                break;
            }
            TokenKind::End => block_depth = block_depth.saturating_sub(1),
            TokenKind::Eof => break,
            _ => {}
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
            book_exposed: function.book_exposed,
            range: TextRange::new(start, end),
            name_range,
        },
        next_cursor: index,
    }
}

fn previous_significant_is_else(tokens: &[Token], index: usize) -> bool {
    tokens[..index]
        .iter()
        .rev()
        .find(|token| !matches!(token.kind, TokenKind::Newline))
        .is_some_and(|token| matches!(token.kind, TokenKind::Else))
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
fn main() -> void
    missing()
end
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
@book
fn launch(level: int) -> void
    let amount = level
    return
end
"#;

        let analysis = analyze_source(source);

        assert!(analysis.diagnostics.is_empty());
        assert_eq!(analysis.functions[0].name, "launch");
        assert!(analysis.functions[0].book_exposed);
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
