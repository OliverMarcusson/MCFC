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
        target: TypedAssignTarget,
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
        kind: TypedForKind,
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
pub enum TypedAssignTarget {
    Variable(String),
    Path(TypedPathExpr),
}

#[derive(Debug, Clone)]
pub enum TypedForKind {
    Range {
        start: TypedExpr,
        end: TypedExpr,
        inclusive: bool,
    },
    Each {
        iterable: TypedExpr,
    },
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
    pub ref_kind: RefKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefKind {
    Unknown,
    Player,
    NonPlayer,
}

#[derive(Debug, Clone)]
pub struct TypedPathExpr {
    pub base: Box<TypedExpr>,
    pub segments: Vec<PathSegment>,
}

#[derive(Debug, Clone)]
pub enum TypedExprKind {
    Int(i64),
    Bool(bool),
    String(String),
    Variable(String),
    Selector(String),
    Block(String),
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
    MethodCall {
        receiver: Box<TypedExpr>,
        method: String,
        args: Vec<TypedExpr>,
    },
    Single(Box<TypedExpr>),
    Exists(Box<TypedExpr>),
    At {
        anchor: Box<TypedExpr>,
        value: Box<TypedExpr>,
    },
    Path(TypedPathExpr),
    Cast {
        kind: CastKind,
        expr: Box<TypedExpr>,
    },
}

#[derive(Debug, Clone, Copy)]
pub enum CastKind {
    Int,
    Bool,
    String,
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
        let mut ref_env = HashMap::new();
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
            ref_env.insert(param.name.clone(), RefKind::Unknown);
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
            &mut ref_env,
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
    ref_env: &mut HashMap<String, RefKind>,
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
                let value =
                    type_check_expr(value, signatures, env, ref_env, called_functions, diagnostics);
                env.insert(name.clone(), value.ty.clone());
                ref_env.insert(name.clone(), value.ref_kind);
                locals.insert(name.clone(), value.ty.clone());
                TypedStmtKind::Let {
                    name: name.clone(),
                    ty: value.ty.clone(),
                    value,
                }
            }
            StmtKind::Assign { target, value } => {
                let value =
                    type_check_expr(value, signatures, env, ref_env, called_functions, diagnostics);
                let target = match target {
                    AssignTarget::Variable(name) => {
                        let Some(existing) = env.get(name).cloned() else {
                            diagnostics.push(Diagnostic::new(
                                format!("undefined variable '{}'", name),
                                statement.span.clone(),
                            ));
                            continue;
                        };
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
                        TypedAssignTarget::Variable(name.clone())
                    }
                    AssignTarget::Path(path) => {
                        let typed_path = type_check_path(
                            path,
                            signatures,
                            env,
                            ref_env,
                            called_functions,
                            diagnostics,
                            statement.span.clone(),
                        );
                        if !matches!(typed_path.base.ty, Type::EntityRef | Type::BlockRef) {
                            diagnostics.push(Diagnostic::new(
                                "path assignment requires an 'entity_ref' or 'block_ref' base",
                                statement.span.clone(),
                            ));
                        }
                        if !matches!(value.ty, Type::Int | Type::Bool | Type::String | Type::Nbt) {
                            diagnostics.push(Diagnostic::new(
                                "path assignment requires a value of type 'int', 'bool', 'string', or 'nbt'",
                                statement.span.clone(),
                            ));
                        }
                        validate_player_path_write(&typed_path, &value, statement.span.clone(), diagnostics);
                        TypedAssignTarget::Path(typed_path)
                    }
                };
                TypedStmtKind::Assign { target, value }
            }
            StmtKind::If {
                condition,
                then_body,
                else_body,
            } => {
                let condition =
                    type_check_expr(condition, signatures, env, ref_env, called_functions, diagnostics);
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
                    &mut ref_env.clone(),
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
                    &mut ref_env.clone(),
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
                    type_check_expr(condition, signatures, env, ref_env, called_functions, diagnostics);
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
                    &mut ref_env.clone(),
                    locals,
                    called_functions,
                    loop_depth + 1,
                    diagnostics,
                );
                TypedStmtKind::While { condition, body }
            }
            StmtKind::For { name, kind, body } => {
                if env.contains_key(name) {
                    diagnostics.push(Diagnostic::new(
                        format!("variable '{}' is already defined", name),
                        statement.span.clone(),
                    ));
                }
                let mut loop_env = env.clone();
                let mut loop_ref_env = ref_env.clone();
                let kind = match kind {
                    ForKind::Range {
                        start,
                        end,
                        inclusive,
                    } => {
                        let start =
                            type_check_expr(start, signatures, env, ref_env, called_functions, diagnostics);
                        let end =
                            type_check_expr(end, signatures, env, ref_env, called_functions, diagnostics);
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
                        loop_env.insert(name.clone(), Type::Int);
                        loop_ref_env.insert(name.clone(), RefKind::Unknown);
                        locals.insert(name.clone(), Type::Int);
                        TypedForKind::Range {
                            start,
                            end,
                            inclusive: *inclusive,
                        }
                    }
                    ForKind::Each { iterable } => {
                        let iterable = type_check_expr(
                            iterable,
                            signatures,
                            env,
                            ref_env,
                            called_functions,
                            diagnostics,
                        );
                        if iterable.ty != Type::EntitySet {
                            diagnostics.push(Diagnostic::new(
                                "for-each iteration requires an 'entity_set'",
                                statement.span.clone(),
                            ));
                        }
                        loop_env.insert(name.clone(), Type::EntityRef);
                        loop_ref_env.insert(name.clone(), iterable.ref_kind);
                        locals.insert(name.clone(), Type::EntityRef);
                        TypedForKind::Each { iterable }
                    }
                };
                let body = type_check_block(
                    body,
                    return_type,
                    signatures,
                    &mut loop_env,
                    &mut loop_ref_env,
                    locals,
                    called_functions,
                    loop_depth + 1,
                    diagnostics,
                );
                TypedStmtKind::For { name: name.clone(), kind, body }
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
                    type_check_expr(expr, signatures, env, ref_env, called_functions, diagnostics)
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
                let expr = type_check_expr(expr, signatures, env, ref_env, called_functions, diagnostics);
                if !matches!(expr.kind, TypedExprKind::Call { .. } | TypedExprKind::MethodCall { .. }) {
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
    ref_env: &HashMap<String, RefKind>,
    called_functions: &mut BTreeSet<String>,
    diagnostics: &mut Diagnostics,
) -> TypedExpr {
    match &expr.kind {
        ExprKind::Int(value) => TypedExpr {
            kind: TypedExprKind::Int(*value),
            ty: Type::Int,
            ref_kind: RefKind::Unknown,
        },
        ExprKind::Bool(value) => TypedExpr {
            kind: TypedExprKind::Bool(*value),
            ty: Type::Bool,
            ref_kind: RefKind::Unknown,
        },
        ExprKind::String(value) => TypedExpr {
            kind: TypedExprKind::String(value.clone()),
            ty: Type::String,
            ref_kind: RefKind::Unknown,
        },
        ExprKind::Path(path) => TypedExpr {
            kind: TypedExprKind::Path(type_check_path(
                path,
                signatures,
                env,
                ref_env,
                called_functions,
                diagnostics,
                expr.span.clone(),
            )),
            ty: Type::Nbt,
            ref_kind: RefKind::Unknown,
        },
        ExprKind::Variable(name) => match env.get(name) {
            Some(ty) => TypedExpr {
                kind: TypedExprKind::Variable(name.clone()),
                ty: ty.clone(),
                ref_kind: ref_env.get(name).copied().unwrap_or(RefKind::Unknown),
            },
            None => {
                diagnostics.push(Diagnostic::new(
                    format!("undefined variable '{}'", name),
                    expr.span.clone(),
                ));
                TypedExpr {
                    kind: TypedExprKind::Variable(name.clone()),
                    ty: Type::Int,
                    ref_kind: RefKind::Unknown,
                }
            }
        },
        ExprKind::Unary { op, expr } => {
            let operand =
                type_check_expr(expr, signatures, env, ref_env, called_functions, diagnostics);
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
                ref_kind: RefKind::Unknown,
            }
        }
        ExprKind::Binary { op, left, right } => {
            let left = type_check_expr(left, signatures, env, ref_env, called_functions, diagnostics);
            let right = type_check_expr(right, signatures, env, ref_env, called_functions, diagnostics);
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
                ref_kind: RefKind::Unknown,
            }
        }
        ExprKind::MethodCall { receiver, method, args } => {
            if let Some(builtin) = type_check_method_call(
                receiver,
                method,
                args,
                expr,
                signatures,
                env,
                ref_env,
                called_functions,
                diagnostics,
            ) {
                return builtin;
            }
            diagnostics.push(Diagnostic::new(
                format!("unknown method '{}'", method),
                expr.span.clone(),
            ));
            TypedExpr {
                kind: TypedExprKind::Int(0),
                ty: Type::Void,
                ref_kind: RefKind::Unknown,
            }
        }
        ExprKind::Call { function, args } => {
            if let Some(builtin) =
                type_check_builtin_call(function, args, expr, signatures, env, ref_env, called_functions, diagnostics)
            {
                return builtin;
            }
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
                                        ref_env,
                                        called_functions,
                                        diagnostics,
                                    )
                                })
                                .collect(),
                        },
                        ty: Type::Void,
                        ref_kind: RefKind::Unknown,
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
                .map(|arg| type_check_expr(arg, signatures, env, ref_env, called_functions, diagnostics))
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
                ref_kind: RefKind::Unknown,
            }
        }
    }
}

fn type_check_path(
    path: &PathExpr,
    signatures: &BTreeMap<String, FunctionSignature>,
    env: &HashMap<String, Type>,
    ref_env: &HashMap<String, RefKind>,
    called_functions: &mut BTreeSet<String>,
    diagnostics: &mut Diagnostics,
    span: Span,
) -> TypedPathExpr {
    let base = type_check_expr(&path.base, signatures, env, ref_env, called_functions, diagnostics);
    if !matches!(base.ty, Type::EntityRef | Type::BlockRef) {
        diagnostics.push(Diagnostic::new(
            "path access requires an 'entity_ref' or 'block_ref' base",
            span.clone(),
        ));
    }
    let typed = TypedPathExpr {
        base: Box::new(base),
        segments: path.segments.clone(),
    };
    validate_player_path_read(&typed, span, diagnostics);
    typed
}

fn type_check_builtin_call(
    function: &str,
    args: &[Expr],
    expr: &Expr,
    signatures: &BTreeMap<String, FunctionSignature>,
    env: &HashMap<String, Type>,
    ref_env: &HashMap<String, RefKind>,
    called_functions: &mut BTreeSet<String>,
    diagnostics: &mut Diagnostics,
) -> Option<TypedExpr> {
    match function {
        "selector" => {
            let args = type_check_args(args, signatures, env, ref_env, called_functions, diagnostics);
            expect_arity(function, &args, 1, expr, diagnostics);
            if let Some(arg) = args.first() {
                if arg.ty != Type::String {
                    diagnostics.push(Diagnostic::new(
                        "selector(...) requires a 'string' argument",
                        expr.span.clone(),
                    ));
                }
            }
            let raw = extract_string_literal(args.first(), "selector", expr, diagnostics);
            Some(TypedExpr {
                kind: TypedExprKind::Selector(raw.clone()),
                ty: Type::EntitySet,
                ref_kind: detect_selector_ref_kind(&raw),
            })
        }
        "block" => {
            let args = type_check_args(args, signatures, env, ref_env, called_functions, diagnostics);
            expect_arity(function, &args, 1, expr, diagnostics);
            if let Some(arg) = args.first() {
                if arg.ty != Type::String {
                    diagnostics.push(Diagnostic::new(
                        "block(...) requires a 'string' argument",
                        expr.span.clone(),
                    ));
                }
            }
            let raw = extract_string_literal(args.first(), "block", expr, diagnostics);
            Some(TypedExpr {
                kind: TypedExprKind::Block(raw),
                ty: Type::BlockRef,
                ref_kind: RefKind::Unknown,
            })
        }
        "single" => {
            let args = type_check_args(args, signatures, env, ref_env, called_functions, diagnostics);
            expect_arity(function, &args, 1, expr, diagnostics);
            let mut arg = args.into_iter().next().unwrap_or(TypedExpr {
                kind: TypedExprKind::Selector(String::new()),
                ty: Type::EntitySet,
                ref_kind: RefKind::Unknown,
            });
            if arg.ty != Type::EntitySet {
                diagnostics.push(Diagnostic::new(
                    "single(...) requires an 'entity_set' argument",
                    expr.span.clone(),
                ));
            }
            rewrite_single_limit(&mut arg, diagnostics, expr.span.clone());
            let ref_kind = arg.ref_kind;
            Some(TypedExpr {
                kind: TypedExprKind::Single(Box::new(arg)),
                ty: Type::EntityRef,
                ref_kind,
            })
        }
        "exists" => {
            let args = type_check_args(args, signatures, env, ref_env, called_functions, diagnostics);
            expect_arity(function, &args, 1, expr, diagnostics);
            let arg = args.into_iter().next().unwrap_or(TypedExpr {
                kind: TypedExprKind::Variable("_error".to_string()),
                ty: Type::EntityRef,
                ref_kind: RefKind::Unknown,
            });
            if arg.ty != Type::EntityRef {
                diagnostics.push(Diagnostic::new(
                    "exists(...) requires an 'entity_ref' argument",
                    expr.span.clone(),
                ));
            }
            Some(TypedExpr {
                kind: TypedExprKind::Exists(Box::new(arg)),
                ty: Type::Bool,
                ref_kind: RefKind::Unknown,
            })
        }
        "at" => {
            let args = type_check_args(args, signatures, env, ref_env, called_functions, diagnostics);
            expect_arity(function, &args, 2, expr, diagnostics);
            let mut iter = args.into_iter();
            let anchor = iter.next().unwrap_or(TypedExpr {
                kind: TypedExprKind::Variable("_error".to_string()),
                ty: Type::EntityRef,
                ref_kind: RefKind::Unknown,
            });
            let value = iter.next().unwrap_or(TypedExpr {
                kind: TypedExprKind::Selector(String::new()),
                ty: Type::EntitySet,
                ref_kind: RefKind::Unknown,
            });
            if anchor.ty != Type::EntityRef {
                diagnostics.push(Diagnostic::new(
                    "at(...) requires an 'entity_ref' anchor",
                    expr.span.clone(),
                ));
            }
            if !matches!(value.ty, Type::EntitySet | Type::EntityRef | Type::BlockRef) {
                diagnostics.push(Diagnostic::new(
                    "at(...) requires an 'entity_set', 'entity_ref', or 'block_ref' value",
                    expr.span.clone(),
                ));
            }
            Some(TypedExpr {
                kind: TypedExprKind::At {
                    anchor: Box::new(anchor),
                    value: Box::new(value.clone()),
                },
                ty: value.ty,
                ref_kind: value.ref_kind,
            })
        }
        "int" | "bool" | "string" => {
            let args = type_check_args(args, signatures, env, ref_env, called_functions, diagnostics);
            expect_arity(function, &args, 1, expr, diagnostics);
            let arg = args.into_iter().next().unwrap_or(TypedExpr {
                kind: TypedExprKind::Variable("_error".to_string()),
                ty: Type::Nbt,
                ref_kind: RefKind::Unknown,
            });
            if arg.ty != Type::Nbt {
                diagnostics.push(Diagnostic::new(
                    format!("{}(...) requires an 'nbt' argument", function),
                    expr.span.clone(),
                ));
            }
            let (kind, ty) = match function {
                "int" => (CastKind::Int, Type::Int),
                "bool" => (CastKind::Bool, Type::Bool),
                _ => (CastKind::String, Type::String),
            };
            Some(TypedExpr {
                kind: TypedExprKind::Cast {
                    kind,
                    expr: Box::new(arg),
                },
                ty,
                ref_kind: RefKind::Unknown,
            })
        }
        _ => None,
    }
}

fn type_check_method_call(
    receiver: &Expr,
    method: &str,
    args: &[Expr],
    expr: &Expr,
    signatures: &BTreeMap<String, FunctionSignature>,
    env: &HashMap<String, Type>,
    ref_env: &HashMap<String, RefKind>,
    called_functions: &mut BTreeSet<String>,
    diagnostics: &mut Diagnostics,
) -> Option<TypedExpr> {
    let receiver = type_check_expr(
        receiver,
        signatures,
        env,
        ref_env,
        called_functions,
        diagnostics,
    );
    let args = type_check_args(args, signatures, env, ref_env, called_functions, diagnostics);
    match method {
        "effect" => {
            if receiver.ref_kind != RefKind::Player || receiver.ty != Type::EntityRef {
                diagnostics.push(Diagnostic::new(
                    "player.effect(...) requires a player 'entity_ref' receiver",
                    expr.span.clone(),
                ));
            }
            expect_arity(method, &args, 3, expr, diagnostics);
            if let Some(arg) = args.first() {
                if arg.ty != Type::String {
                    diagnostics.push(Diagnostic::new(
                        "player.effect(...) effect name must be 'string'",
                        expr.span.clone(),
                    ));
                }
            }
            if args.get(1).map(|arg| arg.ty.clone()) != Some(Type::Int) {
                diagnostics.push(Diagnostic::new(
                    "player.effect(...) duration must be 'int'",
                    expr.span.clone(),
                ));
            }
            if args.get(2).map(|arg| arg.ty.clone()) != Some(Type::Int) {
                diagnostics.push(Diagnostic::new(
                    "player.effect(...) amplifier must be 'int'",
                    expr.span.clone(),
                ));
            }
            Some(TypedExpr {
                kind: TypedExprKind::MethodCall {
                    receiver: Box::new(receiver),
                    method: method.to_string(),
                    args,
                },
                ty: Type::Void,
                ref_kind: RefKind::Unknown,
            })
        }
        _ => None,
    }
}

fn detect_selector_ref_kind(selector: &str) -> RefKind {
    let trimmed = selector.trim().to_ascii_lowercase();
    if trimmed.starts_with("@p")
        || trimmed.starts_with("@a")
        || trimmed.starts_with("@r")
        || trimmed.starts_with("@s")
        || trimmed.contains("type=player")
    {
        RefKind::Player
    } else if trimmed.contains("type=") {
        RefKind::NonPlayer
    } else {
        RefKind::Unknown
    }
}

fn validate_player_path_read(path: &TypedPathExpr, span: Span, diagnostics: &mut Diagnostics) {
    if path.base.ref_kind != RefKind::Player || path.base.ty != Type::EntityRef {
        return;
    }
    let Some(first) = path.segments.first() else {
        return;
    };
    let PathSegment::Field(first) = first else {
        diagnostics.push(Diagnostic::new(
            "player path access must start with a namespace such as 'nbt', 'state', 'tags', 'team', or 'mainhand'",
            span,
        ));
        return;
    };
    if !matches!(first.as_str(), "nbt" | "state" | "tags" | "team" | "mainhand") {
        diagnostics.push(Diagnostic::new(
            "player path access must use 'player.nbt', 'player.state', 'player.tags', 'player.team', or 'player.mainhand'",
            span,
        ));
    }
}

fn validate_player_path_write(
    path: &TypedPathExpr,
    value: &TypedExpr,
    span: Span,
    diagnostics: &mut Diagnostics,
) {
    if path.base.ref_kind != RefKind::Player || path.base.ty != Type::EntityRef {
        return;
    }
    let Some(PathSegment::Field(first)) = path.segments.first() else {
        diagnostics.push(Diagnostic::new(
            "player writes must use a player-safe namespace",
            span,
        ));
        return;
    };
    match first.as_str() {
        "nbt" => diagnostics.push(Diagnostic::new(
            "player.nbt.* is read-only; use player.state, player.tags, player.team, or player.mainhand instead",
            span,
        )),
        "state" => {
            if !matches!(value.ty, Type::Int | Type::Bool) {
                diagnostics.push(Diagnostic::new(
                    "player.state.* currently supports only 'int' and 'bool' values",
                    span,
                ));
            }
        }
        "tags" => {
            if value.ty != Type::Bool {
                diagnostics.push(Diagnostic::new(
                    "player.tags.* assignments require a 'bool' value",
                    span,
                ));
            }
        }
        "team" => {
            if value.ty != Type::String {
                diagnostics.push(Diagnostic::new(
                    "player.team requires a 'string' value",
                    span,
                ));
            }
        }
        "mainhand" => {}
        _ => diagnostics.push(Diagnostic::new(
            "unsafe writable player path; use player.state, player.tags, player.team, or player.mainhand",
            span,
        )),
    }
}

fn type_check_args(
    args: &[Expr],
    signatures: &BTreeMap<String, FunctionSignature>,
    env: &HashMap<String, Type>,
    ref_env: &HashMap<String, RefKind>,
    called_functions: &mut BTreeSet<String>,
    diagnostics: &mut Diagnostics,
) -> Vec<TypedExpr> {
    args.iter()
        .map(|arg| type_check_expr(arg, signatures, env, ref_env, called_functions, diagnostics))
        .collect()
}

fn expect_arity(
    function: &str,
    args: &[TypedExpr],
    expected: usize,
    expr: &Expr,
    diagnostics: &mut Diagnostics,
) {
    if args.len() != expected {
        diagnostics.push(Diagnostic::new(
            format!(
                "wrong arity for '{}': expected {}, found {}",
                function, expected, args.len()
            ),
            expr.span.clone(),
        ));
    }
}

fn extract_string_literal(
    arg: Option<&TypedExpr>,
    function: &str,
    expr: &Expr,
    diagnostics: &mut Diagnostics,
) -> String {
    match arg.map(|value| &value.kind) {
        Some(TypedExprKind::String(value)) => value.clone(),
        _ => {
            diagnostics.push(Diagnostic::new(
                format!("{}(...) currently requires a string literal", function),
                expr.span.clone(),
            ));
            String::new()
        }
    }
}

fn rewrite_single_limit(expr: &mut TypedExpr, diagnostics: &mut Diagnostics, span: Span) {
    match &mut expr.kind {
        TypedExprKind::Selector(value) => {
            *value = add_or_validate_limit(value, diagnostics, span);
        }
        TypedExprKind::At { value, .. } => rewrite_single_limit(value, diagnostics, span),
        _ => {}
    }
}

fn add_or_validate_limit(value: &str, diagnostics: &mut Diagnostics, span: Span) -> String {
    let lower = value.to_ascii_lowercase();
    if let Some(index) = lower.find("limit=") {
        let suffix = &lower[index + 6..];
        let digits: String = suffix.chars().take_while(|ch| ch.is_ascii_digit()).collect();
        if digits == "1" {
            return value.to_string();
        }
        diagnostics.push(Diagnostic::new(
            "single(selector(...)) requires no limit or 'limit=1'",
            span,
        ));
        return value.to_string();
    }

    if let Some(close) = value.rfind(']') {
        let mut rewritten = value.to_string();
        rewritten.insert_str(close, ",limit=1");
        rewritten
    } else {
        format!("{}[limit=1]", value)
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
                if !matches!(
                    ty,
                    Type::Int
                        | Type::Bool
                        | Type::String
                        | Type::EntitySet
                        | Type::EntityRef
                        | Type::BlockRef
                        | Type::Nbt
                ) {
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
