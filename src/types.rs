use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Mutex};

use crate::ast::*;
use crate::diagnostics::{Diagnostic, Diagnostics, Span};
use crate::language_catalog::{
    ENTITY_METHOD_NAMES, GENERIC_AGENT_EVENTS, OLD_ENTITY_METHOD_NAMES, VANILLA_EVENTS,
    accessor_property, capitalized, display_call, event_kind_for_type, event_type_name,
    property_names,
};

thread_local! {
    /// Set while a `setX(...)` call is checked as the assignment it lowers to,
    /// so the assignment isn't reported as property syntax.
    static LOWERING_SETTER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// `bb.max` written in source: properties are read and written through get/set methods.
fn check_property_syntax(
    base_ty: &Type,
    segments: &[PathSegment],
    write: bool,
    span: Span,
    diagnostics: &mut Diagnostics,
) {
    if let [PathSegment::Field(field)] = segments
        && property_names(base_ty).contains(&field.as_str())
    {
        let accessor = if write {
            format!("set{}(...)", capitalized(field))
        } else {
            format!("get{}()", capitalized(field))
        };
        diagnostics.push(Diagnostic::new(
            format!("use '.{accessor}' for the {} property", field),
            span,
        ));
    }
}

/// Host-bridge modules that may be called as `module.fn(...)` from `.mcf` source.
/// Recognising a name here (independent of whether it is enabled) lets the type
/// checker emit precise "not enabled" / "wrong position" diagnostics instead of a
/// generic "unknown variable".
pub const KNOWN_HOST_MODULES: &[&str] = &["http", "file", "kv", "db", "time", "rand", "mcfd"];

pub fn is_known_host_module(name: &str) -> bool {
    KNOWN_HOST_MODULES.contains(&name)
}

/// The set of host modules enabled for a compilation, derived from the
/// `[helper.capabilities]` manifest section.
#[derive(Debug, Clone, Default)]
pub struct HostModules {
    enabled: HashSet<String>,
}

impl HostModules {
    /// Editor analysis needs compiler-provided response and agent payload types
    /// before a manifest is available. `mcfd` registers those structs while
    /// leaving the other host capabilities gated.
    pub fn for_editor() -> Self {
        let mut enabled = HashSet::new();
        enabled.insert("mcfd".to_string());
        HostModules { enabled }
    }

    pub fn from_helper(helper: Option<&crate::project::HelperConfig>) -> Self {
        let mut enabled: HashSet<String> = helper
            .map(|config| {
                config
                    .capabilities
                    .enabled_modules()
                    .into_iter()
                    .map(|module| module.to_string())
                    .collect()
            })
            .unwrap_or_default();
        // The `mcfd` connectivity probe needs no declared capability; it is
        // available whenever a host bridge is configured at all.
        if helper.is_some() {
            enabled.insert("mcfd".to_string());
        }
        HostModules { enabled }
    }

    pub fn is_enabled(&self, module: &str) -> bool {
        self.enabled.contains(module)
    }

    pub fn any_enabled(&self) -> bool {
        !self.enabled.is_empty()
    }
}

/// Compile-time signature of a host call. The return type is always a builtin
/// response struct (see [`builtin_response_structs`]).
pub struct HostCallSig {
    pub params: Vec<Type>,
    pub return_type: Type,
}

/// Resolve `module.function` to its signature, or `None` if the function does not
/// exist on that module.
pub fn host_call_signature(module: &str, function: &str) -> Option<HostCallSig> {
    let string_array = || Type::Array(Box::new(Type::String));
    let (params, return_type): (Vec<Type>, Type) = match (module, function) {
        ("http", "get") => (vec![Type::String], Type::Struct("HttpResponse".to_string())),
        ("http", "get_json_string") => (
            vec![Type::String, Type::String],
            Type::Struct("HttpResponse".to_string()),
        ),
        ("http", "get_json_strings") => (
            vec![Type::String, string_array()],
            Type::Struct("JsonStringsResponse".to_string()),
        ),
        ("http", "post") => (
            vec![Type::String, Type::String],
            Type::Struct("HttpResponse".to_string()),
        ),
        ("file", "read") => (vec![Type::String], Type::Struct("FileResult".to_string())),
        ("file", "write") => (
            vec![Type::String, Type::String],
            Type::Struct("OpResult".to_string()),
        ),
        ("kv", "get") => (vec![Type::String], Type::Struct("KvResult".to_string())),
        ("kv", "set") => (
            vec![Type::String, Type::String],
            Type::Struct("OpResult".to_string()),
        ),
        ("db", "exec") => (
            vec![Type::String, string_array()],
            Type::Struct("DbResult".to_string()),
        ),
        ("db", "query") => (
            vec![Type::String, string_array()],
            Type::Struct("DbResult".to_string()),
        ),
        ("time", "now") => (vec![], Type::Struct("TimeResult".to_string())),
        ("rand", "int") => (
            vec![Type::Int, Type::Int],
            Type::Struct("RandResult".to_string()),
        ),
        ("mcfd", "ping") => (vec![], Type::Struct("PingResult".to_string())),
        _ => return None,
    };
    Some(HostCallSig {
        params,
        return_type,
    })
}

/// The builtin struct types returned by host calls. Registered into `struct_defs`
/// so field access (`response.status`) type-checks. Every response carries `ok`.
fn builtin_response_structs() -> Vec<(&'static str, Vec<(&'static str, Type)>)> {
    vec![
        // JVM-agent event payloads. These are compiler-provided structs rather
        // than user declarations so every agent-enabled pack shares one stable
        // 26.3 wire contract.
        (
            "CommandSender",
            vec![
                ("kind", Type::String),
                ("name", Type::String),
                ("permissionLevel", Type::Int),
                ("player", Type::PlayerRef),
            ],
        ),
        (
            "ChatEvent",
            vec![
                ("player", Type::PlayerRef),
                ("message", Type::String),
                ("cancelled", Type::Bool),
            ],
        ),
        (
            "InventoryClickEvent",
            vec![
                ("player", Type::PlayerRef),
                ("containerId", Type::Int),
                ("stateId", Type::Int),
                ("slot", Type::Int),
                ("button", Type::Int),
                ("cancelled", Type::Bool),
            ],
        ),
        (
            "PlayerActionEvent",
            vec![
                ("player", Type::PlayerRef),
                ("action", Type::String),
                ("face", Type::String),
                ("x", Type::Int),
                ("y", Type::Int),
                ("z", Type::Int),
                ("cancelled", Type::Bool),
            ],
        ),
        (
            "BlockBreakEvent",
            vec![
                ("player", Type::PlayerRef),
                ("x", Type::Int),
                ("y", Type::Int),
                ("z", Type::Int),
                ("cancelled", Type::Bool),
            ],
        ),
        (
            "PlayerInteractBlockEvent",
            vec![
                ("player", Type::PlayerRef),
                ("hand", Type::String),
                ("face", Type::String),
                ("x", Type::Int),
                ("y", Type::Int),
                ("z", Type::Int),
                ("cancelled", Type::Bool),
            ],
        ),
        (
            "PlayerInteractItemEvent",
            vec![
                ("player", Type::PlayerRef),
                ("hand", Type::String),
                ("cancelled", Type::Bool),
            ],
        ),
        (
            "EntityInteractEvent",
            vec![
                ("player", Type::PlayerRef),
                ("targetId", Type::Int),
                ("hand", Type::String),
                ("secondary", Type::Bool),
                ("cancelled", Type::Bool),
            ],
        ),
        (
            "EntityAttackEvent",
            vec![
                ("player", Type::PlayerRef),
                ("targetId", Type::Int),
                ("cancelled", Type::Bool),
            ],
        ),
        (
            "ItemHeldChangeEvent",
            vec![
                ("player", Type::PlayerRef),
                ("slot", Type::Int),
                ("cancelled", Type::Bool),
            ],
        ),
        (
            "InventoryCloseEvent",
            vec![
                ("player", Type::PlayerRef),
                ("containerId", Type::Int),
                ("cancelled", Type::Bool),
            ],
        ),
        (
            "PlayerSwingEvent",
            vec![
                ("player", Type::PlayerRef),
                ("hand", Type::String),
                ("cancelled", Type::Bool),
            ],
        ),
        (
            "PlayerActionToggleEvent",
            vec![
                ("player", Type::PlayerRef),
                ("action", Type::String),
                ("entityId", Type::Int),
                ("data", Type::Int),
                ("cancelled", Type::Bool),
            ],
        ),
        (
            "ItemRenameEvent",
            vec![
                ("player", Type::PlayerRef),
                ("name", Type::String),
                ("cancelled", Type::Bool),
            ],
        ),
        (
            "TradeSelectEvent",
            vec![
                ("player", Type::PlayerRef),
                ("tradeIndex", Type::Int),
                ("cancelled", Type::Bool),
            ],
        ),
        (
            "SignChangeEvent",
            vec![
                ("player", Type::PlayerRef),
                ("x", Type::Int),
                ("y", Type::Int),
                ("z", Type::Int),
                ("front", Type::Bool),
                ("line1", Type::String),
                ("line2", Type::String),
                ("line3", Type::String),
                ("line4", Type::String),
                ("cancelled", Type::Bool),
            ],
        ),
        (
            "RecipePlaceEvent",
            vec![
                ("player", Type::PlayerRef),
                ("containerId", Type::Int),
                ("recipe", Type::String),
                ("useMaxItems", Type::Bool),
                ("cancelled", Type::Bool),
            ],
        ),
        (
            "GameModeRequestEvent",
            vec![
                ("player", Type::PlayerRef),
                ("mode", Type::String),
                ("cancelled", Type::Bool),
            ],
        ),
        (
            "HttpResponse",
            vec![
                ("ok", Type::Bool),
                ("status", Type::Int),
                ("body", Type::String),
                ("err", Type::String),
            ],
        ),
        (
            "JsonStringsResponse",
            vec![
                ("ok", Type::Bool),
                ("status", Type::Int),
                ("values", Type::Nbt),
                ("err", Type::String),
            ],
        ),
        (
            "FileResult",
            vec![("ok", Type::Bool), ("content", Type::String)],
        ),
        (
            "KvResult",
            vec![("ok", Type::Bool), ("value", Type::String)],
        ),
        ("OpResult", vec![("ok", Type::Bool)]),
        (
            "DbResult",
            vec![
                ("ok", Type::Bool),
                ("rowsAffected", Type::Int),
                ("rows", Type::Nbt),
            ],
        ),
        (
            "TimeResult",
            vec![
                ("ok", Type::Bool),
                ("unix", Type::Int),
                ("iso", Type::String),
            ],
        ),
        ("RandResult", vec![("ok", Type::Bool), ("value", Type::Int)]),
        ("PingResult", vec![("ok", Type::Bool), ("pong", Type::Bool)]),
    ]
}

/// If `expr` is a top-level host call (`module.fn(args)` where `module` is a known
/// host module and not shadowed by a local), return its parts.
fn host_call_parts<'a>(
    expr: &'a Expr,
    env: &HashMap<String, Type>,
) -> Option<(&'a str, &'a str, &'a [Expr])> {
    if let ExprKind::MethodCall {
        receiver,
        method,
        args,
    } = &expr.kind
        && let ExprKind::Variable(name) = &receiver.kind
        && is_known_host_module(name)
        && !env.contains_key(name)
    {
        return Some((name.as_str(), method.as_str(), args.as_slice()));
    }
    None
}

#[derive(Debug, Clone)]
pub struct TypedProgram {
    pub struct_defs: BTreeMap<String, StructTypeDef>,
    pub player_states: Vec<PlayerStateDef>,
    pub world_states: Vec<PlayerStateDef>,
    pub functions: Vec<TypedFunction>,
    pub function_signatures: BTreeMap<String, FunctionSignature>,
    pub call_depths: BTreeMap<String, usize>,
    /// Recursive functions to their group id; see `analyze_calls`.
    pub recursion_groups: BTreeMap<String, usize>,
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
    /// Type parameters of a generic function, which is never compiled itself.
    pub type_params: Vec<String>,
    /// Copies of a generic function that calls asked for: name to type
    /// arguments and the first call's span.
    pub instances: Arc<Mutex<BTreeMap<String, GenericCall>>>,
}

/// The type arguments of one copy of a generic function, and the first call
/// that asked for it.
pub type GenericCall = (Vec<Type>, Span);

/// Replace type parameters with the types bound to them.
fn substitute(ty: &Type, bindings: &BTreeMap<String, Type>) -> Type {
    match ty {
        Type::Struct(name) => bindings.get(name).cloned().unwrap_or_else(|| ty.clone()),
        Type::Array(inner) => Type::Array(Box::new(substitute(inner, bindings))),
        Type::Dict(inner) => Type::Dict(Box::new(substitute(inner, bindings))),
        Type::Optional(inner) => Type::Optional(Box::new(substitute(inner, bindings))),
        _ => ty.clone(),
    }
}

/// Index expressions the backend can paste into a storage path: literals,
/// variables, arithmetic, calls, and variables indexed further (`keys[order[i]]`).
/// Anything else, such as `xs[ys.size() - 1]`, needs a variable first.
fn is_simple_index(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Int(_) | ExprKind::String(_) | ExprKind::Bool(_) | ExprKind::Variable(_) => true,
        ExprKind::Unary { expr, .. } => is_simple_index(expr),
        ExprKind::Binary { left, right, .. } => is_simple_index(left) && is_simple_index(right),
        ExprKind::Call { args, .. } => args.iter().all(is_simple_index),
        ExprKind::Path(path) => matches!(path.base.kind, ExprKind::Variable(_))
            && path.segments.iter().all(
                |segment| matches!(segment, PathSegment::Index(index) if is_simple_index(index)),
            ),
        _ => false,
    }
}

/// Where `@WorldState` types live in `struct_defs`, like `@mcfc/player_state`.
const WORLD_STATE: &str = "@mcfc/world_state";
/// A world state `round` becomes the variable `@world.round`; the backend gives
/// it one global slot instead of a per-call one.
pub const WORLD_STATE_PREFIX: &str = "@world.";

fn world_state_type(struct_defs: &BTreeMap<String, StructTypeDef>, name: &str) -> Option<Type> {
    struct_defs.get(WORLD_STATE)?.fields.get(name).cloned()
}

/// Renames world state variables that no local shadows to their `@world.` form.
fn qualify_world_state(
    expr: &mut Expr,
    env: &HashMap<String, Type>,
    struct_defs: &BTreeMap<String, StructTypeDef>,
) {
    match &mut expr.kind {
        ExprKind::Variable(name) => {
            if !env.contains_key(name.as_str()) && world_state_type(struct_defs, name).is_some() {
                *name = format!("{WORLD_STATE_PREFIX}{name}");
            }
        }
        ExprKind::Unary { expr, .. } => qualify_world_state(expr, env, struct_defs),
        ExprKind::Binary { left, right, .. } => {
            qualify_world_state(left, env, struct_defs);
            qualify_world_state(right, env, struct_defs);
        }
        ExprKind::Call { args, .. } => {
            for arg in args {
                qualify_world_state(arg, env, struct_defs);
            }
        }
        ExprKind::Path(path) => {
            qualify_world_state(&mut path.base, env, struct_defs);
            for segment in &mut path.segments {
                if let PathSegment::Index(index) = segment {
                    qualify_world_state(index, env, struct_defs);
                }
            }
        }
        _ => {}
    }
}

/// Replace type parameters in the local declarations of one generic copy,
/// so `List<T> out = List.of();` gets the copy's element type.
fn substitute_body(body: &mut [Stmt], bindings: &BTreeMap<String, Type>) {
    for stmt in body {
        match &mut stmt.kind {
            StmtKind::Let { ty: Some(ty), .. } => *ty = substitute(ty, bindings),
            StmtKind::For { ty, body, .. } => {
                if let Some(ty) = ty {
                    *ty = substitute(ty, bindings);
                }
                substitute_body(body, bindings);
            }
            StmtKind::If {
                then_body,
                else_body,
                ..
            } => {
                substitute_body(then_body, bindings);
                substitute_body(else_body, bindings);
            }
            StmtKind::While { body, step, .. } => {
                substitute_body(body, bindings);
                substitute_body(step, bindings);
            }
            StmtKind::Switch {
                arms, default_body, ..
            } => {
                for arm in arms {
                    substitute_body(&mut arm.body, bindings);
                }
                substitute_body(default_body, bindings);
            }
            StmtKind::Block(body) | StmtKind::Context { body, .. } | StmtKind::Async { body } => {
                substitute_body(body, bindings)
            }
            _ => {}
        }
    }
}

/// Bind the type parameters in `param` by matching it against `arg`.
/// Returns false on a mismatch with an earlier binding. A bare `T` mixing
/// `int` and `float` becomes `float`, so `math.min(1.5, 2)` is `1.5`.
fn bind_type_params(
    param: &Type,
    arg: &Type,
    type_params: &[String],
    bindings: &mut BTreeMap<String, Type>,
    widen: bool,
) -> bool {
    match (param, arg) {
        (Type::Struct(name), _) if type_params.contains(name) => {
            let bound = bindings.entry(name.clone()).or_insert_with(|| arg.clone());
            match (&*bound, arg) {
                (Type::Int, Type::Float) if widen => {
                    *bound = Type::Float;
                    true
                }
                (Type::Float, Type::Int) => widen,
                _ => bound == arg,
            }
        }
        (Type::Array(param), Type::Array(arg))
        | (Type::Dict(param), Type::Dict(arg))
        | (Type::Optional(param), Type::Optional(arg)) => {
            bind_type_params(param, arg, type_params, bindings, false)
        }
        _ => true,
    }
}

/// The name of one copy of a generic function, such as `max__int` or
/// `first__array_float`. It must stay a valid function path.
fn instance_name(function: &str, types: &[Type]) -> String {
    let types: Vec<String> = types
        .iter()
        .map(|ty| {
            ty.as_str()
                .replace("::", "_")
                .replace(['<', '>'], "_")
                .trim_end_matches('_')
                .to_lowercase()
        })
        .collect();
    format!("{function}__{}", types.join("__"))
}

#[derive(Debug, Clone)]
pub struct StructTypeDef {
    pub fields: BTreeMap<String, Type>,
    pub enum_variants: Option<Vec<String>>,
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
    /// `step` runs after the body and on `continue` (a C-style `for` update).
    While {
        condition: TypedExpr,
        body: Vec<TypedStmt>,
        step: Vec<TypedStmt>,
    },
    For {
        name: String,
        iterable: TypedExpr,
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
    /// A suspending host-bridge call (`module.fn(args)`). When `dest` is set the
    /// result struct is bound to that local; otherwise the result is discarded.
    HostCall {
        module: String,
        function: String,
        args: Vec<TypedExpr>,
        dest: Option<String>,
        return_type: Type,
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
    Float(String),
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
    /// `condition ? then_expr : else_expr`; only the chosen branch runs.
    Conditional {
        condition: Box<TypedExpr>,
        then_expr: Box<TypedExpr>,
        else_expr: Box<TypedExpr>,
    },
    /// Evaluates `value` once into the local `name`, then evaluates `body`.
    Bind {
        name: String,
        value: Box<TypedExpr>,
        body: Box<TypedExpr>,
    },
}

#[derive(Debug, Clone, Copy)]
pub enum CastKind {
    Int,
    Float,
    Bool,
    String,
}

pub fn type_check(program: &Program, host: &HostModules) -> Result<TypedProgram, Diagnostics> {
    let mut diagnostics = Diagnostics::new();
    let mut struct_defs = BTreeMap::new();
    let mut signatures = BTreeMap::new();

    // Register builtin host-call response structs so field access type-checks.
    // Only when a helper is configured, to avoid reserving these names otherwise.
    if host.any_enabled() {
        for (name, fields) in builtin_response_structs() {
            let mut field_map = BTreeMap::new();
            for (field, ty) in fields {
                field_map.insert(field.to_string(), ty);
            }
            struct_defs.insert(
                name.to_string(),
                StructTypeDef {
                    fields: field_map,
                    enum_variants: None,
                },
            );
        }
        for kind in GENERIC_AGENT_EVENTS {
            let fields = [
                ("player", Type::PlayerRef),
                ("playerName", Type::String),
                ("source", Type::String),
                ("payload", Type::String),
                ("cancelled", Type::Bool),
            ];
            struct_defs.insert(
                event_type_name(kind),
                StructTypeDef {
                    fields: fields.map(|(f, ty)| (f.to_string(), ty)).into(),
                    enum_variants: None,
                },
            );
        }
    }
    // `@EventHandler void onJoin(PlayerJoinEvent event)`: vanilla events carry the player.
    for kind in VANILLA_EVENTS {
        let mut fields = BTreeMap::from([("player".to_string(), Type::PlayerRef)]);
        if crate::language_catalog::vanilla_event_has_entity(kind) {
            fields.insert("entity".to_string(), Type::EntityRef);
        }
        if crate::language_catalog::vanilla_event_has_block(kind) {
            fields.insert(
                "block".to_string(),
                Type::Optional(Box::new(Type::BlockRef)),
            );
        }
        struct_defs.insert(
            event_type_name(kind),
            StructTypeDef {
                fields,
                enum_variants: None,
            },
        );
    }

    for struct_def in &program.structs {
        let mut fields = BTreeMap::new();
        if struct_defs.contains_key(&struct_def.name) {
            diagnostics.push(Diagnostic::new(
                format!("duplicate record '{}'", struct_def.name),
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
        struct_defs.insert(
            struct_def.name.clone(),
            StructTypeDef {
                fields,
                enum_variants: None,
            },
        );
    }

    for enum_def in &program.enums {
        if struct_defs.contains_key(&enum_def.name) {
            diagnostics.push(Diagnostic::new(
                format!("duplicate type '{}'", enum_def.name),
                enum_def.span.clone(),
            ));
            continue;
        }
        let mut seen = BTreeSet::new();
        for variant in &enum_def.variants {
            if !seen.insert(variant) {
                diagnostics.push(Diagnostic::new(
                    format!("duplicate enum constant '{}.{}'", enum_def.name, variant),
                    enum_def.span.clone(),
                ));
            }
        }
        struct_defs.insert(
            enum_def.name.clone(),
            StructTypeDef {
                fields: BTreeMap::new(),
                enum_variants: Some(enum_def.variants.clone()),
            },
        );
    }
    let mut normalized = program.clone();
    for state in &mut normalized.player_states {
        resolve_enum_type(&mut state.ty, &struct_defs);
    }
    for def in &mut normalized.structs {
        for field in &mut def.fields {
            resolve_enum_type(&mut field.ty, &struct_defs);
        }
    }
    for function in &mut normalized.functions {
        for param in &mut function.params {
            resolve_enum_type(&mut param.ty, &struct_defs);
        }
        resolve_enum_type(&mut function.return_type, &struct_defs);
    }
    for def in &normalized.structs {
        if let Some(registered) = struct_defs.get_mut(&def.name) {
            registered.fields = def
                .fields
                .iter()
                .map(|field| (field.name.clone(), field.ty.clone()))
                .collect();
        }
    }
    let program = &normalized;

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
    let mut entity_state_names = HashSet::new();
    let mut player_state_types = BTreeMap::new();
    let mut entity_state_types = BTreeMap::new();
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
        if !matches!(
            state.ty,
            Type::Int
                | Type::Bool
                | Type::String
                | Type::Float
                | Type::Struct(_)
                | Type::Dict(_)
                | Type::EntityRef
                | Type::PlayerRef
        ) {
            diagnostics.push(Diagnostic::new(
                "state declarations support 'int', 'boolean', 'String', 'float', 'Entity', 'Player', records and maps",
                state.span.clone(),
            ));
        }
        validate_declared_type(
            &state.ty,
            &struct_defs,
            state.span.clone(),
            &mut diagnostics,
        );
        let (names, types, kind) = if state.owner == StateOwner::Player {
            (
                &mut player_state_names,
                &mut player_state_types,
                "player_state",
            )
        } else {
            (
                &mut entity_state_names,
                &mut entity_state_types,
                "entity_state",
            )
        };
        let path_name = state.path.join(".");
        if names.iter().any(|existing: &String| {
            existing.starts_with(&format!("{path_name}."))
                || path_name.starts_with(&format!("{existing}."))
        }) {
            diagnostics.push(Diagnostic::new(
                format!("overlapping {kind} '{path_name}'"),
                state.span.clone(),
            ));
        }
        if !names.insert(path_name.clone()) {
            diagnostics.push(Diagnostic::new(
                format!("duplicate {kind} '{path_name}'"),
                state.span.clone(),
            ));
        }
        types.insert(path_name, state.ty.clone());
    }
    let mut world_state_types = BTreeMap::new();
    for state in &program.world_states {
        if !matches!(
            state.ty,
            Type::Int
                | Type::Bool
                | Type::String
                | Type::Float
                | Type::Struct(_)
                | Type::Dict(_)
                | Type::Array(_)
                | Type::EntityRef
                | Type::PlayerRef
        ) {
            diagnostics.push(Diagnostic::new(
                "@WorldState supports 'int', 'boolean', 'String', 'float', 'Entity', 'Player', records, lists and maps",
                state.span.clone(),
            ));
        }
        validate_declared_type(
            &state.ty,
            &struct_defs,
            state.span.clone(),
            &mut diagnostics,
        );
        let name = state.path.join(".");
        if world_state_types
            .insert(name.clone(), state.ty.clone())
            .is_some()
        {
            diagnostics.push(Diagnostic::new(
                format!("duplicate @WorldState '{name}'"),
                state.span.clone(),
            ));
        }
    }
    struct_defs.insert(
        WORLD_STATE.to_string(),
        StructTypeDef {
            fields: world_state_types,
            enum_variants: None,
        },
    );
    struct_defs.insert(
        "@mcfc/player_state".to_string(),
        StructTypeDef {
            fields: player_state_types,
            enum_variants: None,
        },
    );
    struct_defs.insert(
        "@mcfc/entity_state".to_string(),
        StructTypeDef {
            fields: entity_state_types,
            enum_variants: None,
        },
    );

    for function in &program.functions {
        // Type parameters stand in for any type, so validate them as `int`.
        let placeholders: BTreeMap<String, Type> = function
            .type_params
            .iter()
            .map(|name| (name.clone(), Type::Int))
            .collect();
        for param in &function.params {
            validate_declared_type(
                &substitute(&param.ty, &placeholders),
                &struct_defs,
                param.span.clone(),
                &mut diagnostics,
            );
        }
        validate_declared_type(
            &substitute(&function.return_type, &placeholders),
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
                type_params: function.type_params.clone(),
                instances: Arc::default(),
            },
        );
    }

    let mut functions = Vec::new();
    for function in &program.functions {
        if function.type_params.is_empty() {
            functions.push(type_check_function(
                function,
                &struct_defs,
                &signatures,
                host,
                &mut diagnostics,
            ));
        }
    }

    // Compile each copy of a generic function that a call asked for. A copy
    // can call more generic functions, so repeat until nothing new appears.
    let generics: BTreeMap<&str, &Function> = program
        .functions
        .iter()
        .filter(|function| !function.type_params.is_empty())
        .map(|function| (function.name.as_str(), function))
        .collect();
    let mut compiled = BTreeSet::new();
    loop {
        let pending: Vec<(String, &Function, GenericCall)> = generics
            .values()
            .flat_map(|generic| {
                signatures[&generic.name]
                    .instances
                    .lock()
                    .unwrap()
                    .iter()
                    .map(|(name, call)| (name.clone(), *generic, call.clone()))
                    .collect::<Vec<_>>()
            })
            .filter(|(name, _, _)| !compiled.contains(name))
            .collect();
        if pending.is_empty() {
            break;
        }
        for (name, generic, (types, call)) in pending {
            compiled.insert(name.clone());
            let bindings: BTreeMap<String, Type> =
                generic.type_params.iter().cloned().zip(types).collect();
            let errors_before = diagnostics.0.len();
            let mut instance = generic.clone();
            instance.name = name.clone();
            instance.type_params.clear();
            for param in &mut instance.params {
                param.ty = substitute(&param.ty, &bindings);
            }
            instance.return_type = substitute(&generic.return_type, &bindings);
            substitute_body(&mut instance.body, &bindings);
            signatures.insert(
                name,
                FunctionSignature {
                    params: instance
                        .params
                        .iter()
                        .map(|param| param.ty.clone())
                        .collect(),
                    return_type: instance.return_type.clone(),
                    type_params: Vec::new(),
                    instances: Arc::default(),
                },
            );
            functions.push(type_check_function(
                &instance,
                &struct_defs,
                &signatures,
                host,
                &mut diagnostics,
            ));
            if diagnostics.0.len() > errors_before {
                let types: Vec<String> = bindings
                    .iter()
                    .map(|(param, ty)| format!("{param} = {}", ty.as_str()))
                    .collect();
                diagnostics.push(Diagnostic::new(
                    format!("'{}' does not work with {}", generic.name, types.join(", ")),
                    call,
                ));
            }
        }
    }

    let (call_depths, recursion_groups) = analyze_calls(&functions);

    diagnostics.into_result(TypedProgram {
        struct_defs,
        functions,
        function_signatures: signatures,
        player_states: program.player_states.clone(),
        world_states: program.world_states.clone(),
        call_depths,
        recursion_groups,
    })
}

fn type_check_function(
    function: &Function,
    struct_defs: &BTreeMap<String, StructTypeDef>,
    signatures: &BTreeMap<String, FunctionSignature>,
    host: &HostModules,
    diagnostics: &mut Diagnostics,
) -> TypedFunction {
    {
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
            struct_defs,
            signatures,
            &mut env,
            &mut ref_env,
            &mut locals,
            &mut called_functions,
            0,
            false,
            host,
            diagnostics,
        );

        TypedFunction {
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
        }
    }
}

/// Type-check a host call (`module.fn(args)`) appearing in statement position.
/// Returns the typed statement and the call's result type (for binding in a let).
#[allow(clippy::too_many_arguments)]
fn type_check_host_call(
    module: &str,
    function: &str,
    call_args: &[Expr],
    dest: Option<String>,
    span: Span,
    struct_defs: &BTreeMap<String, StructTypeDef>,
    signatures: &BTreeMap<String, FunctionSignature>,
    env: &HashMap<String, Type>,
    ref_env: &HashMap<String, RefKind>,
    called_functions: &mut BTreeSet<String>,
    host: &HostModules,
    diagnostics: &mut Diagnostics,
) -> (TypedStmtKind, Type) {
    let mut args = type_check_args(
        call_args,
        struct_defs,
        signatures,
        env,
        ref_env,
        called_functions,
        diagnostics,
    );

    if !host.is_enabled(module) {
        diagnostics.push(Diagnostic::new(
            format!(
                "host module '{}' is not enabled in [helper.capabilities]",
                module
            ),
            span.clone(),
        ));
    }

    let return_type = if let Some(sig) = host_call_signature(module, function) {
        if sig.params.len() != args.len() {
            diagnostics.push(Diagnostic::new(
                format!(
                    "wrong arity for '{}.{}': expected {}, found {}",
                    module,
                    function,
                    sig.params.len(),
                    args.len()
                ),
                span.clone(),
            ));
        }
        for (index, expected) in sig.params.iter().enumerate() {
            if let Some(arg) = args.get_mut(index) {
                let coerced = coerce_expr_to_expected_type(arg.clone(), expected);
                if &coerced.ty != expected {
                    diagnostics.push(Diagnostic::new(
                        format!(
                            "{}.{}(...) argument {} must be '{}', found '{}'",
                            module,
                            function,
                            index + 1,
                            expected.as_str(),
                            coerced.ty.as_str()
                        ),
                        span.clone(),
                    ));
                }
                *arg = coerced;
            }
        }
        sig.return_type
    } else {
        diagnostics.push(Diagnostic::new(
            format!("unknown host function '{}.{}'", module, function),
            span.clone(),
        ));
        Type::Void
    };

    (
        TypedStmtKind::HostCall {
            module: module.to_string(),
            function: function.to_string(),
            args,
            dest,
            return_type: return_type.clone(),
        },
        return_type,
    )
}

fn check_declared_type(
    name: &str,
    declared: Option<&Type>,
    found: &Type,
    span: &Span,
    diagnostics: &mut Diagnostics,
) {
    // A declared local's type isn't resolved, so an enum still reads as a record name.
    let same_enum = matches!((declared, found), (Some(Type::Struct(a)), Type::Enum(b)) if a == b);
    if let Some(declared) = declared
        && declared != found
        && !same_enum
    {
        diagnostics.push(Diagnostic::new(
            format!(
                "variable '{}' is declared '{}' but its value is '{}'",
                name,
                declared.as_str(),
                found.as_str()
            ),
            span.clone(),
        ));
    }
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
    host: &HostModules,
    diagnostics: &mut Diagnostics,
) -> Vec<TypedStmt> {
    let mut typed = Vec::new();

    for statement in statements {
        let kind = match &statement.kind {
            StmtKind::Block(body) => {
                typed.extend(type_check_block(
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
                    host,
                    diagnostics,
                ));
                continue;
            }
            StmtKind::Let { name, ty, value } => {
                if env.contains_key(name) {
                    diagnostics.push(Diagnostic::new(
                        format!("variable '{}' is already defined", name),
                        statement.span.clone(),
                    ));
                }
                if let Some((module, function, call_args)) = host_call_parts(value, env) {
                    let (kind, return_type) = type_check_host_call(
                        module,
                        function,
                        call_args,
                        Some(name.clone()),
                        statement.span.clone(),
                        struct_defs,
                        signatures,
                        env,
                        ref_env,
                        called_functions,
                        host,
                        diagnostics,
                    );
                    check_declared_type(
                        name,
                        ty.as_ref(),
                        &return_type,
                        &statement.span,
                        diagnostics,
                    );
                    env.insert(name.clone(), return_type.clone());
                    ref_env.insert(name.clone(), RefKind::Unknown);
                    locals.insert(name.clone(), return_type);
                    kind
                } else {
                    // `List<Integer> xs = List.of();` takes the element type from the declaration.
                    let empty_literal = match (ty, &value.kind) {
                        (Some(Type::Array(_)), ExprKind::ArrayLiteral(items))
                            if items.is_empty() =>
                        {
                            Some(TypedExprKind::ArrayLiteral(Vec::new()))
                        }
                        (Some(Type::Dict(_)), ExprKind::DictLiteral(entries))
                            if entries.is_empty() =>
                        {
                            Some(TypedExprKind::DictLiteral(Vec::new()))
                        }
                        _ => None,
                    };
                    let mut value = match (empty_literal, ty) {
                        (Some(kind), Some(ty)) => TypedExpr {
                            kind,
                            ty: ty.clone(),
                            ref_kind: RefKind::Unknown,
                        },
                        _ => type_check_expr(
                            value,
                            struct_defs,
                            signatures,
                            env,
                            ref_env,
                            called_functions,
                            diagnostics,
                        ),
                    };
                    if let Some(ty) = ty {
                        value = coerce_expr_to_expected_type(value, ty);
                        check_declared_type(
                            name,
                            Some(ty),
                            &value.ty,
                            &statement.span,
                            diagnostics,
                        );
                        value.ty = ty.clone();
                    }
                    env.insert(name.clone(), value.ty.clone());
                    ref_env.insert(name.clone(), value.ref_kind);
                    locals.insert(name.clone(), value.ty.clone());
                    TypedStmtKind::Let {
                        name: name.clone(),
                        ty: value.ty.clone(),
                        value,
                    }
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
                        let (slot_name, existing) = match env.get(name) {
                            Some(ty) => (name.clone(), Some(ty.clone())),
                            None => (
                                format!("{WORLD_STATE_PREFIX}{name}"),
                                world_state_type(struct_defs, name),
                            ),
                        };
                        let Some(existing) = existing else {
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
                        TypedAssignTarget::Variable(slot_name)
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
                        if !LOWERING_SETTER.get() {
                            check_property_syntax(
                                &typed_path.base.ty,
                                &path.segments,
                                true,
                                statement.span.clone(),
                                diagnostics,
                            );
                        }
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
                            let typed_state_write = matches!(
                                typed_path.segments.first(),
                                Some(PathSegment::Field(name)) if name == "state"
                            ) && typed_path
                                .segment_types
                                .iter()
                                .skip(1)
                                .any(|ty| *ty != Type::Nbt);
                            if !matches!(
                                value.ty,
                                Type::Int | Type::Bool | Type::String | Type::Nbt | Type::TextDef
                            ) && !(typed_state_write
                                && matches!(
                                    value.ty,
                                    Type::Float
                                        | Type::Struct(_)
                                        | Type::Dict(_)
                                        | Type::EntityRef
                                        | Type::PlayerRef
                                ))
                                && !(is_player_slot_write && value.ty == Type::ItemDef)
                                && !(is_equipment_item_write && value.ty == Type::ItemDef)
                            {
                                diagnostics.push(Diagnostic::new(
                                    "path assignment requires a value of type 'int', 'boolean', 'String', 'Nbt', or an item builder for inventory slots",
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
                                "path assignment requires an 'Entity', 'Block', bossbar, or storage-backed base",
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
                        "if condition must have type 'boolean'",
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
                    host,
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
                    host,
                    diagnostics,
                );
                TypedStmtKind::If {
                    condition,
                    then_body,
                    else_body,
                }
            }
            StmtKind::While {
                condition,
                body,
                step,
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
                        "while condition must have type 'boolean'",
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
                    host,
                    diagnostics,
                );
                let step = type_check_block(
                    step,
                    return_type,
                    struct_defs,
                    signatures,
                    &mut env.clone(),
                    &mut ref_env.clone(),
                    locals,
                    called_functions,
                    loop_depth,
                    in_async,
                    host,
                    diagnostics,
                );
                TypedStmtKind::While {
                    condition,
                    body,
                    step,
                }
            }
            StmtKind::For {
                name,
                ty,
                iterable,
                body,
            } => {
                if env.contains_key(name) {
                    diagnostics.push(Diagnostic::new(
                        format!("variable '{}' is already defined", name),
                        statement.span.clone(),
                    ));
                }
                let mut loop_env = env.clone();
                let mut loop_ref_env = ref_env.clone();
                let iterable = type_check_expr(
                    iterable,
                    struct_defs,
                    signatures,
                    env,
                    ref_env,
                    called_functions,
                    diagnostics,
                );
                let (mut item_ty, mut item_ref_kind) = match &iterable.ty {
                    Type::EntitySet => (Type::EntityRef, iterable.ref_kind),
                    Type::Array(element) => (*element.clone(), RefKind::Unknown),
                    _ => {
                        diagnostics.push(Diagnostic::new(
                            "for-each iteration requires a 'Selector' or 'List'",
                            statement.span.clone(),
                        ));
                        (Type::Nbt, RefKind::Unknown)
                    }
                };
                match ty {
                    // `for (Player p : selector)` asserts players, like a `(Player)` cast.
                    Some(Type::PlayerRef) if item_ty == Type::EntityRef => {
                        if item_ref_kind == RefKind::NonPlayer {
                            diagnostics.push(Diagnostic::new(
                                "this selector never matches players",
                                statement.span.clone(),
                            ));
                        }
                        item_ty = Type::PlayerRef;
                        item_ref_kind = RefKind::Player;
                    }
                    _ => check_declared_type(
                        name,
                        ty.as_ref(),
                        &item_ty,
                        &statement.span,
                        diagnostics,
                    ),
                }
                loop_env.insert(name.clone(), item_ty.clone());
                loop_ref_env.insert(name.clone(), item_ref_kind);
                locals.insert(name.clone(), item_ty);
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
                    host,
                    diagnostics,
                );
                TypedStmtKind::For {
                    name: name.clone(),
                    iterable,
                    body,
                }
            }
            StmtKind::Switch {
                value,
                arms,
                default_body,
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
                if !matches!(value.ty, Type::Enum(_) | Type::Int | Type::String) {
                    diagnostics.push(Diagnostic::new(
                        "switch value must be an enum, int, or String",
                        statement.span.clone(),
                    ));
                }
                let mut seen = BTreeSet::new();
                let mut typed_arms = Vec::new();
                for arm in arms {
                    // Like Java, enum cases name the bare constant: `case SURVIVAL`.
                    let bare_constant = match (&value.ty, &arm.pattern.kind) {
                        (Type::Enum(enum_name), ExprKind::Variable(constant))
                            if !env.contains_key(constant) =>
                        {
                            Some(Expr {
                                kind: ExprKind::Path(PathExpr {
                                    base: Box::new(Expr {
                                        kind: ExprKind::Variable(enum_name.clone()),
                                        span: arm.pattern.span.clone(),
                                    }),
                                    segments: vec![PathSegment::Field(constant.clone())],
                                }),
                                span: arm.pattern.span.clone(),
                            })
                        }
                        _ => None,
                    };
                    let pattern = type_check_expr(
                        bare_constant.as_ref().unwrap_or(&arm.pattern),
                        struct_defs,
                        signatures,
                        env,
                        ref_env,
                        called_functions,
                        diagnostics,
                    );
                    if pattern.ty != value.ty
                        || !matches!(
                            pattern.kind,
                            TypedExprKind::Int(_) | TypedExprKind::String(_)
                        )
                    {
                        diagnostics.push(Diagnostic::new(
                            "case must be a constant of the switch value's type",
                            arm.pattern.span.clone(),
                        ));
                    }
                    let key = format!("{:?}", pattern.kind);
                    if !seen.insert(key) {
                        diagnostics.push(Diagnostic::new(
                            "duplicate switch case",
                            arm.pattern.span.clone(),
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
                        host,
                        diagnostics,
                    );
                    typed_arms.push((pattern, body));
                }
                if let Type::Enum(name) = &value.ty
                    && default_body.is_empty()
                    && let Some(variants) = struct_defs
                        .get(name)
                        .and_then(|def| def.enum_variants.as_ref())
                {
                    let missing: Vec<_> = (0..variants.len()).filter(|index| !typed_arms.iter().any(|(pattern, _)| matches!(pattern.kind, TypedExprKind::Int(value) if value == *index as i64))).map(|index| variants[index].as_str()).collect();
                    if !missing.is_empty() {
                        diagnostics.push(Diagnostic::new(
                            format!(
                                "non-exhaustive switch on '{}': missing {}",
                                name,
                                missing.join(", ")
                            ),
                            statement.span.clone(),
                        ));
                    }
                }
                let default_body = type_check_block(
                    default_body,
                    return_type,
                    struct_defs,
                    signatures,
                    &mut env.clone(),
                    &mut ref_env.clone(),
                    locals,
                    called_functions,
                    loop_depth,
                    in_async,
                    host,
                    diagnostics,
                );
                let temp_name = format!("__switch_{}", statement.span.line);
                if switch_needs_temp(&value) {
                    locals.insert(temp_name.clone(), value.ty.clone());
                }
                lower_switch_stmt(value, typed_arms, default_body, temp_name)
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
                            "{} context block requires a 'Selector' or 'Entity' anchor",
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
                    host,
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
                    host,
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
            // `bb.setMax(10);` is the assignment `bb.max = 10;`.
            StmtKind::Expr(Expr {
                kind:
                    ExprKind::MethodCall {
                        receiver,
                        method,
                        args,
                    },
                ..
            }) if args.len() == 1
                && let ExprKind::Variable(name) = &receiver.kind
                && let Some(ty) = env.get(name)
                && let Some(property) = accessor_property(method, "set")
                && property_names(ty).contains(&property.as_str()) =>
            {
                let assign = Stmt {
                    kind: StmtKind::Assign {
                        target: AssignTarget::Path(PathExpr {
                            base: receiver.clone(),
                            segments: vec![PathSegment::Field(property)],
                        }),
                        value: args[0].clone(),
                    },
                    span: statement.span.clone(),
                };
                LOWERING_SETTER.set(true);
                let lowered = type_check_block(
                    &[assign],
                    return_type,
                    struct_defs,
                    signatures,
                    env,
                    ref_env,
                    locals,
                    called_functions,
                    loop_depth,
                    in_async,
                    host,
                    diagnostics,
                );
                LOWERING_SETTER.set(false);
                typed.extend(lowered);
                continue;
            }
            // `xs.set(i, v);` and `m.put(k, v);` are `xs[i] = v;` and `m[k] = v;`.
            StmtKind::Expr(Expr {
                kind:
                    ExprKind::MethodCall {
                        receiver,
                        method,
                        args,
                    },
                ..
            }) if matches!(method.as_str(), "set" | "put")
                && args.len() == 2
                && matches!(receiver.kind, ExprKind::Variable(_) | ExprKind::Path(_)) =>
            {
                let index = PathSegment::Index(Box::new(args[0].clone()));
                let path = match &receiver.kind {
                    ExprKind::Path(path) => {
                        let mut path = path.clone();
                        path.segments.push(index);
                        path
                    }
                    _ => PathExpr {
                        base: receiver.clone(),
                        segments: vec![index],
                    },
                };
                let assign = Stmt {
                    kind: StmtKind::Assign {
                        target: AssignTarget::Path(path),
                        value: args[1].clone(),
                    },
                    span: statement.span.clone(),
                };
                typed.extend(type_check_block(
                    &[assign],
                    return_type,
                    struct_defs,
                    signatures,
                    env,
                    ref_env,
                    locals,
                    called_functions,
                    loop_depth,
                    in_async,
                    host,
                    diagnostics,
                ));
                continue;
            }
            StmtKind::Expr(expr) => {
                if let Some((module, function, call_args)) = host_call_parts(expr, env) {
                    let (kind, _return_type) = type_check_host_call(
                        module,
                        function,
                        call_args,
                        None,
                        statement.span.clone(),
                        struct_defs,
                        signatures,
                        env,
                        ref_env,
                        called_functions,
                        host,
                        diagnostics,
                    );
                    kind
                } else if let ExprKind::Call { function, args } = &expr.kind {
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
                                    "sleepTicks(...) duration must have type 'int'".to_string()
                                };
                                diagnostics.push(Diagnostic::new(message, statement.span.clone()));
                            }
                            if matches!(duration.kind, TypedExprKind::Int(value) if value < 1) {
                                let message = if function == "sleep" {
                                    "sleep(...) seconds must be at least 1".to_string()
                                } else {
                                    "sleepTicks(...) duration must be at least 1".to_string()
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
        ExprKind::Float(value) => TypedExpr {
            kind: TypedExprKind::Float(value.clone()),
            ty: Type::Float,
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
                    // Text reads like Java's `+`: `true`, not `1`; `DONE`, not `2`.
                    placeholders: collect_macro_placeholders(
                        value,
                        struct_defs,
                        signatures,
                        env,
                        ref_env,
                        called_functions,
                        expr.span.clone(),
                        diagnostics,
                    )
                    .into_iter()
                    .map(|mut placeholder| {
                        if matches!(placeholder.ty, Type::Bool | Type::Enum(_)) {
                            placeholder.expr = string_operand(
                                placeholder.expr,
                                struct_defs,
                                expr.span.clone(),
                                diagnostics,
                            );
                            placeholder.ty = Type::String;
                        }
                        placeholder
                    })
                    .collect(),
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
                "List.of(...) values must all have one type",
                "an empty List.of() needs a declared type, like 'List<Integer> xs = List.of();'",
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
                "Map.of(...) values must all have one type",
                "an empty Map.of() needs a declared type, like 'Map<String, int> m = Map.of();'",
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
        // Records are rewritten to struct literals during module resolution.
        ExprKind::New { name, .. } => {
            diagnostics.push(Diagnostic::new(
                format!("unknown type '{}'", name.replace("::", ".")),
                expr.span.clone(),
            ));
            TypedExpr {
                kind: TypedExprKind::Variable("_error".to_string()),
                ty: Type::Nbt,
                ref_kind: RefKind::Unknown,
            }
        }
        ExprKind::StructLiteral { name, fields } => {
            let Some(def) = struct_defs.get(name) else {
                diagnostics.push(Diagnostic::new(
                    format!("unknown record '{}'", name),
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
            if let (ExprKind::Variable(enum_name), [PathSegment::Field(variant)]) =
                (&path.base.kind, path.segments.as_slice())
                && let Some(variants) = struct_defs
                    .get(enum_name)
                    .and_then(|def| def.enum_variants.as_ref())
            {
                if let Some(index) = variants.iter().position(|name| name == variant) {
                    return TypedExpr {
                        kind: TypedExprKind::Int(index as i64),
                        ty: Type::Enum(enum_name.clone()),
                        ref_kind: RefKind::Unknown,
                    };
                }
                diagnostics.push(Diagnostic::new(
                    format!("unknown enum constant '{}.{}'", enum_name, variant),
                    expr.span.clone(),
                ));
                return TypedExpr {
                    kind: TypedExprKind::Int(0),
                    ty: Type::Enum(enum_name.clone()),
                    ref_kind: RefKind::Unknown,
                };
            }
            let ast_segments = &path.segments;
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
            check_property_syntax(
                &path.base.ty,
                ast_segments,
                false,
                expr.span.clone(),
                diagnostics,
            );
            let ref_kind = if path.ty == Type::PlayerRef {
                RefKind::Player
            } else {
                RefKind::Unknown
            };
            TypedExpr {
                ty: path.ty.clone(),
                kind: TypedExprKind::Path(path),
                // Struct-backed event payloads can expose a known player ref
                // (for example `event.player`). Preserve that fact through a
                // path expression so player state/inventory APIs remain valid.
                ref_kind,
            }
        }
        ExprKind::Variable(name) => match env.get(name) {
            Some(ty) => TypedExpr {
                kind: TypedExprKind::Variable(name.clone()),
                ty: ty.clone(),
                ref_kind: ref_env.get(name).copied().unwrap_or(RefKind::Unknown),
            },
            None if world_state_type(struct_defs, name).is_some() => TypedExpr {
                kind: TypedExprKind::Variable(format!("{WORLD_STATE_PREFIX}{name}")),
                ty: world_state_type(struct_defs, name).unwrap(),
                ref_kind: RefKind::Unknown,
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
            let operand = match op {
                UnaryOp::Not => coerce_expr_to_expected_type(
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
                ),
                UnaryOp::Neg => {
                    let operand = type_check_expr(
                        expr,
                        struct_defs,
                        signatures,
                        env,
                        ref_env,
                        called_functions,
                        diagnostics,
                    );
                    if operand.ty == Type::Float {
                        operand
                    } else {
                        coerce_expr_to_expected_type(operand, &Type::Int)
                    }
                }
            };
            let ty = match op {
                UnaryOp::Not => {
                    if operand.ty != Type::Bool {
                        diagnostics.push(Diagnostic::new(
                            "'!' requires a 'boolean' operand",
                            expr.span.clone(),
                        ));
                    }
                    Type::Bool
                }
                UnaryOp::Neg if operand.ty == Type::Float => Type::Float,
                UnaryOp::Neg => {
                    if operand.ty != Type::Int {
                        diagnostics.push(Diagnostic::new(
                            "'-' requires an 'int' operand",
                            expr.span.clone(),
                        ));
                    }
                    Type::Int
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
            if *op == BinaryOp::Add && (left.ty == Type::String || right.ty == Type::String) {
                // Like Java, `"n=" + n` converts the other side to text.
                let left = string_operand(left, struct_defs, expr.span.clone(), diagnostics);
                let right = string_operand(right, struct_defs, expr.span.clone(), diagnostics);
                return concat_strings(vec![left, right]);
            }
            let ty = match op {
                BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Rem
                    if left.ty == Type::Float || right.ty == Type::Float =>
                {
                    left = coerce_expr_to_expected_type(left, &Type::Float);
                    right = coerce_expr_to_expected_type(right, &Type::Float);
                    if left.ty != right.ty {
                        diagnostics.push(Diagnostic::new(
                            "arithmetic operators require 'int' or 'float' operands",
                            expr.span.clone(),
                        ));
                    }
                    Type::Float
                }
                BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Rem => {
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
                BinaryOp::BitAnd
                | BinaryOp::BitOr
                | BinaryOp::BitXor
                | BinaryOp::Shl
                | BinaryOp::Shr => {
                    left = coerce_expr_to_expected_type(left, &Type::Int);
                    right = coerce_expr_to_expected_type(right, &Type::Int);
                    if left.ty != Type::Int || right.ty != Type::Int {
                        diagnostics.push(Diagnostic::new(
                            "bitwise operators require 'int' operands",
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
                            "logical operators require 'boolean' operands",
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
                            if !matches!(
                                left.ty,
                                Type::Int | Type::Float | Type::Bool | Type::String | Type::Enum(_)
                            ) {
                                diagnostics.push(Diagnostic::new(
                                    "equality operators currently support only 'int', 'float', 'boolean', and 'String'",
                                    expr.span.clone(),
                                ));
                            }
                        }
                        _ => {
                            if !matches!(left.ty, Type::Int | Type::Float | Type::Bool) {
                                diagnostics.push(Diagnostic::new(
                                    "ordering comparisons currently support only 'int', 'float', and 'boolean'",
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
            // Host calls (`http.get(...)`) are only valid in statement/let position;
            // reaching here means one was nested inside an expression.
            if let ExprKind::Variable(name) = &receiver.kind
                && is_known_host_module(name)
                && !env.contains_key(name)
            {
                for arg in args {
                    type_check_expr(
                        arg,
                        struct_defs,
                        signatures,
                        env,
                        ref_env,
                        called_functions,
                        diagnostics,
                    );
                }
                diagnostics.push(Diagnostic::new(
                        format!(
                            "host call '{}.{}' may only appear as a standalone statement or a variable initializer",
                            name, method
                        ),
                        expr.span.clone(),
                    ));
                return TypedExpr {
                    kind: TypedExprKind::Int(0),
                    ty: Type::Void,
                    ref_kind: RefKind::Unknown,
                };
            }
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
        ExprKind::Conditional {
            condition,
            then_expr,
            else_expr,
        } => {
            let check =
                |expr: &Expr, diagnostics: &mut Diagnostics, called: &mut BTreeSet<String>| {
                    type_check_expr(
                        expr,
                        struct_defs,
                        signatures,
                        env,
                        ref_env,
                        called,
                        diagnostics,
                    )
                };
            let condition = coerce_expr_to_expected_type(
                check(condition, diagnostics, called_functions),
                &Type::Bool,
            );
            if condition.ty != Type::Bool {
                diagnostics.push(Diagnostic::new(
                    format!(
                        "the condition of '?:' must be 'boolean', found '{}'",
                        condition.ty.as_str()
                    ),
                    expr.span.clone(),
                ));
            }
            let then_expr = check(then_expr, diagnostics, called_functions);
            let else_expr = check(else_expr, diagnostics, called_functions);
            let (then_expr, else_expr) =
                unify_branches(then_expr, else_expr, expr.span.clone(), diagnostics);
            conditional_expr(condition, then_expr, else_expr)
        }
        ExprKind::Switch {
            value,
            arms,
            default,
        } => type_check_switch_expr(
            expr,
            value,
            arms,
            default.as_deref(),
            struct_defs,
            signatures,
            env,
            ref_env,
            called_functions,
            diagnostics,
        ),
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
                        display_call(function),
                        signature.params.len(),
                        args.len()
                    ),
                    expr.span.clone(),
                ));
            }

            let args: Vec<_> = args
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
                .collect();
            let (function, params, return_type) = if signature.type_params.is_empty() {
                (
                    function.clone(),
                    signature.params.clone(),
                    signature.return_type.clone(),
                )
            } else {
                let mut bindings = BTreeMap::new();
                for (param, arg) in signature.params.iter().zip(&args) {
                    if !bind_type_params(
                        param,
                        &arg.ty,
                        &signature.type_params,
                        &mut bindings,
                        true,
                    ) {
                        diagnostics.push(Diagnostic::new(
                            format!(
                                "arguments for '{function}' give its type parameters different types"
                            ),
                            expr.span.clone(),
                        ));
                    }
                }
                let mut types = Vec::new();
                for name in &signature.type_params {
                    match bindings.get(name) {
                        Some(ty) => types.push(ty.clone()),
                        None => {
                            diagnostics.push(Diagnostic::new(
                                format!(
                                    "cannot tell what '{name}' is in this call to '{function}'; use it in a parameter"
                                ),
                                expr.span.clone(),
                            ));
                            types.push(Type::Void);
                        }
                    }
                }
                let instance = instance_name(function, &types);
                signature
                    .instances
                    .lock()
                    .unwrap()
                    .entry(instance.clone())
                    .or_insert((types, expr.span.clone()));
                (
                    instance,
                    signature
                        .params
                        .iter()
                        .map(|param| substitute(param, &bindings))
                        .collect(),
                    substitute(&signature.return_type, &bindings),
                )
            };
            let args: Vec<_> = args
                .into_iter()
                .enumerate()
                .map(|(index, typed)| match params.get(index) {
                    Some(expected) => coerce_expr_to_expected_type(typed, expected),
                    None => typed,
                })
                .collect();
            for (index, arg) in args.iter().enumerate() {
                if let Some(expected) = params.get(index)
                    && expected != &arg.ty
                {
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

            // `std.math.sin/cos/tan` are `/compute` providers: inline them so they
            // fuse into the surrounding float expression instead of costing a call.
            if let Some(name) = function.strip_prefix("std::math::")
                && matches!(name, "sin" | "cos" | "tan")
                && let [arg] = args.as_slice()
            {
                return method_call_expr(arg.clone(), name, Vec::new(), Type::Float);
            }
            called_functions.insert(function.clone());
            TypedExpr {
                kind: TypedExprKind::Call { function, args },
                ty: return_type,
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
    let mut segments =
        normalize_builder_path_segments(&base.ty, &path.segments, span.clone(), diagnostics);
    let mut current_ty = base.ty.clone();
    let mut collection_mode = false;
    let mut segment_types = Vec::new();
    let mut player_slot_namespace: Option<String> = None;
    for (index, segment) in segments.iter().enumerate() {
        let next_segment = segments.get(index + 1);
        if let PathSegment::Index(index) = segment
            && !is_simple_index(index)
        {
            diagnostics.push(Diagnostic::new(
                "this index is too complex; store it in a variable first: 'var i = ...;'",
                index.span.clone(),
            ));
        }
        if index > 0
            && is_entity_ref_type(&base.ty)
            && matches!(segments.first(), Some(PathSegment::Field(name)) if name == "state")
        {
            // State paths are rendered once per owner, so only literal indexes fit.
            if let PathSegment::Index(key) = segment
                && !matches!(key.kind, ExprKind::Int(_) | ExprKind::String(_))
            {
                diagnostics.push(Diagnostic::new(
                    "state can only be indexed by a literal; copy it to a variable, change that, and assign it back",
                    key.span.clone(),
                ));
            }
            let declared_path = segments[1..=index]
                .iter()
                .map(|segment| match segment {
                    PathSegment::Field(name) => Some(name.as_str()),
                    _ => None,
                })
                .collect::<Option<Vec<_>>>();
            if let Some(declared_path) = declared_path {
                let map_name = if base.ref_kind == RefKind::Player {
                    "@mcfc/player_state"
                } else {
                    "@mcfc/entity_state"
                };
                if let Some(declared_ty) = struct_defs
                    .get(map_name)
                    .and_then(|def| def.fields.get(&declared_path.join(".")))
                {
                    current_ty = declared_ty.clone();
                    segment_types.push(current_ty.clone());
                    continue;
                }
            }
        }
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
                        "inventory and hotbar are only supported on known player refs; use 'Player' to assert a player",
                        span.clone(),
                    ));
                    current_ty = Type::Nbt;
                } else {
                    current_ty = Type::Array(Box::new(Type::ItemSlot));
                    player_slot_namespace = Some(field.clone());
                }
            }
            (Type::EntityRef | Type::PlayerRef, PathSegment::Field(field))
                if matches!(
                    field.as_str(),
                    "mainhand" | "offhand" | "head" | "chest" | "legs" | "feet"
                ) =>
            {
                let uses_item_slot_surface = match next_segment {
                    None => true,
                    Some(PathSegment::Field(next)) => {
                        matches!(next.as_str(), "exists" | "id" | "count" | "nbt" | "name")
                    }
                    _ => false,
                };
                current_ty = if uses_item_slot_surface {
                    Type::ItemSlot
                } else {
                    Type::Nbt
                };
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
                        "nbt path indices must have type 'int' or 'String'",
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
            (Type::String, PathSegment::Index(index)) => {
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
                        "String index must have type 'int'",
                        span.clone(),
                    ));
                }
                current_ty = Type::String;
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
                            "list index must have type 'int'",
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
                        "map key must have type 'String'",
                        span.clone(),
                    ));
                }
                if let ExprKind::String(key) = &index.kind {
                    validate_dict_key_literal(key, span.clone(), diagnostics);
                }
                current_ty = *value.clone();
            }
            (Type::Struct(name), PathSegment::Field(field)) => {
                if !name.starts_with('@') {
                    diagnostics.push(Diagnostic::new(
                        format!(
                            "read a record component with '.{field}()'; records can't be changed, so build a new one with 'new {}(...)'",
                            name.replace("::", ".")
                        ),
                        span.clone(),
                    ));
                }
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
                    "record values must be accessed with '.field'",
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
                    "path access requires an entity, block, bossbar, item slot, Nbt, List, Map, or String base",
                    span.clone(),
                ));
                current_ty = Type::Nbt;
            }
        }
        segment_types.push(current_ty.clone());
    }
    // Index expressions reach the backend untyped, so name world state here.
    for segment in &mut segments {
        if let PathSegment::Index(index) = segment {
            qualify_world_state(index, env, struct_defs);
        }
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
            "nameVisible" => Some(vec!["nbt", "CustomNameVisible"]),
            "noAi" => Some(vec!["nbt", "NoAI"]),
            "silent" => Some(vec!["nbt", "Silent"]),
            "glowing" => Some(vec!["nbt", "Glowing"]),
            "tags" => Some(vec!["nbt", "Tags"]),
            _ => None,
        },
        Type::BlockDef => match first_name.as_str() {
            "name" => Some(vec!["nbt", "CustomName"]),
            "lock" => Some(vec!["nbt", "Lock"]),
            "lootTable" => Some(vec!["nbt", "LootTable"]),
            "lootSeed" => Some(vec!["nbt", "LootTableSeed"]),
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
                "map key '{}' is not storage-path-safe; use letters, digits, and '_' with a non-digit first character",
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
        Type::Optional(value) => {
            if *value.as_ref() == Type::Void {
                diagnostics.push(Diagnostic::new(
                    "Optional value must be a non-void type",
                    span.clone(),
                ));
            }
            validate_declared_type(value, struct_defs, span, diagnostics);
        }
        Type::Enum(name)
            if !struct_defs
                .get(name)
                .is_some_and(|def| def.enum_variants.is_some()) =>
        {
            diagnostics.push(Diagnostic::new(format!("unknown enum '{}'", name), span))
        }
        Type::Struct(name)
            if !struct_defs
                .get(name)
                .is_some_and(|def| def.enum_variants.is_none()) =>
        {
            diagnostics.push(Diagnostic::new(format!("unknown record '{}'", name), span))
        }
        _ => {}
    }
}

fn resolve_enum_type(ty: &mut Type, defs: &BTreeMap<String, StructTypeDef>) {
    match ty {
        Type::Struct(name)
            if defs
                .get(name)
                .is_some_and(|def| def.enum_variants.is_some()) =>
        {
            *ty = Type::Enum(name.clone());
        }
        Type::Array(inner) | Type::Dict(inner) | Type::Optional(inner) => {
            resolve_enum_type(inner, defs)
        }
        _ => {}
    }
}

fn validate_collection_value_type(ty: &Type, span: Span, diagnostics: &mut Diagnostics) {
    if !matches!(
        ty,
        Type::Int
            | Type::Float
            | Type::Bool
            | Type::String
            | Type::Nbt
            | Type::Array(_)
            | Type::Dict(_)
            | Type::Optional(_)
            | Type::Struct(_)
            | Type::Enum(_)
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
        // A MiniMessage literal the parser turned into SNBT.
        "text_snbt" => match args.first().map(|arg| &arg.kind) {
            Some(ExprKind::String(snbt)) => Some(builtin_call_expr(
                "text_snbt",
                vec![TypedExpr {
                    kind: TypedExprKind::String(snbt.clone()),
                    ty: Type::String,
                    ref_kind: RefKind::Unknown,
                }],
                Type::TextDef,
            )),
            _ => None,
        },
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
                    "{} may only appear as a standalone statement",
                    display_call(function)
                ),
                expr.span.clone(),
            ));
            Some(builtin_call_expr(function, args, Type::Void))
        }
        "gamerule" | "random_weighted" | "random_binomial"
            if !signatures.contains_key(function) =>
        {
            let args = type_check_args(
                args,
                struct_defs,
                signatures,
                env,
                ref_env,
                called_functions,
                diagnostics,
            );
            let arity = if function == "random_binomial" { 2 } else { 1 };
            expect_arity(function, &args, arity, expr, diagnostics);
            let problem = match (function, args.as_slice()) {
                ("gamerule", [arg]) => match &arg.kind {
                    TypedExprKind::String(name)
                        if is_known_id(crate::minecraft_ids::GAME_RULE_IDS, name) =>
                    {
                        None
                    }
                    TypedExprKind::String(name) => Some(format!("unknown game rule '{name}'")),
                    _ => Some("gamerule(...) needs a literal rule name".to_string()),
                },
                ("random_weighted", [arg]) => match &arg.kind {
                    TypedExprKind::ArrayLiteral(items)
                        if !items.is_empty()
                            && items.iter().all(|item| {
                                matches!(item.kind, TypedExprKind::Int(weight) if weight >= 0)
                            }) =>
                    {
                        None
                    }
                    _ => Some(
                        "randomWeighted(...) needs a literal list of weights such as List.of(3, 1)"
                            .to_string(),
                    ),
                },
                ("random_binomial", [n, p]) => (n.ty != Type::Int || p.ty != Type::Float)
                    .then(|| "randomBinomial(n, p) needs an 'int' and a 'float'".to_string()),
                _ => None,
            };
            if let Some(problem) = problem {
                diagnostics.push(Diagnostic::new(problem, expr.span.clone()));
            }
            Some(builtin_call_expr(function, args, Type::Int))
        }
        "game_time" | "world_time" | "border_size" if !signatures.contains_key(function) => {
            let args = type_check_args(
                args,
                struct_defs,
                signatures,
                env,
                ref_env,
                called_functions,
                diagnostics,
            );
            expect_arity(function, &args, 0, expr, diagnostics);
            Some(builtin_call_expr(function, args, Type::Int))
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
            if let Some(arg) = args.first()
                && arg.ty != Type::String
            {
                diagnostics.push(Diagnostic::new(
                    "Selector.of(...) requires a 'String' argument",
                    expr.span.clone(),
                ));
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
            if args.len() == 3 {
                for index in 0..3 {
                    expect_arg_type(
                        function,
                        &args,
                        index,
                        Type::Int,
                        "coordinate",
                        expr,
                        diagnostics,
                    );
                }
                return Some(TypedExpr {
                    kind: TypedExprKind::Call {
                        function: "block_at".to_string(),
                        args,
                    },
                    ty: Type::BlockRef,
                    ref_kind: RefKind::Unknown,
                });
            }
            expect_arity(function, &args, 1, expr, diagnostics);
            if let Some(arg) = args.first()
                && arg.ty != Type::String
            {
                diagnostics.push(Diagnostic::new(
                    "Block.of(...) requires a 'String' or three 'int' coordinates",
                    expr.span.clone(),
                ));
            }
            let raw = extract_string_literal(args.first(), "block", expr, diagnostics);
            Some(TypedExpr {
                kind: TypedExprKind::Block(raw),
                ty: Type::BlockRef,
                ref_kind: RefKind::Unknown,
            })
        }
        "log_debug" | "log_info" | "log_warn" | "log_error" | "log_level" | "log_dump"
        | "assert_fail" => {
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
            match function {
                "log_dump" => {}
                "log_level" => {
                    if !matches!(
                        args.first().map(|arg| &arg.kind),
                        Some(TypedExprKind::String(level)) if LOG_LEVELS.contains(&level.as_str())
                    ) {
                        diagnostics.push(Diagnostic::new(
                            "Log.setLevel(...) takes \"debug\", \"info\", \"warn\", \"error\" or \"off\"",
                            expr.span.clone(),
                        ));
                    }
                }
                _ => expect_arg_type(
                    function,
                    &args,
                    0,
                    Type::String,
                    "message",
                    expr,
                    diagnostics,
                ),
            }
            Some(builtin_call_expr(function, args, Type::Void))
        }
        "sidebar_title" | "sidebar_line" | "sidebar_remove_line" | "sidebar_clear" => {
            let args = type_check_args(
                args,
                struct_defs,
                signatures,
                env,
                ref_env,
                called_functions,
                diagnostics,
            );
            check_sidebar_args(function, &args, expr, diagnostics);
            Some(builtin_call_expr(function, args, Type::Void))
        }
        "__mcfc_event_block" => Some(TypedExpr {
            kind: TypedExprKind::Call {
                function: function.to_string(),
                args: Vec::new(),
            },
            ty: Type::Optional(Box::new(Type::BlockRef)),
            ref_kind: RefKind::Unknown,
        }),
        "find_first" => {
            let mut args = type_check_args(
                args,
                struct_defs,
                signatures,
                env,
                ref_env,
                called_functions,
                diagnostics,
            );
            expect_arity(function, &args, 1, expr, diagnostics);
            if let Some(arg) = args.first_mut() {
                if arg.ty != Type::EntitySet {
                    diagnostics.push(Diagnostic::new(
                        "Selector.findFirst() requires a Selector receiver",
                        expr.span.clone(),
                    ));
                } else if !can_narrow_single_selector(arg) {
                    diagnostics.push(Diagnostic::new(
                        "findFirst() needs a Selector.of(\"...\") literal so it can add limit=1",
                        expr.span.clone(),
                    ));
                } else {
                    rewrite_single_limit(arg, diagnostics, expr.span.clone());
                }
            }
            Some(TypedExpr {
                kind: TypedExprKind::Call {
                    function: function.to_string(),
                    args,
                },
                ty: Type::Optional(Box::new(Type::EntityRef)),
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
                    "Selector.getFirst() requires a Selector receiver",
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
                    "(Player) casts require an 'Entity'",
                    expr.span.clone(),
                ));
            }
            if arg.ref_kind == RefKind::NonPlayer {
                diagnostics.push(Diagnostic::new(
                    "(Player) cannot cast a known non-player entity",
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
                    "Entity.isValid() requires an Entity receiver",
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
                    "hasData(...) requires a storage-backed variable or path",
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
                    "at(...) requires an 'Entity' anchor",
                    expr.span.clone(),
                ));
            }
            if !matches!(
                value.ty,
                Type::EntitySet | Type::EntityRef | Type::PlayerRef | Type::BlockRef
            ) {
                diagnostics.push(Diagnostic::new(
                    "at(...) requires a 'Selector', 'Entity', or 'Block' value",
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
                    "as(...) requires a 'Selector' or 'Entity' anchor",
                    expr.span.clone(),
                ));
            }
            if !matches!(
                value.ty,
                Type::EntitySet | Type::EntityRef | Type::PlayerRef | Type::BlockRef
            ) {
                diagnostics.push(Diagnostic::new(
                    "as(...) requires a 'Selector', 'Entity', or 'Block' value",
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
        "int" | "float" | "bool" | "string" => {
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
            let numeric = matches!(
                (function, &arg.ty),
                ("int", Type::Float) | ("float", Type::Int)
            );
            if arg.ty != Type::Nbt && !numeric {
                diagnostics.push(Diagnostic::new(
                    match function {
                        "int" => format!("cannot cast '{}' to int", arg.ty.as_str()),
                        "float" => format!("cannot cast '{}' to float", arg.ty.as_str()),
                        "bool" => format!(
                            "cannot cast '{}' to boolean; compare it instead, like 'x != 0'",
                            arg.ty.as_str()
                        ),
                        _ => format!(
                            "cannot cast '{}' to String; use String.valueOf(x) or \"\" + x",
                            arg.ty.as_str()
                        ),
                    },
                    expr.span.clone(),
                ));
            }
            let (kind, ty) = match function {
                "int" => (CastKind::Int, Type::Int),
                "float" => (CastKind::Float, Type::Float),
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
    // Keep the input view tied to its player; it has no stored representation.
    if let ExprKind::MethodCall {
        receiver: player,
        method: input,
        args: input_args,
    } = &receiver.kind
        && input == "getCurrentInput"
        && let Some(key) = method.strip_prefix("is")
        && matches!(
            key,
            "Forward" | "Backward" | "Left" | "Right" | "Jump" | "Sneak" | "Sprint"
        )
    {
        if !input_args.is_empty() || !args.is_empty() {
            diagnostics.push(Diagnostic::new(
                "getCurrentInput() and its checks take no arguments",
                expr.span.clone(),
            ));
        }
        let checked = type_check_expr(
            player,
            struct_defs,
            signatures,
            env,
            ref_env,
            called_functions,
            diagnostics,
        );
        if checked.ty != Type::PlayerRef && checked.ref_kind != RefKind::Player {
            diagnostics.push(Diagnostic::new(
                "getCurrentInput() requires a Player receiver",
                expr.span.clone(),
            ));
        }
        return Some(method_call_expr(
            checked,
            &format!("input_{}", key.to_ascii_lowercase()),
            Vec::new(),
            Type::Bool,
        ));
    }
    let recheck = |kind: ExprKind, called: &mut BTreeSet<String>, diagnostics: &mut Diagnostics| {
        let rewritten = Expr {
            kind,
            span: expr.span.clone(),
        };
        type_check_expr(
            &rewritten,
            struct_defs,
            signatures,
            env,
            ref_env,
            called,
            diagnostics,
        )
    };
    match (method, args) {
        // `a.equals(b)` is `a == b`: MCFC compares values, never references.
        ("equals", [other]) => {
            return Some(recheck(
                ExprKind::Binary {
                    op: BinaryOp::Eq,
                    left: Box::new(receiver.clone()),
                    right: Box::new(other.clone()),
                },
                called_functions,
                diagnostics,
            ));
        }
        ("getOrDefault", [key, fallback]) => {
            let get = Expr {
                kind: ExprKind::MethodCall {
                    receiver: Box::new(receiver.clone()),
                    method: "get".to_string(),
                    args: vec![key.clone()],
                },
                span: expr.span.clone(),
            };
            return Some(recheck(
                ExprKind::MethodCall {
                    receiver: Box::new(get),
                    method: "orElse".to_string(),
                    args: vec![fallback.clone()],
                },
                called_functions,
                diagnostics,
            ));
        }
        _ => {}
    }
    if let ExprKind::Variable(enum_name) = &receiver.kind
        && !env.contains_key(enum_name)
        && let Some(variants) = struct_defs
            .get(enum_name)
            .and_then(|def| def.enum_variants.as_ref())
    {
        if method != "values" || !args.is_empty() {
            diagnostics.push(Diagnostic::new(
                format!("unknown method '{enum_name}.{method}'; enums have 'values()'"),
                expr.span.clone(),
            ));
        }
        let ty = Type::Enum(enum_name.clone());
        return Some(TypedExpr {
            kind: TypedExprKind::ArrayLiteral(
                (0..variants.len())
                    .map(|index| TypedExpr {
                        kind: TypedExprKind::Int(index as i64),
                        ty: ty.clone(),
                        ref_kind: RefKind::Unknown,
                    })
                    .collect(),
            ),
            ty: Type::Array(Box::new(ty)),
            ref_kind: RefKind::Unknown,
        });
    }
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
    // Entity actions written in `std/player.mcf`; they take an entity or a `Selector`.
    let std_method = match (method, args.len()) {
        (
            "setGameMode" | "getGameMode" | "setLevel" | "giveExp" | "giveExpLevels" | "remove"
            | "spectate" | "stopSpectating",
            _,
        ) => Some(method),
        ("teleport", 3) => Some("teleportFacing"),
        ("sendTitle", 2) => Some("sendTitleSubtitle"),
        ("sendTitle", 5) => Some("sendTitleTimed"),
        _ => None,
    };
    if let Some(std_method) = std_method
        && (is_entity_ref_type(&receiver.ty) || receiver.ty == Type::EntitySet)
    {
        let mut call_args = vec![receiver_expr.clone()];
        call_args.extend(args.iter().cloned());
        return Some(recheck(
            ExprKind::Call {
                function: format!("std::player::{std_method}"),
                args: call_args,
            },
            called_functions,
            diagnostics,
        ));
    }
    // Adventure-style `Component` methods, written in `std/text.mcf`.
    let text_method = match (method, args.len()) {
        (
            "color" | "decorate" | "append" | "appendNewline" | "appendSpace" | "clickEvent"
            | "hoverEvent" | "insertion" | "font",
            _,
        )
        | ("decoration", 2)
        | ("children", 0) => Some(method),
        ("children", 1) => Some("withChildren"),
        _ => None,
    };
    if let Some(text_method) = text_method
        && receiver.ty == Type::TextDef
    {
        let mut call_args = vec![receiver_expr.clone()];
        call_args.extend(args.iter().cloned());
        return Some(recheck(
            ExprKind::Call {
                function: format!("std::text::{text_method}"),
                args: call_args,
            },
            called_functions,
            diagnostics,
        ));
    }
    // `bb.getMax()` reads the `max` property.
    if args.is_empty()
        && let Some(property) = accessor_property(method, "get")
        && property_names(&receiver.ty).contains(&property.as_str())
    {
        let path = PathExpr {
            base: Box::new(receiver_expr.clone()),
            segments: vec![PathSegment::Field(property)],
        };
        let path = type_check_path(
            &path,
            struct_defs,
            signatures,
            env,
            ref_env,
            called_functions,
            diagnostics,
            expr.span.clone(),
        );
        return Some(TypedExpr {
            ty: path.ty.clone(),
            kind: TypedExprKind::Path(path),
            ref_kind: RefKind::Unknown,
        });
    }
    let on_world = matches!(
        receiver.ty,
        Type::EntitySet | Type::EntityRef | Type::PlayerRef | Type::BlockRef
    );
    if on_world
        && let Some((_, new_name)) = OLD_ENTITY_METHOD_NAMES
            .iter()
            .find(|(old, _)| *old == method)
    {
        diagnostics.push(Diagnostic::new(
            format!("use '.{new_name}(...)'"),
            expr.span.clone(),
        ));
    }
    let method = if on_world {
        ENTITY_METHOD_NAMES
            .iter()
            .find(|(java, _)| *java == method)
            .map_or(method, |(_, internal)| *internal)
    } else {
        method
    };
    if receiver.ty == Type::EntitySet && matches!(method, "first" | "findFirst") {
        if !args.is_empty() {
            diagnostics.push(Diagnostic::new(
                format!("{} takes no arguments", display_call(method)),
                expr.span.clone(),
            ));
        }
        let mut arg = receiver;
        if method == "findFirst" && !can_narrow_single_selector(&arg) {
            diagnostics.push(Diagnostic::new(
                "findFirst() needs a Selector.of(\"...\") literal so it can add limit=1",
                expr.span.clone(),
            ));
        }
        rewrite_single_limit(&mut arg, diagnostics, expr.span.clone());
        let ref_kind = arg.ref_kind;
        return Some(if method == "findFirst" {
            TypedExpr {
                kind: TypedExprKind::Call {
                    function: "find_first".to_string(),
                    args: vec![arg],
                },
                ty: Type::Optional(Box::new(Type::EntityRef)),
                ref_kind: RefKind::Unknown,
            }
        } else {
            TypedExpr {
                kind: TypedExprKind::Single(Box::new(arg)),
                ty: Type::EntityRef,
                ref_kind,
            }
        });
    }
    if method == "isValid" && is_entity_ref_type(&receiver.ty) {
        if !args.is_empty() {
            diagnostics.push(Diagnostic::new(
                format!("{} takes no arguments", display_call(method)),
                expr.span.clone(),
            ));
        }
        return Some(TypedExpr {
            kind: TypedExprKind::Exists(Box::new(receiver)),
            ty: Type::Bool,
            ref_kind: RefKind::Unknown,
        });
    }
    // A record component is read like Java: `quest.name()`.
    if let Type::Struct(name) = &receiver.ty
        && !name.starts_with('@')
        && args.is_empty()
        && let Some(field_ty) = struct_defs.get(name).and_then(|def| def.fields.get(method))
    {
        return Some(record_component(receiver, method, field_ty.clone()));
    }
    if matches!(receiver.ty, Type::EntityRef | Type::PlayerRef) {
        let is_player = receiver.ty == Type::PlayerRef || receiver.ref_kind == RefKind::Player;
        if let Some(read) =
            entity_read_expr(receiver_expr, method, args, is_player, expr, diagnostics)
        {
            return Some(type_check_expr(
                &read,
                struct_defs,
                signatures,
                env,
                ref_env,
                called_functions,
                diagnostics,
            ));
        }
    }
    let mut args = type_check_args(
        args,
        struct_defs,
        signatures,
        env,
        ref_env,
        called_functions,
        diagnostics,
    );
    let int = |value: i64| TypedExpr {
        kind: TypedExprKind::Int(value),
        ty: Type::Int,
        ref_kind: RefKind::Unknown,
    };
    match method {
        "isEmpty" => {
            expect_arity(method, &args, 0, expr, diagnostics);
            return Some(match &receiver.ty {
                Type::Optional(_) => TypedExpr {
                    kind: TypedExprKind::Unary {
                        op: UnaryOp::Not,
                        expr: Box::new(method_call_expr(receiver, "isPresent", args, Type::Bool)),
                    },
                    ty: Type::Bool,
                    ref_kind: RefKind::Unknown,
                },
                Type::String | Type::Array(_) | Type::Dict(_) => TypedExpr {
                    kind: TypedExprKind::Binary {
                        op: BinaryOp::Eq,
                        left: Box::new(method_call_expr(receiver, "len", args, Type::Int)),
                        right: Box::new(int(0)),
                    },
                    ty: Type::Bool,
                    ref_kind: RefKind::Unknown,
                },
                other => {
                    diagnostics.push(Diagnostic::new(
                        format!("'{}' has no isEmpty()", other.as_str()),
                        expr.span.clone(),
                    ));
                    int(0)
                }
            });
        }
        "get" if matches!(receiver.ty, Type::Optional(_)) => {
            expect_arity(method, &args, 0, expr, diagnostics);
            let Type::Optional(value) = receiver.ty.clone() else {
                unreachable!()
            };
            return Some(method_call_expr(receiver, "get", args, *value));
        }
        "contains" | "startsWith" | "endsWith" | "index_of" if receiver.ty == Type::String => {
            expect_arity(method, &args, 1, expr, diagnostics);
            if args.first().is_some_and(|arg| arg.ty != Type::String) {
                diagnostics.push(Diagnostic::new(
                    format!("{} needs a 'String' argument", display_call(method)),
                    expr.span.clone(),
                ));
            }
            let (function, ty) = match method {
                "contains" => ("std::str::contains", Type::Bool),
                "startsWith" => ("std::str::startsWith", Type::Bool),
                "endsWith" => ("std::str::endsWith", Type::Bool),
                _ => ("std::str::find", Type::Int),
            };
            called_functions.insert(function.to_string());
            let mut call_args = vec![receiver];
            call_args.extend(args);
            return Some(TypedExpr {
                kind: TypedExprKind::Call {
                    function: function.to_string(),
                    args: call_args,
                },
                ty,
                ref_kind: RefKind::Unknown,
            });
        }
        "replace" | "split" | "toUpperCase" | "toLowerCase" if receiver.ty == Type::String => {
            let (arity, ty) = match method {
                "replace" => (2, Type::String),
                "split" => (1, Type::Array(Box::new(Type::String))),
                _ => (0, Type::String),
            };
            expect_arity(method, &args, arity, expr, diagnostics);
            if args.iter().any(|arg| arg.ty != Type::String) {
                diagnostics.push(Diagnostic::new(
                    format!("{} needs 'String' arguments", display_call(method)),
                    expr.span.clone(),
                ));
            }
            let function = format!("std::str::{method}");
            called_functions.insert(function.clone());
            let mut call_args = vec![receiver];
            call_args.extend(args);
            return Some(TypedExpr {
                kind: TypedExprKind::Call {
                    function,
                    args: call_args,
                },
                ty,
                ref_kind: RefKind::Unknown,
            });
        }
        "charAt" if receiver.ty == Type::String => {
            expect_arity(method, &args, 1, expr, diagnostics);
            let index = args.into_iter().next().unwrap_or_else(|| int(0));
            if index.ty != Type::Int {
                diagnostics.push(Diagnostic::new(
                    "charAt(...) index must be 'int'",
                    expr.span.clone(),
                ));
            }
            let end = TypedExpr {
                kind: TypedExprKind::Binary {
                    op: BinaryOp::Add,
                    left: Box::new(index.clone()),
                    right: Box::new(int(1)),
                },
                ty: Type::Int,
                ref_kind: RefKind::Unknown,
            };
            // ponytail: no char type, so charAt gives a one-character String.
            return Some(method_call_expr(
                receiver,
                "slice",
                vec![index, end],
                Type::String,
            ));
        }
        "ordinal" if matches!(receiver.ty, Type::Enum(_)) => {
            expect_arity(method, &args, 0, expr, diagnostics);
            let mut receiver = receiver;
            receiver.ty = Type::Int;
            return Some(receiver);
        }
        "name" if matches!(receiver.ty, Type::Enum(_)) => {
            expect_arity(method, &args, 0, expr, diagnostics);
            let Type::Enum(name) = &receiver.ty else {
                unreachable!()
            };
            let variants = struct_defs
                .get(name)
                .and_then(|def| def.enum_variants.clone())
                .unwrap_or_default();
            return Some(enum_name_expr(receiver, &variants));
        }
        "Integer.parseInt" => {
            expect_arity(method, &args, 0, expr, diagnostics);
            if receiver.ty != Type::String {
                diagnostics.push(Diagnostic::new(
                    format!(
                        "Integer.parseInt(...) needs a 'String', found '{}'",
                        receiver.ty.as_str()
                    ),
                    expr.span.clone(),
                ));
            }
            return Some(method_call_expr(receiver, "parse_int", args, Type::Int));
        }
        _ if method.starts_with("Math.") => {
            return Some(type_check_math_call(
                &method["Math.".len()..],
                receiver,
                args,
                expr,
                diagnostics,
            ));
        }
        "sqrt" | "sin" | "cos" | "tan" | "floor" | "ceil" | "round" | "trunc" | "pow" | "hypot"
            if matches!(receiver.ty, Type::Float | Type::Int) =>
        {
            diagnostics.push(Diagnostic::new(
                format!("use 'Math.{method}(x, ...)'; numbers have no methods besides toString()"),
                expr.span.clone(),
            ));
            return Some(int(0));
        }
        "parseInt" | "parse_int" => {
            diagnostics.push(Diagnostic::new(
                "use 'Integer.parseInt(s)'",
                expr.span.clone(),
            ));
            return Some(int(0));
        }
        _ => {}
    }
    match method {
        "get" if matches!(receiver.ty, Type::Array(_) | Type::Dict(_)) => {
            expect_arity(method, &args, 1, expr, diagnostics);
            let (element, key_type) = match &receiver.ty {
                Type::Array(element) => (element.as_ref().clone(), Type::Int),
                Type::Dict(element) => (element.as_ref().clone(), Type::String),
                _ => unreachable!(),
            };
            if args.first().is_some_and(|arg| arg.ty != key_type) {
                diagnostics.push(Diagnostic::new(
                    format!("get(...) key must be '{}'", key_type.as_str()),
                    expr.span.clone(),
                ));
            }
            Some(method_call_expr(
                receiver,
                method,
                args,
                Type::Optional(Box::new(element)),
            ))
        }
        "isPresent" if matches!(receiver.ty, Type::Optional(_)) => {
            expect_arity(method, &args, 0, expr, diagnostics);
            Some(method_call_expr(receiver, method, args, Type::Bool))
        }
        "orElse" if matches!(receiver.ty, Type::Optional(_)) => {
            expect_arity(method, &args, 1, expr, diagnostics);
            let Type::Optional(value) = receiver.ty.clone() else {
                unreachable!()
            };
            if let Some(arg) = args.first_mut() {
                *arg = coerce_expr_to_expected_type(arg.clone(), &value);
                if arg.ty != *value {
                    diagnostics.push(Diagnostic::new(
                        format!(
                            "orElse(...) fallback must be '{}', found '{}'",
                            value.as_str(),
                            arg.ty.as_str()
                        ),
                        expr.span.clone(),
                    ));
                }
            }
            Some(method_call_expr(receiver, method, args, *value))
        }
        "to_string" if matches!(receiver.ty, Type::Int | Type::Float | Type::String) => {
            expect_arity(method, &args, 0, expr, diagnostics);
            Some(concat_strings(vec![receiver]))
        }
        "slice" if receiver.ty == Type::String => {
            if !(1..=2).contains(&args.len()) {
                diagnostics.push(Diagnostic::new(
                    format!(
                        "wrong arity for 'substring': expected 1 or 2, found {}",
                        args.len()
                    ),
                    expr.span.clone(),
                ));
            }
            if args.iter().any(|arg| arg.ty != Type::Int) {
                diagnostics.push(Diagnostic::new(
                    "substring() requires 'int' indices",
                    expr.span.clone(),
                ));
            }
            Some(method_call_expr(receiver, method, args, Type::String))
        }
        "cancel" => {
            expect_arity(method, &args, 0, expr, diagnostics);
            let is_agent_event = matches!(&receiver.ty, Type::Struct(name) if event_kind_for_type(name).is_some_and(|kind| !VANILLA_EVENTS.contains(&kind)));
            if !is_agent_event {
                diagnostics.push(Diagnostic::new(
                    "cancel() is only available on a typed agent event payload",
                    expr.span.clone(),
                ));
            }
            Some(method_call_expr(receiver, method, args, Type::Void))
        }
        "as_nbt" => {
            expect_arity(method, &args, 0, expr, diagnostics);
            if !matches!(
                receiver.ty,
                Type::EntityDef | Type::BlockDef | Type::ItemDef
            ) {
                diagnostics.push(Diagnostic::new(
                    "asNbt() requires an 'EntityData', 'BlockData', or 'ItemStack' receiver",
                    expr.span.clone(),
                ));
            }
            Some(method_call_expr(receiver, method, args, Type::Nbt))
        }
        "keys" if matches!(receiver.ty, Type::Dict(_)) => {
            expect_arity(method, &args, 0, expr, diagnostics);
            Some(method_call_expr(
                receiver,
                method,
                args,
                Type::Array(Box::new(Type::String)),
            ))
        }
        "len" => {
            expect_arity(method, &args, 0, expr, diagnostics);
            if !matches!(receiver.ty, Type::Array(_) | Type::String | Type::Dict(_)) {
                diagnostics.push(Diagnostic::new(
                    "size() requires a 'List', 'String' or 'Map' receiver",
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
                    "add(...) requires a variable or collection element receiver",
                    expr.span.clone(),
                ));
            }
            let expected = match &receiver.ty {
                Type::Array(element) => Some(element.as_ref()),
                _ => {
                    diagnostics.push(Diagnostic::new(
                        "add(...) requires a 'List' receiver",
                        expr.span.clone(),
                    ));
                    None
                }
            };
            if let (Some(expected), Some(arg)) = (expected, args.first_mut())
                && *expected == Type::Nbt
            {
                *arg = coerce_expr_to_nbt(arg.clone());
            }
            if let (Some(expected), Some(arg)) = (expected, args.first())
                && &arg.ty != expected
            {
                diagnostics.push(Diagnostic::new(
                    format!(
                        "add(...) value must be '{}', found '{}'",
                        expected.as_str(),
                        arg.ty.as_str()
                    ),
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
        "clear" | "insert" | "reverse" | "sort" | "first" | "last" | "contains" | "index_of"
            if matches!(receiver.ty, Type::Array(_)) =>
        {
            let Type::Array(element) = receiver.ty.clone() else {
                unreachable!()
            };
            let element = *element;
            // The value argument is the last one: insert(index, value), contains(value).
            let (arity, value_arg) = match method {
                "insert" => (2, Some(1)),
                "contains" | "index_of" => (1, Some(0)),
                _ => (0, None),
            };
            expect_arity(method, &args, arity, expr, diagnostics);
            let mutates = matches!(method, "clear" | "insert" | "reverse" | "sort");
            if method == "sort" && !matches!(element, Type::Int | Type::Float) {
                diagnostics.push(Diagnostic::new(
                    format!(
                        "sort() needs 'List<Integer>' or 'List<Float>', found 'List<{}>'",
                        element.as_str()
                    ),
                    expr.span.clone(),
                ));
            }
            if mutates && !is_storage_lvalue_expr(receiver_expr) {
                diagnostics.push(Diagnostic::new(
                    format!(
                        "{} requires a variable or collection element receiver",
                        display_call(method)
                    ),
                    expr.span.clone(),
                ));
            }
            if method == "insert" && args.first().is_some_and(|arg| arg.ty != Type::Int) {
                diagnostics.push(Diagnostic::new(
                    "add(index, ...) index must be 'int'",
                    expr.span.clone(),
                ));
            }
            if let Some(arg) = value_arg.and_then(|index| args.get_mut(index)) {
                if element == Type::Nbt {
                    *arg = coerce_expr_to_nbt(arg.clone());
                }
                if arg.ty != element {
                    diagnostics.push(Diagnostic::new(
                        format!(
                            "{} value must be '{}', found '{}'",
                            display_call(method),
                            element.as_str(),
                            arg.ty.as_str()
                        ),
                        expr.span.clone(),
                    ));
                }
            }
            let ty = match method {
                "first" | "last" => element,
                "contains" => Type::Bool,
                "index_of" => Type::Int,
                _ => Type::Void,
            };
            Some(method_call_expr(receiver, method, args, ty))
        }
        "pop" => {
            expect_arity(method, &args, 0, expr, diagnostics);
            if !is_storage_lvalue_expr(receiver_expr) {
                diagnostics.push(Diagnostic::new(
                    "removeLast() requires a variable or collection element receiver",
                    expr.span.clone(),
                ));
            }
            let ty = match &receiver.ty {
                Type::Array(element) => *element.clone(),
                _ => {
                    diagnostics.push(Diagnostic::new(
                        "removeLast() requires a 'List' receiver",
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
        "has" => {
            expect_arity(method, &args, 1, expr, diagnostics);
            if !matches!(receiver.ty, Type::Dict(_)) {
                diagnostics.push(Diagnostic::new(
                    "containsKey(...) requires a 'Map' receiver",
                    expr.span.clone(),
                ));
            }
            if args.first().map(|arg| &arg.ty) != Some(&Type::String) {
                diagnostics.push(Diagnostic::new(
                    "containsKey(...) key must be 'String'",
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
                            "remove(...) key must be 'String'",
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
                        "remove(...) requires a 'List', 'Map', or 'BossBar' receiver",
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
                "an 'Entity' or 'Block'",
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
                    "heal(...) requires an 'Entity' receiver",
                    expr.span.clone(),
                ));
            }
            match receiver.ref_kind {
                RefKind::Player => diagnostics.push(Diagnostic::new(
                    "heal(...) only supports known non-player 'Entity' receivers in v1",
                    expr.span.clone(),
                )),
                RefKind::Unknown => diagnostics.push(Diagnostic::new(
                    "heal(...) rejects ambiguous 'Entity' receivers in v1",
                    expr.span.clone(),
                )),
                RefKind::NonPlayer => {}
            }
            expect_arity(method, &args, 1, expr, diagnostics);
            expect_arg_type(method, &args, 0, Type::Int, "amount", expr, diagnostics);
            Some(method_call_expr(receiver, method, args, Type::Void))
        }
        "setVelocity" => {
            if receiver.ref_kind != RefKind::NonPlayer {
                diagnostics.push(Diagnostic::new(
                    "setVelocity() requires a known non-player Entity; use addVelocity() for players",
                    expr.span.clone(),
                ));
            }
            expect_arity(method, &args, 3, expr, diagnostics);
            for (index, name) in ["x", "y", "z"].iter().enumerate() {
                expect_arg_type(method, &args, index, Type::Float, name, expr, diagnostics);
            }
            Some(method_call_expr(receiver, method, args, Type::Void))
        }
        "addVelocity" => {
            if !is_entity_ref_type(&receiver.ty) {
                diagnostics.push(Diagnostic::new(
                    "addVelocity() requires an Entity receiver",
                    expr.span.clone(),
                ));
            }
            expect_arity(method, &args, 3, expr, diagnostics);
            for (index, name) in ["x", "y", "z"].iter().enumerate() {
                expect_arg_type(method, &args, index, Type::Float, name, expr, diagnostics);
            }
            Some(method_call_expr(receiver, method, args, Type::Void))
        }
        "setHealth" => {
            if !is_entity_ref_type(&receiver.ty) {
                diagnostics.push(Diagnostic::new(
                    "setHealth() requires an Entity receiver",
                    expr.span.clone(),
                ));
            }
            expect_arity(method, &args, 1, expr, diagnostics);
            expect_arg_type(method, &args, 0, Type::Float, "health", expr, diagnostics);
            Some(method_call_expr(receiver, method, args, Type::Void))
        }
        "setFoodLevel" => {
            if receiver.ty != Type::PlayerRef && receiver.ref_kind != RefKind::Player {
                diagnostics.push(Diagnostic::new(
                    "setFoodLevel() requires a Player receiver",
                    expr.span.clone(),
                ));
            }
            expect_arity(method, &args, 1, expr, diagnostics);
            expect_arg_type(method, &args, 0, Type::Int, "food level", expr, diagnostics);
            Some(method_call_expr(receiver, method, args, Type::Void))
        }
        "setSidebarTitle" | "setSidebarLine" | "removeSidebarLine" | "clearSidebar" => {
            if receiver.ty != Type::PlayerRef && receiver.ref_kind != RefKind::Player {
                diagnostics.push(Diagnostic::new(
                    format!("{method}() requires a Player receiver"),
                    expr.span.clone(),
                ));
            }
            check_sidebar_args(method, &args, expr, diagnostics);
            Some(method_call_expr(receiver, method, args, Type::Void))
        }
        "getAttribute" | "setAttribute" => {
            if !is_entity_ref_type(&receiver.ty) {
                diagnostics.push(Diagnostic::new(
                    format!("{method}() requires an Entity receiver"),
                    expr.span.clone(),
                ));
            }
            expect_arity(
                method,
                &args,
                if method == "getAttribute" { 1 } else { 2 },
                expr,
                diagnostics,
            );
            expect_arg_matches(
                method,
                &args,
                0,
                |ty| {
                    ty == &Type::String
                        || matches!(ty, Type::Enum(name) if name == "std::attribute::Attribute")
                },
                "a String or std.attribute.Attribute",
                "attribute id",
                expr,
                diagnostics,
            );
            if method == "setAttribute" {
                expect_arg_type(
                    method,
                    &args,
                    1,
                    Type::Float,
                    "base value",
                    expr,
                    diagnostics,
                );
            }
            Some(method_call_expr(
                receiver,
                method,
                args,
                if method == "getAttribute" {
                    Type::Float
                } else {
                    Type::Void
                },
            ))
        }
        "setRotation" => {
            expect_entity_receiver(method, &receiver, expr, diagnostics);
            expect_arity(method, &args, 2, expr, diagnostics);
            expect_arg_type(method, &args, 0, Type::Float, "yaw", expr, diagnostics);
            expect_arg_type(method, &args, 1, Type::Float, "pitch", expr, diagnostics);
            Some(method_call_expr(receiver, method, args, Type::Void))
        }
        "lookAt" => {
            expect_entity_receiver(method, &receiver, expr, diagnostics);
            expect_arity(method, &args, 1, expr, diagnostics);
            expect_arg_matches(
                method,
                &args,
                0,
                |ty| matches!(ty, Type::EntityRef | Type::PlayerRef | Type::BlockRef),
                "an Entity or Block",
                "target",
                expr,
                diagnostics,
            );
            Some(method_call_expr(receiver, method, args, Type::Void))
        }
        "yawTo" | "pitchTo" => {
            if !is_entity_ref_type(&receiver.ty) {
                diagnostics.push(Diagnostic::new(
                    format!("{method}() requires an Entity receiver"),
                    expr.span.clone(),
                ));
            }
            expect_arity(method, &args, 1, expr, diagnostics);
            expect_arg_matches(
                method,
                &args,
                0,
                |ty| matches!(ty, Type::EntityRef | Type::PlayerRef | Type::BlockRef),
                "an Entity or Block",
                "target",
                expr,
                diagnostics,
            );
            Some(method_call_expr(receiver, method, args, Type::Float))
        }
        "setInterpolationDuration"
        | "setInterpolationDelay"
        | "setTeleportDuration"
        | "setTranslation"
        | "setScale"
        | "setLeftRotation"
        | "animate" => {
            if !is_entity_ref_type(&receiver.ty) || receiver.ref_kind == RefKind::Player {
                diagnostics.push(Diagnostic::new(
                    format!("{method}() requires a display Entity"),
                    expr.span.clone(),
                ));
            }
            let vec3 = |ty: &Type| matches!(ty, Type::Struct(name) if name == "std::vec::Vec3");
            let params: &[&str] = match method {
                "setTranslation" | "setScale" => &["vec"],
                "setLeftRotation" => &["float", "vec"],
                "animate" => &["int", "vec", "vec"],
                _ => &["int"],
            };
            expect_arity(method, &args, params.len(), expr, diagnostics);
            for (index, param) in params.iter().enumerate() {
                match *param {
                    "int" => {
                        expect_arg_type(method, &args, index, Type::Int, "ticks", expr, diagnostics)
                    }
                    "float" => expect_arg_type(
                        method,
                        &args,
                        index,
                        Type::Float,
                        "angle",
                        expr,
                        diagnostics,
                    ),
                    _ => expect_arg_matches(
                        method,
                        &args,
                        index,
                        vec3,
                        "a std.vec.Vec3",
                        "vector",
                        expr,
                        diagnostics,
                    ),
                }
            }
            Some(method_call_expr(receiver, method, args, Type::Void))
        }
        "setOwner" | "getOwner" => {
            if !is_entity_ref_type(&receiver.ty) {
                diagnostics.push(Diagnostic::new(
                    format!("{method}() requires an Entity receiver"),
                    expr.span.clone(),
                ));
            }
            if method == "getOwner" {
                expect_arity(method, &args, 0, expr, diagnostics);
                return Some(method_call_expr(
                    receiver,
                    method,
                    args,
                    Type::Optional(Box::new(Type::EntityRef)),
                ));
            }
            expect_arity(method, &args, 1, expr, diagnostics);
            expect_arg_matches(
                method,
                &args,
                0,
                |ty| matches!(ty, Type::EntityRef | Type::PlayerRef),
                "an Entity",
                "owner",
                expr,
                diagnostics,
            );
            Some(method_call_expr(receiver, method, args, Type::Void))
        }
        "getTargetBlock" | "getTargetEntity" => {
            if !is_entity_ref_type(&receiver.ty) {
                diagnostics.push(Diagnostic::new(
                    format!("{method}() requires an Entity receiver"),
                    expr.span.clone(),
                ));
            }
            expect_arity(method, &args, 1, expr, diagnostics);
            expect_arg_type(
                method,
                &args,
                0,
                Type::Float,
                "max distance",
                expr,
                diagnostics,
            );
            let hit = if method == "getTargetBlock" {
                Type::BlockRef
            } else {
                Type::EntityRef
            };
            Some(method_call_expr(
                receiver,
                method,
                args,
                Type::Optional(Box::new(hit)),
            ))
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
        "actionbar" if args.len() == 2 => {
            expect_entity_receiver(method, &receiver, expr, diagnostics);
            expect_arg_matches(
                method,
                &args,
                0,
                |ty| matches!(ty, Type::String | Type::TextDef),
                "'String' or 'Component'",
                "message",
                expr,
                diagnostics,
            );
            if !matches!(&args[1].kind, TypedExprKind::String(priority)
                if ACTIONBAR_PRIORITIES.contains(&priority.as_str()))
            {
                diagnostics.push(Diagnostic::new(
                    "the actionbar priority is \"override\", \"notification\", \"conditional\" or \"persistent\"",
                    expr.span.clone(),
                ));
            }
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
                "'String' or 'Component'",
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
        "copyTo" if receiver.ty == Type::BlockRef => {
            expect_arity(method, &args, 1, expr, diagnostics);
            expect_arg_type(
                method,
                &args,
                0,
                Type::BlockRef,
                "destination",
                expr,
                diagnostics,
            );
            Some(method_call_expr(receiver, "copy_to", args, Type::Void))
        }
        "getState" if receiver.ty == Type::BlockRef => {
            expect_arity(method, &args, 1, expr, diagnostics);
            match args.first().map(|arg| &arg.kind) {
                Some(TypedExprKind::String(name))
                    if crate::minecraft_ids::BLOCK_PROPERTIES
                        .iter()
                        .any(|(property, _)| property == name) => {}
                Some(TypedExprKind::String(name)) => diagnostics.push(Diagnostic::new(
                    format!("unknown block state '{name}'"),
                    expr.span.clone(),
                )),
                _ => diagnostics.push(Diagnostic::new(
                    "getState(...) needs a literal state name, such as \"facing\"",
                    expr.span.clone(),
                )),
            }
            Some(method_call_expr(
                receiver,
                "block_state",
                args,
                Type::String,
            ))
        }
        "getType" if receiver.ty == Type::BlockRef => {
            expect_arity(method, &args, 0, expr, diagnostics);
            Some(method_call_expr(receiver, "block_type", args, Type::String))
        }
        "light" | "biome" | "in_biome" | "environment" | "x" | "y" | "z"
            if receiver.ty == Type::BlockRef =>
        {
            let (arity, ty) = match method {
                "light" | "x" | "y" | "z" => (0, Type::Int),
                "biome" => (0, Type::String),
                "in_biome" => (1, Type::Bool),
                _ => (1, Type::Float),
            };
            expect_arity(method, &args, arity, expr, diagnostics);
            if arity == 1 {
                expect_arg_type(method, &args, 0, Type::String, "id", expr, diagnostics);
            }
            if let Some(TypedExprKind::String(id)) = args.first().map(|arg| &arg.kind) {
                let known = if method == "in_biome" {
                    id.starts_with('#') || is_known_id(crate::minecraft_ids::BIOME_IDS, id)
                } else {
                    NUMERIC_ENVIRONMENT_ATTRIBUTES.contains(&id.trim_start_matches("minecraft:"))
                };
                if !known {
                    let what = if method == "in_biome" {
                        "biome"
                    } else {
                        "numeric environment attribute"
                    };
                    diagnostics.push(Diagnostic::new(
                        format!("unknown {what} '{id}'"),
                        expr.span.clone(),
                    ));
                }
            } else if method == "environment" {
                diagnostics.push(Diagnostic::new(
                    "environment(...) needs a literal attribute id such as 'gameplay/sky_light_level'",
                    expr.span.clone(),
                ));
            }
            Some(method_call_expr(receiver, method, args, ty))
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
                    "'String' or 'BlockData'",
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
        "isLoaded" => {
            expect_block_receiver(method, &receiver, expr, diagnostics);
            expect_arity(method, &args, 0, expr, diagnostics);
            Some(method_call_expr(receiver, method, args, Type::Bool))
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
                        display_call(method),
                        args.len()
                    ),
                    expr.span.clone(),
                ));
            }
            match args.first().map(|arg| &arg.ty) {
                Some(Type::String | Type::EntityDef) => {}
                _ => diagnostics.push(Diagnostic::new(
                    "block.summon(...) entity id must be 'String' or 'EntityData'",
                    expr.span.clone(),
                )),
            }
            if args.len() == 2 && args.first().map(|arg| &arg.ty) != Some(&Type::String) {
                diagnostics.push(Diagnostic::new(
                    "block.summon(entityId, data) requires a 'String' entity id",
                    expr.span.clone(),
                ));
            }
            if let Some(arg) = args.get_mut(1) {
                *arg = coerce_expr_to_nbt(arg.clone());
            }
            if args.len() >= 2 && args.get(1).map(|arg| &arg.ty) != Some(&Type::Nbt) {
                diagnostics.push(Diagnostic::new(
                    "block.summon(..., data) requires 'Nbt' summon data",
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
                "'String' or 'BlockData'",
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
                        display_call(method),
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
                        display_call(method),
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
            // Adding and removing apply to every match of a selector; `hasTag` asks one entity.
            let set_ok = method != "has_tag" && receiver.ty == Type::EntitySet;
            if !is_entity_ref_type(&receiver.ty) && !set_ok {
                diagnostics.push(Diagnostic::new(
                    format!("{} requires an 'Entity' receiver", display_call(method)),
                    expr.span.clone(),
                ));
            }
            expect_arity(method, &args, 1, expr, diagnostics);
            if args.first().map(|arg| &arg.ty) != Some(&Type::String) {
                diagnostics.push(Diagnostic::new(
                    format!("{} tag name must be 'String'", display_call(method)),
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
        "countItem" => {
            if !is_entity_ref_type(&receiver.ty) {
                diagnostics.push(Diagnostic::new(
                    "countItem(...) requires a 'Player' receiver",
                    expr.span.clone(),
                ));
            }
            expect_arity(method, &args, 1, expr, diagnostics);
            if args.first().is_some_and(|arg| arg.ty != Type::String) {
                diagnostics.push(Diagnostic::new(
                    "countItem(...) item must be a 'String', such as \"minecraft:emerald\"",
                    expr.span.clone(),
                ));
            }
            Some(method_call_expr(receiver, method, args, Type::Int))
        }
        "effect" => {
            if !is_entity_ref_type(&receiver.ty) && receiver.ty != Type::EntitySet {
                diagnostics.push(Diagnostic::new(
                    "effect(...) requires an 'Entity' receiver",
                    expr.span.clone(),
                ));
            }
            expect_arity(method, &args, 3, expr, diagnostics);
            if let Some(arg) = args.first()
                && arg.ty != Type::String
            {
                diagnostics.push(Diagnostic::new(
                    "player.effect(...) effect name must be 'String'",
                    expr.span.clone(),
                ));
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
            "summon(...) entity id must be 'String' or 'EntityData'",
            expr.span.clone(),
        )),
    }
    if args.len() == 2 && args.first().map(|arg| &arg.ty) != Some(&Type::String) {
        diagnostics.push(Diagnostic::new(
            "summon(entityId, data) requires a 'String' entity id",
            expr.span.clone(),
        ));
    }
    if args.len() >= 2 && args.get(1).map(|arg| &arg.ty) != Some(&Type::Nbt) {
        diagnostics.push(Diagnostic::new(
            "summon(..., data) requires 'Nbt' summon data",
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
    if !(args.is_empty() || args.len() == 1) {
        diagnostics.push(Diagnostic::new(
            format!(
                "wrong arity for 'text': expected 0 or 1, found {}",
                args.len()
            ),
            expr.span.clone(),
        ));
    }
    if let Some(arg) = args.first()
        && arg.ty != Type::String
    {
        diagnostics.push(Diagnostic::new(
            format!(
                "argument 1 for 'text' must be 'String', found '{}'",
                arg.ty.as_str()
            ),
            expr.span.clone(),
        ));
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
        "'String' or 'Component'",
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
        "loot_give" => "target.lootGive(table)",
        "loot_insert" => "position.lootInsert(table)",
        "loot_spawn" => "position.lootSpawn(table)",
        "tellraw" => "target.sendMessage(message)",
        "title" => "target.sendTitle(message)",
        "actionbar" => "target.sendActionBar(message)",
        "debug_marker" => "position.debugMarker(label)",
        "debug_entity" => "target.debugEntity(label)",
        "bossbar_add" => "var bb = new BossBar(id, name);",
        "bossbar_remove" => "bb.remove()",
        "bossbar_name" => "bb.setName(name)",
        "bossbar_value" => "bb.setValue(value)",
        "bossbar_max" => "bb.setMax(max)",
        "bossbar_visible" => "bb.setVisible(visible)",
        "bossbar_players" => "bb.setPlayers(targets)",
        "playsound" => "target.playSound(sound, category)",
        "stopsound" => "target.stopSound(category, sound)",
        "particle" => "position.spawnParticle(name, count?, viewers?)",
        "setblock" => "position.setBlock(block_id)",
        "fill" => "from.fill(to, block_id)",
        _ => "the method/property-style API",
    };
    format!(
        "{} has been replaced by object-style syntax; use {}",
        display_call(function),
        replacement
    )
}

/// `Log.setLevel` names, lowest first; a message shows at or above the level.
/// Smithed Actionbar priorities, highest first.
const ACTIONBAR_PRIORITIES: &[&str] = &["override", "notification", "conditional", "persistent"];

pub(crate) const LOG_LEVELS: &[&str] = &["debug", "info", "warn", "error", "off"];

/// `Sidebar.*` and `player.*Sidebar*`: title `(String)`, line `(int, String)`,
/// remove `(int)`, clear `()`.
fn check_sidebar_args(name: &str, args: &[TypedExpr], expr: &Expr, diagnostics: &mut Diagnostics) {
    let (has_line, has_text) = match name {
        "sidebar_title" | "setSidebarTitle" => (false, true),
        "sidebar_line" | "setSidebarLine" => (true, true),
        "sidebar_remove_line" | "removeSidebarLine" => (true, false),
        _ => (false, false),
    };
    expect_arity(
        name,
        args,
        usize::from(has_line) + usize::from(has_text),
        expr,
        diagnostics,
    );
    if has_line {
        expect_arg_type(name, args, 0, Type::Int, "line", expr, diagnostics);
    }
    if has_text {
        expect_arg_matches(
            name,
            args,
            usize::from(has_line),
            |ty| matches!(ty, Type::String | Type::TextDef),
            "a String or Component",
            "text",
            expr,
            diagnostics,
        );
    }
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
                "an 'Entity' or 'Block'",
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
                "an 'Entity'",
                "target",
                expr,
                diagnostics,
            );
            if let Some(target) = args.first() {
                match target.ref_kind {
                    RefKind::Player => diagnostics.push(Diagnostic::new(
                        "heal(...) only supports known non-player 'Entity' targets in v1",
                        expr.span.clone(),
                    )),
                    RefKind::Unknown => diagnostics.push(Diagnostic::new(
                        "heal(...) rejects ambiguous 'Entity' targets in v1",
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
            if let Some(message) = args.get(1)
                && !matches!(message.ty, Type::String | Type::TextDef)
            {
                diagnostics.push(Diagnostic::new(
                    format!(
                        "argument 2 for '{}' must be 'String' or 'Component', found '{}'",
                        function,
                        message.ty.as_str()
                    ),
                    expr.span.clone(),
                ));
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
                        display_call(function),
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
            if let Some(name) = args.get(1)
                && !matches!(name.ty, Type::String | Type::TextDef)
            {
                diagnostics.push(Diagnostic::new(
                    format!(
                        "argument 2 for '{}' must be 'String' or 'Component', found '{}'",
                        function,
                        name.ty.as_str()
                    ),
                    expr.span.clone(),
                ));
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
                        display_call(function),
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
                "'String' or 'BlockData'",
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
                "'String' or 'BlockData'",
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

/// Rewrite typed entity reads such as `pig.x()` or `player.food()` into the
/// NBT reads they stand for, so they lower through the existing casts.
/// Returns `None` when `method` is not one of these reads.
fn entity_read_expr(
    receiver: &Expr,
    method: &str,
    args: &[Expr],
    is_player: bool,
    expr: &Expr,
    diagnostics: &mut Diagnostics,
) -> Option<Expr> {
    let span = expr.span.clone();
    let node = |kind: ExprKind| Expr {
        kind,
        span: span.clone(),
    };
    let read = |target: &Expr, cast: &str, key: &str, index: Option<i64>| {
        let mut segments = vec![
            PathSegment::Field("nbt".to_string()),
            PathSegment::Field(key.to_string()),
        ];
        if let Some(index) = index {
            segments.push(PathSegment::Index(Box::new(node(ExprKind::Int(index)))));
        }
        node(ExprKind::Call {
            function: cast.to_string(),
            args: vec![node(ExprKind::Path(PathExpr {
                base: Box::new(target.clone()),
                segments,
            }))],
        })
    };
    let (cast, key, index, player_only) = match method {
        "x" => ("float", "Pos", Some(0), false),
        "y" => ("float", "Pos", Some(1), false),
        "z" => ("float", "Pos", Some(2), false),
        "yaw" => ("float", "Rotation", Some(0), false),
        "pitch" => ("float", "Rotation", Some(1), false),
        "health" => ("float", "Health", None, false),
        "food" => ("int", "foodLevel", None, true),
        "xp_level" => ("int", "XpLevel", None, true),
        "selected_slot" => ("int", "SelectedItemSlot", None, true),
        "dimension" => ("string", "Dimension", None, true),
        "distance_to" => {
            let [other] = args else {
                diagnostics.push(Diagnostic::new(
                    format!(
                        "wrong arity for 'distance_to': expected 1, found {}",
                        args.len()
                    ),
                    span.clone(),
                ));
                return None;
            };
            // |a - b| as hypot(hypot(dx, dy), dz), one /compute command.
            let diff = |axis: i64| {
                node(ExprKind::Binary {
                    op: BinaryOp::Sub,
                    left: Box::new(read(receiver, "float", "Pos", Some(axis))),
                    right: Box::new(read(other, "float", "Pos", Some(axis))),
                })
            };
            let hypot = |a: Expr, b: Expr| {
                node(ExprKind::MethodCall {
                    receiver: Box::new(a),
                    method: "Math.hypot".to_string(),
                    args: vec![b],
                })
            };
            return Some(hypot(hypot(diff(0), diff(1)), diff(2)));
        }
        "look_x" | "look_y" | "look_z" => {
            if !args.is_empty() {
                diagnostics.push(Diagnostic::new(
                    format!(
                        "wrong arity for '{method}': expected 0, found {}",
                        args.len()
                    ),
                    span.clone(),
                ));
            }
            // The unit vector the entity faces. Rotation is in degrees and
            // /compute trigonometry takes radians. Yaw 0 faces +Z and pitch
            // 90 faces straight down.
            let radians = |index: i64| {
                node(ExprKind::Binary {
                    op: BinaryOp::Mul,
                    left: Box::new(read(receiver, "float", "Rotation", Some(index))),
                    right: Box::new(node(ExprKind::Float("0.017453292".to_string()))),
                })
            };
            let call = |value: Expr, name: &str| {
                node(ExprKind::MethodCall {
                    receiver: Box::new(value),
                    method: format!("Math.{name}"),
                    args: Vec::new(),
                })
            };
            let neg = |value: Expr| {
                node(ExprKind::Unary {
                    op: UnaryOp::Neg,
                    expr: Box::new(value),
                })
            };
            let mul = |left: Expr, right: Expr| {
                node(ExprKind::Binary {
                    op: BinaryOp::Mul,
                    left: Box::new(left),
                    right: Box::new(right),
                })
            };
            return Some(match method {
                "look_x" => neg(mul(call(radians(0), "sin"), call(radians(1), "cos"))),
                "look_y" => neg(call(radians(1), "sin")),
                _ => mul(call(radians(0), "cos"), call(radians(1), "cos")),
            });
        }
        _ => return None,
    };
    if !args.is_empty() {
        diagnostics.push(Diagnostic::new(
            format!(
                "wrong arity for '{}': expected 0, found {}",
                display_call(method),
                args.len()
            ),
            span.clone(),
        ));
    }
    if player_only && !is_player {
        diagnostics.push(Diagnostic::new(
            format!(
                "{} is only available on players; cast it with (Player)",
                display_call(method)
            ),
            span.clone(),
        ));
    }
    Some(read(receiver, cast, key, index))
}

/// Join string-like values into one interpolated string. Literal parts stay in
/// the template and interpolated parts are spliced in, so `"a" + b + "c"`
/// lowers to a single macro call instead of one per `+`.
fn concat_strings(parts: Vec<TypedExpr>) -> TypedExpr {
    let mut template = String::new();
    let mut placeholders: Vec<MacroPlaceholder> = Vec::new();
    for part in parts {
        match part.kind {
            TypedExprKind::String(value) => template.push_str(&value),
            TypedExprKind::InterpolatedString {
                template: inner,
                placeholders: inner_placeholders,
            } => {
                template.push_str(&inner);
                placeholders.extend(inner_placeholders);
            }
            _ => {
                template.push_str("$(value)");
                placeholders.push(MacroPlaceholder {
                    key: String::new(),
                    ty: part.ty.clone(),
                    expr: part,
                });
            }
        }
    }
    for (index, placeholder) in placeholders.iter_mut().enumerate() {
        placeholder.key = format!("p{}", index + 1);
    }
    TypedExpr {
        kind: if placeholders.is_empty() {
            TypedExprKind::String(template)
        } else {
            TypedExprKind::InterpolatedString {
                template,
                placeholders,
            }
        },
        ty: Type::String,
        ref_kind: RefKind::Unknown,
    }
}

fn record_component(receiver: TypedExpr, field: &str, ty: Type) -> TypedExpr {
    let mut path = match receiver.kind {
        TypedExprKind::Path(path) => path,
        kind => TypedPathExpr {
            base: Box::new(TypedExpr {
                kind,
                ty: receiver.ty.clone(),
                ref_kind: receiver.ref_kind,
            }),
            segments: Vec::new(),
            segment_types: Vec::new(),
            ty: receiver.ty.clone(),
        },
    };
    path.segments.push(PathSegment::Field(field.to_string()));
    path.segment_types.push(ty.clone());
    path.ty = ty.clone();
    TypedExpr {
        ref_kind: if ty == Type::PlayerRef {
            RefKind::Player
        } else {
            RefKind::Unknown
        },
        kind: TypedExprKind::Path(path),
        ty,
    }
}

/// `Math.f(x, ...)` with `x` as the receiver. Every `Math` function is `float`;
/// `abs`, `min`, `max`, `clamp` and `sign` live in `std.math`.
fn type_check_math_call(
    name: &str,
    receiver: TypedExpr,
    args: Vec<TypedExpr>,
    expr: &Expr,
    diagnostics: &mut Diagnostics,
) -> TypedExpr {
    let arity = match name {
        "pow" | "hypot" => 1,
        _ => 0,
    };
    if args.len() != arity {
        diagnostics.push(Diagnostic::new(
            format!(
                "wrong arity for 'Math.{name}': expected {}, found {}",
                arity + 1,
                args.len() + 1
            ),
            expr.span.clone(),
        ));
    }
    let values: Vec<&TypedExpr> = std::iter::once(&receiver).chain(&args).collect();
    if values
        .iter()
        .any(|value| !matches!(value.ty, Type::Int | Type::Float))
    {
        diagnostics.push(Diagnostic::new(
            format!("Math.{name}(...) needs 'int' or 'float' arguments"),
            expr.span.clone(),
        ));
    }
    let receiver = coerce_expr_to_expected_type(receiver, &Type::Float);
    let args: Vec<_> = args
        .into_iter()
        .map(|arg| coerce_expr_to_expected_type(arg, &Type::Float))
        .collect();
    match name {
        // Java's Math.round gives a whole number.
        "round" => TypedExpr {
            kind: TypedExprKind::Cast {
                kind: CastKind::Int,
                expr: Box::new(method_call_expr(receiver, "round", args, Type::Float)),
            },
            ty: Type::Int,
            ref_kind: RefKind::Unknown,
        },
        _ => method_call_expr(receiver, name, args, Type::Float),
    }
}

fn conditional_expr(condition: TypedExpr, then_expr: TypedExpr, else_expr: TypedExpr) -> TypedExpr {
    TypedExpr {
        ty: then_expr.ty.clone(),
        ref_kind: if then_expr.ref_kind == else_expr.ref_kind {
            then_expr.ref_kind
        } else {
            RefKind::Unknown
        },
        kind: TypedExprKind::Conditional {
            condition: Box::new(condition),
            then_expr: Box::new(then_expr),
            else_expr: Box::new(else_expr),
        },
    }
}

/// Gives two branches one type, widening `int` to `float` and `Player` to `Entity`.
fn unify_branches(
    then_expr: TypedExpr,
    else_expr: TypedExpr,
    span: Span,
    diagnostics: &mut Diagnostics,
) -> (TypedExpr, TypedExpr) {
    let target = match (&then_expr.ty, &else_expr.ty) {
        (a, b) if a == b => a.clone(),
        (Type::Int, Type::Float) | (Type::Float, Type::Int) => Type::Float,
        (a, b) if is_entity_ref_type(a) && is_entity_ref_type(b) => Type::EntityRef,
        (a, b) => {
            diagnostics.push(Diagnostic::new(
                format!(
                    "both results must have the same type, found '{}' and '{}'",
                    a.as_str(),
                    b.as_str()
                ),
                span,
            ));
            a.clone()
        }
    };
    (
        coerce_expr_to_expected_type(then_expr, &target),
        coerce_expr_to_expected_type(else_expr, &target),
    )
}

/// The text Java's `+` would give for a value joined to a `String`.
fn string_operand(
    operand: TypedExpr,
    struct_defs: &BTreeMap<String, StructTypeDef>,
    span: Span,
    diagnostics: &mut Diagnostics,
) -> TypedExpr {
    let text = |value: &str| TypedExpr {
        kind: TypedExprKind::String(value.to_string()),
        ty: Type::String,
        ref_kind: RefKind::Unknown,
    };
    match &operand.ty {
        Type::String | Type::Int | Type::Float => operand,
        Type::Bool => conditional_expr(operand, text("true"), text("false")),
        Type::Enum(name) => match struct_defs
            .get(name)
            .and_then(|def| def.enum_variants.as_ref())
        {
            Some(variants) => enum_name_expr(operand, variants),
            None => operand,
        },
        other => {
            diagnostics.push(Diagnostic::new(
                format!("cannot join '{}' to a String", other.as_str()),
                span,
            ));
            operand
        }
    }
}

/// `e.name()`: a chain of `e == 0 ? "A" : e == 1 ? "B" : ...` over the constants.
fn enum_name_expr(value: TypedExpr, variants: &[String]) -> TypedExpr {
    let arms = variants
        .iter()
        .enumerate()
        .map(|(index, name)| {
            (
                TypedExpr {
                    kind: TypedExprKind::Int(index as i64),
                    ty: value.ty.clone(),
                    ref_kind: RefKind::Unknown,
                },
                TypedExpr {
                    kind: TypedExprKind::String(name.clone()),
                    ty: Type::String,
                    ref_kind: RefKind::Unknown,
                },
            )
        })
        .collect();
    switch_chain(value, arms, None, "__enum_name")
}

/// Lowers a switch expression to nested `?:`, binding a non-trivial value once.
/// Without a default, the last arm is the fallback (the arms are exhaustive).
fn switch_chain(
    value: TypedExpr,
    mut arms: Vec<(TypedExpr, TypedExpr)>,
    default: Option<TypedExpr>,
    temp_name: &str,
) -> TypedExpr {
    let direct = !switch_needs_temp(&value);
    let subject = if direct {
        value.clone()
    } else {
        TypedExpr {
            kind: TypedExprKind::Variable(temp_name.to_string()),
            ty: value.ty.clone(),
            ref_kind: value.ref_kind,
        }
    };
    let mut result = match default {
        Some(default) => default,
        None => match arms.pop() {
            Some((_, last)) => last,
            None => return value,
        },
    };
    for (pattern, arm) in arms.into_iter().rev() {
        let condition = TypedExpr {
            kind: TypedExprKind::Binary {
                op: BinaryOp::Eq,
                left: Box::new(subject.clone()),
                right: Box::new(pattern),
            },
            ty: Type::Bool,
            ref_kind: RefKind::Unknown,
        };
        result = conditional_expr(condition, arm, result);
    }
    if direct {
        result
    } else {
        TypedExpr {
            ty: result.ty.clone(),
            ref_kind: result.ref_kind,
            kind: TypedExprKind::Bind {
                name: temp_name.to_string(),
                value: Box::new(value),
                body: Box::new(result),
            },
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn type_check_switch_expr(
    expr: &Expr,
    value: &Expr,
    arms: &[(Expr, Expr)],
    default: Option<&Expr>,
    struct_defs: &BTreeMap<String, StructTypeDef>,
    signatures: &BTreeMap<String, FunctionSignature>,
    env: &HashMap<String, Type>,
    ref_env: &HashMap<String, RefKind>,
    called_functions: &mut BTreeSet<String>,
    diagnostics: &mut Diagnostics,
) -> TypedExpr {
    let mut check = |expr: &Expr, diagnostics: &mut Diagnostics| {
        type_check_expr(
            expr,
            struct_defs,
            signatures,
            env,
            ref_env,
            called_functions,
            diagnostics,
        )
    };
    let value = check(value, diagnostics);
    if !matches!(value.ty, Type::Int | Type::String | Type::Enum(_)) {
        diagnostics.push(Diagnostic::new(
            format!(
                "switch works on enum, 'int' and 'String' values, found '{}'",
                value.ty.as_str()
            ),
            expr.span.clone(),
        ));
    }
    let mut seen = BTreeSet::new();
    let mut typed_arms: Vec<(TypedExpr, TypedExpr)> = Vec::new();
    for (pattern, result) in arms {
        let bare_constant = match (&value.ty, &pattern.kind) {
            (Type::Enum(enum_name), ExprKind::Variable(constant))
                if !env.contains_key(constant) =>
            {
                Some(Expr {
                    kind: ExprKind::Path(PathExpr {
                        base: Box::new(Expr {
                            kind: ExprKind::Variable(enum_name.clone()),
                            span: pattern.span.clone(),
                        }),
                        segments: vec![PathSegment::Field(constant.clone())],
                    }),
                    span: pattern.span.clone(),
                })
            }
            _ => None,
        };
        let typed_pattern = check(bare_constant.as_ref().unwrap_or(pattern), diagnostics);
        if typed_pattern.ty != value.ty
            || !matches!(
                typed_pattern.kind,
                TypedExprKind::Int(_) | TypedExprKind::String(_)
            )
        {
            diagnostics.push(Diagnostic::new(
                "case must be a constant of the switch value's type",
                pattern.span.clone(),
            ));
        }
        if !seen.insert(format!("{:?}", typed_pattern.kind)) {
            diagnostics.push(Diagnostic::new(
                "duplicate switch case",
                pattern.span.clone(),
            ));
        }
        let typed_result = check(result, diagnostics);
        typed_arms.push((typed_pattern, typed_result));
    }
    let mut default = default.map(|default| check(default, diagnostics));
    if default.is_none() {
        let exhaustive = match &value.ty {
            Type::Enum(name) => struct_defs
                .get(name)
                .and_then(|def| def.enum_variants.as_ref())
                .is_some_and(|variants| {
                    (0..variants.len()).all(|index| {
                        typed_arms.iter().any(|(pattern, _)| {
                            matches!(pattern.kind, TypedExprKind::Int(value) if value == index as i64)
                        })
                    })
                }),
            _ => false,
        };
        if !exhaustive {
            diagnostics.push(Diagnostic::new(
                "a switch expression needs a 'default' unless it lists every enum constant",
                expr.span.clone(),
            ));
        }
    }
    // Every result gets the type all results share.
    let mut results: Vec<TypedExpr> = typed_arms
        .iter()
        .map(|(_, result)| result.clone())
        .collect();
    results.extend(default.clone());
    let Some(mut unified) = results.first().cloned() else {
        diagnostics.push(Diagnostic::new(
            "switch requires at least one arm",
            expr.span.clone(),
        ));
        return value;
    };
    for result in &results[1..] {
        unified = unify_branches(unified, result.clone(), expr.span.clone(), diagnostics).0;
    }
    let ty = unified.ty.clone();
    for (_, result) in typed_arms.iter_mut() {
        *result = coerce_expr_to_expected_type(result.clone(), &ty);
    }
    if let Some(default) = default.as_mut() {
        *default = coerce_expr_to_expected_type(default.clone(), &ty);
    }
    let temp_name = format!("__switch_expr_{}_{}", expr.span.line, expr.span.column);
    switch_chain(value, typed_arms, default, &temp_name)
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
    // `List<Component> parts = c.extra;` reads a component's children back.
    if expr.ty == Type::Nbt
        && matches!(expected, Type::TextDef) | (*expected == Type::Array(Box::new(Type::TextDef)))
        && matches!(&expr.kind, TypedExprKind::Path(path) if path.base.ty == Type::TextDef)
    {
        return TypedExpr {
            ty: expected.clone(),
            ..expr
        };
    }
    // Java widens `int` to `float` wherever a `float` is expected.
    if *expected == Type::Float && expr.ty == Type::Int {
        return TypedExpr {
            kind: TypedExprKind::Cast {
                kind: CastKind::Float,
                expr: Box::new(expr),
            },
            ty: Type::Float,
            ref_kind: RefKind::Unknown,
        };
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
                "{} requires an 'Entity' or 'Selector' receiver",
                display_call(method)
            ),
            expr.span.clone(),
        ));
    }
}

fn is_entity_ref_type(ty: &Type) -> bool {
    matches!(ty, Type::EntityRef | Type::PlayerRef)
}

/// Environment attributes that `/compute` can read as a number
/// (`NumericalEnvironmentAttribute` in vanilla-mcdoc 26.3).
const NUMERIC_ENVIRONMENT_ATTRIBUTES: &[&str] = &[
    "visual/cloud_height",
    "visual/fog_start_distance",
    "visual/moon_angle",
    "visual/star_angle",
    "visual/sun_angle",
    "visual/water_fog_start_distance",
    "visual/cloud_fog_end_distance",
    "visual/fog_end_distance",
    "visual/sky_fog_end_distance",
    "visual/water_fog_end_distance",
    "visual/sky_light_factor",
    "visual/star_brightness",
    "audio/music_volume",
    "gameplay/cat_waking_up_gift_chance",
    "gameplay/creature_world_gen_spawn_probability",
    "gameplay/surface_slime_spawn_chance",
    "gameplay/turtle_egg_hatch_chance",
    "gameplay/sky_light_level",
];

/// `ids` hold `minecraft:` names; `id` may leave the namespace out.
fn is_known_id(ids: &[&str], id: &str) -> bool {
    let full = if id.contains(':') {
        id.to_string()
    } else {
        format!("minecraft:{id}")
    };
    ids.contains(&full.as_str())
}

fn expect_block_receiver(
    method: &str,
    receiver: &TypedExpr,
    expr: &Expr,
    diagnostics: &mut Diagnostics,
) {
    if receiver.ty != Type::BlockRef {
        diagnostics.push(Diagnostic::new(
            format!("{} requires a 'Block' receiver", display_call(method)),
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
        "an 'Entity' or 'Selector'",
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
    if let Some(arg) = args.get(index)
        && !predicate(&arg.ty)
    {
        diagnostics.push(Diagnostic::new(
            format!(
                "{} {} must be {}, found '{}'",
                display_call(function),
                label,
                expected,
                arg.ty.as_str()
            ),
            expr.span.clone(),
        ));
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
            | Type::Optional(_)
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
                    "block builder states require an 'int', 'boolean', or 'String' value",
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
            "entity.position is read-only; use methods such as entity.position.setBlock(...)",
            span,
        )),
        "nbt" if path.base.ref_kind == RefKind::Player => diagnostics.push(Diagnostic::new(
            "player.nbt.* is read-only; use player.state, player.tags, player.team, or equipment namespaces instead",
            span,
        )),
        "state" => {
            let declared = path.segment_types.iter().skip(1).any(|ty| *ty != Type::Nbt);
            if declared && path.ty != value.ty {
                diagnostics.push(Diagnostic::new(
                    format!("state path requires '{}', found '{}'", path.ty.as_str(), value.ty.as_str()),
                    span,
                ));
            } else if !declared && !matches!(value.ty, Type::Int | Type::Bool) {
                diagnostics.push(Diagnostic::new(
                    if path.base.ref_kind == RefKind::Player {
                        "undeclared player.state.* supports only 'int' and 'boolean' values"
                    } else {
                        "undeclared entity.state.* supports only 'int' and 'boolean' values"
                    },
                    span,
                ));
            }
        }
        "tags" => {
            if path.base.ref_kind == RefKind::Player && value.ty != Type::Bool {
                diagnostics.push(Diagnostic::new(
                    "player.tags.* assignments require a 'boolean' value",
                    span,
                ));
            }
        }
        "team" => {
            if value.ty != Type::String {
                diagnostics.push(Diagnostic::new(
                    "team requires a 'String' value",
                    span,
                ));
            }
        }
        "inventory" | "hotbar" => {
            if path.base.ref_kind != RefKind::Player {
                diagnostics.push(Diagnostic::new(
                    "inventory and hotbar are only supported on known player refs; use 'Player' to assert a player",
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
                        "equipment.{} requires a 'String'{} value",
                        field,
                        if field == "item" {
                            " or 'ItemStack'"
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
                "whole-slot inventory assignment requires an 'ItemStack' value",
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
                    "item slot.name requires a 'String' value",
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
                display_call(function),
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

/// A constant or a local is compared directly (arm bodies run after every
/// test that picks them), so constant values fold away.
fn switch_needs_temp(value: &TypedExpr) -> bool {
    !matches!(
        value.kind,
        TypedExprKind::Variable(_)
            | TypedExprKind::Int(_)
            | TypedExprKind::String(_)
            | TypedExprKind::Bool(_)
    )
}

fn lower_switch_stmt(
    value: TypedExpr,
    arms: Vec<(TypedExpr, Vec<TypedStmt>)>,
    default_body: Vec<TypedStmt>,
    temp_name: String,
) -> TypedStmtKind {
    let direct = !switch_needs_temp(&value);
    let temp = if direct {
        value.clone()
    } else {
        TypedExpr {
            kind: TypedExprKind::Variable(temp_name.clone()),
            ty: value.ty.clone(),
            ref_kind: value.ref_kind,
        }
    };
    let mut else_body = default_body;
    for (pattern, body) in arms.into_iter().rev() {
        let condition = TypedExpr {
            kind: TypedExprKind::Binary {
                op: BinaryOp::Eq,
                left: Box::new(temp.clone()),
                right: Box::new(pattern),
            },
            ty: Type::Bool,
            ref_kind: RefKind::Unknown,
        };
        else_body = vec![TypedStmt {
            kind: TypedStmtKind::If {
                condition,
                then_body: body,
                else_body,
            },
        }];
    }
    let mut body = Vec::new();
    if !direct {
        body.push(TypedStmt {
            kind: TypedStmtKind::Let {
                name: temp_name,
                ty: value.ty.clone(),
                value,
            },
        });
    }
    body.extend(else_body);
    TypedStmtKind::If {
        condition: TypedExpr {
            kind: TypedExprKind::Bool(true),
            ty: Type::Bool,
            ref_kind: RefKind::Unknown,
        },
        then_body: body,
        else_body: Vec::new(),
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
                format!(
                    "{} currently requires a string literal",
                    display_call(function)
                ),
                expr.span.clone(),
            ));
            String::new()
        }
    }
}

fn can_narrow_single_selector(expr: &TypedExpr) -> bool {
    match &expr.kind {
        TypedExprKind::Selector(_) => true,
        TypedExprKind::At { value, .. } | TypedExprKind::As { value, .. } => {
            can_narrow_single_selector(value)
        }
        _ => false,
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
    // `@s` is intrinsically a single target.  Appending `limit=1` to it is
    // redundant and, on the target Minecraft version, prevents the selector
    // produced inside generated event/command execution contexts from being
    // resolved reliably.
    let trimmed = value.trim();
    if trimmed == "@s" || trimmed.starts_with("@s[") {
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
            "the selector must have no limit or 'limit=1'",
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

/// Call depths and recursion groups. A group is a cycle in the call graph (a
/// strongly connected component of 2+ functions, or one that calls itself);
/// its members share one frame depth and save their frames around calls to
/// each other. Depths are longest paths over the graph with groups condensed.
fn analyze_calls(
    functions: &[TypedFunction],
) -> (BTreeMap<String, usize>, BTreeMap<String, usize>) {
    let graph: BTreeMap<&str, Vec<&str>> = functions
        .iter()
        .map(|function| {
            let callees = function.called_functions.iter().map(String::as_str);
            (function.name.as_str(), callees.collect())
        })
        .collect();
    let mut tarjan = Tarjan::default();
    for name in graph.keys() {
        if !tarjan.index.contains_key(name) {
            tarjan.visit(name, &graph);
        }
    }
    // Tarjan emits a component only after every component it calls.
    let mut component_of = HashMap::new();
    let mut depths = BTreeMap::new();
    let mut groups = BTreeMap::new();
    for (id, component) in tarjan.components.iter().enumerate() {
        for name in component {
            component_of.insert(*name, id);
        }
        let depth = component
            .iter()
            .flat_map(|name| &graph[name])
            .filter(|callee| component_of.get(*callee) != Some(&id))
            .map(|callee| 1 + depths.get(*callee).copied().unwrap_or(0))
            .max()
            .unwrap_or(0);
        let recursive = component.len() > 1 || graph[component[0]].contains(&component[0]);
        for name in component {
            depths.insert(name.to_string(), depth);
            if recursive {
                groups.insert(name.to_string(), id);
            }
        }
    }
    (depths, groups)
}

#[derive(Default)]
struct Tarjan<'a> {
    next: usize,
    index: HashMap<&'a str, usize>,
    stack: Vec<&'a str>,
    on_stack: HashSet<&'a str>,
    components: Vec<Vec<&'a str>>,
}

impl<'a> Tarjan<'a> {
    /// Returns the node's low-link.
    fn visit(&mut self, node: &'a str, graph: &BTreeMap<&'a str, Vec<&'a str>>) -> usize {
        let index = self.next;
        self.next += 1;
        self.index.insert(node, index);
        self.stack.push(node);
        self.on_stack.insert(node);
        let mut low = index;
        for &callee in &graph[node] {
            if !graph.contains_key(callee) {
                continue;
            }
            match self.index.get(callee) {
                None => low = low.min(self.visit(callee, graph)),
                Some(&seen) if self.on_stack.contains(callee) => low = low.min(seen),
                Some(_) => {}
            }
        }
        if low == index {
            let mut component = Vec::new();
            while let Some(member) = self.stack.pop() {
                self.on_stack.remove(member);
                component.push(member);
                if member == node {
                    break;
                }
            }
            self.components.push(component);
        }
        low
    }
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
                | Type::Float
                | Type::Bool
                | Type::Enum(_)
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
