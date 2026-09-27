use crate::ast::{Program, Stmt, StmtKind, Type};
use crate::diagnostics::{Diagnostic, Diagnostics, TextRange};
use crate::parser;
use crate::types::{self, RefKind, TypedProgram};

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
    pub type_params: Vec<String>,
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
            .map(|(name, ty)| format!("{} {}", ty.as_str(), name))
            .collect::<Vec<_>>()
            .join(", ");
        let generics = if self.type_params.is_empty() {
            String::new()
        } else {
            format!("<{}> ", self.type_params.join(", "))
        };
        format!(
            "{}{} {}({})",
            generics,
            self.return_type.as_str(),
            self.name,
            params
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
    analyze_modules(source, host_modules, &[])
}

/// Analyze a merged multi-file source whose layout is described by `modules`.
pub fn analyze_modules(
    source: &str,
    host_modules: &types::HostModules,
    modules: &[crate::modules::ModuleSource],
) -> AnalysisResult {
    match parser::parse(source) {
        Ok(program) => {
            let mut functions = collect_functions(&program);
            let program = match crate::modules::resolve(program.clone(), modules) {
                Ok(resolved) => resolved,
                Err(diagnostics) => {
                    return AnalysisResult {
                        diagnostics: diagnostics.0,
                        program: Some(program),
                        typed_program: None,
                        functions,
                        locals: Vec::new(),
                    };
                }
            };
            // Functions were matched to tokens by source name; switch to the
            // resolved `module::name` so they line up with typed locals.
            for (info, function) in functions.iter_mut().zip(&program.functions) {
                info.name = function.name.clone();
            }
            // Every module's `tick()` merges into one, as in a build.
            let checked = match crate::compiler::normalize_special_functions(program.clone()) {
                Ok(checked) => checked,
                Err(diagnostics) => {
                    return AnalysisResult {
                        diagnostics: diagnostics.0,
                        program: Some(program),
                        typed_program: None,
                        functions,
                        locals: Vec::new(),
                    };
                }
            };
            match types::type_check(&checked, host_modules) {
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

fn collect_functions(program: &Program) -> Vec<FunctionInfo> {
    program
        .functions
        .iter()
        .map(|function| FunctionInfo {
            name: function.name.clone(),
            type_params: function.type_params.clone(),
            params: function
                .params
                .iter()
                .map(|param| (param.name.clone(), param.ty.clone()))
                .collect(),
            return_type: function.return_type.clone(),
            range: TextRange::new(function.span.range.start, function.end),
            name_range: function.span.range,
        })
        .collect()
}

pub fn function_at_offset(analysis: &AnalysisResult, offset: usize) -> Option<&FunctionInfo> {
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
            StmtKind::While { body, step, .. } => {
                collect_statement_let_names(body, names);
                collect_statement_let_names(step, names);
            }
            StmtKind::For { name, body, .. } => {
                names.push(name.clone());
                collect_statement_let_names(body, names);
            }
            StmtKind::Block(body) | StmtKind::Async { body } | StmtKind::Context { body, .. } => {
                collect_statement_let_names(body, names);
            }
            StmtKind::Switch {
                arms, default_body, ..
            } => {
                for arm in arms {
                    collect_statement_let_names(&arm.body, names);
                }
                collect_statement_let_names(default_body, names);
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

    #[test]
    fn reports_parser_diagnostics() {
        let analysis = analyze_source(
            "void main() {
    var x =
}
",
        );

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
void main() {
    missing();
}
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
void launch(int level) {
    var amount = level;
    return;
}
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
            r#"@PlayerState("Coins") int coins;
@EventHandler
void onChat(ChatEvent event) {
    event.player().sendMessage(event.message());
}
@Command("status")
void status() {
    var player = Selector.of("@s").getFirst();
    player.sendMessage("ok");
}
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
        let source = "mc(\"å\");
var value = 1;
";
        let offset = source.find("value").unwrap() + 2;
        let (word, range) = word_at_offset(source, offset).unwrap();

        assert_eq!(word, "value");
        assert_eq!(&source[range.start..range.end], "value");
    }
    #[test]
    fn collects_switch_arm_let_names() {
        let program = crate::parser::parse(
            r#"
void main() {
    switch ("idle") {
        case "idle" -> {
            var inner = 1;
        }
        default -> {
            var fallback = 2;
        }
    }
}
"#,
        )
        .unwrap();
        let mut names = Vec::new();

        collect_statement_let_names(&program.functions[0].body, &mut names);

        assert!(names.contains(&"inner".to_string()));
        assert!(names.contains(&"fallback".to_string()));
    }
}
