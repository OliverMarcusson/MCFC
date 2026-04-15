use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::ast::*;
use crate::diagnostics::{Diagnostic, Diagnostics, Span};

#[derive(Debug, Clone)]
pub struct TypedProgram {
    pub struct_defs: BTreeMap<String, StructTypeDef>,
    pub player_states: Vec<PlayerStateDef>,
    pub functions: Vec<TypedFunction>,
    pub function_signatures: BTreeMap<String, FunctionSignature>,
    pub call_depths: BTreeMap<String, usize>,
}

#[derive(Debug, Clone)]
pub struct TypedFunction {
    pub name: String,
    pub params: Vec<TypedParam>,
    pub return_type: Type,
    pub body: Vec<TypedStmt>,
    pub locals: BTreeMap<String, Type>,
    pub local_ref_kinds: BTreeMap<String, RefKind>,
    pub called_functions: BTreeSet<String>,
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
pub struct StructTypeDef {
    pub fields: BTreeMap<String, Type>,
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
    Context {
        kind: ContextKind,
        anchor: TypedExpr,
        body: Vec<TypedStmt>,
    },
    Async {
        captures: Vec<AsyncCapture>,
        body: Vec<TypedStmt>,
        locals: BTreeMap<String, Type>,
        local_ref_kinds: BTreeMap<String, RefKind>,
        called_functions: BTreeSet<String>,
    },
    Break,
    Continue,
    Return(Option<TypedExpr>),
    RawCommand(String),
    MacroCommand {
        template: String,
        placeholders: Vec<MacroPlaceholder>,
    },
    Sleep {
        duration: TypedExpr,
        unit: SleepUnit,
    },
    Expr(TypedExpr),
}

#[derive(Debug, Clone)]
pub struct AsyncCapture {
    pub name: String,
    pub ty: Type,
    pub ref_kind: RefKind,
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
    pub key: String,
    pub expr: TypedExpr,
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
    pub segment_types: Vec<Type>,
    pub ty: Type,
}

#[derive(Debug, Clone)]
pub enum TypedExprKind {
    Int(i64),
    Bool(bool),
    String(String),
    InterpolatedString {
        template: String,
        placeholders: Vec<MacroPlaceholder>,
    },
    ArrayLiteral(Vec<TypedExpr>),
    DictLiteral(Vec<(String, TypedExpr)>),
    StructLiteral {
        name: String,
        fields: Vec<(String, TypedExpr)>,
    },
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
    HasData(Box<TypedExpr>),
    At {
        anchor: Box<TypedExpr>,
        value: Box<TypedExpr>,
    },
    As {
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
    let mut struct_defs = BTreeMap::new();
    let mut signatures = BTreeMap::new();

    for struct_def in &program.structs {
        let mut fields = BTreeMap::new();
        if struct_defs.contains_key(&struct_def.name) {
            diagnostics.push(Diagnostic::new(
                format!("duplicate struct '{}'", struct_def.name),
                struct_def.span.clone(),
            ));
            continue;
        }
        for field in &struct_def.fields {
            if fields
                .insert(field.name.clone(), field.ty.clone())
                .is_some()
            {
                diagnostics.push(Diagnostic::new(
                    format!("duplicate field '{}.{}'", struct_def.name, field.name),
                    field.span.clone(),
                ));
            }
        }
        struct_defs.insert(struct_def.name.clone(), StructTypeDef { fields });
    }

    for struct_def in &program.structs {
        for field in &struct_def.fields {
            validate_declared_type(
                &field.ty,
                &struct_defs,
                field.span.clone(),
                &mut diagnostics,
            );
        }
    }

    let mut player_state_names = HashSet::new();
    for state in &program.player_states {
        for segment in &state.path {
            if !is_storage_path_safe_key(segment) {
                diagnostics.push(Diagnostic::new(
                    format!(
                        "player state path segment '{}' is not storage-path-safe",
                        segment
                    ),
                    state.span.clone(),
                ));
            }
        }
        if !matches!(state.ty, Type::Int | Type::Bool) {
            diagnostics.push(Diagnostic::new(
                "player_state declarations currently support only 'int' and 'bool'",
                state.span.clone(),
            ));
        }
        if !player_state_names.insert(state.path.join(".")) {
            diagnostics.push(Diagnostic::new(
                format!("duplicate player_state '{}'", state.path.join(".")),
                state.span.clone(),
            ));
        }
    }

    for function in &program.functions {
        for param in &function.params {
            validate_declared_type(
                &param.ty,
                &struct_defs,
                param.span.clone(),
                &mut diagnostics,
            );
        }
        validate_declared_type(
            &function.return_type,
            &struct_defs,
            function.span.clone(),
            &mut diagnostics,
        );
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
            ref_env.insert(
                param.name.clone(),
                if param.ty == Type::PlayerRef {
                    RefKind::Player
                } else {
                    RefKind::Unknown
                },
            );
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
            &struct_defs,
            &signatures,
            &mut env,
            &mut ref_env,
            &mut locals,
            &mut called_functions,
            0,
            false,
            &mut diagnostics,
        );

        functions.push(TypedFunction {
            name: function.name.clone(),
            params,
            return_type: function.return_type.clone(),
            body,
            locals,
            local_ref_kinds: ref_env
                .iter()
                .map(|(name, kind)| (name.clone(), *kind))
                .collect(),
            called_functions,
        });
    }

    detect_recursion(&functions, &mut diagnostics);
    let call_depths = compute_call_depths(&functions, &mut diagnostics);

    diagnostics.into_result(TypedProgram {
        struct_defs,
        functions,
        function_signatures: signatures,
        player_states: program.player_states.clone(),
        call_depths,
    })
}

fn type_check_block(
    statements: &[Stmt],
    return_type: &Type,
    struct_defs: &BTreeMap<String, StructTypeDef>,
    signatures: &BTreeMap<String, FunctionSignature>,
    env: &mut HashMap<String, Type>,
    ref_env: &mut HashMap<String, RefKind>,
    locals: &mut BTreeMap<String, Type>,
    called_functions: &mut BTreeSet<String>,
    loop_depth: usize,
    in_async: bool,
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
                let value = type_check_expr(
                    value,
                    struct_defs,
                    signatures,
                    env,
                    ref_env,
                    called_functions,
                    diagnostics,
                );
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
                let mut value = type_check_expr(
                    value,
                    struct_defs,
                    signatures,
                    env,
                    ref_env,
                    called_functions,
                    diagnostics,
                );
                let target = match target {
                    AssignTarget::Variable(name) => {
                        let Some(existing) = env.get(name).cloned() else {
                            diagnostics.push(Diagnostic::new(
                                format!("undefined variable '{}'", name),
                                statement.span.clone(),
                            ));
                            continue;
                        };
                        value = coerce_expr_to_expected_type(value, &existing);
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
                            struct_defs,
                            signatures,
                            env,
                            ref_env,
                            called_functions,
                            diagnostics,
                            statement.span.clone(),
                        );
                        let is_equipment_item_def_write = is_entity_ref_type(&typed_path.base.ty)
                            && value.ty == Type::ItemDef
                            && matches!(
                                typed_path.segments.as_slice(),
                                [PathSegment::Field(slot), PathSegment::Field(field), ..]
                                    if matches!(
                                        slot.as_str(),
                                        "mainhand" | "offhand" | "head" | "chest" | "legs" | "feet"
                                    ) && field == "item"
                            );
                        if !is_equipment_item_def_write {
                            value = coerce_expr_to_expected_type(value, &typed_path.ty);
                        }
                        if matches!(
                            typed_path.base.ty,
                            Type::EntityRef | Type::PlayerRef | Type::BlockRef
                        ) {
                            let is_player_slot_write = is_entity_ref_type(&typed_path.base.ty)
                                && typed_path.base.ref_kind == RefKind::Player
                                && matches!(
                                    typed_path.segments.first(),
                                    Some(PathSegment::Field(name))
                                        if matches!(name.as_str(), "inventory" | "hotbar")
                                );
                            let is_equipment_item_write = is_entity_ref_type(&typed_path.base.ty)
                                && matches!(
                                    typed_path.segments.as_slice(),
                                    [
                                        PathSegment::Field(slot),
                                        PathSegment::Field(field),
                                        ..
                                    ] if matches!(
                                        slot.as_str(),
                                        "mainhand" | "offhand" | "head" | "chest" | "legs" | "feet"
                                    ) && field == "item"
                                );
                            if !matches!(
                                value.ty,
                                Type::Int | Type::Bool | Type::String | Type::Nbt | Type::TextDef
                            ) && !(is_player_slot_write && value.ty == Type::ItemDef)
                                && !(is_equipment_item_write && value.ty == Type::ItemDef)
                            {
                                diagnostics.push(Diagnostic::new(
                                    "path assignment requires a value of type 'int', 'bool', 'string', 'nbt', or an item builder for inventory slots",
                                    statement.span.clone(),
                                ));
                            }
                            validate_player_path_write(
                                &typed_path,
                                &value,
                                statement.span.clone(),
                                diagnostics,
                            );
                        } else if matches!(
                            typed_path.base.ty,
                            Type::Array(_)
                                | Type::Dict(_)
                                | Type::Struct(_)
                                | Type::Nbt
                                | Type::EntityDef
                                | Type::BlockDef
                                | Type::ItemDef
                                | Type::TextDef
                                | Type::ItemSlot
                        ) {
                            if !is_storage_lvalue_expr(&path.base) {
                                diagnostics.push(Diagnostic::new(
                                    "collection assignment requires a variable or collection element base",
                                    statement.span.clone(),
                                ));
                            }
                            if !storage_path_accepts_value(&typed_path, &value) {
                                diagnostics.push(Diagnostic::new(
                                    storage_path_assignment_message(&typed_path, &value),
                                    statement.span.clone(),
                                ));
                            }
                            validate_builder_path_write(
                                &typed_path,
                                &value,
                                statement.span.clone(),
                                diagnostics,
                            );
                        } else if matches!(typed_path.base.ty, Type::Bossbar) {
                            validate_bossbar_path_write(
                                &typed_path,
                                &value,
                                statement.span.clone(),
                                diagnostics,
                            );
                        } else {
                            diagnostics.push(Diagnostic::new(
                                "path assignment requires an 'entity_ref', 'block_ref', bossbar, or storage-backed base",
                                statement.span.clone(),
                            ));
                        }
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
                let condition = coerce_expr_to_expected_type(
                    type_check_expr(
                        condition,
                        struct_defs,
                        signatures,
                        env,
                        ref_env,
                        called_functions,
                        diagnostics,
                    ),
                    &Type::Bool,
                );
                if condition.ty != Type::Bool {
                    diagnostics.push(Diagnostic::new(
                        "if condition must have type 'bool'",
                        statement.span.clone(),
                    ));
                }
                let then_body = type_check_block(
                    then_body,
                    return_type,
                    struct_defs,
                    signatures,
                    &mut env.clone(),
                    &mut ref_env.clone(),
                    locals,
                    called_functions,
                    loop_depth,
                    in_async,
                    diagnostics,
                );
                let else_body = type_check_block(
                    else_body,
                    return_type,
                    struct_defs,
                    signatures,
                    &mut env.clone(),
                    &mut ref_env.clone(),
                    locals,
                    called_functions,
                    loop_depth,
                    in_async,
                    diagnostics,
                );
                TypedStmtKind::If {
                    condition,
                    then_body,
                    else_body,
                }
            }
            StmtKind::While { condition, body } => {
                let condition = coerce_expr_to_expected_type(
                    type_check_expr(
                        condition,
                        struct_defs,
                        signatures,
                        env,
                        ref_env,
                        called_functions,
                        diagnostics,
                    ),
                    &Type::Bool,
                );
                if condition.ty != Type::Bool {
                    diagnostics.push(Diagnostic::new(
                        "while condition must have type 'bool'",
                        statement.span.clone(),
                    ));
                }
                let body = type_check_block(
                    body,
                    return_type,
                    struct_defs,
                    signatures,
                    &mut env.clone(),
                    &mut ref_env.clone(),
                    locals,
                    called_functions,
                    loop_depth + 1,
                    in_async,
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
                        let start = type_check_expr(
                            start,
                            struct_defs,
                            signatures,
                            env,
                            ref_env,
                            called_functions,
                            diagnostics,
                        );
                        let end = type_check_expr(
                            end,
                            struct_defs,
                            signatures,
                            env,
                            ref_env,
                            called_functions,
                            diagnostics,
                        );
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
                            struct_defs,
                            signatures,
                            env,
                            ref_env,
                            called_functions,
                            diagnostics,
                        );
                        let (item_ty, item_ref_kind) = match &iterable.ty {
                            Type::EntitySet => (Type::EntityRef, iterable.ref_kind),
                            Type::Array(element) => (*element.clone(), RefKind::Unknown),
                            _ => {
                                diagnostics.push(Diagnostic::new(
                                    "for-each iteration requires an 'entity_set' or 'array'",
                                    statement.span.clone(),
                                ));
                                (Type::Nbt, RefKind::Unknown)
                            }
                        };
                        loop_env.insert(name.clone(), item_ty.clone());
                        loop_ref_env.insert(name.clone(), item_ref_kind);
                        locals.insert(name.clone(), item_ty);
                        TypedForKind::Each { iterable }
                    }
                };
                let body = type_check_block(
                    body,
                    return_type,
                    struct_defs,
                    signatures,
                    &mut loop_env,
                    &mut loop_ref_env,
                    locals,
                    called_functions,
                    loop_depth + 1,
                    in_async,
                    diagnostics,
                );
                TypedStmtKind::For {
                    name: name.clone(),
                    kind,
                    body,
                }
            }
            StmtKind::Match {
                value,
                arms,
                else_body,
            } => {
                let value = type_check_expr(
                    value,
                    struct_defs,
                    signatures,
                    env,
                    ref_env,
                    called_functions,
                    diagnostics,
                );
                if value.ty != Type::String {
                    diagnostics.push(Diagnostic::new(
                        "match value must have type 'string'",
                        statement.span.clone(),
                    ));
                }
                let mut seen = BTreeSet::new();
                let mut typed_arms = Vec::new();
                for arm in arms {
                    if !seen.insert(arm.pattern.clone()) {
                        diagnostics.push(Diagnostic::new(
                            format!("duplicate match arm '{}'", arm.pattern),
                            statement.span.clone(),
                        ));
                    }
                    let body = type_check_block(
                        &arm.body,
                        return_type,
                        struct_defs,
                        signatures,
                        &mut env.clone(),
                        &mut ref_env.clone(),
                        locals,
                        called_functions,
                        loop_depth,
                        in_async,
                        diagnostics,
                    );
                    typed_arms.push((arm.pattern.clone(), body));
                }
                let else_body = type_check_block(
                    else_body,
                    return_type,
                    struct_defs,
                    signatures,
                    &mut env.clone(),
                    &mut ref_env.clone(),
                    locals,
                    called_functions,
                    loop_depth,
                    in_async,
                    diagnostics,
                );
                lower_string_match_stmt(value, typed_arms, else_body)
            }
            StmtKind::Context { kind, anchor, body } => {
                let anchor = type_check_expr(
                    anchor,
                    struct_defs,
                    signatures,
                    env,
                    ref_env,
                    called_functions,
                    diagnostics,
                );
                if !matches!(
                    anchor.ty,
                    Type::EntitySet | Type::EntityRef | Type::PlayerRef
                ) {
                    diagnostics.push(Diagnostic::new(
                        format!(
                            "{} context block requires an 'entity_set' or 'entity_ref' anchor",
                            context_name(*kind)
                        ),
                        statement.span.clone(),
                    ));
                }
                let body = type_check_block(
                    body,
                    return_type,
                    struct_defs,
                    signatures,
                    &mut env.clone(),
                    &mut ref_env.clone(),
                    locals,
                    called_functions,
                    loop_depth,
                    in_async,
                    diagnostics,
                );
                TypedStmtKind::Context {
                    kind: *kind,
                    anchor,
                    body,
                }
            }
            StmtKind::Async { body } => {
                let mut capture_items: Vec<_> = env
                    .iter()
                    .filter(|(_, ty)| **ty != Type::Void)
                    .map(|(name, ty)| AsyncCapture {
                        name: name.clone(),
                        ty: ty.clone(),
                        ref_kind: ref_env.get(name).copied().unwrap_or(RefKind::Unknown),
                    })
                    .collect();
                capture_items.sort_by(|left, right| left.name.cmp(&right.name));

                let mut async_env = env.clone();
                let mut async_ref_env = ref_env.clone();
                let mut async_locals: BTreeMap<String, Type> = capture_items
                    .iter()
                    .map(|capture| (capture.name.clone(), capture.ty.clone()))
                    .collect();
                let mut async_called = BTreeSet::new();
                let typed_body = type_check_block(
                    body,
                    &Type::Void,
                    struct_defs,
                    signatures,
                    &mut async_env,
                    &mut async_ref_env,
                    &mut async_locals,
                    &mut async_called,
                    loop_depth,
                    true,
                    diagnostics,
                );
                called_functions.extend(async_called.iter().cloned());
                TypedStmtKind::Async {
                    captures: capture_items,
                    body: typed_body,
                    locals: async_locals,
                    local_ref_kinds: async_ref_env
                        .iter()
                        .map(|(name, kind)| (name.clone(), *kind))
                        .collect(),
                    called_functions: async_called,
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
                if in_async {
                    diagnostics.push(Diagnostic::new(
                        "return may not appear inside an async block",
                        statement.span.clone(),
                    ));
                }
                let value = value.as_ref().map(|expr| {
                    let value = type_check_expr(
                        expr,
                        struct_defs,
                        signatures,
                        env,
                        ref_env,
                        called_functions,
                        diagnostics,
                    );
                    coerce_expr_to_expected_type(value, return_type)
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
                let placeholders = collect_macro_placeholders(
                    template,
                    struct_defs,
                    signatures,
                    env,
                    ref_env,
                    called_functions,
                    statement.span.clone(),
                    diagnostics,
                );
                TypedStmtKind::MacroCommand {
                    template: template.clone(),
                    placeholders,
                }
            }
            StmtKind::Expr(expr) => {
                if let ExprKind::Call { function, args } = &expr.kind {
                    if matches!(function.as_str(), "sleep" | "sleep_ticks") {
                        let args = type_check_args(
                            args,
                            struct_defs,
                            signatures,
                            env,
                            ref_env,
                            called_functions,
                            diagnostics,
                        );
                        expect_arity(function, &args, 1, expr, diagnostics);
                        if let Some(duration) = args.first() {
                            if duration.ty != Type::Int {
                                let message = if function == "sleep" {
                                    "sleep(...) seconds must have type 'int'".to_string()
                                } else {
                                    "sleep_ticks(...) duration must have type 'int'".to_string()
                                };
                                diagnostics.push(Diagnostic::new(message, statement.span.clone()));
                            }
                            if matches!(duration.kind, TypedExprKind::Int(value) if value < 1) {
                                let message = if function == "sleep" {
                                    "sleep(...) seconds must be at least 1".to_string()
                                } else {
                                    "sleep_ticks(...) duration must be at least 1".to_string()
                                };
                                diagnostics.push(Diagnostic::new(message, statement.span.clone()));
                            }
                        }
                        let duration = args.into_iter().next().unwrap_or(TypedExpr {
                            kind: TypedExprKind::Int(1),
                            ty: Type::Int,
                            ref_kind: RefKind::Unknown,
                        });
                        let unit = if function == "sleep_ticks" {
                            SleepUnit::Ticks
                        } else {
                            SleepUnit::Seconds
                        };
                        TypedStmtKind::Sleep { duration, unit }
                    } else {
                        let expr = type_check_expr(
                            expr,
                            struct_defs,
                            signatures,
                            env,
                            ref_env,
                            called_functions,
                            diagnostics,
                        );
                        if !matches!(
                            expr.kind,
                            TypedExprKind::Call { .. } | TypedExprKind::MethodCall { .. }
                        ) {
                            diagnostics.push(Diagnostic::new(
                                "only function calls may appear as bare expression statements",
                                statement.span.clone(),
                            ));
                        }
                        TypedStmtKind::Expr(expr)
                    }
                } else {
                    let expr = type_check_expr(
                        expr,
                        struct_defs,
                        signatures,
                        env,
                        ref_env,
                        called_functions,
                        diagnostics,
                    );
                    if !matches!(
                        expr.kind,
                        TypedExprKind::Call { .. } | TypedExprKind::MethodCall { .. }
                    ) {
                        diagnostics.push(Diagnostic::new(
                            "only function calls may appear as bare expression statements",
                            statement.span.clone(),
                        ));
                    }
                    TypedStmtKind::Expr(expr)
                }
            }
        };

        typed.push(TypedStmt { kind });
    }

    typed
}

fn type_check_expr(
    expr: &Expr,
    struct_defs: &BTreeMap<String, StructTypeDef>,
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
            kind: if value.contains("$(") {
                TypedExprKind::InterpolatedString {
                    template: value.clone(),
                    placeholders: collect_macro_placeholders(
                        value,
                        struct_defs,
                        signatures,
                        env,
                        ref_env,
                        called_functions,
                        expr.span.clone(),
                        diagnostics,
                    ),
                }
            } else {
                TypedExprKind::String(value.clone())
            },
            ty: Type::String,
            ref_kind: RefKind::Unknown,
        },
        ExprKind::ArrayLiteral(values) => {
            let values: Vec<_> = values
                .iter()
                .map(|value| {
                    type_check_expr(
                        value,
                        struct_defs,
                        signatures,
                        env,
                        ref_env,
                        called_functions,
                        diagnostics,
                    )
                })
                .collect();
            let ty = infer_collection_type(
                values.iter().map(|value| &value.ty),
                "array literals must contain values of one type",
                "empty array literals require type context",
                expr.span.clone(),
                diagnostics,
            );
            validate_collection_value_type(&ty, expr.span.clone(), diagnostics);
            TypedExpr {
                kind: TypedExprKind::ArrayLiteral(values),
                ty: Type::Array(Box::new(ty)),
                ref_kind: RefKind::Unknown,
            }
        }
        ExprKind::DictLiteral(entries) => {
            let entries: Vec<_> = entries
                .iter()
                .map(|(key, value)| {
                    (
                        key.clone(),
                        type_check_expr(
                            value,
                            struct_defs,
                            signatures,
                            env,
                            ref_env,
                            called_functions,
                            diagnostics,
                        ),
                    )
                })
                .collect();
            for (key, _) in &entries {
                validate_dict_key_literal(key, expr.span.clone(), diagnostics);
            }
            let ty = infer_collection_type(
                entries.iter().map(|(_, value)| &value.ty),
                "dictionary literals must contain values of one type",
                "empty dictionary literals require type context",
                expr.span.clone(),
                diagnostics,
            );
            validate_collection_value_type(&ty, expr.span.clone(), diagnostics);
            TypedExpr {
                kind: TypedExprKind::DictLiteral(entries),
                ty: Type::Dict(Box::new(ty)),
                ref_kind: RefKind::Unknown,
            }
        }
        ExprKind::StructLiteral { name, fields } => {
            let Some(def) = struct_defs.get(name) else {
                diagnostics.push(Diagnostic::new(
                    format!("unknown struct '{}'", name),
                    expr.span.clone(),
                ));
                return TypedExpr {
                    kind: TypedExprKind::StructLiteral {
                        name: name.clone(),
                        fields: Vec::new(),
                    },
                    ty: Type::Nbt,
                    ref_kind: RefKind::Unknown,
                };
            };
            let mut seen = BTreeSet::new();
            let mut typed_fields = Vec::new();
            for (field_name, field_value) in fields {
                if !seen.insert(field_name.clone()) {
                    diagnostics.push(Diagnostic::new(
                        format!("duplicate field '{}.{}'", name, field_name),
                        expr.span.clone(),
                    ));
                }
                let value = type_check_expr(
                    field_value,
                    struct_defs,
                    signatures,
                    env,
                    ref_env,
                    called_functions,
                    diagnostics,
                );
                let value = match def.fields.get(field_name) {
                    Some(expected) => coerce_expr_to_expected_type(value, expected),
                    None => value,
                };
                match def.fields.get(field_name) {
                    Some(expected) if expected != &value.ty => diagnostics.push(Diagnostic::new(
                        format!(
                            "field '{}.{}' expects '{}', found '{}'",
                            name,
                            field_name,
                            expected.as_str(),
                            value.ty.as_str()
                        ),
                        expr.span.clone(),
                    )),
                    None => diagnostics.push(Diagnostic::new(
                        format!("unknown field '{}.{}'", name, field_name),
                        expr.span.clone(),
                    )),
                    _ => {}
                }
                typed_fields.push((field_name.clone(), value));
            }
            for required in def.fields.keys() {
                if !seen.contains(required) {
                    diagnostics.push(Diagnostic::new(
                        format!("missing field '{}.{}'", name, required),
                        expr.span.clone(),
                    ));
                }
            }
            TypedExpr {
                kind: TypedExprKind::StructLiteral {
                    name: name.clone(),
                    fields: typed_fields,
                },
                ty: Type::Struct(name.clone()),
                ref_kind: RefKind::Unknown,
            }
        }
        ExprKind::Path(path) => {
            let path = type_check_path(
                path,
                struct_defs,
                signatures,
                env,
                ref_env,
                called_functions,
                diagnostics,
                expr.span.clone(),
            );
            TypedExpr {
                ty: path.ty.clone(),
                kind: TypedExprKind::Path(path),
                ref_kind: RefKind::Unknown,
            }
        }
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
            let operand = coerce_expr_to_expected_type(
                type_check_expr(
                    expr,
                    struct_defs,
                    signatures,
                    env,
                    ref_env,
                    called_functions,
                    diagnostics,
                ),
                &Type::Bool,
            );
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
            let mut left = type_check_expr(
                left,
                struct_defs,
                signatures,
                env,
                ref_env,
                called_functions,
                diagnostics,
            );
            let mut right = type_check_expr(
                right,
                struct_defs,
                signatures,
                env,
                ref_env,
                called_functions,
                diagnostics,
            );
            let ty = match op {
                BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div => {
                    left = coerce_expr_to_expected_type(left, &Type::Int);
                    right = coerce_expr_to_expected_type(right, &Type::Int);
                    if left.ty != Type::Int || right.ty != Type::Int {
                        diagnostics.push(Diagnostic::new(
                            "arithmetic operators require 'int' operands",
                            expr.span.clone(),
                        ));
                    }
                    Type::Int
                }
                BinaryOp::And | BinaryOp::Or => {
                    left = coerce_expr_to_expected_type(left, &Type::Bool);
                    right = coerce_expr_to_expected_type(right, &Type::Bool);
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
                    left = coerce_expr_to_expected_type(left, &right.ty);
                    right = coerce_expr_to_expected_type(right, &left.ty);
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
        ExprKind::MethodCall {
            receiver,
            method,
            args,
        } => {
            if let Some(builtin) = type_check_method_call(
                receiver,
                method,
                args,
                expr,
                struct_defs,
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
            if let Some(builtin) = type_check_builtin_call(
                function,
                args,
                expr,
                struct_defs,
                signatures,
                env,
                ref_env,
                called_functions,
                diagnostics,
            ) {
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
                                        struct_defs,
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
                .enumerate()
                .map(|(index, arg)| {
                    let typed = type_check_expr(
                        arg,
                        struct_defs,
                        signatures,
                        env,
                        ref_env,
                        called_functions,
                        diagnostics,
                    );
                    match signature.params.get(index) {
                        Some(expected) => coerce_expr_to_expected_type(typed, expected),
                        None => typed,
                    }
                })
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
    struct_defs: &BTreeMap<String, StructTypeDef>,
    signatures: &BTreeMap<String, FunctionSignature>,
    env: &HashMap<String, Type>,
    ref_env: &HashMap<String, RefKind>,
    called_functions: &mut BTreeSet<String>,
    diagnostics: &mut Diagnostics,
    span: Span,
) -> TypedPathExpr {
    let base = type_check_expr(
        &path.base,
        struct_defs,
        signatures,
        env,
        ref_env,
        called_functions,
        diagnostics,
    );
    let segments =
        normalize_builder_path_segments(&base.ty, &path.segments, span.clone(), diagnostics);
    let mut current_ty = base.ty.clone();
    let mut collection_mode = false;
    let mut segment_types = Vec::new();
    let mut player_slot_namespace: Option<String> = None;
    for segment in &segments {
        match (&current_ty, segment) {
            (Type::EntityRef | Type::PlayerRef, PathSegment::Field(field))
                if field == "position" =>
            {
                current_ty = Type::BlockRef;
            }
            (Type::EntityRef | Type::PlayerRef, PathSegment::Field(field))
                if matches!(field.as_str(), "inventory" | "hotbar") =>
            {
                if base.ref_kind != RefKind::Player {
                    diagnostics.push(Diagnostic::new(
                        "inventory and hotbar are only supported on known player refs; use 'player_ref' to assert a player",
                        span.clone(),
                    ));
                    current_ty = Type::Nbt;
                } else {
                    current_ty = Type::Array(Box::new(Type::ItemSlot));
                    player_slot_namespace = Some(field.clone());
                }
            }
            (Type::EntityRef | Type::PlayerRef | Type::BlockRef, PathSegment::Field(_)) => {
                current_ty = Type::Nbt;
            }
            (Type::EntityRef | Type::PlayerRef | Type::BlockRef, PathSegment::Index(index)) => {
                if !matches!(index.kind, ExprKind::Int(_)) {
                    diagnostics.push(Diagnostic::new(
                        "entity and block path indices must be integer literals",
                        span.clone(),
                    ));
                }
                current_ty = Type::Nbt;
            }
            (Type::EntityDef, PathSegment::Field(field)) => {
                current_ty = match field.as_str() {
                    "id" => Type::String,
                    "nbt" => Type::Nbt,
                    _ => {
                        diagnostics.push(Diagnostic::new(
                            "entity builder path access must use 'id', 'nbt', or a supported alias such as 'name'",
                            span.clone(),
                        ));
                        Type::Nbt
                    }
                };
            }
            (Type::EntityDef, PathSegment::Index(_)) => {
                diagnostics.push(Diagnostic::new(
                    "entity builder values must be accessed with '.field'",
                    span.clone(),
                ));
                current_ty = Type::Nbt;
            }
            (Type::BlockDef, PathSegment::Field(field)) => {
                current_ty = match field.as_str() {
                    "id" => Type::String,
                    "states" | "nbt" => Type::Nbt,
                    _ => {
                        diagnostics.push(Diagnostic::new(
                            "block builder path access must use 'id', 'states', 'nbt', or a supported alias such as 'name'",
                            span.clone(),
                        ));
                        Type::Nbt
                    }
                };
            }
            (Type::BlockDef, PathSegment::Index(_)) => {
                diagnostics.push(Diagnostic::new(
                    "block builder values must be accessed with '.field'",
                    span.clone(),
                ));
                current_ty = Type::Nbt;
            }
            (Type::ItemDef, PathSegment::Field(field)) => {
                current_ty = match field.as_str() {
                    "id" => Type::String,
                    "count" => Type::Int,
                    "nbt" => Type::Nbt,
                    "name" => Type::Nbt,
                    _ => {
                        diagnostics.push(Diagnostic::new(
                            "item builder path access must use 'id', 'count', 'nbt', or a supported alias such as 'name'",
                            span.clone(),
                        ));
                        Type::Nbt
                    }
                };
            }
            (Type::ItemDef, PathSegment::Index(_)) => {
                diagnostics.push(Diagnostic::new(
                    "item builder values must be accessed with '.field'",
                    span.clone(),
                ));
                current_ty = Type::Nbt;
            }
            (Type::TextDef, PathSegment::Field(_)) => {
                current_ty = Type::Nbt;
            }
            (Type::TextDef, PathSegment::Index(_)) => {
                diagnostics.push(Diagnostic::new(
                    "text builder values must be accessed with '.field'",
                    span.clone(),
                ));
                current_ty = Type::Nbt;
            }
            (Type::ItemSlot, PathSegment::Field(field)) => {
                current_ty = match field.as_str() {
                    "exists" => Type::Bool,
                    "id" => Type::String,
                    "count" => Type::Int,
                    "nbt" => Type::Nbt,
                    "name" => Type::String,
                    _ => {
                        diagnostics.push(Diagnostic::new(
                            "item slot access must use 'exists', 'id', 'count', 'nbt', or the alias 'name'",
                            span.clone(),
                        ));
                        Type::Nbt
                    }
                };
            }
            (Type::ItemSlot, PathSegment::Index(_)) => {
                diagnostics.push(Diagnostic::new(
                    "item slots must be accessed with '.field'",
                    span.clone(),
                ));
                current_ty = Type::Nbt;
            }
            (Type::Nbt, PathSegment::Field(_)) => {
                current_ty = Type::Nbt;
            }
            (Type::Nbt, PathSegment::Index(index)) => {
                let index = type_check_expr(
                    index,
                    struct_defs,
                    signatures,
                    env,
                    ref_env,
                    called_functions,
                    diagnostics,
                );
                if !matches!(index.ty, Type::Int | Type::String) {
                    diagnostics.push(Diagnostic::new(
                        "nbt path indices must have type 'int' or 'string'",
                        span.clone(),
                    ));
                }
                if !matches!(index.kind, TypedExprKind::Int(_) | TypedExprKind::String(_))
                    && !is_storage_data_expr(&base)
                {
                    diagnostics.push(Diagnostic::new(
                        "dynamic nbt path indices require a storage-backed base",
                        span.clone(),
                    ));
                }
                current_ty = Type::Nbt;
            }
            (Type::Array(element), PathSegment::Index(index)) => {
                collection_mode = true;
                if let Some(namespace) = player_slot_namespace.take() {
                    let typed_index = type_check_expr(
                        index,
                        struct_defs,
                        signatures,
                        env,
                        ref_env,
                        called_functions,
                        diagnostics,
                    );
                    if typed_index.ty != Type::Int {
                        diagnostics.push(Diagnostic::new(
                            format!("player.{}[...] slot index must have type 'int'", namespace),
                            span.clone(),
                        ));
                    }
                    if let ExprKind::Int(value) = &index.kind {
                        let max = if namespace == "hotbar" { 8 } else { 26 };
                        if *value < 0 || *value > max {
                            diagnostics.push(Diagnostic::new(
                                format!(
                                    "player.{}[...] slot index must be between 0 and {}",
                                    namespace, max
                                ),
                                span.clone(),
                            ));
                        }
                    }
                } else {
                    let index = type_check_expr(
                        index,
                        struct_defs,
                        signatures,
                        env,
                        ref_env,
                        called_functions,
                        diagnostics,
                    );
                    if index.ty != Type::Int {
                        diagnostics.push(Diagnostic::new(
                            "array index must have type 'int'",
                            span.clone(),
                        ));
                    }
                }
                current_ty = *element.clone();
            }
            (Type::Dict(value), PathSegment::Index(index)) => {
                collection_mode = true;
                let key = type_check_expr(
                    index,
                    struct_defs,
                    signatures,
                    env,
                    ref_env,
                    called_functions,
                    diagnostics,
                );
                if key.ty != Type::String {
                    diagnostics.push(Diagnostic::new(
                        "dictionary key must have type 'string'",
                        span.clone(),
                    ));
                }
                if let ExprKind::String(key) = &index.kind {
                    validate_dict_key_literal(key, span.clone(), diagnostics);
                }
                current_ty = *value.clone();
            }
            (Type::Struct(name), PathSegment::Field(field)) => {
                match struct_defs
                    .get(name)
                    .and_then(|def| def.fields.get(field))
                    .cloned()
                {
                    Some(ty) => current_ty = ty,
                    None => {
                        diagnostics.push(Diagnostic::new(
                            format!("unknown field '{}.{}'", name, field),
                            span.clone(),
                        ));
                        current_ty = Type::Nbt;
                    }
                }
            }
            (Type::Struct(_), PathSegment::Index(_)) => {
                diagnostics.push(Diagnostic::new(
                    "struct values must be accessed with '.field'",
                    span.clone(),
                ));
                current_ty = Type::Nbt;
            }
            (Type::Bossbar, PathSegment::Field(field)) => {
                current_ty = match field.as_str() {
                    "name" => Type::String,
                    "value" | "max" => Type::Int,
                    "visible" => Type::Bool,
                    "players" => Type::EntitySet,
                    _ => {
                        diagnostics.push(Diagnostic::new(
                            format!("unknown bossbar property '{}'", field),
                            span.clone(),
                        ));
                        Type::Nbt
                    }
                };
            }
            (Type::Bossbar, PathSegment::Index(_)) => {
                diagnostics.push(Diagnostic::new(
                    "bossbar values must be accessed with '.property'",
                    span.clone(),
                ));
                current_ty = Type::Nbt;
            }
            (Type::Array(_) | Type::Dict(_), PathSegment::Field(_)) => {
                diagnostics.push(Diagnostic::new(
                    "collection values must be accessed with '[...]'",
                    span.clone(),
                ));
                current_ty = Type::Nbt;
            }
            _ => {
                diagnostics.push(Diagnostic::new(
                    "path access requires an entity, block, bossbar, item slot, nbt, array, or dictionary base",
                    span.clone(),
                ));
                current_ty = Type::Nbt;
            }
        }
        segment_types.push(current_ty.clone());
    }
    let typed = TypedPathExpr {
        base: Box::new(base),
        segments,
        segment_types,
        ty: current_ty,
    };
    if !collection_mode {
        validate_player_path_read(&typed, span, diagnostics);
    }
    typed
}

fn normalize_builder_path_segments(
    base_ty: &Type,
    segments: &[PathSegment],
    span: Span,
    diagnostics: &mut Diagnostics,
) -> Vec<PathSegment> {
    let Some(first) = segments.first() else {
        return Vec::new();
    };
    let PathSegment::Field(first_name) = first else {
        if matches!(
            base_ty,
            Type::EntityDef | Type::BlockDef | Type::ItemDef | Type::TextDef
        ) {
            diagnostics.push(Diagnostic::new(
                "builder path access must start with a field such as '.nbt', '.states', or '.count'",
                span,
            ));
        }
        return segments.to_vec();
    };
    let rewritten = match base_ty {
        Type::EntityDef => match first_name.as_str() {
            "name" => Some(vec!["nbt", "CustomName"]),
            "name_visible" => Some(vec!["nbt", "CustomNameVisible"]),
            "no_ai" => Some(vec!["nbt", "NoAI"]),
            "silent" => Some(vec!["nbt", "Silent"]),
            "glowing" => Some(vec!["nbt", "Glowing"]),
            "tags" => Some(vec!["nbt", "Tags"]),
            _ => None,
        },
        Type::BlockDef => match first_name.as_str() {
            "name" => Some(vec!["nbt", "CustomName"]),
            "lock" => Some(vec!["nbt", "Lock"]),
            "loot_table" => Some(vec!["nbt", "LootTable"]),
            "loot_seed" => Some(vec!["nbt", "LootTableSeed"]),
            _ => None,
        },
        Type::ItemDef => match first_name.as_str() {
            "name" => Some(vec!["nbt", "display", "Name"]),
            _ => None,
        },
        Type::TextDef => None,
        _ => None,
    };
    let Some(rewritten) = rewritten else {
        return segments.to_vec();
    };
    rewritten
        .into_iter()
        .map(|segment| PathSegment::Field(segment.to_string()))
        .chain(segments.iter().skip(1).cloned())
        .collect()
}

fn infer_collection_type<'a>(
    mut values: impl Iterator<Item = &'a Type>,
    mismatch: &str,
    empty: &str,
    span: Span,
    diagnostics: &mut Diagnostics,
) -> Type {
    let Some(first) = values.next().cloned() else {
        diagnostics.push(Diagnostic::new(empty, span));
        return Type::Nbt;
    };
    for value in values {
        if value != &first {
            diagnostics.push(Diagnostic::new(mismatch, span.clone()));
            break;
        }
    }
    first
}

fn validate_dict_key_literal(key: &str, span: Span, diagnostics: &mut Diagnostics) {
    if !is_storage_path_safe_key(key) {
        diagnostics.push(Diagnostic::new(
            format!(
                "dictionary key '{}' is not storage-path-safe; use letters, digits, and '_' with a non-digit first character",
                key
            ),
            span,
        ));
    }
}

fn is_storage_path_safe_key(key: &str) -> bool {
    let mut chars = key.chars();
    chars
        .next()
        .map(|ch| ch.is_ascii_alphabetic() || ch == '_')
        .unwrap_or(false)
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn validate_declared_type(
    ty: &Type,
    struct_defs: &BTreeMap<String, StructTypeDef>,
    span: Span,
    diagnostics: &mut Diagnostics,
) {
    match ty {
        Type::Array(element) => {
            validate_collection_value_type(element, span.clone(), diagnostics);
            validate_declared_type(element, struct_defs, span, diagnostics);
        }
        Type::Dict(value) => {
            validate_collection_value_type(value, span.clone(), diagnostics);
            validate_declared_type(value, struct_defs, span, diagnostics);
        }
        Type::Struct(name) if !struct_defs.contains_key(name) => {
            diagnostics.push(Diagnostic::new(format!("unknown struct '{}'", name), span))
        }
        _ => {}
    }
}

fn validate_collection_value_type(ty: &Type, span: Span, diagnostics: &mut Diagnostics) {
    if !matches!(
        ty,
        Type::Int
            | Type::Bool
            | Type::String
            | Type::Nbt
            | Type::Array(_)
            | Type::Dict(_)
            | Type::Struct(_)
            | Type::EntityDef
            | Type::BlockDef
            | Type::ItemDef
            | Type::TextDef
            | Type::ItemSlot
            | Type::Bossbar
    ) {
        diagnostics.push(Diagnostic::new(
            format!(
                "collection values may not have unsupported type '{}'",
                ty.as_str()
            ),
            span,
        ));
    }
}

fn is_storage_lvalue_expr(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Variable(_) => true,
        ExprKind::Path(path) => is_storage_lvalue_expr(&path.base),
        _ => false,
    }
}

fn is_storage_data_expr(expr: &TypedExpr) -> bool {
    match &expr.kind {
        TypedExprKind::Variable(_) => !matches!(
            expr.ty,
            Type::Int
                | Type::Bool
                | Type::EntitySet
                | Type::EntityRef
                | Type::PlayerRef
                | Type::BlockRef
        ),
        TypedExprKind::Path(path) => is_storage_data_expr(&path.base),
        _ => false,
    }
}

#[allow(unreachable_patterns)]
fn type_check_builtin_call(
    function: &str,
    args: &[Expr],
    expr: &Expr,
    struct_defs: &BTreeMap<String, StructTypeDef>,
    signatures: &BTreeMap<String, FunctionSignature>,
    env: &HashMap<String, Type>,
    ref_env: &HashMap<String, RefKind>,
    called_functions: &mut BTreeSet<String>,
    diagnostics: &mut Diagnostics,
) -> Option<TypedExpr> {
    match function {
        "entity" => Some(type_check_entity_constructor(
            args,
            expr,
            struct_defs,
            signatures,
            env,
            ref_env,
            called_functions,
            diagnostics,
        )),
        "block_type" => Some(type_check_block_type_constructor(
            args,
            expr,
            struct_defs,
            signatures,
            env,
            ref_env,
            called_functions,
            diagnostics,
        )),
        "item" => Some(type_check_item_constructor(
            args,
            expr,
            struct_defs,
            signatures,
            env,
            ref_env,
            called_functions,
            diagnostics,
        )),
        "text" => Some(type_check_text_constructor(
            args,
            expr,
            struct_defs,
            signatures,
            env,
            ref_env,
            called_functions,
            diagnostics,
        )),
        "summon" => Some(type_check_summon_builtin(
            args,
            expr,
            struct_defs,
            signatures,
            env,
            ref_env,
            called_functions,
            diagnostics,
        )),
        "sleep" | "sleep_ticks" => {
            let args = type_check_args(
                args,
                struct_defs,
                signatures,
                env,
                ref_env,
                called_functions,
                diagnostics,
            );
            diagnostics.push(Diagnostic::new(
                format!(
                    "{}(...) may only appear as a standalone statement",
                    function
                ),
                expr.span.clone(),
            ));
            Some(builtin_call_expr(function, args, Type::Void))
        }
        "random" => Some(type_check_random_builtin(
            args,
            expr,
            struct_defs,
            signatures,
            env,
            ref_env,
            called_functions,
            diagnostics,
        )),
        "bossbar" => Some(type_check_bossbar_constructor(
            args,
            expr,
            struct_defs,
            signatures,
            env,
            ref_env,
            called_functions,
            diagnostics,
        )),
        "teleport" | "damage" | "heal" | "give" | "clear" | "loot_give" | "loot_insert"
        | "loot_spawn" | "tellraw" | "title" | "actionbar" | "debug_marker" | "debug_entity"
        | "bossbar_add" | "bossbar_remove" | "bossbar_name" | "bossbar_value" | "bossbar_max"
        | "bossbar_visible" | "bossbar_players" | "playsound" | "stopsound" | "particle"
        | "setblock" | "fill" => {
            let args = type_check_args(
                args,
                struct_defs,
                signatures,
                env,
                ref_env,
                called_functions,
                diagnostics,
            );
            diagnostics.push(Diagnostic::new(
                removed_builtin_message(function),
                expr.span.clone(),
            ));
            Some(builtin_call_expr(function, args, Type::Void))
        }
        "teleport" => Some(type_check_gameplay_call(
            function,
            args,
            expr,
            struct_defs,
            signatures,
            env,
            ref_env,
            called_functions,
            diagnostics,
            GameplayBuiltinKind::Teleport,
        )),
        "damage" => Some(type_check_gameplay_call(
            function,
            args,
            expr,
            struct_defs,
            signatures,
            env,
            ref_env,
            called_functions,
            diagnostics,
            GameplayBuiltinKind::Damage,
        )),
        "heal" => Some(type_check_gameplay_call(
            function,
            args,
            expr,
            struct_defs,
            signatures,
            env,
            ref_env,
            called_functions,
            diagnostics,
            GameplayBuiltinKind::Heal,
        )),
        "give" => Some(type_check_gameplay_call(
            function,
            args,
            expr,
            struct_defs,
            signatures,
            env,
            ref_env,
            called_functions,
            diagnostics,
            GameplayBuiltinKind::Give,
        )),
        "clear" => Some(type_check_gameplay_call(
            function,
            args,
            expr,
            struct_defs,
            signatures,
            env,
            ref_env,
            called_functions,
            diagnostics,
            GameplayBuiltinKind::Clear,
        )),
        "loot_give" => Some(type_check_gameplay_call(
            function,
            args,
            expr,
            struct_defs,
            signatures,
            env,
            ref_env,
            called_functions,
            diagnostics,
            GameplayBuiltinKind::LootGive,
        )),
        "loot_insert" => Some(type_check_gameplay_call(
            function,
            args,
            expr,
            struct_defs,
            signatures,
            env,
            ref_env,
            called_functions,
            diagnostics,
            GameplayBuiltinKind::LootInsert,
        )),
        "loot_spawn" => Some(type_check_gameplay_call(
            function,
            args,
            expr,
            struct_defs,
            signatures,
            env,
            ref_env,
            called_functions,
            diagnostics,
            GameplayBuiltinKind::LootSpawn,
        )),
        "tellraw" => Some(type_check_gameplay_call(
            function,
            args,
            expr,
            struct_defs,
            signatures,
            env,
            ref_env,
            called_functions,
            diagnostics,
            GameplayBuiltinKind::Tellraw,
        )),
        "title" => Some(type_check_gameplay_call(
            function,
            args,
            expr,
            struct_defs,
            signatures,
            env,
            ref_env,
            called_functions,
            diagnostics,
            GameplayBuiltinKind::Title,
        )),
        "actionbar" => Some(type_check_gameplay_call(
            function,
            args,
            expr,
            struct_defs,
            signatures,
            env,
            ref_env,
            called_functions,
            diagnostics,
            GameplayBuiltinKind::Actionbar,
        )),
        "debug" => Some(type_check_gameplay_call(
            function,
            args,
            expr,
            struct_defs,
            signatures,
            env,
            ref_env,
            called_functions,
            diagnostics,
            GameplayBuiltinKind::Debug,
        )),
        "debug_marker" => Some(type_check_gameplay_call(
            function,
            args,
            expr,
            struct_defs,
            signatures,
            env,
            ref_env,
            called_functions,
            diagnostics,
            GameplayBuiltinKind::DebugMarker,
        )),
        "debug_entity" => Some(type_check_gameplay_call(
            function,
            args,
            expr,
            struct_defs,
            signatures,
            env,
            ref_env,
            called_functions,
            diagnostics,
            GameplayBuiltinKind::DebugEntity,
        )),
        "bossbar_add" => Some(type_check_gameplay_call(
            function,
            args,
            expr,
            struct_defs,
            signatures,
            env,
            ref_env,
            called_functions,
            diagnostics,
            GameplayBuiltinKind::BossbarAdd,
        )),
        "bossbar_remove" => Some(type_check_gameplay_call(
            function,
            args,
            expr,
            struct_defs,
            signatures,
            env,
            ref_env,
            called_functions,
            diagnostics,
            GameplayBuiltinKind::BossbarRemove,
        )),
        "bossbar_name" => Some(type_check_gameplay_call(
            function,
            args,
            expr,
            struct_defs,
            signatures,
            env,
            ref_env,
            called_functions,
            diagnostics,
            GameplayBuiltinKind::BossbarName,
        )),
        "bossbar_value" => Some(type_check_gameplay_call(
            function,
            args,
            expr,
            struct_defs,
            signatures,
            env,
            ref_env,
            called_functions,
            diagnostics,
            GameplayBuiltinKind::BossbarValue,
        )),
        "bossbar_max" => Some(type_check_gameplay_call(
            function,
            args,
            expr,
            struct_defs,
            signatures,
            env,
            ref_env,
            called_functions,
            diagnostics,
            GameplayBuiltinKind::BossbarMax,
        )),
        "bossbar_visible" => Some(type_check_gameplay_call(
            function,
            args,
            expr,
            struct_defs,
            signatures,
            env,
            ref_env,
            called_functions,
            diagnostics,
            GameplayBuiltinKind::BossbarVisible,
        )),
        "bossbar_players" => Some(type_check_gameplay_call(
            function,
            args,
            expr,
            struct_defs,
            signatures,
            env,
            ref_env,
            called_functions,
            diagnostics,
            GameplayBuiltinKind::BossbarPlayers,
        )),
        "playsound" => Some(type_check_gameplay_call(
            function,
            args,
            expr,
            struct_defs,
            signatures,
            env,
            ref_env,
            called_functions,
            diagnostics,
            GameplayBuiltinKind::Playsound,
        )),
        "stopsound" => Some(type_check_gameplay_call(
            function,
            args,
            expr,
            struct_defs,
            signatures,
            env,
            ref_env,
            called_functions,
            diagnostics,
            GameplayBuiltinKind::Stopsound,
        )),
        "particle" => Some(type_check_gameplay_call(
            function,
            args,
            expr,
            struct_defs,
            signatures,
            env,
            ref_env,
            called_functions,
            diagnostics,
            GameplayBuiltinKind::Particle,
        )),
        "setblock" => Some(type_check_gameplay_call(
            function,
            args,
            expr,
            struct_defs,
            signatures,
            env,
            ref_env,
            called_functions,
            diagnostics,
            GameplayBuiltinKind::Setblock,
        )),
        "fill" => Some(type_check_gameplay_call(
            function,
            args,
            expr,
            struct_defs,
            signatures,
            env,
            ref_env,
            called_functions,
            diagnostics,
            GameplayBuiltinKind::Fill,
        )),
        "selector" => {
            let args = type_check_args(
                args,
                struct_defs,
                signatures,
                env,
                ref_env,
                called_functions,
                diagnostics,
            );
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
            let args = type_check_args(
                args,
                struct_defs,
                signatures,
                env,
                ref_env,
                called_functions,
                diagnostics,
            );
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
            let args = type_check_args(
                args,
                struct_defs,
                signatures,
                env,
                ref_env,
                called_functions,
                diagnostics,
            );
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
        "player_ref" => {
            let args = type_check_args(
                args,
                struct_defs,
                signatures,
                env,
                ref_env,
                called_functions,
                diagnostics,
            );
            expect_arity(function, &args, 1, expr, diagnostics);
            let mut arg = args.into_iter().next().unwrap_or(TypedExpr {
                kind: TypedExprKind::Variable("_error".to_string()),
                ty: Type::EntityRef,
                ref_kind: RefKind::Unknown,
            });
            if !is_entity_ref_type(&arg.ty) {
                diagnostics.push(Diagnostic::new(
                    "player_ref(...) requires an 'entity_ref' argument",
                    expr.span.clone(),
                ));
            }
            if arg.ref_kind == RefKind::NonPlayer {
                diagnostics.push(Diagnostic::new(
                    "player_ref(...) cannot assert a known non-player entity",
                    expr.span.clone(),
                ));
            }
            arg.ty = Type::PlayerRef;
            arg.ref_kind = RefKind::Player;
            Some(arg)
        }
        "exists" => {
            let args = type_check_args(
                args,
                struct_defs,
                signatures,
                env,
                ref_env,
                called_functions,
                diagnostics,
            );
            expect_arity(function, &args, 1, expr, diagnostics);
            let arg = args.into_iter().next().unwrap_or(TypedExpr {
                kind: TypedExprKind::Variable("_error".to_string()),
                ty: Type::EntityRef,
                ref_kind: RefKind::Unknown,
            });
            if !is_entity_ref_type(&arg.ty) {
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
        "has_data" => {
            let args = type_check_args(
                args,
                struct_defs,
                signatures,
                env,
                ref_env,
                called_functions,
                diagnostics,
            );
            expect_arity(function, &args, 1, expr, diagnostics);
            let arg = args
                .into_iter()
                .next()
                .map(coerce_expr_to_nbt)
                .unwrap_or(TypedExpr {
                    kind: TypedExprKind::Variable("_error".to_string()),
                    ty: Type::Nbt,
                    ref_kind: RefKind::Unknown,
                });
            if !is_storage_data_expr(&arg) {
                diagnostics.push(Diagnostic::new(
                    "has_data(...) requires a storage-backed variable or path",
                    expr.span.clone(),
                ));
            }
            Some(TypedExpr {
                kind: TypedExprKind::HasData(Box::new(arg)),
                ty: Type::Bool,
                ref_kind: RefKind::Unknown,
            })
        }
        "at" => {
            let args = type_check_args(
                args,
                struct_defs,
                signatures,
                env,
                ref_env,
                called_functions,
                diagnostics,
            );
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
            if !is_entity_ref_type(&anchor.ty) {
                diagnostics.push(Diagnostic::new(
                    "at(...) requires an 'entity_ref' anchor",
                    expr.span.clone(),
                ));
            }
            if !matches!(
                value.ty,
                Type::EntitySet | Type::EntityRef | Type::PlayerRef | Type::BlockRef
            ) {
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
        "as" => {
            let args = type_check_args(
                args,
                struct_defs,
                signatures,
                env,
                ref_env,
                called_functions,
                diagnostics,
            );
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
            if !matches!(
                anchor.ty,
                Type::EntitySet | Type::EntityRef | Type::PlayerRef
            ) {
                diagnostics.push(Diagnostic::new(
                    "as(...) requires an 'entity_set' or 'entity_ref' anchor",
                    expr.span.clone(),
                ));
            }
            if !matches!(
                value.ty,
                Type::EntitySet | Type::EntityRef | Type::PlayerRef | Type::BlockRef
            ) {
                diagnostics.push(Diagnostic::new(
                    "as(...) requires an 'entity_set', 'entity_ref', or 'block_ref' value",
                    expr.span.clone(),
                ));
            }
            Some(TypedExpr {
                kind: TypedExprKind::As {
                    anchor: Box::new(anchor),
                    value: Box::new(value.clone()),
                },
                ty: value.ty,
                ref_kind: value.ref_kind,
            })
        }
        "int" | "bool" | "string" => {
            let args = type_check_args(
                args,
                struct_defs,
                signatures,
                env,
                ref_env,
                called_functions,
                diagnostics,
            );
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
    struct_defs: &BTreeMap<String, StructTypeDef>,
    signatures: &BTreeMap<String, FunctionSignature>,
    env: &HashMap<String, Type>,
    ref_env: &HashMap<String, RefKind>,
    called_functions: &mut BTreeSet<String>,
    diagnostics: &mut Diagnostics,
) -> Option<TypedExpr> {
    let receiver_expr = receiver;
    let receiver = type_check_expr(
        receiver_expr,
        struct_defs,
        signatures,
        env,
        ref_env,
        called_functions,
        diagnostics,
    );
    let mut args = type_check_args(
        args,
        struct_defs,
        signatures,
        env,
        ref_env,
        called_functions,
        diagnostics,
    );
    match method {
        "as_nbt" => {
            expect_arity(method, &args, 0, expr, diagnostics);
            if !matches!(
                receiver.ty,
                Type::EntityDef | Type::BlockDef | Type::ItemDef
            ) {
                diagnostics.push(Diagnostic::new(
                    "as_nbt() requires an 'entity_def', 'block_def', or 'item_def' receiver",
                    expr.span.clone(),
                ));
            }
            Some(method_call_expr(receiver, method, args, Type::Nbt))
        }
        "len" => {
            expect_arity(method, &args, 0, expr, diagnostics);
            if !matches!(receiver.ty, Type::Array(_)) {
                diagnostics.push(Diagnostic::new(
                    "len() requires an 'array' receiver",
                    expr.span.clone(),
                ));
            }
            Some(TypedExpr {
                kind: TypedExprKind::MethodCall {
                    receiver: Box::new(receiver),
                    method: method.to_string(),
                    args,
                },
                ty: Type::Int,
                ref_kind: RefKind::Unknown,
            })
        }
        "push" => {
            expect_arity(method, &args, 1, expr, diagnostics);
            if !is_storage_lvalue_expr(receiver_expr) {
                diagnostics.push(Diagnostic::new(
                    "push(...) requires a variable or collection element receiver",
                    expr.span.clone(),
                ));
            }
            let expected = match &receiver.ty {
                Type::Array(element) => Some(element.as_ref()),
                _ => {
                    diagnostics.push(Diagnostic::new(
                        "push(...) requires an 'array' receiver",
                        expr.span.clone(),
                    ));
                    None
                }
            };
            if let (Some(expected), Some(arg)) = (expected, args.first_mut()) {
                if *expected == Type::Nbt {
                    *arg = coerce_expr_to_nbt(arg.clone());
                }
            }
            if let (Some(expected), Some(arg)) = (expected, args.first()) {
                if &arg.ty != expected {
                    diagnostics.push(Diagnostic::new(
                        format!(
                            "push(...) value must be '{}', found '{}'",
                            expected.as_str(),
                            arg.ty.as_str()
                        ),
                        expr.span.clone(),
                    ));
                }
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
        "pop" => {
            expect_arity(method, &args, 0, expr, diagnostics);
            if !is_storage_lvalue_expr(receiver_expr) {
                diagnostics.push(Diagnostic::new(
                    "pop() requires a variable or collection element receiver",
                    expr.span.clone(),
                ));
            }
            let ty = match &receiver.ty {
                Type::Array(element) => *element.clone(),
                _ => {
                    diagnostics.push(Diagnostic::new(
                        "pop() requires an 'array' receiver",
                        expr.span.clone(),
                    ));
                    Type::Nbt
                }
            };
            Some(TypedExpr {
                kind: TypedExprKind::MethodCall {
                    receiver: Box::new(receiver),
                    method: method.to_string(),
                    args,
                },
                ty,
                ref_kind: RefKind::Unknown,
            })
        }
        "remove_at" => {
            expect_arity(method, &args, 1, expr, diagnostics);
            if !is_storage_lvalue_expr(receiver_expr) {
                diagnostics.push(Diagnostic::new(
                    "remove_at(...) requires a variable or collection element receiver",
                    expr.span.clone(),
                ));
            }
            let ty = match &receiver.ty {
                Type::Array(element) => *element.clone(),
                _ => {
                    diagnostics.push(Diagnostic::new(
                        "remove_at(...) requires an 'array' receiver",
                        expr.span.clone(),
                    ));
                    Type::Nbt
                }
            };
            if args.first().map(|arg| &arg.ty) != Some(&Type::Int) {
                diagnostics.push(Diagnostic::new(
                    "remove_at(...) index must be 'int'",
                    expr.span.clone(),
                ));
            }
            Some(TypedExpr {
                kind: TypedExprKind::MethodCall {
                    receiver: Box::new(receiver),
                    method: method.to_string(),
                    args,
                },
                ty,
                ref_kind: RefKind::Unknown,
            })
        }
        "has" => {
            expect_arity(method, &args, 1, expr, diagnostics);
            if !matches!(receiver.ty, Type::Dict(_)) {
                diagnostics.push(Diagnostic::new(
                    "has(...) requires a 'dict' receiver",
                    expr.span.clone(),
                ));
            }
            if args.first().map(|arg| &arg.ty) != Some(&Type::String) {
                diagnostics.push(Diagnostic::new(
                    "has(...) key must be 'string'",
                    expr.span.clone(),
                ));
            }
            Some(TypedExpr {
                kind: TypedExprKind::MethodCall {
                    receiver: Box::new(receiver),
                    method: method.to_string(),
                    args,
                },
                ty: Type::Bool,
                ref_kind: RefKind::Unknown,
            })
        }
        "remove" => {
            if receiver.ty == Type::Bossbar {
                expect_arity(method, &args, 0, expr, diagnostics);
                return Some(TypedExpr {
                    kind: TypedExprKind::MethodCall {
                        receiver: Box::new(receiver),
                        method: method.to_string(),
                        args,
                    },
                    ty: Type::Void,
                    ref_kind: RefKind::Unknown,
                });
            }
            expect_arity(method, &args, 1, expr, diagnostics);
            if !is_storage_lvalue_expr(receiver_expr) {
                diagnostics.push(Diagnostic::new(
                    "remove(...) requires a variable or collection element receiver",
                    expr.span.clone(),
                ));
            }
            match receiver.ty.clone() {
                Type::Array(element) => {
                    if args.first().map(|arg| &arg.ty) != Some(&Type::Int) {
                        diagnostics.push(Diagnostic::new(
                            "remove(...) index must be 'int'",
                            expr.span.clone(),
                        ));
                    }
                    Some(TypedExpr {
                        kind: TypedExprKind::MethodCall {
                            receiver: Box::new(receiver),
                            method: method.to_string(),
                            args,
                        },
                        ty: *element,
                        ref_kind: RefKind::Unknown,
                    })
                }
                Type::Dict(_) => {
                    if args.first().map(|arg| &arg.ty) != Some(&Type::String) {
                        diagnostics.push(Diagnostic::new(
                            "remove(...) key must be 'string'",
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
                _ => {
                    diagnostics.push(Diagnostic::new(
                        "remove(...) requires an 'array', 'dict', or 'bossbar' receiver",
                        expr.span.clone(),
                    ));
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
            }
        }
        "teleport" => {
            expect_entity_receiver(method, &receiver, expr, diagnostics);
            expect_arity(method, &args, 1, expr, diagnostics);
            expect_arg_matches(
                method,
                &args,
                0,
                |ty| matches!(ty, Type::EntityRef | Type::PlayerRef | Type::BlockRef),
                "an 'entity_ref' or 'block_ref'",
                "destination",
                expr,
                diagnostics,
            );
            Some(method_call_expr(receiver, method, args, Type::Void))
        }
        "damage" => {
            expect_entity_receiver(method, &receiver, expr, diagnostics);
            expect_arity(method, &args, 1, expr, diagnostics);
            expect_arg_type(method, &args, 0, Type::Int, "amount", expr, diagnostics);
            Some(method_call_expr(receiver, method, args, Type::Void))
        }
        "heal" => {
            if !is_entity_ref_type(&receiver.ty) {
                diagnostics.push(Diagnostic::new(
                    "heal(...) requires an 'entity_ref' receiver",
                    expr.span.clone(),
                ));
            }
            match receiver.ref_kind {
                RefKind::Player => diagnostics.push(Diagnostic::new(
                    "heal(...) only supports known non-player 'entity_ref' receivers in v1",
                    expr.span.clone(),
                )),
                RefKind::Unknown => diagnostics.push(Diagnostic::new(
                    "heal(...) rejects ambiguous 'entity_ref' receivers in v1",
                    expr.span.clone(),
                )),
                RefKind::NonPlayer => {}
            }
            expect_arity(method, &args, 1, expr, diagnostics);
            expect_arg_type(method, &args, 0, Type::Int, "amount", expr, diagnostics);
            Some(method_call_expr(receiver, method, args, Type::Void))
        }
        "clear" if receiver.ty == Type::ItemSlot => {
            expect_arity(method, &args, 0, expr, diagnostics);
            Some(method_call_expr(receiver, method, args, Type::Void))
        }
        "give" => {
            expect_entity_receiver(method, &receiver, expr, diagnostics);
            if args.len() == 1 {
                expect_arg_type(method, &args, 0, Type::ItemDef, "stack", expr, diagnostics);
            } else {
                expect_arity(method, &args, 2, expr, diagnostics);
                expect_arg_type(method, &args, 0, Type::String, "item id", expr, diagnostics);
                expect_arg_type(method, &args, 1, Type::Int, "count", expr, diagnostics);
            }
            Some(method_call_expr(receiver, method, args, Type::Void))
        }
        "clear" => {
            expect_entity_receiver(method, &receiver, expr, diagnostics);
            expect_arity(method, &args, 2, expr, diagnostics);
            expect_arg_type(method, &args, 0, Type::String, "item id", expr, diagnostics);
            expect_arg_type(method, &args, 1, Type::Int, "count", expr, diagnostics);
            Some(method_call_expr(receiver, method, args, Type::Void))
        }
        "loot_give" => {
            expect_entity_receiver(method, &receiver, expr, diagnostics);
            expect_arity(method, &args, 1, expr, diagnostics);
            expect_arg_type(
                method,
                &args,
                0,
                Type::String,
                "loot table",
                expr,
                diagnostics,
            );
            Some(method_call_expr(receiver, method, args, Type::Void))
        }
        "tellraw" | "title" | "actionbar" => {
            expect_entity_receiver(method, &receiver, expr, diagnostics);
            expect_arity(method, &args, 1, expr, diagnostics);
            expect_arg_matches(
                method,
                &args,
                0,
                |ty| matches!(ty, Type::String | Type::TextDef),
                "'string' or 'text_def'",
                "message",
                expr,
                diagnostics,
            );
            Some(method_call_expr(receiver, method, args, Type::Void))
        }
        "playsound" => {
            expect_entity_receiver(method, &receiver, expr, diagnostics);
            expect_arity(method, &args, 2, expr, diagnostics);
            expect_arg_type(
                method,
                &args,
                0,
                Type::String,
                "sound id",
                expr,
                diagnostics,
            );
            expect_arg_type(
                method,
                &args,
                1,
                Type::String,
                "category",
                expr,
                diagnostics,
            );
            Some(method_call_expr(receiver, method, args, Type::Void))
        }
        "stopsound" => {
            expect_entity_receiver(method, &receiver, expr, diagnostics);
            expect_arity(method, &args, 2, expr, diagnostics);
            expect_arg_type(
                method,
                &args,
                0,
                Type::String,
                "category",
                expr,
                diagnostics,
            );
            expect_arg_type(
                method,
                &args,
                1,
                Type::String,
                "sound id",
                expr,
                diagnostics,
            );
            Some(method_call_expr(receiver, method, args, Type::Void))
        }
        "debug_entity" => {
            expect_entity_receiver(method, &receiver, expr, diagnostics);
            expect_arity(method, &args, 1, expr, diagnostics);
            expect_arg_type(method, &args, 0, Type::String, "label", expr, diagnostics);
            Some(method_call_expr(receiver, method, args, Type::Void))
        }
        "loot_insert" | "loot_spawn" | "setblock" => {
            expect_block_receiver(method, &receiver, expr, diagnostics);
            expect_arity(method, &args, 1, expr, diagnostics);
            if method == "setblock" {
                expect_arg_matches(
                    method,
                    &args,
                    0,
                    |ty| matches!(ty, Type::String | Type::BlockDef),
                    "'string' or 'block_def'",
                    "block",
                    expr,
                    diagnostics,
                );
            } else {
                expect_arg_type(
                    method,
                    &args,
                    0,
                    Type::String,
                    "loot table",
                    expr,
                    diagnostics,
                );
            }
            Some(method_call_expr(receiver, method, args, Type::Void))
        }
        "is" => {
            expect_block_receiver(method, &receiver, expr, diagnostics);
            expect_arity(method, &args, 1, expr, diagnostics);
            expect_arg_type(
                method,
                &args,
                0,
                Type::String,
                "block id",
                expr,
                diagnostics,
            );
            Some(method_call_expr(receiver, method, args, Type::Bool))
        }
        "summon" => {
            expect_block_receiver(method, &receiver, expr, diagnostics);
            if !(args.len() == 1 || args.len() == 2) {
                diagnostics.push(Diagnostic::new(
                    format!(
                        "wrong arity for '{}': expected 1 or 2, found {}",
                        method,
                        args.len()
                    ),
                    expr.span.clone(),
                ));
            }
            match args.first().map(|arg| &arg.ty) {
                Some(Type::String | Type::EntityDef) => {}
                _ => diagnostics.push(Diagnostic::new(
                    "block.summon(...) entity id must be 'string' or 'entity_def'",
                    expr.span.clone(),
                )),
            }
            if args.len() == 2 && args.first().map(|arg| &arg.ty) != Some(&Type::String) {
                diagnostics.push(Diagnostic::new(
                    "block.summon(entity_id, data) requires a 'string' entity id",
                    expr.span.clone(),
                ));
            }
            if let Some(arg) = args.get_mut(1) {
                *arg = coerce_expr_to_nbt(arg.clone());
            }
            if args.len() >= 2 && args.get(1).map(|arg| &arg.ty) != Some(&Type::Nbt) {
                diagnostics.push(Diagnostic::new(
                    "block.summon(..., data) requires 'nbt' summon data",
                    expr.span.clone(),
                ));
            }
            Some(method_call_expr(receiver, method, args, Type::EntityRef))
        }
        "spawn_item" => {
            expect_block_receiver(method, &receiver, expr, diagnostics);
            expect_arity(method, &args, 1, expr, diagnostics);
            expect_arg_type(method, &args, 0, Type::ItemDef, "stack", expr, diagnostics);
            Some(method_call_expr(receiver, method, args, Type::EntityRef))
        }
        "fill" => {
            expect_block_receiver(method, &receiver, expr, diagnostics);
            expect_arity(method, &args, 2, expr, diagnostics);
            expect_arg_type(method, &args, 0, Type::BlockRef, "to", expr, diagnostics);
            expect_arg_matches(
                method,
                &args,
                1,
                |ty| matches!(ty, Type::String | Type::BlockDef),
                "'string' or 'block_def'",
                "block",
                expr,
                diagnostics,
            );
            Some(method_call_expr(receiver, method, args, Type::Void))
        }
        "debug_marker" => {
            expect_block_receiver(method, &receiver, expr, diagnostics);
            if !(args.len() == 1 || args.len() == 2) {
                diagnostics.push(Diagnostic::new(
                    format!(
                        "wrong arity for '{}': expected 1 or 2, found {}",
                        method,
                        args.len()
                    ),
                    expr.span.clone(),
                ));
            }
            expect_arg_type(method, &args, 0, Type::String, "label", expr, diagnostics);
            if args.len() >= 2 {
                expect_arg_type(
                    method,
                    &args,
                    1,
                    Type::String,
                    "marker block id",
                    expr,
                    diagnostics,
                );
            }
            Some(method_call_expr(receiver, method, args, Type::Void))
        }
        "particle" => {
            expect_block_receiver(method, &receiver, expr, diagnostics);
            if !(args.len() == 1 || args.len() == 2 || args.len() == 3) {
                diagnostics.push(Diagnostic::new(
                    format!(
                        "wrong arity for '{}': expected 1, 2, or 3, found {}",
                        method,
                        args.len()
                    ),
                    expr.span.clone(),
                ));
            }
            expect_arg_type(
                method,
                &args,
                0,
                Type::String,
                "particle id",
                expr,
                diagnostics,
            );
            if args.len() >= 2 {
                expect_arg_type(method, &args, 1, Type::Int, "count", expr, diagnostics);
            }
            if args.len() >= 3 {
                expect_entity_target_arg(method, &args, 2, expr, diagnostics);
            }
            Some(method_call_expr(receiver, method, args, Type::Void))
        }
        "add_tag" | "remove_tag" | "has_tag" => {
            if !is_entity_ref_type(&receiver.ty) {
                diagnostics.push(Diagnostic::new(
                    format!(
                        "{}.{}(...) requires an 'entity_ref' receiver",
                        "entity", method
                    ),
                    expr.span.clone(),
                ));
            }
            expect_arity(method, &args, 1, expr, diagnostics);
            if args.first().map(|arg| &arg.ty) != Some(&Type::String) {
                diagnostics.push(Diagnostic::new(
                    format!("{}(...) tag name must be 'string'", method),
                    expr.span.clone(),
                ));
            }
            Some(TypedExpr {
                kind: TypedExprKind::MethodCall {
                    receiver: Box::new(receiver),
                    method: method.to_string(),
                    args,
                },
                ty: if method == "has_tag" {
                    Type::Bool
                } else {
                    Type::Void
                },
                ref_kind: RefKind::Unknown,
            })
        }
        "effect" => {
            if !is_entity_ref_type(&receiver.ty) {
                diagnostics.push(Diagnostic::new(
                    "effect(...) requires an 'entity_ref' receiver",
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

#[derive(Clone, Copy)]
enum GameplayBuiltinKind {
    Teleport,
    Damage,
    Heal,
    Give,
    Clear,
    LootGive,
    LootInsert,
    LootSpawn,
    Tellraw,
    Title,
    Actionbar,
    Debug,
    DebugMarker,
    DebugEntity,
    BossbarAdd,
    BossbarRemove,
    BossbarName,
    BossbarValue,
    BossbarMax,
    BossbarVisible,
    BossbarPlayers,
    Playsound,
    Stopsound,
    Particle,
    Setblock,
    Fill,
}

fn type_check_summon_builtin(
    args: &[Expr],
    expr: &Expr,
    struct_defs: &BTreeMap<String, StructTypeDef>,
    signatures: &BTreeMap<String, FunctionSignature>,
    env: &HashMap<String, Type>,
    ref_env: &HashMap<String, RefKind>,
    called_functions: &mut BTreeSet<String>,
    diagnostics: &mut Diagnostics,
) -> TypedExpr {
    let mut args = type_check_args(
        args,
        struct_defs,
        signatures,
        env,
        ref_env,
        called_functions,
        diagnostics,
    );
    if let Some(arg) = args.get_mut(1) {
        *arg = coerce_expr_to_nbt(arg.clone());
    }
    if !(args.len() == 1 || args.len() == 2) {
        diagnostics.push(Diagnostic::new(
            format!(
                "wrong arity for 'summon': expected 1 or 2, found {}",
                args.len()
            ),
            expr.span.clone(),
        ));
    }
    match args.first().map(|arg| &arg.ty) {
        Some(Type::String | Type::EntityDef) => {}
        _ => diagnostics.push(Diagnostic::new(
            "summon(...) entity id must be 'string' or 'entity_def'",
            expr.span.clone(),
        )),
    }
    if args.len() == 2 && args.first().map(|arg| &arg.ty) != Some(&Type::String) {
        diagnostics.push(Diagnostic::new(
            "summon(entity_id, data) requires a 'string' entity id",
            expr.span.clone(),
        ));
    }
    if args.len() >= 2 && args.get(1).map(|arg| &arg.ty) != Some(&Type::Nbt) {
        diagnostics.push(Diagnostic::new(
            "summon(..., data) requires 'nbt' summon data",
            expr.span.clone(),
        ));
    }
    TypedExpr {
        kind: TypedExprKind::Call {
            function: "summon".to_string(),
            args,
        },
        ty: Type::EntityRef,
        ref_kind: RefKind::NonPlayer,
    }
}

fn type_check_entity_constructor(
    args: &[Expr],
    expr: &Expr,
    struct_defs: &BTreeMap<String, StructTypeDef>,
    signatures: &BTreeMap<String, FunctionSignature>,
    env: &HashMap<String, Type>,
    ref_env: &HashMap<String, RefKind>,
    called_functions: &mut BTreeSet<String>,
    diagnostics: &mut Diagnostics,
) -> TypedExpr {
    let args = type_check_args(
        args,
        struct_defs,
        signatures,
        env,
        ref_env,
        called_functions,
        diagnostics,
    );
    expect_arity("entity", &args, 1, expr, diagnostics);
    expect_arg_type("entity", &args, 0, Type::String, "id", expr, diagnostics);
    builtin_call_expr("entity", args, Type::EntityDef)
}

fn type_check_block_type_constructor(
    args: &[Expr],
    expr: &Expr,
    struct_defs: &BTreeMap<String, StructTypeDef>,
    signatures: &BTreeMap<String, FunctionSignature>,
    env: &HashMap<String, Type>,
    ref_env: &HashMap<String, RefKind>,
    called_functions: &mut BTreeSet<String>,
    diagnostics: &mut Diagnostics,
) -> TypedExpr {
    let args = type_check_args(
        args,
        struct_defs,
        signatures,
        env,
        ref_env,
        called_functions,
        diagnostics,
    );
    expect_arity("block_type", &args, 1, expr, diagnostics);
    expect_arg_type(
        "block_type",
        &args,
        0,
        Type::String,
        "id",
        expr,
        diagnostics,
    );
    builtin_call_expr("block_type", args, Type::BlockDef)
}

fn type_check_item_constructor(
    args: &[Expr],
    expr: &Expr,
    struct_defs: &BTreeMap<String, StructTypeDef>,
    signatures: &BTreeMap<String, FunctionSignature>,
    env: &HashMap<String, Type>,
    ref_env: &HashMap<String, RefKind>,
    called_functions: &mut BTreeSet<String>,
    diagnostics: &mut Diagnostics,
) -> TypedExpr {
    let args = type_check_args(
        args,
        struct_defs,
        signatures,
        env,
        ref_env,
        called_functions,
        diagnostics,
    );
    expect_arity("item", &args, 1, expr, diagnostics);
    expect_arg_type("item", &args, 0, Type::String, "id", expr, diagnostics);
    builtin_call_expr("item", args, Type::ItemDef)
}

fn type_check_text_constructor(
    args: &[Expr],
    expr: &Expr,
    struct_defs: &BTreeMap<String, StructTypeDef>,
    signatures: &BTreeMap<String, FunctionSignature>,
    env: &HashMap<String, Type>,
    ref_env: &HashMap<String, RefKind>,
    called_functions: &mut BTreeSet<String>,
    diagnostics: &mut Diagnostics,
) -> TypedExpr {
    let args = type_check_args(
        args,
        struct_defs,
        signatures,
        env,
        ref_env,
        called_functions,
        diagnostics,
    );
    if !(args.len() == 0 || args.len() == 1) {
        diagnostics.push(Diagnostic::new(
            format!(
                "wrong arity for 'text': expected 0 or 1, found {}",
                args.len()
            ),
            expr.span.clone(),
        ));
    }
    if let Some(arg) = args.first() {
        if arg.ty != Type::String {
            diagnostics.push(Diagnostic::new(
                format!(
                    "argument 1 for 'text' must be 'string', found '{}'",
                    arg.ty.as_str()
                ),
                expr.span.clone(),
            ));
        }
    }
    builtin_call_expr("text", args, Type::TextDef)
}

fn type_check_random_builtin(
    args: &[Expr],
    expr: &Expr,
    struct_defs: &BTreeMap<String, StructTypeDef>,
    signatures: &BTreeMap<String, FunctionSignature>,
    env: &HashMap<String, Type>,
    ref_env: &HashMap<String, RefKind>,
    called_functions: &mut BTreeSet<String>,
    diagnostics: &mut Diagnostics,
) -> TypedExpr {
    let args = type_check_args(
        args,
        struct_defs,
        signatures,
        env,
        ref_env,
        called_functions,
        diagnostics,
    );
    if args.len() > 2 {
        diagnostics.push(Diagnostic::new(
            format!(
                "wrong arity for 'random': expected 0, 1, or 2, found {}",
                args.len()
            ),
            expr.span.clone(),
        ));
    }
    for (index, arg) in args.iter().enumerate() {
        if arg.ty != Type::Int {
            diagnostics.push(Diagnostic::new(
                format!(
                    "argument {} for 'random' must be 'int', found '{}'",
                    index + 1,
                    arg.ty.as_str()
                ),
                expr.span.clone(),
            ));
        }
    }
    builtin_call_expr("random", args, Type::Int)
}

fn type_check_bossbar_constructor(
    args: &[Expr],
    expr: &Expr,
    struct_defs: &BTreeMap<String, StructTypeDef>,
    signatures: &BTreeMap<String, FunctionSignature>,
    env: &HashMap<String, Type>,
    ref_env: &HashMap<String, RefKind>,
    called_functions: &mut BTreeSet<String>,
    diagnostics: &mut Diagnostics,
) -> TypedExpr {
    let args = type_check_args(
        args,
        struct_defs,
        signatures,
        env,
        ref_env,
        called_functions,
        diagnostics,
    );
    expect_arity("bossbar", &args, 2, expr, diagnostics);
    expect_arg_type("bossbar", &args, 0, Type::String, "id", expr, diagnostics);
    expect_arg_matches(
        "bossbar",
        &args,
        1,
        |ty| matches!(ty, Type::String | Type::TextDef),
        "'string' or 'text_def'",
        "name",
        expr,
        diagnostics,
    );
    builtin_call_expr("bossbar", args, Type::Bossbar)
}

fn removed_builtin_message(function: &str) -> String {
    let replacement = match function {
        "teleport" => "target.teleport(destination)",
        "damage" => "target.damage(amount)",
        "heal" => "target.heal(amount)",
        "give" => "target.give(item_id, count)",
        "clear" => "target.clear(item_id, count)",
        "loot_give" => "target.loot_give(table)",
        "loot_insert" => "position.loot_insert(table)",
        "loot_spawn" => "position.loot_spawn(table)",
        "tellraw" => "target.tellraw(message)",
        "title" => "target.title(message)",
        "actionbar" => "target.actionbar(message)",
        "debug_marker" => "position.debug_marker(label)",
        "debug_entity" => "target.debug_entity(label)",
        "bossbar_add" => "let bb = bossbar(id, name)",
        "bossbar_remove" => "bb.remove()",
        "bossbar_name" => "bb.name = name",
        "bossbar_value" => "bb.value = value",
        "bossbar_max" => "bb.max = max",
        "bossbar_visible" => "bb.visible = visible",
        "bossbar_players" => "bb.players = targets",
        "playsound" => "target.playsound(sound, category)",
        "stopsound" => "target.stopsound(category, sound)",
        "particle" => "position.particle(name, count?, viewers?)",
        "setblock" => "position.setblock(block_id)",
        "fill" => "from.fill(to, block_id)",
        _ => "the method/property-style API",
    };
    format!(
        "{}(...) has been replaced by object-style syntax; use {}",
        function, replacement
    )
}

fn type_check_gameplay_call(
    function: &str,
    args: &[Expr],
    expr: &Expr,
    struct_defs: &BTreeMap<String, StructTypeDef>,
    signatures: &BTreeMap<String, FunctionSignature>,
    env: &HashMap<String, Type>,
    ref_env: &HashMap<String, RefKind>,
    called_functions: &mut BTreeSet<String>,
    diagnostics: &mut Diagnostics,
    kind: GameplayBuiltinKind,
) -> TypedExpr {
    let args = type_check_args(
        args,
        struct_defs,
        signatures,
        env,
        ref_env,
        called_functions,
        diagnostics,
    );
    match kind {
        GameplayBuiltinKind::Teleport => {
            expect_arity(function, &args, 2, expr, diagnostics);
            expect_entity_target_arg(function, &args, 0, expr, diagnostics);
            expect_arg_matches(
                function,
                &args,
                1,
                |ty| matches!(ty, Type::EntityRef | Type::PlayerRef | Type::BlockRef),
                "an 'entity_ref' or 'block_ref'",
                "destination",
                expr,
                diagnostics,
            );
            builtin_call_expr(function, args, Type::Void)
        }
        GameplayBuiltinKind::Damage => {
            expect_arity(function, &args, 2, expr, diagnostics);
            expect_entity_target_arg(function, &args, 0, expr, diagnostics);
            expect_arg_type(function, &args, 1, Type::Int, "amount", expr, diagnostics);
            builtin_call_expr(function, args, Type::Void)
        }
        GameplayBuiltinKind::Heal => {
            expect_arity(function, &args, 2, expr, diagnostics);
            expect_arg_matches(
                function,
                &args,
                0,
                is_entity_ref_type,
                "an 'entity_ref'",
                "target",
                expr,
                diagnostics,
            );
            if let Some(target) = args.first() {
                match target.ref_kind {
                    RefKind::Player => diagnostics.push(Diagnostic::new(
                        "heal(...) only supports known non-player 'entity_ref' targets in v1",
                        expr.span.clone(),
                    )),
                    RefKind::Unknown => diagnostics.push(Diagnostic::new(
                        "heal(...) rejects ambiguous 'entity_ref' targets in v1",
                        expr.span.clone(),
                    )),
                    RefKind::NonPlayer => {}
                }
            }
            expect_arg_type(function, &args, 1, Type::Int, "amount", expr, diagnostics);
            builtin_call_expr(function, args, Type::Void)
        }
        GameplayBuiltinKind::Give | GameplayBuiltinKind::Clear => {
            expect_arity(function, &args, 3, expr, diagnostics);
            expect_entity_target_arg(function, &args, 0, expr, diagnostics);
            expect_arg_type(
                function,
                &args,
                1,
                Type::String,
                "item id",
                expr,
                diagnostics,
            );
            expect_arg_type(function, &args, 2, Type::Int, "count", expr, diagnostics);
            builtin_call_expr(function, args, Type::Void)
        }
        GameplayBuiltinKind::LootGive => {
            expect_arity(function, &args, 2, expr, diagnostics);
            expect_entity_target_arg(function, &args, 0, expr, diagnostics);
            expect_arg_type(
                function,
                &args,
                1,
                Type::String,
                "loot table",
                expr,
                diagnostics,
            );
            builtin_call_expr(function, args, Type::Void)
        }
        GameplayBuiltinKind::LootInsert | GameplayBuiltinKind::LootSpawn => {
            expect_arity(function, &args, 2, expr, diagnostics);
            expect_arg_type(
                function,
                &args,
                0,
                Type::BlockRef,
                "position",
                expr,
                diagnostics,
            );
            expect_arg_type(
                function,
                &args,
                1,
                Type::String,
                "loot table",
                expr,
                diagnostics,
            );
            builtin_call_expr(function, args, Type::Void)
        }
        GameplayBuiltinKind::Tellraw
        | GameplayBuiltinKind::Title
        | GameplayBuiltinKind::Actionbar => {
            expect_arity(function, &args, 2, expr, diagnostics);
            expect_entity_target_arg(function, &args, 0, expr, diagnostics);
            if let Some(message) = args.get(1) {
                if !matches!(message.ty, Type::String | Type::TextDef) {
                    diagnostics.push(Diagnostic::new(
                        format!(
                            "argument 2 for '{}' must be 'string' or 'text_def', found '{}'",
                            function,
                            message.ty.as_str()
                        ),
                        expr.span.clone(),
                    ));
                }
            }
            builtin_call_expr(function, args, Type::Void)
        }
        GameplayBuiltinKind::Debug => {
            expect_arity(function, &args, 1, expr, diagnostics);
            expect_arg_type(
                function,
                &args,
                0,
                Type::String,
                "message",
                expr,
                diagnostics,
            );
            builtin_call_expr(function, args, Type::Void)
        }
        GameplayBuiltinKind::DebugMarker => {
            if !(args.len() == 2 || args.len() == 3) {
                diagnostics.push(Diagnostic::new(
                    format!(
                        "wrong arity for '{}': expected 2 or 3, found {}",
                        function,
                        args.len()
                    ),
                    expr.span.clone(),
                ));
            }
            expect_arg_type(
                function,
                &args,
                0,
                Type::BlockRef,
                "position",
                expr,
                diagnostics,
            );
            expect_arg_type(function, &args, 1, Type::String, "label", expr, diagnostics);
            if args.len() >= 3 {
                expect_arg_type(
                    function,
                    &args,
                    2,
                    Type::String,
                    "marker block id",
                    expr,
                    diagnostics,
                );
            }
            builtin_call_expr(function, args, Type::Void)
        }
        GameplayBuiltinKind::DebugEntity => {
            expect_arity(function, &args, 2, expr, diagnostics);
            expect_entity_target_arg(function, &args, 0, expr, diagnostics);
            expect_arg_type(function, &args, 1, Type::String, "label", expr, diagnostics);
            builtin_call_expr(function, args, Type::Void)
        }
        GameplayBuiltinKind::BossbarAdd | GameplayBuiltinKind::BossbarName => {
            expect_arity(function, &args, 2, expr, diagnostics);
            expect_arg_type(
                function,
                &args,
                0,
                Type::String,
                "bossbar id",
                expr,
                diagnostics,
            );
            if let Some(name) = args.get(1) {
                if !matches!(name.ty, Type::String | Type::TextDef) {
                    diagnostics.push(Diagnostic::new(
                        format!(
                            "argument 2 for '{}' must be 'string' or 'text_def', found '{}'",
                            function,
                            name.ty.as_str()
                        ),
                        expr.span.clone(),
                    ));
                }
            }
            builtin_call_expr(function, args, Type::Void)
        }
        GameplayBuiltinKind::BossbarRemove => {
            expect_arity(function, &args, 1, expr, diagnostics);
            expect_arg_type(
                function,
                &args,
                0,
                Type::String,
                "bossbar id",
                expr,
                diagnostics,
            );
            builtin_call_expr(function, args, Type::Void)
        }
        GameplayBuiltinKind::BossbarValue | GameplayBuiltinKind::BossbarMax => {
            expect_arity(function, &args, 2, expr, diagnostics);
            expect_arg_type(
                function,
                &args,
                0,
                Type::String,
                "bossbar id",
                expr,
                diagnostics,
            );
            expect_arg_type(function, &args, 1, Type::Int, "value", expr, diagnostics);
            builtin_call_expr(function, args, Type::Void)
        }
        GameplayBuiltinKind::BossbarVisible => {
            expect_arity(function, &args, 2, expr, diagnostics);
            expect_arg_type(
                function,
                &args,
                0,
                Type::String,
                "bossbar id",
                expr,
                diagnostics,
            );
            expect_arg_type(function, &args, 1, Type::Bool, "visible", expr, diagnostics);
            builtin_call_expr(function, args, Type::Void)
        }
        GameplayBuiltinKind::BossbarPlayers => {
            expect_arity(function, &args, 2, expr, diagnostics);
            expect_arg_type(
                function,
                &args,
                0,
                Type::String,
                "bossbar id",
                expr,
                diagnostics,
            );
            expect_entity_target_arg(function, &args, 1, expr, diagnostics);
            builtin_call_expr(function, args, Type::Void)
        }
        GameplayBuiltinKind::Playsound => {
            expect_arity(function, &args, 3, expr, diagnostics);
            expect_arg_type(
                function,
                &args,
                0,
                Type::String,
                "sound id",
                expr,
                diagnostics,
            );
            expect_arg_type(
                function,
                &args,
                1,
                Type::String,
                "category",
                expr,
                diagnostics,
            );
            expect_entity_target_arg(function, &args, 2, expr, diagnostics);
            builtin_call_expr(function, args, Type::Void)
        }
        GameplayBuiltinKind::Stopsound => {
            expect_arity(function, &args, 3, expr, diagnostics);
            expect_entity_target_arg(function, &args, 0, expr, diagnostics);
            expect_arg_type(
                function,
                &args,
                1,
                Type::String,
                "category",
                expr,
                diagnostics,
            );
            expect_arg_type(
                function,
                &args,
                2,
                Type::String,
                "sound id",
                expr,
                diagnostics,
            );
            builtin_call_expr(function, args, Type::Void)
        }
        GameplayBuiltinKind::Particle => {
            if !(args.len() == 2 || args.len() == 3 || args.len() == 4) {
                diagnostics.push(Diagnostic::new(
                    format!(
                        "wrong arity for '{}': expected 2, 3, or 4, found {}",
                        function,
                        args.len()
                    ),
                    expr.span.clone(),
                ));
            }
            expect_arg_type(
                function,
                &args,
                0,
                Type::String,
                "particle id",
                expr,
                diagnostics,
            );
            expect_arg_type(
                function,
                &args,
                1,
                Type::BlockRef,
                "position",
                expr,
                diagnostics,
            );
            if args.len() >= 3 {
                expect_arg_type(function, &args, 2, Type::Int, "count", expr, diagnostics);
            }
            if args.len() >= 4 {
                expect_entity_target_arg(function, &args, 3, expr, diagnostics);
            }
            builtin_call_expr(function, args, Type::Void)
        }
        GameplayBuiltinKind::Setblock => {
            expect_arity(function, &args, 2, expr, diagnostics);
            expect_arg_type(
                function,
                &args,
                0,
                Type::BlockRef,
                "position",
                expr,
                diagnostics,
            );
            expect_arg_matches(
                function,
                &args,
                1,
                |ty| matches!(ty, Type::String | Type::BlockDef),
                "'string' or 'block_def'",
                "block",
                expr,
                diagnostics,
            );
            builtin_call_expr(function, args, Type::Void)
        }
        GameplayBuiltinKind::Fill => {
            expect_arity(function, &args, 3, expr, diagnostics);
            expect_arg_type(
                function,
                &args,
                0,
                Type::BlockRef,
                "from",
                expr,
                diagnostics,
            );
            expect_arg_type(function, &args, 1, Type::BlockRef, "to", expr, diagnostics);
            expect_arg_matches(
                function,
                &args,
                2,
                |ty| matches!(ty, Type::String | Type::BlockDef),
                "'string' or 'block_def'",
                "block",
                expr,
                diagnostics,
            );
            builtin_call_expr(function, args, Type::Void)
        }
    }
}

fn builtin_call_expr(function: &str, args: Vec<TypedExpr>, ty: Type) -> TypedExpr {
    TypedExpr {
        kind: TypedExprKind::Call {
            function: function.to_string(),
            args,
        },
        ty,
        ref_kind: RefKind::Unknown,
    }
}

fn method_call_expr(
    receiver: TypedExpr,
    method: &str,
    args: Vec<TypedExpr>,
    ty: Type,
) -> TypedExpr {
    TypedExpr {
        kind: TypedExprKind::MethodCall {
            receiver: Box::new(receiver),
            method: method.to_string(),
            args,
        },
        ty,
        ref_kind: RefKind::Unknown,
    }
}

fn coerce_expr_to_expected_type(expr: TypedExpr, expected: &Type) -> TypedExpr {
    if matches!(expected, Type::Int | Type::Bool)
        && expr.ty == Type::Nbt
        && is_entity_state_path_expr(&expr)
    {
        return TypedExpr {
            kind: TypedExprKind::Cast {
                kind: match expected {
                    Type::Int => CastKind::Int,
                    Type::Bool => CastKind::Bool,
                    _ => unreachable!(),
                },
                expr: Box::new(expr),
            },
            ty: expected.clone(),
            ref_kind: RefKind::Unknown,
        };
    }
    if *expected == Type::Nbt {
        return coerce_expr_to_nbt(expr);
    }
    if *expected == Type::PlayerRef && is_entity_ref_type(&expr.ty) {
        let mut expr = expr;
        expr.ty = Type::PlayerRef;
        expr.ref_kind = RefKind::Player;
        return expr;
    }
    if *expected == Type::EntityRef && expr.ty == Type::PlayerRef {
        let mut expr = expr;
        expr.ty = Type::EntityRef;
        expr.ref_kind = RefKind::Player;
        return expr;
    }
    expr
}

fn is_entity_state_path_expr(expr: &TypedExpr) -> bool {
    matches!(
        &expr.kind,
        TypedExprKind::Path(path)
            if is_entity_ref_type(&path.base.ty)
                && path.segments.len() > 1
                && matches!(path.segments.first(), Some(PathSegment::Field(name)) if name == "state")
    )
}

fn coerce_expr_to_nbt(expr: TypedExpr) -> TypedExpr {
    match expr.ty {
        Type::EntityDef | Type::BlockDef | Type::ItemDef => {
            method_call_expr(expr, "as_nbt", Vec::new(), Type::Nbt)
        }
        Type::TextDef => TypedExpr {
            kind: expr.kind,
            ty: Type::Nbt,
            ref_kind: expr.ref_kind,
        },
        _ => expr,
    }
}

fn expect_entity_receiver(
    method: &str,
    receiver: &TypedExpr,
    expr: &Expr,
    diagnostics: &mut Diagnostics,
) {
    if !matches!(
        receiver.ty,
        Type::EntityRef | Type::PlayerRef | Type::EntitySet
    ) {
        diagnostics.push(Diagnostic::new(
            format!(
                "{}(...) requires an 'entity_ref' or 'entity_set' receiver",
                method
            ),
            expr.span.clone(),
        ));
    }
}

fn is_entity_ref_type(ty: &Type) -> bool {
    matches!(ty, Type::EntityRef | Type::PlayerRef)
}

fn expect_block_receiver(
    method: &str,
    receiver: &TypedExpr,
    expr: &Expr,
    diagnostics: &mut Diagnostics,
) {
    if receiver.ty != Type::BlockRef {
        diagnostics.push(Diagnostic::new(
            format!("{}(...) requires a 'block_ref' receiver", method),
            expr.span.clone(),
        ));
    }
}

fn expect_entity_target_arg(
    function: &str,
    args: &[TypedExpr],
    index: usize,
    expr: &Expr,
    diagnostics: &mut Diagnostics,
) {
    expect_arg_matches(
        function,
        args,
        index,
        |ty| matches!(ty, Type::EntityRef | Type::PlayerRef | Type::EntitySet),
        "an 'entity_ref' or 'entity_set'",
        "target",
        expr,
        diagnostics,
    );
}

fn expect_arg_type(
    function: &str,
    args: &[TypedExpr],
    index: usize,
    expected: Type,
    label: &str,
    expr: &Expr,
    diagnostics: &mut Diagnostics,
) {
    expect_arg_matches(
        function,
        args,
        index,
        |ty| *ty == expected,
        &expected.as_str(),
        label,
        expr,
        diagnostics,
    );
}

fn expect_arg_matches(
    function: &str,
    args: &[TypedExpr],
    index: usize,
    predicate: impl Fn(&Type) -> bool,
    expected: &str,
    label: &str,
    expr: &Expr,
    diagnostics: &mut Diagnostics,
) {
    if let Some(arg) = args.get(index) {
        if !predicate(&arg.ty) {
            diagnostics.push(Diagnostic::new(
                format!(
                    "{}(...) {} must be {}, found '{}'",
                    function,
                    label,
                    expected,
                    arg.ty.as_str()
                ),
                expr.span.clone(),
            ));
        }
    }
}

fn detect_selector_ref_kind(selector: &str) -> RefKind {
    if is_plain_player_name_target(selector) {
        return RefKind::Player;
    }
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
    if path.base.ref_kind != RefKind::Player || !is_entity_ref_type(&path.base.ty) {
        return;
    }
    let Some(first) = path.segments.first() else {
        return;
    };
    let PathSegment::Field(first) = first else {
        diagnostics.push(Diagnostic::new(
            "player path access must start with a namespace such as 'nbt', 'state', 'tags', 'team', 'inventory', 'hotbar', or 'mainhand'",
            span,
        ));
        return;
    };
    if !matches!(
        first.as_str(),
        "nbt"
            | "state"
            | "tags"
            | "team"
            | "inventory"
            | "hotbar"
            | "mainhand"
            | "offhand"
            | "head"
            | "chest"
            | "legs"
            | "feet"
            | "position"
    ) {
        diagnostics.push(Diagnostic::new(
            "player path access must use 'player.nbt', 'player.state', 'player.tags', 'player.team', 'player.position', 'player.inventory[index]', 'player.hotbar[index]', or an equipment namespace such as 'mainhand'",
            span,
        ));
    }
}

fn storage_path_accepts_value(path: &TypedPathExpr, value: &TypedExpr) -> bool {
    if path.ty == value.ty {
        return true;
    }
    if path.ty == Type::Nbt {
        return is_nbt_compatible_type(&value.ty);
    }
    false
}

fn storage_path_assignment_message(path: &TypedPathExpr, value: &TypedExpr) -> String {
    if path.ty == Type::Nbt {
        return format!(
            "cannot assign '{}' to NBT path; expected an NBT-compatible value",
            value.ty.as_str()
        );
    }
    format!(
        "cannot assign '{}' to collection element of type '{}'",
        value.ty.as_str(),
        path.ty.as_str()
    )
}

fn is_nbt_compatible_type(ty: &Type) -> bool {
    matches!(
        ty,
        Type::Int
            | Type::Bool
            | Type::String
            | Type::Nbt
            | Type::Array(_)
            | Type::Dict(_)
            | Type::Struct(_)
            | Type::ItemDef
            | Type::TextDef
            | Type::Bossbar
    )
}

fn validate_builder_path_write(
    path: &TypedPathExpr,
    value: &TypedExpr,
    span: Span,
    diagnostics: &mut Diagnostics,
) {
    match path.base.ty {
        Type::EntityDef => validate_entity_builder_path_write(path, value, span, diagnostics),
        Type::BlockDef => validate_block_builder_path_write(path, value, span, diagnostics),
        Type::ItemDef => validate_item_builder_path_write(path, value, span, diagnostics),
        Type::TextDef => validate_text_builder_path_write(path, value, span, diagnostics),
        _ => {}
    }
}

fn validate_entity_builder_path_write(
    path: &TypedPathExpr,
    value: &TypedExpr,
    span: Span,
    diagnostics: &mut Diagnostics,
) {
    let Some(PathSegment::Field(first)) = path.segments.first() else {
        diagnostics.push(Diagnostic::new(
            "entity builder writes must use 'nbt' or a supported alias such as 'name'",
            span,
        ));
        return;
    };
    match first.as_str() {
        "id" => diagnostics.push(Diagnostic::new("entity builder id is read-only", span)),
        "nbt" => {
            if !is_nbt_compatible_type(&value.ty) {
                diagnostics.push(Diagnostic::new(
                    "entity builder NBT requires an NBT-compatible value",
                    span,
                ));
            }
        }
        _ => diagnostics.push(Diagnostic::new(
            "entity builder writes must use 'nbt' or a supported alias such as 'name'",
            span,
        )),
    }
}

fn validate_block_builder_path_write(
    path: &TypedPathExpr,
    value: &TypedExpr,
    span: Span,
    diagnostics: &mut Diagnostics,
) {
    let Some(PathSegment::Field(first)) = path.segments.first() else {
        diagnostics.push(Diagnostic::new(
            "block builder writes must use 'states', 'nbt', or a supported alias such as 'name'",
            span,
        ));
        return;
    };
    match first.as_str() {
        "id" => diagnostics.push(Diagnostic::new("block builder id is read-only", span)),
        "states" => {
            if path.segments.len() == 1 {
                diagnostics.push(Diagnostic::new(
                    "block builder state writes must target a field such as 'states.facing'",
                    span,
                ));
                return;
            }
            if !matches!(value.ty, Type::Int | Type::Bool | Type::String) {
                diagnostics.push(Diagnostic::new(
                    "block builder states require an 'int', 'bool', or 'string' value",
                    span,
                ));
            }
        }
        "nbt" => {
            if !is_nbt_compatible_type(&value.ty) {
                diagnostics.push(Diagnostic::new(
                    "block builder NBT requires an NBT-compatible value",
                    span,
                ));
            }
        }
        _ => diagnostics.push(Diagnostic::new(
            "block builder writes must use 'states', 'nbt', or a supported alias such as 'name'",
            span,
        )),
    }
}

fn validate_item_builder_path_write(
    path: &TypedPathExpr,
    value: &TypedExpr,
    span: Span,
    diagnostics: &mut Diagnostics,
) {
    let Some(PathSegment::Field(first)) = path.segments.first() else {
        diagnostics.push(Diagnostic::new(
            "item builder writes must use 'count', 'nbt', or a supported alias such as 'name'",
            span,
        ));
        return;
    };
    match first.as_str() {
        "id" => diagnostics.push(Diagnostic::new("item builder id is read-only", span)),
        "count" => {
            if value.ty != Type::Int {
                diagnostics.push(Diagnostic::new(
                    "item builder count requires an 'int' value",
                    span,
                ));
            }
        }
        "nbt" | "name" => {
            if !is_nbt_compatible_type(&value.ty) {
                diagnostics.push(Diagnostic::new(
                    "item builder NBT requires an NBT-compatible value",
                    span,
                ));
            }
        }
        _ => diagnostics.push(Diagnostic::new(
            "item builder writes must use 'count', 'nbt', or a supported alias such as 'name'",
            span,
        )),
    }
}

fn validate_text_builder_path_write(
    path: &TypedPathExpr,
    value: &TypedExpr,
    span: Span,
    diagnostics: &mut Diagnostics,
) {
    let Some(PathSegment::Field(_)) = path.segments.first() else {
        diagnostics.push(Diagnostic::new(
            "text builder writes must use '.field' access",
            span,
        ));
        return;
    };
    if !is_nbt_compatible_type(&value.ty) {
        diagnostics.push(Diagnostic::new(
            "text builder fields require an NBT-compatible value",
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
    if !is_entity_ref_type(&path.base.ty) {
        return;
    }
    let Some(PathSegment::Field(first)) = path.segments.first() else {
        diagnostics.push(Diagnostic::new(
            "entity writes must use a supported gameplay namespace or raw NBT path",
            span,
        ));
        return;
    };
    match first.as_str() {
        "position" => diagnostics.push(Diagnostic::new(
            "entity.position is read-only; use methods such as entity.position.setblock(...)",
            span,
        )),
        "nbt" if path.base.ref_kind == RefKind::Player => diagnostics.push(Diagnostic::new(
            "player.nbt.* is read-only; use player.state, player.tags, player.team, or equipment namespaces instead",
            span,
        )),
        "state" => {
            if !matches!(value.ty, Type::Int | Type::Bool) {
                diagnostics.push(Diagnostic::new(
                    if path.base.ref_kind == RefKind::Player {
                        "player.state.* currently supports only 'int' and 'bool' values"
                    } else {
                        "entity.state.* currently supports only 'int' and 'bool' values"
                    },
                    span,
                ));
            }
        }
        "tags" => {
            if path.base.ref_kind == RefKind::Player && value.ty != Type::Bool {
                diagnostics.push(Diagnostic::new(
                    "player.tags.* assignments require a 'bool' value",
                    span,
                ));
            }
        }
        "team" => {
            if value.ty != Type::String {
                diagnostics.push(Diagnostic::new(
                    "team requires a 'string' value",
                    span,
                ));
            }
        }
        "inventory" | "hotbar" => {
            if path.base.ref_kind != RefKind::Player {
                diagnostics.push(Diagnostic::new(
                    "inventory and hotbar are only supported on known player refs; use 'player_ref' to assert a player",
                    span,
                ));
            } else {
                validate_player_inventory_path_write(path, value, span, diagnostics);
            }
        }
        "mainhand" | "offhand" | "head" | "chest" | "legs" | "feet" => {
            validate_equipment_path_write(path, value, span, diagnostics);
        }
        _ if path.base.ref_kind == RefKind::Player => diagnostics.push(Diagnostic::new(
            "unsafe writable player path; use player.state, player.tags, player.team, player.inventory, player.hotbar, or equipment namespaces",
            span,
        )),
        _ => {}
    }
}

fn validate_bossbar_path_write(
    path: &TypedPathExpr,
    value: &TypedExpr,
    span: Span,
    diagnostics: &mut Diagnostics,
) {
    let [PathSegment::Field(field)] = path.segments.as_slice() else {
        diagnostics.push(Diagnostic::new(
            "bossbar assignment must target one property such as '.value'",
            span,
        ));
        return;
    };
    let valid = match field.as_str() {
        "name" => matches!(value.ty, Type::String | Type::TextDef),
        "value" | "max" => value.ty == Type::Int,
        "visible" => value.ty == Type::Bool,
        "players" => matches!(
            value.ty,
            Type::EntityRef | Type::PlayerRef | Type::EntitySet
        ),
        _ => {
            diagnostics.push(Diagnostic::new(
                format!("unknown bossbar property '{}'", field),
                span,
            ));
            return;
        }
    };
    if !valid {
        diagnostics.push(Diagnostic::new(
            format!(
                "bossbar.{} cannot be assigned a value of type '{}'",
                field,
                value.ty.as_str()
            ),
            span,
        ));
    }
}

fn validate_equipment_path_write(
    path: &TypedPathExpr,
    value: &TypedExpr,
    span: Span,
    diagnostics: &mut Diagnostics,
) {
    let Some(PathSegment::Field(field)) = path.segments.get(1) else {
        diagnostics.push(Diagnostic::new(
            "equipment writes must target '.item', '.name', or '.count'",
            span,
        ));
        return;
    };
    match field.as_str() {
        "item" | "name" => {
            if field == "item" && value.ty == Type::ItemDef {
                return;
            }
            if value.ty != Type::String {
                diagnostics.push(Diagnostic::new(
                    format!(
                        "equipment.{} requires a 'string'{} value",
                        field,
                        if field == "item" {
                            " or 'item_def'"
                        } else {
                            ""
                        }
                    ),
                    span,
                ));
            }
        }
        "count" => {
            if value.ty != Type::Int {
                diagnostics.push(Diagnostic::new(
                    "equipment.count requires an 'int' value",
                    span,
                ));
            }
        }
        _ => diagnostics.push(Diagnostic::new(
            "equipment writes must target '.item', '.name', or '.count'",
            span,
        )),
    }
}

fn validate_player_inventory_path_write(
    path: &TypedPathExpr,
    value: &TypedExpr,
    span: Span,
    diagnostics: &mut Diagnostics,
) {
    if path.segments.len() < 2 {
        diagnostics.push(Diagnostic::new(
            "inventory and hotbar writes must target a slot such as 'player.inventory[0]'",
            span,
        ));
        return;
    }
    if path.segments.len() == 2 {
        if value.ty != Type::ItemDef {
            diagnostics.push(Diagnostic::new(
                "whole-slot inventory assignment requires an 'item_def' value",
                span,
            ));
        }
        return;
    }
    let Some(PathSegment::Field(field)) = path.segments.get(2) else {
        diagnostics.push(Diagnostic::new(
            "item slot writes must target '.count', '.nbt', or the alias '.name'",
            span,
        ));
        return;
    };
    match field.as_str() {
        "exists" | "id" => diagnostics.push(Diagnostic::new(
            format!("item slot.{} is read-only", field),
            span,
        )),
        "count" => {
            if value.ty != Type::Int {
                diagnostics.push(Diagnostic::new(
                    "item slot.count requires an 'int' value",
                    span,
                ));
            }
        }
        "name" => {
            if value.ty != Type::String {
                diagnostics.push(Diagnostic::new(
                    "item slot.name requires a 'string' value",
                    span,
                ));
            }
        }
        "nbt" => {
            if !is_nbt_compatible_type(&value.ty) {
                diagnostics.push(Diagnostic::new(
                    "item slot NBT requires an NBT-compatible value",
                    span,
                ));
            }
        }
        _ => diagnostics.push(Diagnostic::new(
            "item slot writes must target '.count', '.nbt', or the alias '.name'",
            span,
        )),
    }
}

fn type_check_args(
    args: &[Expr],
    struct_defs: &BTreeMap<String, StructTypeDef>,
    signatures: &BTreeMap<String, FunctionSignature>,
    env: &HashMap<String, Type>,
    ref_env: &HashMap<String, RefKind>,
    called_functions: &mut BTreeSet<String>,
    diagnostics: &mut Diagnostics,
) -> Vec<TypedExpr> {
    args.iter()
        .map(|arg| {
            type_check_expr(
                arg,
                struct_defs,
                signatures,
                env,
                ref_env,
                called_functions,
                diagnostics,
            )
        })
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
                function,
                expected,
                args.len()
            ),
            expr.span.clone(),
        ));
    }
}

fn context_name(kind: ContextKind) -> &'static str {
    match kind {
        ContextKind::As => "as",
        ContextKind::At => "at",
    }
}

fn lower_string_match_stmt(
    value: TypedExpr,
    arms: Vec<(String, Vec<TypedStmt>)>,
    else_body: Vec<TypedStmt>,
) -> TypedStmtKind {
    let mut current_else = else_body;
    for (pattern, body) in arms.into_iter().rev() {
        let condition = TypedExpr {
            kind: TypedExprKind::Binary {
                op: BinaryOp::Eq,
                left: Box::new(value.clone()),
                right: Box::new(TypedExpr {
                    kind: TypedExprKind::String(pattern),
                    ty: Type::String,
                    ref_kind: RefKind::Unknown,
                }),
            },
            ty: Type::Bool,
            ref_kind: RefKind::Unknown,
        };
        current_else = vec![TypedStmt {
            kind: TypedStmtKind::If {
                condition,
                then_body: body,
                else_body: current_else,
            },
        }];
    }

    current_else
        .into_iter()
        .next()
        .map(|stmt| stmt.kind)
        .unwrap_or(TypedStmtKind::If {
            condition: TypedExpr {
                kind: TypedExprKind::Bool(false),
                ty: Type::Bool,
                ref_kind: RefKind::Unknown,
            },
            then_body: Vec::new(),
            else_body: Vec::new(),
        })
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
        TypedExprKind::As { value, .. } => rewrite_single_limit(value, diagnostics, span),
        _ => {}
    }
}

fn add_or_validate_limit(value: &str, diagnostics: &mut Diagnostics, span: Span) -> String {
    if is_plain_player_name_target(value) {
        return value.to_string();
    }
    let lower = value.to_ascii_lowercase();
    if let Some(index) = lower.find("limit=") {
        let suffix = &lower[index + 6..];
        let digits: String = suffix
            .chars()
            .take_while(|ch| ch.is_ascii_digit())
            .collect();
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

fn is_plain_player_name_target(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty()
        && !trimmed.starts_with('@')
        && trimmed.len() <= 16
        && trimmed
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
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

fn collect_macro_placeholders(
    template: &str,
    struct_defs: &BTreeMap<String, StructTypeDef>,
    signatures: &BTreeMap<String, FunctionSignature>,
    env: &HashMap<String, Type>,
    ref_env: &HashMap<String, RefKind>,
    called_functions: &mut BTreeSet<String>,
    span: Span,
    diagnostics: &mut Diagnostics,
) -> Vec<MacroPlaceholder> {
    let mut placeholders = Vec::new();
    for (index, body) in scan_macro_placeholders(template, span.clone(), diagnostics)
        .into_iter()
        .enumerate()
    {
        if body.trim().is_empty() {
            diagnostics.push(Diagnostic::new(
                "macro placeholder expression cannot be empty",
                span.clone(),
            ));
            continue;
        }
        let parsed = match crate::parser::parse_expression(&body) {
            Ok(expr) => expr,
            Err(parse_diags) => {
                for diag in parse_diags.0 {
                    diagnostics.push(Diagnostic::new(
                        format!(
                            "invalid macro placeholder expression '{}': {}",
                            body, diag.message
                        ),
                        span.clone(),
                    ));
                }
                continue;
            }
        };
        let typed = type_check_expr(
            &parsed,
            struct_defs,
            signatures,
            env,
            ref_env,
            called_functions,
            diagnostics,
        );
        if !matches!(
            typed.ty,
            Type::Int
                | Type::Bool
                | Type::String
                | Type::EntitySet
                | Type::EntityRef
                | Type::PlayerRef
                | Type::BlockRef
                | Type::Nbt
                | Type::TextDef
                | Type::Array(_)
                | Type::Dict(_)
                | Type::Struct(_)
        ) {
            diagnostics.push(Diagnostic::new(
                format!(
                    "macro placeholder '{}' has unsupported type '{}'",
                    body,
                    typed.ty.as_str()
                ),
                span.clone(),
            ));
            continue;
        }
        placeholders.push(MacroPlaceholder {
            key: format!("p{}", index + 1),
            ty: typed.ty.clone(),
            expr: typed,
        });
    }
    placeholders
}

fn scan_macro_placeholders(
    template: &str,
    span: Span,
    diagnostics: &mut Diagnostics,
) -> Vec<String> {
    let bytes = template.as_bytes();
    let mut index = 0usize;
    let mut placeholders = Vec::new();
    while index + 1 < bytes.len() {
        if bytes[index] == b'$' && bytes[index + 1] == b'(' {
            let start = index + 2;
            index = start;
            let mut paren_depth = 1usize;
            let mut in_string = false;
            let mut string_delim = b'"';
            while index < bytes.len() {
                let ch = bytes[index];
                if in_string {
                    if ch == b'\\' {
                        index += 2;
                        continue;
                    }
                    if ch == string_delim {
                        in_string = false;
                    }
                    index += 1;
                    continue;
                }
                match ch {
                    b'"' | b'\'' => {
                        in_string = true;
                        string_delim = ch;
                    }
                    b'(' => paren_depth += 1,
                    b')' => {
                        paren_depth -= 1;
                        if paren_depth == 0 {
                            placeholders.push(template[start..index].to_string());
                            break;
                        }
                    }
                    _ => {}
                }
                index += 1;
            }
            if index >= bytes.len() || paren_depth != 0 {
                diagnostics.push(Diagnostic::new(
                    "unterminated macro placeholder",
                    span.clone(),
                ));
                break;
            }
        }
        index += 1;
    }
    placeholders
}
