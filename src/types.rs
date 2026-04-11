use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::ast::*;
use crate::diagnostics::{Diagnostic, Diagnostics, Span};

#[derive(Debug, Clone)]
pub struct TypedProgram {
    pub functions: Vec<TypedFunction>,
    pub function_signatures: BTreeMap<String, FunctionSignature>,
    pub call_depths: BTreeMap<String, usize>,
    pub book_commands: BTreeMap<String, BookCommand>,
}

#[derive(Debug, Clone)]
pub struct TypedFunction {
    pub name: String,
    pub params: Vec<TypedParam>,
    pub return_type: Type,
    pub body: Vec<TypedStmt>,
    pub locals: BTreeMap<String, Type>,
    pub called_functions: BTreeSet<String>,
    pub book_exposed: bool,
}

#[derive(Debug, Clone)]
pub struct TypedParam {
    pub name: String,
    pub ty: Type,
}

#[derive(Debug, Clone)]
pub struct FunctionSignature {
    pub params: Vec<Type>,
    pub return_type: Type,
}

#[derive(Debug, Clone)]
pub struct BookCommand {
    pub function_name: String,
    pub arg_count: usize,
}

#[derive(Debug, Clone)]
pub struct TypedStmt {
    pub kind: TypedStmtKind,
}

#[derive(Debug, Clone)]
pub enum TypedStmtKind {
    Let {
        name: String,
        ty: Type,
        value: TypedExpr,
    },
    Assign {
        name: String,
        value: TypedExpr,
    },
    If {
        condition: TypedExpr,
        then_body: Vec<TypedStmt>,
        else_body: Vec<TypedStmt>,
    },
    While {
        condition: TypedExpr,
        body: Vec<TypedStmt>,
    },
    For {
        name: String,
        start: TypedExpr,
        end: TypedExpr,
        inclusive: bool,
        body: Vec<TypedStmt>,
    },
    Break,
    Continue,
    Return(Option<TypedExpr>),
    RawCommand(String),
    MacroCommand {
        template: String,
        placeholders: Vec<MacroPlaceholder>,
    },
    Expr(TypedExpr),
}

#[derive(Debug, Clone)]
pub struct MacroPlaceholder {
    pub name: String,
    pub ty: Type,
}

#[derive(Debug, Clone)]
pub struct TypedExpr {
    pub kind: TypedExprKind,
    pub ty: Type,
}

#[derive(Debug, Clone)]
pub enum TypedExprKind {
    Int(i64),
    Bool(bool),
    String(String),
    Variable(String),
    Unary {
        op: UnaryOp,
        expr: Box<TypedExpr>,
    },
    Binary {
        op: BinaryOp,
        left: Box<TypedExpr>,
        right: Box<TypedExpr>,
    },
    Call {
        function: String,
        args: Vec<TypedExpr>,
    },
}

pub fn type_check(program: &Program) -> Result<TypedProgram, Diagnostics> {
    let mut diagnostics = Diagnostics::new();
    let mut signatures = BTreeMap::new();

    for function in &program.functions {
        if signatures.contains_key(&function.name) {
            diagnostics.push(Diagnostic::new(
                format!("duplicate function '{}'", function.name),
                function.span.clone(),
            ));
            continue;
        }
        signatures.insert(
            function.name.clone(),
            FunctionSignature {
                params: function
                    .params
                    .iter()
                    .map(|param| param.ty.clone())
                    .collect(),
                return_type: function.return_type.clone(),
            },
        );
    }

    let mut functions = Vec::new();
    let mut book_commands = BTreeMap::new();
    for function in &program.functions {
        let mut env = HashMap::new();
        let mut locals = BTreeMap::new();
        let mut seen_params = HashSet::new();
        let mut params = Vec::new();

        for param in &function.params {
            if !seen_params.insert(param.name.clone()) {
                diagnostics.push(Diagnostic::new(
                    format!("duplicate parameter '{}'", param.name),
                    param.span.clone(),
                ));
            }
            env.insert(param.name.clone(), param.ty.clone());
            locals.insert(param.name.clone(), param.ty.clone());
            params.push(TypedParam {
                name: param.name.clone(),
                ty: param.ty.clone(),
            });
        }

        let mut called_functions = BTreeSet::new();
        let body = type_check_block(
            &function.body,
            &function.return_type,
            &signatures,
            &mut env,
            &mut locals,
            &mut called_functions,
            0,
            &mut diagnostics,
        );

        functions.push(TypedFunction {
            name: function.name.clone(),
            params,
            return_type: function.return_type.clone(),
            body,
            locals,
            called_functions,
            book_exposed: function.book_exposed,
        });

        if function.book_exposed {
            if function.return_type != Type::Void {
                diagnostics.push(Diagnostic::new(
                    format!("@book function '{}' must return 'void'", function.name),
                    function.span.clone(),
                ));
            }
            if function.params.iter().any(|param| param.ty != Type::Int) {
                diagnostics.push(Diagnostic::new(
                    format!(
                        "@book function '{}' may only have 'int' parameters",
                        function.name
                    ),
                    function.span.clone(),
                ));
            }
            if book_commands.contains_key(&function.name) {
                diagnostics.push(Diagnostic::new(
                    format!("duplicate @book command '{}'", function.name),
                    function.span.clone(),
                ));
            } else {
                book_commands.insert(
                    function.name.clone(),
                    BookCommand {
                        function_name: function.name.clone(),
                        arg_count: function.params.len(),
                    },
                );
            }
        }
    }

    detect_recursion(&functions, &mut diagnostics);
    let call_depths = compute_call_depths(&functions, &mut diagnostics);

    diagnostics.into_result(TypedProgram {
        functions,
        function_signatures: signatures,
        call_depths,
        book_commands,
    })
}

fn type_check_block(
    statements: &[Stmt],
    return_type: &Type,
    signatures: &BTreeMap<String, FunctionSignature>,
    env: &mut HashMap<String, Type>,
    locals: &mut BTreeMap<String, Type>,
    called_functions: &mut BTreeSet<String>,
    loop_depth: usize,
    diagnostics: &mut Diagnostics,
) -> Vec<TypedStmt> {
    let mut typed = Vec::new();

    for statement in statements {
        let kind = match &statement.kind {
            StmtKind::Let { name, value } => {
                if env.contains_key(name) {
                    diagnostics.push(Diagnostic::new(
                        format!("variable '{}' is already defined", name),
                        statement.span.clone(),
                    ));
                }
                let value = type_check_expr(value, signatures, env, called_functions, diagnostics);
                env.insert(name.clone(), value.ty.clone());
                locals.insert(name.clone(), value.ty.clone());
                TypedStmtKind::Let {
                    name: name.clone(),
                    ty: value.ty.clone(),
                    value,
                }
            }
            StmtKind::Assign { name, value } => {
                let Some(existing) = env.get(name).cloned() else {
                    diagnostics.push(Diagnostic::new(
                        format!("undefined variable '{}'", name),
                        statement.span.clone(),
                    ));
                    continue;
                };
                let value = type_check_expr(value, signatures, env, called_functions, diagnostics);
                if existing != value.ty {
                    diagnostics.push(Diagnostic::new(
                        format!(
                            "cannot assign '{}' to variable '{}' of type '{}'",
                            value.ty.as_str(),
                            name,
                            existing.as_str()
                        ),
                        statement.span.clone(),
                    ));
                }
                TypedStmtKind::Assign {
                    name: name.clone(),
                    value,
                }
            }
            StmtKind::If {
                condition,
                then_body,
                else_body,
            } => {
                let condition =
                    type_check_expr(condition, signatures, env, called_functions, diagnostics);
                if condition.ty != Type::Bool {
                    diagnostics.push(Diagnostic::new(
                        "if condition must have type 'bool'",
                        statement.span.clone(),
                    ));
                }
                let then_body = type_check_block(
                    then_body,
                    return_type,
                    signatures,
                    &mut env.clone(),
                    locals,
                    called_functions,
                    loop_depth,
                    diagnostics,
                );
                let else_body = type_check_block(
                    else_body,
                    return_type,
                    signatures,
                    &mut env.clone(),
                    locals,
                    called_functions,
                    loop_depth,
                    diagnostics,
                );
                TypedStmtKind::If {
                    condition,
                    then_body,
                    else_body,
                }
            }
            StmtKind::While { condition, body } => {
                let condition =
                    type_check_expr(condition, signatures, env, called_functions, diagnostics);
                if condition.ty != Type::Bool {
                    diagnostics.push(Diagnostic::new(
                        "while condition must have type 'bool'",
                        statement.span.clone(),
                    ));
                }
                let body = type_check_block(
                    body,
                    return_type,
                    signatures,
                    &mut env.clone(),
                    locals,
                    called_functions,
                    loop_depth + 1,
                    diagnostics,
                );
                TypedStmtKind::While { condition, body }
            }
            StmtKind::For {
                name,
                start,
                end,
                inclusive,
                body,
            } => {
                if env.contains_key(name) {
                    diagnostics.push(Diagnostic::new(
                        format!("variable '{}' is already defined", name),
                        statement.span.clone(),
                    ));
                }
                let start = type_check_expr(start, signatures, env, called_functions, diagnostics);
                let end = type_check_expr(end, signatures, env, called_functions, diagnostics);
                if start.ty != Type::Int {
                    diagnostics.push(Diagnostic::new(
                        "for range start must have type 'int'",
                        statement.span.clone(),
                    ));
                }
                if end.ty != Type::Int {
                    diagnostics.push(Diagnostic::new(
                        "for range end must have type 'int'",
                        statement.span.clone(),
                    ));
                }

                let mut loop_env = env.clone();
                loop_env.insert(name.clone(), Type::Int);
                locals.insert(name.clone(), Type::Int);
                let body = type_check_block(
                    body,
                    return_type,
                    signatures,
                    &mut loop_env,
                    locals,
                    called_functions,
                    loop_depth + 1,
                    diagnostics,
                );
                TypedStmtKind::For {
                    name: name.clone(),
                    start,
                    end,
                    inclusive: *inclusive,
                    body,
                }
            }
            StmtKind::Break => {
                if loop_depth == 0 {
                    diagnostics.push(Diagnostic::new(
                        "'break' may only appear inside a loop",
                        statement.span.clone(),
                    ));
                }
                TypedStmtKind::Break
            }
            StmtKind::Continue => {
                if loop_depth == 0 {
                    diagnostics.push(Diagnostic::new(
                        "'continue' may only appear inside a loop",
                        statement.span.clone(),
                    ));
                }
                TypedStmtKind::Continue
            }
            StmtKind::Return(value) => {
                let value = value.as_ref().map(|expr| {
                    type_check_expr(expr, signatures, env, called_functions, diagnostics)
                });
                match (return_type, &value) {
                    (Type::Void, None) => {}
                    (Type::Void, Some(_)) => diagnostics.push(Diagnostic::new(
                        "void function cannot return a value",
                        statement.span.clone(),
                    )),
                    (expected, Some(expr)) if expected != &expr.ty => {
                        diagnostics.push(Diagnostic::new(
                            format!(
                                "return type mismatch: expected '{}', found '{}'",
                                expected.as_str(),
                                expr.ty.as_str()
                            ),
                            statement.span.clone(),
                        ))
                    }
                    (expected, None) if expected != &Type::Void => {
                        diagnostics.push(Diagnostic::new(
                            format!(
                                "return statement must produce a value of type '{}'",
                                expected.as_str()
                            ),
                            statement.span.clone(),
                        ))
                    }
                    _ => {}
                }
                TypedStmtKind::Return(value)
            }
            StmtKind::RawCommand(raw) => TypedStmtKind::RawCommand(raw.clone()),
            StmtKind::MacroCommand(template) => {
                let names =
                    validate_macro_placeholders(template, env, statement.span.clone(), diagnostics);
                let placeholders = names
                    .into_iter()
                    .filter_map(|name| {
                        env.get(&name)
                            .cloned()
                            .map(|ty| MacroPlaceholder { name, ty })
                    })
                    .collect();
                TypedStmtKind::MacroCommand {
                    template: template.clone(),
                    placeholders,
                }
            }
            StmtKind::Expr(expr) => {
                let expr = type_check_expr(expr, signatures, env, called_functions, diagnostics);
                if !matches!(expr.kind, TypedExprKind::Call { .. }) {
                    diagnostics.push(Diagnostic::new(
                        "only function calls may appear as bare expression statements",
                        statement.span.clone(),
                    ));
                }
                TypedStmtKind::Expr(expr)
            }
        };

        typed.push(TypedStmt { kind });
    }

    typed
}

fn type_check_expr(
    expr: &Expr,
    signatures: &BTreeMap<String, FunctionSignature>,
    env: &HashMap<String, Type>,
    called_functions: &mut BTreeSet<String>,
    diagnostics: &mut Diagnostics,
) -> TypedExpr {
    match &expr.kind {
        ExprKind::Int(value) => TypedExpr {
            kind: TypedExprKind::Int(*value),
            ty: Type::Int,
        },
        ExprKind::Bool(value) => TypedExpr {
            kind: TypedExprKind::Bool(*value),
            ty: Type::Bool,
        },
        ExprKind::String(value) => TypedExpr {
            kind: TypedExprKind::String(value.clone()),
            ty: Type::String,
        },
        ExprKind::Variable(name) => match env.get(name) {
            Some(ty) => TypedExpr {
                kind: TypedExprKind::Variable(name.clone()),
                ty: ty.clone(),
            },
            None => {
                diagnostics.push(Diagnostic::new(
                    format!("undefined variable '{}'", name),
                    expr.span.clone(),
                ));
                TypedExpr {
                    kind: TypedExprKind::Variable(name.clone()),
                    ty: Type::Int,
                }
            }
        },
        ExprKind::Unary { op, expr } => {
            let operand = type_check_expr(expr, signatures, env, called_functions, diagnostics);
            let ty = match op {
                UnaryOp::Not => {
                    if operand.ty != Type::Bool {
                        diagnostics.push(Diagnostic::new(
                            "'not' requires a 'bool' operand",
                            expr.span.clone(),
                        ));
                    }
                    Type::Bool
                }
            };
            TypedExpr {
                kind: TypedExprKind::Unary {
                    op: *op,
                    expr: Box::new(operand),
                },
                ty,
            }
        }
        ExprKind::Binary { op, left, right } => {
            let left = type_check_expr(left, signatures, env, called_functions, diagnostics);
            let right = type_check_expr(right, signatures, env, called_functions, diagnostics);
            let ty = match op {
                BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div => {
                    if left.ty != Type::Int || right.ty != Type::Int {
                        diagnostics.push(Diagnostic::new(
                            "arithmetic operators require 'int' operands",
                            expr.span.clone(),
                        ));
                    }
                    Type::Int
                }
                BinaryOp::And | BinaryOp::Or => {
                    if left.ty != Type::Bool || right.ty != Type::Bool {
                        diagnostics.push(Diagnostic::new(
                            "logical operators require 'bool' operands",
                            expr.span.clone(),
                        ));
                    }
                    Type::Bool
                }
                BinaryOp::Eq
                | BinaryOp::NotEq
                | BinaryOp::Lt
                | BinaryOp::Lte
                | BinaryOp::Gt
                | BinaryOp::Gte => {
                    if left.ty != right.ty {
                        diagnostics.push(Diagnostic::new(
                            "comparison operands must have matching types",
                            expr.span.clone(),
                        ));
                    }
                    match op {
                        BinaryOp::Eq | BinaryOp::NotEq => {
                            if !matches!(left.ty, Type::Int | Type::Bool | Type::String) {
                                diagnostics.push(Diagnostic::new(
                                    "equality operators currently support only 'int', 'bool', and 'string'",
                                    expr.span.clone(),
                                ));
                            }
                        }
                        _ => {
                            if !matches!(left.ty, Type::Int | Type::Bool) {
                                diagnostics.push(Diagnostic::new(
                                    "ordering comparisons currently support only 'int' and 'bool'",
                                    expr.span.clone(),
                                ));
                            }
                            if matches!(left.ty, Type::String) {
                                diagnostics.push(Diagnostic::new(
                                    "strings only support '==' and '!=' comparisons",
                                    expr.span.clone(),
                                ));
                            }
                        }
                    }
                    Type::Bool
                }
            };
            TypedExpr {
                kind: TypedExprKind::Binary {
                    op: *op,
                    left: Box::new(left),
                    right: Box::new(right),
                },
                ty,
            }
        }
        ExprKind::Call { function, args } => {
            let signature = match signatures.get(function) {
                Some(signature) => signature,
                None => {
                    diagnostics.push(Diagnostic::new(
                        format!("unknown function '{}'", function),
                        expr.span.clone(),
                    ));
                    return TypedExpr {
                        kind: TypedExprKind::Call {
                            function: function.clone(),
                            args: args
                                .iter()
                                .map(|arg| {
                                    type_check_expr(
                                        arg,
                                        signatures,
                                        env,
                                        called_functions,
                                        diagnostics,
                                    )
                                })
                                .collect(),
                        },
                        ty: Type::Void,
                    };
                }
            };

            if signature.params.len() != args.len() {
                diagnostics.push(Diagnostic::new(
                    format!(
                        "wrong arity for '{}': expected {}, found {}",
                        function,
                        signature.params.len(),
                        args.len()
                    ),
                    expr.span.clone(),
                ));
            }

            let args: Vec<_> = args
                .iter()
                .map(|arg| type_check_expr(arg, signatures, env, called_functions, diagnostics))
                .collect();
            for (index, arg) in args.iter().enumerate() {
                if let Some(expected) = signature.params.get(index) {
                    if expected != &arg.ty {
                        diagnostics.push(Diagnostic::new(
                            format!(
                                "argument {} for '{}' must be '{}', found '{}'",
                                index + 1,
                                function,
                                expected.as_str(),
                                arg.ty.as_str()
                            ),
                            expr.span.clone(),
                        ));
                    }
                }
            }

            called_functions.insert(function.clone());
            TypedExpr {
                kind: TypedExprKind::Call {
                    function: function.clone(),
                    args,
                },
                ty: signature.return_type.clone(),
            }
        }
    }
}

fn detect_recursion(functions: &[TypedFunction], diagnostics: &mut Diagnostics) {
    let graph: BTreeMap<_, _> = functions
        .iter()
        .map(|function| (function.name.clone(), function.called_functions.clone()))
        .collect();
    let mut visiting = HashSet::new();
    let mut visited = HashSet::new();
    let mut emitted = HashSet::new();

    for function in functions {
        dfs_cycle(
            &function.name,
            &graph,
            &mut visiting,
            &mut visited,
            &mut emitted,
            diagnostics,
        );
    }
}

fn dfs_cycle(
    node: &str,
    graph: &BTreeMap<String, BTreeSet<String>>,
    visiting: &mut HashSet<String>,
    visited: &mut HashSet<String>,
    emitted: &mut HashSet<String>,
    diagnostics: &mut Diagnostics,
) {
    if visited.contains(node) {
        return;
    }
    visiting.insert(node.to_string());

    if let Some(neighbors) = graph.get(node) {
        for neighbor in neighbors {
            if visiting.contains(neighbor) {
                let key = format!("{}->{}", node, neighbor);
                if emitted.insert(key) {
                    diagnostics.push(Diagnostic::new(
                        format!("recursion is not supported: cycle includes '{}'", neighbor),
                        Span::new(1, 1),
                    ));
                }
                continue;
            }
            dfs_cycle(neighbor, graph, visiting, visited, emitted, diagnostics);
        }
    }

    visiting.remove(node);
    visited.insert(node.to_string());
}

fn compute_call_depths(
    functions: &[TypedFunction],
    diagnostics: &mut Diagnostics,
) -> BTreeMap<String, usize> {
    let graph: BTreeMap<_, _> = functions
        .iter()
        .map(|function| (function.name.clone(), function.called_functions.clone()))
        .collect();
    let mut memo = BTreeMap::new();
    for function in functions {
        let depth = longest_path(
            &function.name,
            &graph,
            &mut memo,
            &mut HashSet::new(),
            diagnostics,
        );
        memo.insert(function.name.clone(), depth);
    }
    memo
}

fn longest_path(
    node: &str,
    graph: &BTreeMap<String, BTreeSet<String>>,
    memo: &mut BTreeMap<String, usize>,
    visiting: &mut HashSet<String>,
    diagnostics: &mut Diagnostics,
) -> usize {
    if let Some(depth) = memo.get(node) {
        return *depth;
    }
    if !visiting.insert(node.to_string()) {
        diagnostics.push(Diagnostic::new(
            format!("recursion is not supported: '{}'", node),
            Span::new(1, 1),
        ));
        return 0;
    }

    let mut best = 0;
    if let Some(callees) = graph.get(node) {
        for callee in callees {
            best = best.max(1 + longest_path(callee, graph, memo, visiting, diagnostics));
        }
    }
    visiting.remove(node);
    memo.insert(node.to_string(), best);
    best
}

fn validate_macro_placeholders(
    template: &str,
    env: &HashMap<String, Type>,
    span: Span,
    diagnostics: &mut Diagnostics,
) -> Vec<String> {
    let mut names = Vec::new();
    let mut seen = HashSet::new();
    let bytes = template.as_bytes();
    let mut index = 0usize;

    while index < bytes.len() {
        if bytes[index] == b'$' && index + 1 < bytes.len() && bytes[index + 1] == b'(' {
            index += 2;
            let start = index;
            let mut invalid = false;
            while index < bytes.len() && bytes[index] != b')' {
                let ch = bytes[index] as char;
                if !ch.is_ascii_alphanumeric() && ch != '_' {
                    diagnostics.push(Diagnostic::new(
                        format!("invalid macro placeholder character '{}'", ch),
                        span.clone(),
                    ));
                    while index < bytes.len() && bytes[index] != b')' {
                        index += 1;
                    }
                    invalid = true;
                    break;
                }
                index += 1;
            }

            if index >= bytes.len() {
                diagnostics.push(Diagnostic::new(
                    "unterminated macro placeholder",
                    span.clone(),
                ));
                break;
            }

            if start == index {
                diagnostics.push(Diagnostic::new(
                    "macro placeholder name cannot be empty",
                    span.clone(),
                ));
                index += 1;
                continue;
            }

            if invalid {
                index += 1;
                continue;
            }

            let name = &template[start..index];
            if let Some(ty) = env.get(name) {
                if !matches!(ty, Type::Int | Type::Bool | Type::String) {
                    diagnostics.push(Diagnostic::new(
                        format!(
                            "macro placeholder '{}' has unsupported type '{}'",
                            name,
                            ty.as_str()
                        ),
                        span.clone(),
                    ));
                } else if seen.insert(name.to_string()) {
                    names.push(name.to_string());
                }
            } else {
                diagnostics.push(Diagnostic::new(
                    format!("unknown macro placeholder '{}'", name),
                    span.clone(),
                ));
            }
        }
        index += 1;
    }

    names
}
