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
    /// Each class's supertypes, nearest first: its parent chain, then its
    /// interfaces. Set by `type_check` for coercions, which see no definitions.
    static SUPERTYPES: std::cell::RefCell<HashMap<String, Vec<String>>> =
        std::cell::RefCell::new(HashMap::new());
    /// Methods some subclass overrides; calls go through `F__mcfcVirtual`.
    static VIRTUAL: std::cell::RefCell<HashSet<String>> = std::cell::RefCell::new(HashSet::new());
}

/// A call to `@direct:F` calls `F` itself, never a subclass's override: a
/// dispatcher's branches and `super.m()`.
const DIRECT: &str = "@direct:";
const VIRTUAL_SUFFIX: &str = "__mcfcVirtual";

/// `name` and its supertypes, nearest first.
fn supertypes(name: &str) -> Vec<String> {
    std::iter::once(name.to_string())
        .chain(SUPERTYPES.with(|map| map.borrow().get(name).cloned().unwrap_or_default()))
        .collect()
}

fn is_subclass(sub: &str, sup: &str) -> bool {
    sub == sup
        || SUPERTYPES.with(|map| {
            map.borrow()
                .get(sub)
                .is_some_and(|all| all.iter().any(|s| s == sup))
        })
}

/// The function a call to method `function` runs: its dispatcher when a
/// subclass overrides it.
fn dispatched(function: String) -> String {
    if VIRTUAL.with(|set| set.borrow().contains(&function)) {
        format!("{function}{VIRTUAL_SUFFIX}")
    } else {
        function
    }
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
    /// On an `@overload:name` entry, the mangled name of each overload.
    pub overloads: Vec<String>,
    pub is_pub: bool,
    /// A record or enum method whose first parameter is `this`.
    pub instance: bool,
    /// Declared in a record or enum.
    pub method: bool,
}

fn display_function(name: &str) -> String {
    name.replace("::", ".")
}

/// What a bare `name` in a method means: `this.name()` for a record
/// component, `this.name` for an enum field.
fn this_member(
    env: &HashMap<String, Type>,
    struct_defs: &BTreeMap<String, StructTypeDef>,
    name: &str,
    span: &Span,
) -> Option<ExprKind> {
    // Enum constants are in scope in every method of their enum.
    if let Some(owner) = env_tag(env, OWNER_TAG)
        && struct_defs
            .get(owner)?
            .enum_variants
            .as_ref()
            .is_some_and(|variants| variants.iter().any(|v| v == name))
    {
        return Some(ExprKind::Path(PathExpr {
            base: Box::new(Expr {
                kind: ExprKind::Variable(owner.to_string()),
                span: span.clone(),
            }),
            segments: vec![PathSegment::Field(name.to_string())],
        }));
    }
    // A static field, also in static methods.
    if let Some(owner) = env_tag(env, OWNER_TAG)
        && let Some(state) = static_field_state(struct_defs, owner, name)
    {
        return Some(ExprKind::Variable(state));
    }
    let (Type::Struct(owner) | Type::Enum(owner) | Type::Class(owner)) = env.get("this")? else {
        return None;
    };
    let def = struct_defs.get(owner)?;
    let this = Box::new(Expr {
        kind: ExprKind::Variable("this".to_string()),
        span: span.clone(),
    });
    if def.class.is_some() && def.fields.contains_key(name) {
        Some(ExprKind::Path(PathExpr {
            base: this,
            segments: vec![PathSegment::Field(name.to_string())],
        }))
    } else if def.fields.contains_key(name) {
        Some(ExprKind::MethodCall {
            receiver: this,
            method: name.to_string(),
            args: Vec::new(),
        })
    } else if def.enum_fields.contains_key(name) {
        Some(ExprKind::Path(PathExpr {
            base: this,
            segments: vec![PathSegment::Field(name.to_string())],
        }))
    } else {
        None
    }
}

/// `planet.mass` as `switch (planet) { case MERCURY -> 3.3; ... }`.
fn enum_field_switch(
    path: &PathExpr,
    struct_defs: &BTreeMap<String, StructTypeDef>,
    signatures: &BTreeMap<String, FunctionSignature>,
    env: &HashMap<String, Type>,
    ref_env: &HashMap<String, RefKind>,
) -> Option<(ExprKind, Type)> {
    let [PathSegment::Field(field)] = path.segments.as_slice() else {
        return None;
    };
    if !struct_defs
        .values()
        .any(|def| def.enum_fields.contains_key(field))
    {
        return None;
    }
    let base = type_check_expr(
        &path.base,
        struct_defs,
        signatures,
        env,
        ref_env,
        &mut BTreeSet::new(),
        &mut Diagnostics::new(),
    );
    let Type::Enum(owner) = &base.ty else {
        return None;
    };
    let def = struct_defs.get(owner)?;
    let (ty, values) = def.enum_fields.get(field)?;
    let arms = def
        .enum_variants
        .as_ref()?
        .iter()
        .zip(values)
        .map(|(variant, value)| {
            (
                Expr {
                    kind: ExprKind::Variable(variant.clone()),
                    span: value.span.clone(),
                },
                value.clone(),
            )
        })
        .collect();
    Some((
        ExprKind::Switch {
            value: path.base.clone(),
            arms,
            default: None,
        },
        ty.clone(),
    ))
}

const MODULE_TAG: &str = "@module:";
const OWNER_TAG: &str = "@owner:";
/// Set in a class's constructors and static initializer, which may set `final` fields.
const INIT_TAG: &str = "@init:";
/// `@heap:C` is the variable for the heap seen as a list of `@class:C` slots.
const HEAP: &str = "@heap:";
const HEAP_SLOT: &str = "@class:";

/// A path through an object's field: `object.field.rest...`, where `field` is
/// the last object field in the path.
struct FieldAccess {
    owner: String,
    field: String,
    /// Made simple where it can be; see `simple_object`.
    object: Expr,
    rest: Vec<PathSegment>,
}

fn class_field_access(
    path: &PathExpr,
    struct_defs: &BTreeMap<String, StructTypeDef>,
    signatures: &BTreeMap<String, FunctionSignature>,
    env: &HashMap<String, Type>,
    ref_env: &HashMap<String, RefKind>,
    span: &Span,
) -> Option<FieldAccess> {
    if !struct_defs.values().any(|def| def.class.is_some()) {
        return None;
    }
    for (index, segment) in path.segments.iter().enumerate().rev() {
        let PathSegment::Field(field) = segment else {
            continue;
        };
        let object = if index == 0 {
            (*path.base).clone()
        } else {
            Expr {
                kind: ExprKind::Path(PathExpr {
                    base: path.base.clone(),
                    segments: path.segments[..index].to_vec(),
                }),
                span: span.clone(),
            }
        };
        let object_ty = type_check_expr(
            &object,
            struct_defs,
            signatures,
            env,
            ref_env,
            &mut BTreeSet::new(),
            &mut Diagnostics::new(),
        )
        .ty;
        let Type::Class(owner) = object_ty else {
            continue;
        };
        return Some(FieldAccess {
            object: simple_object(object, struct_defs, signatures, env, ref_env, span),
            owner,
            field: field.clone(),
            rest: path.segments[index + 1..].to_vec(),
        });
    }
    None
}

/// The class that declares instance field `field` of `owner`.
fn field_declarer(
    struct_defs: &BTreeMap<String, StructTypeDef>,
    owner: &str,
    field: &str,
) -> String {
    struct_defs
        .get(owner)
        .and_then(|def| def.class.as_ref()?.declared_in.get(field).cloned())
        .unwrap_or_else(|| owner.to_string())
}

/// `Node__mcfcGet_value`, generated in the class that declares the field.
fn field_getter(struct_defs: &BTreeMap<String, StructTypeDef>, owner: &str, field: &str) -> String {
    format!(
        "{}__mcfcGet_{field}",
        field_declarer(struct_defs, owner, field)
    )
}

fn check_field_visible(
    access: &FieldAccess,
    struct_defs: &BTreeMap<String, StructTypeDef>,
    env: &HashMap<String, Type>,
    span: &Span,
    diagnostics: &mut Diagnostics,
) {
    let private = struct_defs
        .get(&access.owner)
        .and_then(|def| def.class.as_ref())
        .is_some_and(|info| info.private_fields.contains(&access.field));
    let declarer = field_declarer(struct_defs, &access.owner, &access.field);
    if !module_visible(env, &declarer, !private) {
        diagnostics.push(Diagnostic::new(
            format!(
                "field '{}' of '{}' is private",
                access.field,
                access.owner.replace("::", ".")
            ),
            span.clone(),
        ));
    }
}

/// `a.b.c` where `a.b` is an object reads `@heap:B[a.b].c`: the path restarts
/// at the heap after its last object. The object id goes in a macro index, so
/// an object read from another object goes through its class's getter.
#[allow(clippy::too_many_arguments)]
fn class_heap_path(
    path: &PathExpr,
    struct_defs: &BTreeMap<String, StructTypeDef>,
    signatures: &BTreeMap<String, FunctionSignature>,
    env: &HashMap<String, Type>,
    ref_env: &HashMap<String, RefKind>,
    span: &Span,
    diagnostics: &mut Diagnostics,
) -> Option<PathExpr> {
    let access = class_field_access(path, struct_defs, signatures, env, ref_env, span)?;
    check_field_visible(&access, struct_defs, env, span, diagnostics);
    let mut segments = vec![
        PathSegment::Index(Box::new(access.object)),
        PathSegment::Field(access.field),
    ];
    segments.extend(access.rest);
    Some(PathExpr {
        base: Box::new(Expr {
            kind: ExprKind::Variable(format!("{HEAP}{}", access.owner)),
            span: span.clone(),
        }),
        segments,
    })
}

/// A read through an object that can't be a macro index, like
/// `a.items()[0].x`, calls the field's getter: `Node__mcfcGet_x(a.items()[0])`.
fn complex_field_read(
    path: &PathExpr,
    struct_defs: &BTreeMap<String, StructTypeDef>,
    signatures: &BTreeMap<String, FunctionSignature>,
    env: &HashMap<String, Type>,
    ref_env: &HashMap<String, RefKind>,
    span: &Span,
    diagnostics: &mut Diagnostics,
) -> Option<Expr> {
    let access = class_field_access(path, struct_defs, signatures, env, ref_env, span)?;
    if is_simple_index(&access.object) {
        return None;
    }
    check_field_visible(&access, struct_defs, env, span, diagnostics);
    let getter = Expr {
        kind: ExprKind::Call {
            function: field_getter(struct_defs, &access.owner, &access.field),
            args: vec![access.object],
        },
        span: span.clone(),
    };
    Some(if access.rest.is_empty() {
        getter
    } else {
        Expr {
            kind: ExprKind::Path(PathExpr {
                base: Box::new(getter),
                segments: access.rest,
            }),
            span: span.clone(),
        }
    })
}

/// An object expression the backend can paste into a macro index: `a.b.c`
/// becomes `B__mcfcGet_c(A__mcfcGet_b(a))`.
fn simple_object(
    object: Expr,
    struct_defs: &BTreeMap<String, StructTypeDef>,
    signatures: &BTreeMap<String, FunctionSignature>,
    env: &HashMap<String, Type>,
    ref_env: &HashMap<String, RefKind>,
    span: &Span,
) -> Expr {
    if is_simple_index(&object) {
        return object;
    }
    let type_of = |expr: &Expr| {
        type_check_expr(
            expr,
            struct_defs,
            signatures,
            env,
            ref_env,
            &mut BTreeSet::new(),
            &mut Diagnostics::new(),
        )
        .ty
    };
    // `a.next().value`: the method as a call, `Node__next(a)`.
    if let ExprKind::MethodCall {
        receiver,
        method,
        args,
    } = &object.kind
        && let Type::Class(owner) = type_of(receiver)
        && let Some((function, true)) = find_method(signatures, &owner, method)
    {
        let receiver = simple_object(
            (**receiver).clone(),
            struct_defs,
            signatures,
            env,
            ref_env,
            span,
        );
        return Expr {
            kind: ExprKind::Call {
                function,
                args: std::iter::once(receiver)
                    .chain(args.iter().cloned())
                    .collect(),
            },
            span: span.clone(),
        };
    }
    if let ExprKind::Path(path) = &object.kind
        && let Some((PathSegment::Field(field), rest)) = path.segments.split_last()
    {
        let inner = if rest.is_empty() {
            (*path.base).clone()
        } else {
            Expr {
                kind: ExprKind::Path(PathExpr {
                    base: path.base.clone(),
                    segments: rest.to_vec(),
                }),
                span: span.clone(),
            }
        };
        if let Type::Class(owner) = type_of(&inner) {
            let inner = simple_object(inner, struct_defs, signatures, env, ref_env, span);
            return Expr {
                kind: ExprKind::Call {
                    function: field_getter(struct_defs, &owner, field),
                    args: vec![inner],
                },
                span: span.clone(),
            };
        }
    }
    // Anything else stays as it is, and `type_check_path` reports it as too complex.
    object
}

/// The world state holding static field `field` of class `owner`, if it has one.
fn static_field_state(
    struct_defs: &BTreeMap<String, StructTypeDef>,
    owner: &str,
    field: &str,
) -> Option<String> {
    struct_defs.get(owner)?.class.as_ref()?;
    let state = static_state_name(owner, field);
    world_state_type(struct_defs, &state).map(|_| state)
}

/// Static field `field` of class `a::b::C` is the world state `a_b_C__field`.
fn static_state_name(owner: &str, field: &str) -> String {
    format!("{}__{field}", owner.replace("::", "_"))
}

/// `Counter.total` (and `Counter.total[0]`) as the static field's world state.
fn static_field_path(
    path: &PathExpr,
    struct_defs: &BTreeMap<String, StructTypeDef>,
    env: &HashMap<String, Type>,
    diagnostics: &mut Diagnostics,
) -> Option<Expr> {
    let ExprKind::Variable(owner) = &path.base.kind else {
        return None;
    };
    let Some(PathSegment::Field(field)) = path.segments.first() else {
        return None;
    };
    if env.contains_key(owner) {
        return None;
    }
    let state = static_field_state(struct_defs, owner, field)?;
    let private = struct_defs[owner]
        .class
        .as_ref()
        .is_some_and(|info| info.private_fields.contains(field));
    if !module_visible(env, owner, !private) {
        diagnostics.push(Diagnostic::new(
            format!(
                "field '{field}' of '{}' is private",
                owner.replace("::", ".")
            ),
            path.base.span.clone(),
        ));
    }
    let base = Expr {
        kind: ExprKind::Variable(state),
        span: path.base.span.clone(),
    };
    Some(if path.segments.len() == 1 {
        base
    } else {
        Expr {
            kind: ExprKind::Path(PathExpr {
                base: Box::new(base.clone()),
                segments: path.segments[1..].to_vec(),
            }),
            span: base.span,
        }
    })
}

/// Where the collector copies a storage local of type `ty` holding objects,
/// and the function that marks what it holds. The backend derives the same names.
pub fn gc_scan_names(ty: &Type) -> (String, String) {
    let key: String = ty
        .as_str()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    (
        format!("mcfcScratch_{key}"),
        format!("__mcfc_gc_scan_{key}"),
    )
}

/// Written by the backend: marks what every function's locals point to.
pub const GC_LOCALS: &str = "__mcfc_gc_locals";

/// Whether a value of type `ty` can hold an object reference.
fn holds_objects(ty: &Type, struct_defs: &BTreeMap<String, StructTypeDef>, depth: usize) -> bool {
    match ty {
        Type::Class(_) => true,
        Type::Array(inner) | Type::Dict(inner) => holds_objects(inner, struct_defs, depth),
        Type::Struct(name) if depth < 8 && !name.starts_with('@') => {
            struct_defs.get(name).is_some_and(|def| {
                def.class.is_none()
                    && def
                        .fields
                        .values()
                        .any(|field| holds_objects(field, struct_defs, depth + 1))
            })
        }
        _ => false,
    }
}

/// Statements marking every object `value` (of type `ty`) points to.
fn gc_mark_stmts(
    value: Expr,
    ty: &Type,
    struct_defs: &BTreeMap<String, StructTypeDef>,
    names: &mut usize,
) -> Vec<Stmt> {
    if !holds_objects(ty, struct_defs, 0) {
        return Vec::new();
    }
    let span = value.span.clone();
    let at = |kind: ExprKind| Expr {
        kind,
        span: span.clone(),
    };
    let stmt = |kind: StmtKind| Stmt {
        kind,
        span: span.clone(),
    };
    *names += 1;
    let name = format!("mcfcItem{names}");
    match ty {
        Type::Class(_) => vec![stmt(StmtKind::Expr(at(ExprKind::Call {
            function: "std::heap::mark".to_string(),
            args: vec![at(ExprKind::Call {
                function: "__mcfc_id".to_string(),
                args: vec![value],
            })],
        })))],
        Type::Array(inner) => {
            let item = at(ExprKind::Variable(name.clone()));
            let body = gc_mark_stmts(item, inner, struct_defs, names);
            vec![stmt(StmtKind::For {
                name,
                ty: Some((**inner).clone()),
                iterable: value,
                body,
            })]
        }
        Type::Dict(inner) => {
            let key = PathSegment::Index(Box::new(at(ExprKind::Variable(name.clone()))));
            let entry = match value.kind.clone() {
                ExprKind::Path(mut path) => {
                    path.segments.push(key);
                    path
                }
                _ => PathExpr {
                    base: Box::new(value.clone()),
                    segments: vec![key],
                },
            };
            let body = gc_mark_stmts(at(ExprKind::Path(entry)), inner, struct_defs, names);
            vec![stmt(StmtKind::For {
                name,
                ty: Some(Type::String),
                iterable: at(ExprKind::MethodCall {
                    receiver: Box::new(value),
                    method: "keys".to_string(),
                    args: Vec::new(),
                }),
                body,
            })]
        }
        Type::Struct(record) => {
            let def = &struct_defs[record];
            def.order
                .iter()
                .flat_map(|field| {
                    let component = at(ExprKind::MethodCall {
                        receiver: Box::new(value.clone()),
                        method: field.clone(),
                        args: Vec::new(),
                    });
                    gc_mark_stmts(component, &def.fields[field], struct_defs, names)
                })
                .collect()
        }
        _ => Vec::new(),
    }
}

/// Adds state `path` (`stats.best`) of type `ty` to the record `name`, with a
/// nested record for each dotted segment, like the state's storage compound.
fn add_state_field(
    struct_defs: &mut BTreeMap<String, StructTypeDef>,
    name: &str,
    path: &[String],
    ty: &Type,
) {
    let field = &path[0];
    let field_ty = if path.len() == 1 {
        ty.clone()
    } else {
        let child = format!("{name}_{field}");
        add_state_field(struct_defs, &child, &path[1..], ty);
        Type::Struct(child)
    };
    let def = struct_defs
        .entry(name.to_string())
        .or_insert_with(|| StructTypeDef {
            fields: BTreeMap::new(),
            enum_variants: None,
            enum_fields: BTreeMap::new(),
            order: Vec::new(),
            class: None,
        });
    if def.fields.insert(field.clone(), field_ty).is_none() {
        def.order.push(field.clone());
    }
}

/// The collector's program-specific half, as functions to type-check like any
/// other: `C__trace` marks what an object's fields point to, `__mcfc_gc_scan_*`
/// marks a local the backend copied into world state, and `__mcfc_gc` runs a
/// collection.
fn gc_functions(
    program: &Program,
    struct_defs: &mut BTreeMap<String, StructTypeDef>,
    world_states: &mut Vec<PlayerStateDef>,
    functions: &[TypedFunction],
) -> Vec<Function> {
    let span = program.classes[0].span.clone();
    let at = |kind: ExprKind| Expr {
        kind,
        span: span.clone(),
    };
    let stmt = |kind: StmtKind| Stmt {
        kind,
        span: span.clone(),
    };
    let call = |function: &str, args: Vec<Expr>| {
        at(ExprKind::Call {
            function: function.to_string(),
            args,
        })
    };
    let id = || at(ExprKind::Variable("id".to_string()));
    let function = |name: &str, with_id: bool, body: Vec<Stmt>| Function {
        name: name.to_string(),
        is_pub: true,
        type_params: Vec::new(),
        params: if with_id {
            vec![Param {
                name: "id".to_string(),
                ty: Type::Int,
                span: span.clone(),
            }]
        } else {
            Vec::new()
        },
        return_type: Type::Void,
        body,
        span: span.clone(),
        end: 0,
        owner: None,
        module: String::new(),
        is_abstract: false,
        is_override: false,
    };
    let mut names = 0;
    let mut out = Vec::new();

    // Each class's fields, read from its heap slot, picked by the slot's class id.
    let mut dispatch = vec![stmt(StmtKind::Let {
        name: "classId".to_string(),
        ty: Some(Type::Int),
        value: call("std::heap::classOf", vec![id()]),
    })];
    for class in &program.classes {
        let Some(def) = struct_defs.get(&class.name) else {
            continue;
        };
        let Some(info) = def.class.clone().filter(|info| !info.is_abstract) else {
            continue;
        };
        let mut body = Vec::new();
        for (field, ty) in def.fields.clone() {
            let value = at(ExprKind::Path(PathExpr {
                base: Box::new(at(ExprKind::Variable(format!("{HEAP}{}", class.name)))),
                segments: vec![
                    PathSegment::Index(Box::new(id())),
                    PathSegment::Field(field),
                ],
            }));
            body.extend(gc_mark_stmts(value, &ty, struct_defs, &mut names));
        }
        if body.is_empty() {
            continue;
        }
        let trace = format!("{}__trace", class.name);
        dispatch.push(stmt(StmtKind::If {
            condition: at(ExprKind::Binary {
                op: BinaryOp::Eq,
                left: Box::new(at(ExprKind::Variable("classId".to_string()))),
                right: Box::new(at(ExprKind::Int(info.id as i64))),
            }),
            then_body: vec![stmt(StmtKind::Expr(call(&trace, vec![id()])))],
            else_body: Vec::new(),
        }));
        out.push(function(&trace, true, body));
    }
    out.push(function("__mcfc_gc_trace", true, dispatch));

    // Storage locals holding objects are copied into world state to be marked.
    let mut scanned = BTreeSet::new();
    for local in functions
        .iter()
        .flat_map(|function| function.locals.values())
    {
        if matches!(local, Type::Class(_))
            || !holds_objects(local, struct_defs, 0)
            || !scanned.insert(local.clone())
        {
            continue;
        }
        let (scratch, scan) = gc_scan_names(local);
        struct_defs
            .get_mut(WORLD_STATE)
            .unwrap()
            .fields
            .insert(scratch.clone(), local.clone());
        world_states.push(PlayerStateDef {
            owner: StateOwner::World,
            path: vec![scratch.clone()],
            ty: local.clone(),
            display_name: scratch.clone(),
            span: span.clone(),
        });
        let value = at(ExprKind::Variable(scratch));
        let body = gc_mark_stmts(value, local, struct_defs, &mut names);
        out.push(function(&scan, false, body));
    }
    // Player and entity state: the backend copies each owner's whole state
    // compound, keyed by UUID, into world state shaped as a record per owner.
    for (owner, key) in [
        (StateOwner::Player, "players"),
        (StateOwner::Entity, "entities"),
    ] {
        let root = format!("mcfcState_{key}");
        let mut found = false;
        for state in &program.player_states {
            if state.owner == owner && holds_objects(&state.ty, struct_defs, 0) {
                add_state_field(struct_defs, &root, &state.path, &state.ty);
                found = true;
            }
        }
        if !found {
            continue;
        }
        let scratch = format!("mcfcScratch_state_{key}");
        let ty = Type::Dict(Box::new(Type::Struct(root)));
        struct_defs
            .get_mut(WORLD_STATE)
            .unwrap()
            .fields
            .insert(scratch.clone(), ty.clone());
        world_states.push(PlayerStateDef {
            owner: StateOwner::World,
            path: vec![scratch.clone()],
            ty: ty.clone(),
            display_name: scratch.clone(),
            span: span.clone(),
        });
        let value = at(ExprKind::Variable(scratch));
        let body = gc_mark_stmts(value, &ty, struct_defs, &mut names);
        out.push(function(
            &format!("__mcfc_gc_scan_state_{key}"),
            false,
            body,
        ));
    }
    out.push(function(GC_LOCALS, false, Vec::new()));

    let mut collect = vec![
        stmt(StmtKind::If {
            condition: at(ExprKind::Unary {
                op: UnaryOp::Not,
                expr: Box::new(call("std::heap::due", Vec::new())),
            }),
            then_body: vec![stmt(StmtKind::Return(None))],
            else_body: Vec::new(),
        }),
        stmt(StmtKind::Expr(call("std::heap::begin", Vec::new()))),
    ];
    // World state (static fields included) roots everything it points to.
    for state in world_states.iter() {
        let name = &state.path[0];
        if !name.starts_with("mcfc") {
            let value = at(ExprKind::Variable(name.clone()));
            collect.extend(gc_mark_stmts(value, &state.ty, struct_defs, &mut names));
        }
    }
    collect.push(stmt(StmtKind::Expr(call(GC_LOCALS, Vec::new()))));
    collect.push(stmt(StmtKind::While {
        condition: call("std::heap::hasGray", Vec::new()),
        body: vec![stmt(StmtKind::Expr(call(
            "__mcfc_gc_trace",
            vec![call("std::heap::nextGray", Vec::new())],
        )))],
        step: Vec::new(),
    }));
    collect.push(stmt(StmtKind::Expr(call("std::heap::sweep", Vec::new()))));
    out.push(function("__mcfc_gc", false, collect));
    out
}

/// The class and field an assignment sets, when that field is `final`.
fn final_field_target(
    target: &AssignTarget,
    struct_defs: &BTreeMap<String, StructTypeDef>,
    signatures: &BTreeMap<String, FunctionSignature>,
    env: &HashMap<String, Type>,
    ref_env: &HashMap<String, RefKind>,
    span: &Span,
) -> Option<(String, String)> {
    let classes = || {
        struct_defs
            .iter()
            .filter_map(|(name, def)| Some((name, def.class.as_ref()?)))
    };
    match target {
        AssignTarget::Variable(state) => classes().find_map(|(owner, info)| {
            info.final_fields
                .iter()
                .find(|field| static_state_name(owner, field) == *state)
                .map(|field| (owner.clone(), field.clone()))
        }),
        AssignTarget::Path(path) => {
            let access = class_field_access(path, struct_defs, signatures, env, ref_env, span)?;
            let info = struct_defs.get(&access.owner)?.class.as_ref()?;
            (access.rest.is_empty() && info.final_fields.contains(&access.field)).then(|| {
                (
                    field_declarer(struct_defs, &access.owner, &access.field),
                    access.field,
                )
            })
        }
    }
}

/// The value a field has before its constructor sets it, like Java's defaults.
fn default_value(ty: &Type) -> TypedExprKind {
    match ty {
        Type::Float => TypedExprKind::Float("0.0".to_string()),
        Type::Bool => TypedExprKind::Bool(false),
        Type::String => TypedExprKind::String(String::new()),
        Type::Array(_) => TypedExprKind::ArrayLiteral(Vec::new()),
        Type::Dict(_) => TypedExprKind::DictLiteral(Vec::new()),
        _ => TypedExprKind::Int(0),
    }
}

fn env_tag<'a>(env: &'a HashMap<String, Type>, tag: &str) -> Option<&'a str> {
    env.keys().find_map(|key| key.strip_prefix(tag))
}

/// Whether the method `function` (`a::b::Type__m`) may be called here: it is
/// public, or the caller is in its module or one below it, like any private item.
fn method_visible(
    env: &HashMap<String, Type>,
    function: &str,
    signature: &FunctionSignature,
) -> bool {
    module_visible(env, function, signature.is_pub)
}

/// Whether item `a::b::Name` may be used here: it is public, or the caller is
/// in its module or one below it.
fn module_visible(env: &HashMap<String, Type>, item: &str, is_pub: bool) -> bool {
    let Some(caller) = env_tag(env, MODULE_TAG) else {
        return true;
    };
    let owner = item.rsplit_once("::").map_or("", |(module, _)| module);
    is_pub || owner.is_empty() || caller == owner || caller.starts_with(&format!("{owner}::"))
}

/// The function behind method `method` of type `owner`, trying the Java
/// names the parser mapped away (`add` became `push`). An overloaded method
/// returns its bare name, which the call resolves.
fn find_method(
    signatures: &BTreeMap<String, FunctionSignature>,
    owner: &str,
    method: &str,
) -> Option<(String, bool)> {
    let names: Vec<&str> = std::iter::once(method)
        .chain(crate::language_catalog::java_method_names(method))
        .collect();
    supertypes(owner)
        .into_iter()
        .flat_map(|owner| names.iter().map(move |name| format!("{owner}__{name}")))
        .find_map(|function| {
            let instance = match signatures.get(&overload_key(&function)) {
                Some(entry) => signatures.get(entry.overloads.first()?)?.instance,
                None => signatures.get(&function)?.instance,
            };
            Some((function, instance))
        })
}

/// The signature entry listing the overloads of `name`.
fn overload_key(name: &str) -> String {
    format!("@overload:{name}")
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

/// The overload of `name` that `args` call, like Java: one taking the argument
/// types as they are wins over one they convert to (`int` to `float`), which
/// wins over a generic one. Returns `name` itself after reporting an error.
fn pick_overload(
    name: &str,
    overloads: &[String],
    args: &[TypedExpr],
    signatures: &BTreeMap<String, FunctionSignature>,
    span: &Span,
    diagnostics: &mut Diagnostics,
) -> String {
    let fits = |candidate: &String, convert: bool| {
        let signature = &signatures[candidate];
        let mut bindings = BTreeMap::new();
        signature.params.len() == args.len()
            && signature.params.iter().zip(args).all(|(param, arg)| {
                if !signature.type_params.is_empty() {
                    bind_type_params(param, &arg.ty, &signature.type_params, &mut bindings, false)
                } else if convert {
                    coerce_expr_to_expected_type(arg.clone(), param).ty == *param
                } else {
                    *param == arg.ty
                }
            })
    };
    let exact: Vec<&String> = overloads
        .iter()
        .filter(|c| signatures[*c].type_params.is_empty() && fits(c, false))
        .collect();
    let converted: Vec<&String> = overloads
        .iter()
        .filter(|c| signatures[*c].type_params.is_empty() && fits(c, true))
        .collect();
    let generic: Vec<&String> = overloads
        .iter()
        .filter(|c| !signatures[*c].type_params.is_empty() && fits(c, false))
        .collect();
    let describe = |candidate: &String| {
        let params: Vec<String> = signatures[candidate]
            .params
            .iter()
            .map(Type::as_str)
            .collect();
        format!("{}({})", display_function(name), params.join(", "))
    };
    for found in [exact, converted, generic] {
        match found.as_slice() {
            [] => continue,
            [one] => return (*one).clone(),
            many => {
                let matches: Vec<String> = many.iter().map(|c| describe(c)).collect();
                diagnostics.push(Diagnostic::new(
                    format!("ambiguous call; it matches {}", matches.join(" and ")),
                    span.clone(),
                ));
                return (*many[0]).clone();
            }
        }
    }
    let types: Vec<String> = args.iter().map(|arg| arg.ty.as_str()).collect();
    let options: Vec<String> = overloads.iter().map(describe).collect();
    diagnostics.push(Diagnostic::new(
        format!(
            "no overload of '{}' takes ({}); there is {}",
            display_function(name),
            types.join(", "),
            options.join(", ")
        ),
        span.clone(),
    ));
    name.to_string()
}

#[derive(Debug, Clone)]
pub struct StructTypeDef {
    pub fields: BTreeMap<String, Type>,
    pub enum_variants: Option<Vec<String>>,
    /// An enum field's type and its value for each constant, in variant order.
    pub enum_fields: BTreeMap<String, (Type, Vec<Expr>)>,
    /// Record components in declaration order, for `toString()`.
    pub order: Vec<String>,
    /// Set for a class, whose `fields` are its instance fields.
    pub class: Option<ClassInfo>,
}

#[derive(Debug, Clone)]
pub struct ClassInfo {
    /// Stored in the object's heap slot as `mcfcClass`; 0 marks a free slot.
    pub id: usize,
    pub private_fields: BTreeSet<String>,
    /// Instance and static fields only a constructor or initializer can set.
    pub final_fields: BTreeSet<String>,
    /// The class each instance field is declared in, which may be a parent.
    pub declared_in: BTreeMap<String, String>,
    pub parent: Option<String>,
    /// Abstract classes and interfaces, which have no objects of their own.
    pub is_abstract: bool,
    pub is_interface: bool,
    /// Parent classes, nearest first, then interfaces.
    pub supertypes: Vec<String>,
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
                    enum_fields: BTreeMap::new(),
                    order: Vec::new(),
                    class: None,
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
                    enum_fields: BTreeMap::new(),
                    order: Vec::new(),
                    class: None,
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
                enum_fields: BTreeMap::new(),
                order: Vec::new(),
                class: None,
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
                enum_fields: BTreeMap::new(),
                order: Vec::new(),
                class: None,
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
                enum_fields: BTreeMap::new(),
                order: Vec::new(),
                class: None,
            },
        );
    }
    // A class is registered twice: `C` for its references, and `@class:C` for
    // its heap slot, whose fields are read like a record's.
    for (index, class_def) in program.classes.iter().enumerate() {
        if struct_defs.contains_key(&class_def.name) {
            diagnostics.push(Diagnostic::new(
                format!("duplicate type '{}'", class_def.name),
                class_def.span.clone(),
            ));
            continue;
        }
        let mut fields = BTreeMap::new();
        for field in class_def.fields.iter().filter(|field| !field.is_static) {
            if fields
                .insert(field.name.clone(), field.ty.clone())
                .is_some()
            {
                diagnostics.push(Diagnostic::new(
                    format!("duplicate field '{}.{}'", class_def.name, field.name),
                    field.span.clone(),
                ));
            }
        }
        let slot = StructTypeDef {
            fields,
            enum_variants: None,
            enum_fields: BTreeMap::new(),
            order: Vec::new(),
            class: None,
        };
        struct_defs.insert(format!("{HEAP_SLOT}{}", class_def.name), slot.clone());
        struct_defs.insert(
            class_def.name.clone(),
            StructTypeDef {
                class: Some(ClassInfo {
                    id: index + 1,
                    private_fields: class_def
                        .fields
                        .iter()
                        .filter(|field| !field.is_pub)
                        .map(|field| field.name.clone())
                        .collect(),
                    final_fields: class_def
                        .fields
                        .iter()
                        .filter(|field| field.is_final)
                        .map(|field| field.name.clone())
                        .collect(),
                    declared_in: class_def
                        .fields
                        .iter()
                        .filter(|field| !field.is_static)
                        .map(|field| (field.name.clone(), class_def.name.clone()))
                        .collect(),
                    parent: class_def.parent.clone(),
                    is_abstract: class_def.is_abstract,
                    is_interface: class_def.is_interface,
                    supertypes: Vec::new(),
                }),
                ..slot
            },
        );
    }
    // Field types can name classes, so resolve them once all are registered.
    let class_names: Vec<String> = program.classes.iter().map(|c| c.name.clone()).collect();
    for name in class_names {
        for key in [name.clone(), format!("{HEAP_SLOT}{name}")] {
            let mut fields = struct_defs[&key].fields.clone();
            for ty in fields.values_mut() {
                resolve_enum_type(ty, &struct_defs);
            }
            struct_defs.get_mut(&key).unwrap().fields = fields;
        }
    }
    let supertype_map = class_hierarchy(program, &mut struct_defs, &mut diagnostics);
    for (name, supertypes) in &supertype_map {
        if let Some(info) = struct_defs.get_mut(name).and_then(|def| def.class.as_mut()) {
            info.supertypes = supertypes.clone();
        }
    }
    SUPERTYPES.with(|map| *map.borrow_mut() = supertype_map);
    let mut normalized = program.clone();
    for class_def in &program.classes {
        for field in class_def.fields.iter().filter(|field| field.is_static) {
            let name = static_state_name(&class_def.name, &field.name);
            normalized.world_states.push(PlayerStateDef {
                owner: StateOwner::World,
                display_name: name.clone(),
                path: vec![name],
                ty: field.ty.clone(),
                span: field.span.clone(),
            });
        }
    }
    for state in normalized
        .player_states
        .iter_mut()
        .chain(normalized.world_states.iter_mut())
    {
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
    // Overloads compile under `name__<parameter types>` (a zero-parameter one
    // keeps the bare name), and a call picks one by its argument types.
    let unmangled: Vec<String> = normalized
        .functions
        .iter()
        .map(|f| f.name.clone())
        .collect();
    let mut by_name: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (index, function) in normalized.functions.iter().enumerate() {
        by_name
            .entry(function.name.clone())
            .or_default()
            .push(index);
    }
    let mut overloads = BTreeMap::new();
    for (name, indexes) in by_name.into_iter().filter(|(_, found)| found.len() > 1) {
        let mut mangled = Vec::new();
        for index in indexes {
            let function = &mut normalized.functions[index];
            let types: Vec<Type> = function
                .params
                .iter()
                .filter(|param| param.name != "this")
                .map(|param| param.ty.clone())
                .collect();
            if !types.is_empty() {
                function.name = instance_name(&name, &types);
            }
            mangled.push(function.name.clone());
        }
        overloads.insert(name, mangled);
    }
    let class_functions = class_functions(&normalized, &unmangled, &struct_defs, &mut diagnostics);
    normalized.functions.extend(class_functions);
    for def in &normalized.structs {
        if let Some(registered) = struct_defs.get_mut(&def.name) {
            registered.order = def.fields.iter().map(|field| field.name.clone()).collect();
            registered.fields = def
                .fields
                .iter()
                .map(|field| (field.name.clone(), field.ty.clone()))
                .collect();
        }
    }
    // Enum fields read as a switch over the constant; each constant's value is
    // the argument it passes for the field's constructor parameter.
    for def in &mut normalized.enums {
        for param in &mut def.constructor {
            resolve_enum_type(&mut param.ty, &struct_defs);
        }
        for (variant, args) in def.variants.iter().zip(&def.args) {
            if args.len() != def.constructor.len() {
                diagnostics.push(Diagnostic::new(
                    format!(
                        "enum constant '{variant}' passes {} arguments, but the constructor takes {}",
                        args.len(),
                        def.constructor.len()
                    ),
                    def.span.clone(),
                ));
            }
        }
        let mut fields = BTreeMap::new();
        for field in &mut def.fields {
            resolve_enum_type(&mut field.ty, &struct_defs);
            let Some(param) = field.param else { continue };
            if def.constructor[param].ty != field.ty {
                diagnostics.push(Diagnostic::new(
                    format!(
                        "field '{}' is '{}', but it is set from a '{}' parameter",
                        field.name,
                        field.ty.as_str(),
                        def.constructor[param].ty.as_str()
                    ),
                    field.span.clone(),
                ));
            }
            let values = def
                .args
                .iter()
                .map(|args| {
                    args.get(param).cloned().unwrap_or(Expr {
                        kind: ExprKind::Int(0),
                        span: def.span.clone(),
                    })
                })
                .collect();
            fields.insert(field.name.clone(), (field.ty.clone(), values));
        }
        if let Some(registered) = struct_defs.get_mut(&def.name) {
            registered.enum_fields = fields;
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
                | Type::Class(_)
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
    for state in &normalized.world_states {
        if !matches!(
            state.ty,
            Type::Int
                | Type::Bool
                | Type::String
                | Type::Float
                | Type::Struct(_)
                | Type::Dict(_)
                | Type::Array(_)
                | Type::Enum(_)
                | Type::Class(_)
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
            enum_fields: BTreeMap::new(),
            order: Vec::new(),
            class: None,
        },
    );
    struct_defs.insert(
        "@mcfc/player_state".to_string(),
        StructTypeDef {
            fields: player_state_types,
            enum_variants: None,
            enum_fields: BTreeMap::new(),
            order: Vec::new(),
            class: None,
        },
    );
    struct_defs.insert(
        "@mcfc/entity_state".to_string(),
        StructTypeDef {
            fields: entity_state_types,
            enum_variants: None,
            enum_fields: BTreeMap::new(),
            order: Vec::new(),
            class: None,
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
                format!("duplicate function '{}'", display_function(&function.name)),
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
                overloads: Vec::new(),
                is_pub: function.is_pub,
                instance: function
                    .params
                    .first()
                    .is_some_and(|param| param.name == "this"),
                method: function.owner.is_some(),
            },
        );
    }
    for (name, overloads) in overloads {
        signatures.insert(
            overload_key(&name),
            FunctionSignature {
                params: Vec::new(),
                return_type: Type::Void,
                type_params: Vec::new(),
                instances: Arc::default(),
                overloads,
                is_pub: true,
                instance: false,
                method: false,
            },
        );
    }

    let mut functions = Vec::new();
    for function in &program.functions {
        if function.type_params.is_empty() && !function.is_abstract {
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
                    overloads: Vec::new(),
                    is_pub: generic.is_pub,
                    instance: generic
                        .params
                        .first()
                        .is_some_and(|param| param.name == "this"),
                    method: generic.owner.is_some(),
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

    let mut world_states = program.world_states.clone();
    if !program.classes.is_empty() && signatures.contains_key("std::heap::mark") {
        let gc = gc_functions(program, &mut struct_defs, &mut world_states, &functions);
        for function in &gc {
            signatures.insert(
                function.name.clone(),
                FunctionSignature {
                    params: function
                        .params
                        .iter()
                        .map(|param| param.ty.clone())
                        .collect(),
                    return_type: Type::Void,
                    type_params: Vec::new(),
                    instances: Arc::default(),
                    overloads: Vec::new(),
                    is_pub: true,
                    instance: false,
                    method: false,
                },
            );
        }
        for function in &gc {
            let mut typed =
                type_check_function(function, &struct_defs, &signatures, host, &mut diagnostics);
            // The backend's marking code calls these; say so for pruning and depths.
            if typed.name == GC_LOCALS {
                typed.called_functions.insert("std::heap::mark".to_string());
                typed.called_functions.extend(
                    gc.iter()
                        .filter(|scan| scan.name.starts_with("__mcfc_gc_scan_"))
                        .map(|scan| scan.name.clone()),
                );
            }
            functions.push(typed);
        }
    }

    let (call_depths, recursion_groups) = analyze_calls(&functions);

    diagnostics.into_result(TypedProgram {
        struct_defs,
        functions,
        function_signatures: signatures,
        player_states: program.player_states.clone(),
        world_states,
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

        // Hidden entries naming the caller's module and a method's type; `Void`
        // keeps them out of `async` captures. The merged `tick` spans modules.
        if function.name != "tick" {
            env.insert(format!("{MODULE_TAG}{}", function.module), Type::Void);
        }
        if let Some(owner) = &function.owner {
            env.insert(format!("{OWNER_TAG}{owner}"), Type::Void);
            let is_named = |method: &str| {
                let method = format!("{owner}__{method}");
                function.name == method || function.name.starts_with(&format!("{method}__"))
            };
            if is_named("new")
                || is_named("mcfcInit")
                || function.name == format!("{owner}__clinit")
            {
                env.insert(format!("{INIT_TAG}{owner}"), Type::Void);
            }
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
    let same_enum = matches!((declared, found), (Some(Type::Struct(a)), Type::Enum(b)) if a == b)
        || matches!((declared, found), (Some(Type::Struct(a) | Type::Class(a)), Type::Class(b)) if is_subclass(b, a));
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
        // `if (x instanceof Circle c)`, `switch (shape) { case Circle c -> ... }`
        // and `R r = switch (shape) { ... };` become plain statements first.
        if let Some(lowered) = lower_patterns(
            statement,
            struct_defs,
            signatures,
            env,
            ref_env,
            diagnostics,
        ) {
            let (block, scoped) = lowered;
            let (mut inner_env, mut inner_refs) = (env.clone(), ref_env.clone());
            let (env, ref_env) = if scoped {
                (&mut inner_env, &mut inner_refs)
            } else {
                (&mut *env, &mut *ref_env)
            };
            typed.extend(type_check_block(
                &block,
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
        let kind = match &statement.kind {
            // A write through an object that can't be a macro index, like
            // `a.items()[0].x = 1;`, goes through a local holding the object.
            StmtKind::Assign {
                target: AssignTarget::Path(path),
                value,
            } if let Some(access) = class_field_access(
                path,
                struct_defs,
                signatures,
                env,
                ref_env,
                &statement.span,
            ) && !is_simple_index(&access.object) =>
            {
                let span = statement.span.clone();
                let local = format!("mcfcObject{}", span.range.start);
                let mut segments = vec![PathSegment::Field(access.field)];
                segments.extend(access.rest);
                let writes = [
                    Stmt {
                        kind: StmtKind::Let {
                            name: local.clone(),
                            ty: Some(Type::Class(access.owner)),
                            value: access.object,
                        },
                        span: span.clone(),
                    },
                    Stmt {
                        kind: StmtKind::Assign {
                            target: AssignTarget::Path(PathExpr {
                                base: Box::new(Expr {
                                    kind: ExprKind::Variable(local),
                                    span: span.clone(),
                                }),
                                segments,
                            }),
                            value: value.clone(),
                        },
                        span: span.clone(),
                    },
                ];
                typed.extend(type_check_block(
                    &writes,
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
                // A written enum type parses as a record name until resolved here.
                let ty = &ty.clone().map(|mut ty| {
                    resolve_enum_type(&mut ty, struct_defs);
                    ty
                });
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
                        (Some(ty), ExprKind::Variable(name)) if name == "__mcfc_default" => {
                            Some(default_value(ty))
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
                // A constructor sets each field without an initializer to its default.
                // An empty `List.of()` or `Map.of()` takes its type from the target too.
                let is_default = match &value.kind {
                    ExprKind::Variable(name) => name == "__mcfc_default",
                    ExprKind::ArrayLiteral(items) => items.is_empty(),
                    ExprKind::DictLiteral(entries) => entries.is_empty(),
                    _ => false,
                };
                let mut value = if is_default {
                    TypedExpr {
                        kind: TypedExprKind::Int(0),
                        ty: Type::Int,
                        ref_kind: RefKind::Unknown,
                    }
                } else {
                    type_check_expr(
                        value,
                        struct_defs,
                        signatures,
                        env,
                        ref_env,
                        called_functions,
                        diagnostics,
                    )
                };
                // In a method, `count = 1;` sets `this.count`, or a static field.
                let as_target = |kind: ExprKind| match kind {
                    ExprKind::Path(path) => Some(AssignTarget::Path(path)),
                    ExprKind::Variable(name) => Some(AssignTarget::Variable(name)),
                    _ => None,
                };
                let field_target = match target {
                    AssignTarget::Variable(name) if !env.contains_key(name) => {
                        this_member(env, struct_defs, name, &statement.span).and_then(as_target)
                    }
                    AssignTarget::Path(path) => {
                        static_field_path(path, struct_defs, env, diagnostics)
                            .and_then(|state| as_target(state.kind))
                    }
                    _ => None,
                };
                if let Some((owner, field)) = final_field_target(
                    field_target.as_ref().unwrap_or(target),
                    struct_defs,
                    signatures,
                    env,
                    ref_env,
                    &statement.span,
                ) && env_tag(env, INIT_TAG) != Some(owner.as_str())
                {
                    diagnostics.push(Diagnostic::new(
                        format!(
                            "field '{field}' of '{}' is final; only a constructor or its initializer can set it",
                            owner.replace("::", ".")
                        ),
                        statement.span.clone(),
                    ));
                }
                let target = match field_target.as_ref().unwrap_or(target) {
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
                        if is_default {
                            value = TypedExpr {
                                kind: default_value(&existing),
                                ty: existing.clone(),
                                ref_kind: RefKind::Unknown,
                            };
                        }
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
                        if is_default {
                            value = TypedExpr {
                                kind: default_value(&typed_path.ty),
                                ty: typed_path.ty.clone(),
                                ref_kind: RefKind::Unknown,
                            };
                        }
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
                                Type::Int
                                    | Type::Bool
                                    | Type::String
                                    | Type::Nbt
                                    | Type::TextDef
                                    | Type::Class(_)
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
                    _ => {
                        check_declared_type(
                            name,
                            ty.as_ref(),
                            &item_ty,
                            &statement.span,
                            diagnostics,
                        );
                        // `for (Animal a : dogs)` sees each dog as an animal.
                        if let (Some(Type::Struct(declared)), Type::Class(_)) = (ty, &item_ty) {
                            item_ty = Type::Class(declared.clone());
                        }
                    }
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
                    if matches!(&expr.kind, ExprKind::Variable(name) if name == "__mcfc_default") {
                        return TypedExpr {
                            kind: default_value(return_type),
                            ty: return_type.clone(),
                            ref_kind: RefKind::Unknown,
                        };
                    }
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
                        if matches!(placeholder.ty, Type::Bool | Type::Enum(_) | Type::Class(_))
                            || matches!(&placeholder.ty, Type::Struct(name) if !name.starts_with('@'))
                        {
                            placeholder.expr = string_operand(
                                placeholder.expr,
                                struct_defs,
                                signatures,
                                called_functions,
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
            if let Some(read) = complex_field_read(
                path,
                struct_defs,
                signatures,
                env,
                ref_env,
                &expr.span,
                diagnostics,
            ) {
                return type_check_expr(
                    &read,
                    struct_defs,
                    signatures,
                    env,
                    ref_env,
                    called_functions,
                    diagnostics,
                );
            }
            if let Some(state) = static_field_path(path, struct_defs, env, diagnostics) {
                return type_check_expr(
                    &state,
                    struct_defs,
                    signatures,
                    env,
                    ref_env,
                    called_functions,
                    diagnostics,
                );
            }
            if let Some((kind, ty)) = enum_field_switch(path, struct_defs, signatures, env, ref_env)
            {
                let value = type_check_expr(
                    &Expr {
                        kind,
                        span: expr.span.clone(),
                    },
                    struct_defs,
                    signatures,
                    env,
                    ref_env,
                    called_functions,
                    diagnostics,
                );
                return coerce_expr_to_expected_type(value, &ty);
            }
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
            // `null` is object id 0, and fits any class.
            None if name == "null" => TypedExpr {
                kind: TypedExprKind::Int(0),
                ty: Type::Class(String::new()),
                ref_kind: RefKind::Unknown,
            },
            // The heap, seen as a list of `class`'s slots; see `class_heap_path`.
            None if name.starts_with(HEAP) => TypedExpr {
                kind: TypedExprKind::Variable(format!("{WORLD_STATE_PREFIX}mcfcHeap")),
                ty: Type::Array(Box::new(Type::Struct(format!(
                    "{HEAP_SLOT}{}",
                    &name[HEAP.len()..]
                )))),
                ref_kind: RefKind::Unknown,
            },
            // In a method, a bare component or field name reads it from `this`.
            None if this_member(env, struct_defs, name, &expr.span).is_some() => type_check_expr(
                &Expr {
                    kind: this_member(env, struct_defs, name, &expr.span).unwrap(),
                    span: expr.span.clone(),
                },
                struct_defs,
                signatures,
                env,
                ref_env,
                called_functions,
                diagnostics,
            ),
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
                let left = string_operand(
                    left,
                    struct_defs,
                    signatures,
                    called_functions,
                    expr.span.clone(),
                    diagnostics,
                );
                let right = string_operand(
                    right,
                    struct_defs,
                    signatures,
                    called_functions,
                    expr.span.clone(),
                    diagnostics,
                );
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
                                Type::Int
                                    | Type::Float
                                    | Type::Bool
                                    | Type::String
                                    | Type::Enum(_)
                                    | Type::Class(_)
                            ) && !matches!(&left.ty, Type::Struct(name) if !name.starts_with('@'))
                            {
                                diagnostics.push(Diagnostic::new(
                                    "equality operators support 'int', 'float', 'boolean', 'String', enums and records",
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
        // `super.m()` runs the parent's `m`, not this class's override.
        ExprKind::MethodCall {
            receiver,
            method,
            args,
        } if matches!(&receiver.kind, ExprKind::Variable(name) if name == "super")
            && !env.contains_key("super") =>
        {
            let found = env_tag(env, OWNER_TAG)
                .and_then(|owner| struct_defs.get(owner)?.class.as_ref()?.parent.clone())
                .and_then(|parent| find_method(signatures, &parent, method));
            let Some((function, true)) = found else {
                diagnostics.push(Diagnostic::new(
                    format!("the parent class has no method '{method}'"),
                    expr.span.clone(),
                ));
                return void_expr();
            };
            let this = Expr {
                kind: ExprKind::Variable("this".to_string()),
                span: expr.span.clone(),
            };
            type_check_expr(
                &Expr {
                    kind: ExprKind::Call {
                        function: format!("{DIRECT}{function}"),
                        args: std::iter::once(this).chain(args.iter().cloned()).collect(),
                    },
                    span: expr.span.clone(),
                },
                struct_defs,
                signatures,
                env,
                ref_env,
                called_functions,
                diagnostics,
            )
        }
        ExprKind::InstanceOf {
            expr: object,
            ty,
            binding,
        } => {
            if binding.is_some() {
                diagnostics.push(Diagnostic::new(
                    "a pattern variable works in an 'if' condition or a 'case'",
                    expr.span.clone(),
                ));
            }
            let mut ty = ty.clone();
            resolve_enum_type(&mut ty, struct_defs);
            let typed = type_check_expr(
                object,
                struct_defs,
                signatures,
                env,
                ref_env,
                called_functions,
                diagnostics,
            );
            let (Type::Class(target), Type::Class(_)) = (&ty, &typed.ty) else {
                diagnostics.push(Diagnostic::new(
                    format!(
                        "'instanceof' compares an object with a class, not '{}' with '{}'",
                        typed.ty.as_str(),
                        ty.as_str()
                    ),
                    expr.span.clone(),
                ));
                return TypedExpr {
                    kind: TypedExprKind::Bool(false),
                    ty: Type::Bool,
                    ref_kind: RefKind::Unknown,
                };
            };
            let call = |function: &str, args: Vec<Expr>| Expr {
                kind: ExprKind::Call {
                    function: function.to_string(),
                    args,
                },
                span: expr.span.clone(),
            };
            let class_id = call(
                "std::heap::classOf",
                vec![call("__mcfc_id", vec![(**object).clone()])],
            );
            type_check_expr(
                &call(&format!("{target}{IS_SUFFIX}"), vec![class_id]),
                struct_defs,
                signatures,
                env,
                ref_env,
                called_functions,
                diagnostics,
            )
        }
        // `(Circle) shape` trusts the programmer: the object keeps its id.
        ExprKind::Cast { ty, expr: object } => {
            let mut ty = ty.clone();
            resolve_enum_type(&mut ty, struct_defs);
            let typed = type_check_expr(
                object,
                struct_defs,
                signatures,
                env,
                ref_env,
                called_functions,
                diagnostics,
            );
            let is_interface = |name: &str| {
                struct_defs
                    .get(name)
                    .and_then(|def| def.class.as_ref())
                    .is_some_and(|info| info.is_interface)
            };
            match (&typed.ty, &ty) {
                (Type::Class(from), Type::Class(to))
                    if from.is_empty()
                        || is_subclass(from, to)
                        || is_subclass(to, from)
                        || is_interface(from)
                        || is_interface(to) =>
                {
                    TypedExpr { ty, ..typed }
                }
                _ => {
                    diagnostics.push(Diagnostic::new(
                        format!("cannot cast '{}' to '{}'", typed.ty.as_str(), ty.as_str()),
                        expr.span.clone(),
                    ));
                    typed
                }
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
        ExprKind::Switch { .. } if type_switch_expr(expr).is_some() => {
            diagnostics.push(Diagnostic::new(
                "a switch on types works as a statement, a variable's value, an assignment or a 'return'",
                expr.span.clone(),
            ));
            void_expr()
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
        // The collector's view of an object: its id as a plain `int`.
        ExprKind::Call { function, args } if function == "__mcfc_id" && args.len() == 1 => {
            let object = type_check_expr(
                &args[0],
                struct_defs,
                signatures,
                env,
                ref_env,
                called_functions,
                diagnostics,
            );
            TypedExpr {
                ty: Type::Int,
                ..object
            }
        }
        // `super(args)` runs the parent's constructor on this object.
        ExprKind::Call { function, args } if function == "super" => {
            let parent = env_tag(env, OWNER_TAG)
                .and_then(|owner| struct_defs.get(owner)?.class.as_ref()?.parent.clone());
            let Some(parent) = parent else {
                diagnostics.push(Diagnostic::new(
                    "'super(...)' needs a parent class",
                    expr.span.clone(),
                ));
                return void_expr();
            };
            let this = Expr {
                kind: ExprKind::Variable("this".to_string()),
                span: expr.span.clone(),
            };
            type_check_expr(
                &Expr {
                    kind: ExprKind::Call {
                        function: format!("{parent}__mcfcInit"),
                        args: std::iter::once(this).chain(args.iter().cloned()).collect(),
                    },
                    span: expr.span.clone(),
                },
                struct_defs,
                signatures,
                env,
                ref_env,
                called_functions,
                diagnostics,
            )
        }
        ExprKind::Call { function, args } if function == "__mcfc_alloc" => {
            // A constructor's first step: take a heap slot tagged with the class.
            let owner = env_tag(env, OWNER_TAG).unwrap_or_default().to_string();
            let id = struct_defs
                .get(&owner)
                .and_then(|def| def.class.as_ref())
                .map_or(0, |info| info.id);
            let _ = args;
            called_functions.insert("std::heap::alloc".to_string());
            TypedExpr {
                kind: TypedExprKind::Call {
                    function: "std::heap::alloc".to_string(),
                    args: vec![TypedExpr {
                        kind: TypedExprKind::Int(id as i64),
                        ty: Type::Int,
                        ref_kind: RefKind::Unknown,
                    }],
                },
                ty: Type::Class(owner),
                ref_kind: RefKind::Unknown,
            }
        }
        ExprKind::Call { function, args } => {
            let direct = function.starts_with(DIRECT);
            let function = &function.trim_start_matches(DIRECT).to_string();
            // In a method, a bare `m(...)` calls another method of the same type.
            if !function.contains("::")
                && let Some(owner) = env_tag(env, OWNER_TAG)
                && let Some((method, instance)) = find_method(signatures, owner, function)
            {
                let mut call_args = Vec::new();
                if instance {
                    if !env.contains_key("this") {
                        diagnostics.push(Diagnostic::new(
                            format!(
                                "'{function}' needs an instance; a static method has no 'this'"
                            ),
                            expr.span.clone(),
                        ));
                    }
                    call_args.push(Expr {
                        kind: ExprKind::Variable("this".to_string()),
                        span: expr.span.clone(),
                    });
                }
                call_args.extend(args.iter().cloned());
                return type_check_expr(
                    &Expr {
                        kind: ExprKind::Call {
                            function: method,
                            args: call_args,
                        },
                        span: expr.span.clone(),
                    },
                    struct_defs,
                    signatures,
                    env,
                    ref_env,
                    called_functions,
                    diagnostics,
                );
            }
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
            let function = &match signatures.get(&overload_key(function)) {
                Some(entry) => pick_overload(
                    function,
                    &entry.overloads,
                    &args,
                    signatures,
                    &expr.span,
                    diagnostics,
                ),
                None => function.clone(),
            };
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
                            args,
                        },
                        ty: Type::Void,
                        ref_kind: RefKind::Unknown,
                    };
                }
            };

            if signature.method && !method_visible(env, function, signature) {
                diagnostics.push(Diagnostic::new(
                    format!(
                        "method '{}' is private; mark it 'public'",
                        display_function(function).replacen("__", ".", 1)
                    ),
                    expr.span.clone(),
                ));
            }
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
            let function = if direct {
                function
            } else {
                dispatched(function)
            };
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
    if let Some(heap_path) = class_heap_path(
        path,
        struct_defs,
        signatures,
        env,
        ref_env,
        &span,
        diagnostics,
    ) {
        return type_check_path(
            &heap_path,
            struct_defs,
            signatures,
            env,
            ref_env,
            called_functions,
            diagnostics,
            span,
        );
    }
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
                    if !matches!(index.ty, Type::Int | Type::Class(_)) {
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
                // Inside a record method, `this.x` reads a component like Java.
                let this_component = index == 0
                    && matches!(&path.base.kind, ExprKind::Variable(base) if base == "this");
                if !name.starts_with('@') && !this_component {
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
        Type::Class(name) if !struct_defs.get(name).is_some_and(|def| def.class.is_some()) => {
            diagnostics.push(Diagnostic::new(format!("unknown class '{}'", name), span))
        }
        _ => {}
    }
}

/// A written type name parses as a record; make it an enum or class type.
fn resolve_enum_type(ty: &mut Type, defs: &BTreeMap<String, StructTypeDef>) {
    match ty {
        Type::Struct(name)
            if defs
                .get(name)
                .is_some_and(|def| def.enum_variants.is_some()) =>
        {
            *ty = Type::Enum(name.clone());
        }
        Type::Struct(name) if defs.get(name).is_some_and(|def| def.class.is_some()) => {
            *ty = Type::Class(name.clone());
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
            | Type::Class(_)
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
    // A record or enum method: `v.add(w)` calls `Vec3__add(v, w)`.
    if let Type::Struct(owner) | Type::Enum(owner) | Type::Class(owner) = &receiver.ty
        && let Some((function, instance)) = find_method(signatures, owner, method)
    {
        let mut call_args = Vec::new();
        if instance {
            call_args.push(receiver_expr.clone());
        }
        call_args.extend(args.iter().cloned());
        return Some(recheck(
            ExprKind::Call {
                function,
                args: call_args,
            },
            called_functions,
            diagnostics,
        ));
    }
    // `a.equals(b)` is `a == b`: MCFC compares values, never references.
    if let ("equals", [other]) = (method, args) {
        return Some(recheck(
            ExprKind::Binary {
                op: BinaryOp::Eq,
                left: Box::new(receiver_expr.clone()),
                right: Box::new(other.clone()),
            },
            called_functions,
            diagnostics,
        ));
    }
    if let Type::Struct(owner) = &receiver.ty
        && method == "to_string"
        && args.is_empty()
        && let Some(def) = struct_defs.get(owner).filter(|_| !owner.starts_with('@'))
    {
        return Some(record_to_string(
            receiver.clone(),
            owner,
            def,
            struct_defs,
            signatures,
            called_functions,
            expr.span.clone(),
            diagnostics,
        ));
    }
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
            if let (Some(expected), Some(arg)) = (expected, args.first_mut()) {
                *arg = coerce_expr_to_expected_type(arg.clone(), expected);
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
                *arg = coerce_expr_to_expected_type(arg.clone(), &element);
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
    // The shared sidebar's `setLine` takes an optional right-aligned value.
    let text_value = |ty: &Type| matches!(ty, Type::String | Type::TextDef);
    if name == "sidebar_line" && args.len() == 3 {
        expect_arg_matches(
            name,
            args,
            2,
            text_value,
            "a String or Component",
            "value",
            expr,
            diagnostics,
        );
    } else {
        expect_arity(
            name,
            args,
            usize::from(has_line) + usize::from(has_text),
            expr,
            diagnostics,
        );
    }
    if has_line {
        expect_arg_type(name, args, 0, Type::Int, "line", expr, diagnostics);
    }
    if has_text {
        expect_arg_matches(
            name,
            args,
            usize::from(has_line),
            text_value,
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
    signatures: &BTreeMap<String, FunctionSignature>,
    called_functions: &mut BTreeSet<String>,
    span: Span,
    diagnostics: &mut Diagnostics,
) -> TypedExpr {
    let text = |value: &str| TypedExpr {
        kind: TypedExprKind::String(value.to_string()),
        ty: Type::String,
        ref_kind: RefKind::Unknown,
    };
    // A declared `toString()` wins, like Java.
    if let Type::Struct(owner) | Type::Enum(owner) | Type::Class(owner) = &operand.ty
        && let Some((function, true)) = find_method(signatures, owner, "toString")
        && signatures
            .get(&function)
            .is_some_and(|s| s.return_type == Type::String)
    {
        let function = dispatched(function);
        called_functions.insert(function.clone());
        return TypedExpr {
            kind: TypedExprKind::Call {
                function,
                args: vec![operand],
            },
            ty: Type::String,
            ref_kind: RefKind::Unknown,
        };
    }
    if let Type::Struct(owner) = &operand.ty
        && let Some(def) = struct_defs.get(owner).filter(|_| !owner.starts_with('@'))
    {
        return record_to_string(
            operand.clone(),
            owner,
            def,
            struct_defs,
            signatures,
            called_functions,
            span,
            diagnostics,
        );
    }
    match &operand.ty {
        Type::String | Type::Int | Type::Float => operand,
        Type::Bool => conditional_expr(operand, text("true"), text("false")),
        // Java prints `Counter@1b6d3586`; the id is what tells objects apart here.
        Type::Class(name) => {
            let short = name.rsplit("::").next().unwrap_or(name);
            let id = TypedExpr {
                ty: Type::Int,
                ..operand
            };
            called_functions.insert("std::heap::describe".to_string());
            TypedExpr {
                kind: TypedExprKind::Call {
                    function: "std::heap::describe".to_string(),
                    args: vec![text(short), id],
                },
                ty: Type::String,
                ref_kind: RefKind::Unknown,
            }
        }
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

/// Java's record text: `Quest[name=Mine, reward=5]`.
#[allow(clippy::too_many_arguments)]
fn record_to_string(
    record: TypedExpr,
    owner: &str,
    def: &StructTypeDef,
    struct_defs: &BTreeMap<String, StructTypeDef>,
    signatures: &BTreeMap<String, FunctionSignature>,
    called_functions: &mut BTreeSet<String>,
    span: Span,
    diagnostics: &mut Diagnostics,
) -> TypedExpr {
    let text = |value: String| TypedExpr {
        kind: TypedExprKind::String(value),
        ty: Type::String,
        ref_kind: RefKind::Unknown,
    };
    let short = owner.rsplit("::").next().unwrap_or(owner);
    let mut parts = vec![text(format!("{short}["))];
    for (index, field) in def.order.iter().enumerate() {
        let separator = if index == 0 { "" } else { ", " };
        parts.push(text(format!("{separator}{field}=")));
        let value = record_component(record.clone(), field, def.fields[field].clone());
        parts.push(string_operand(
            value,
            struct_defs,
            signatures,
            called_functions,
            span.clone(),
            diagnostics,
        ));
    }
    parts.push(text("]".to_string()));
    concat_strings(parts)
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
    // `null`, or a subclass where its parent or interface is expected.
    if let (Type::Class(from), Type::Class(to)) = (&expr.ty, expected)
        && (from.is_empty() || is_subclass(from, to))
    {
        return TypedExpr {
            ty: expected.clone(),
            ..expr
        };
    }
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
                | Type::Class(_)
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

/// `C__mcfcIs(classId)`: whether an object of class `classId` is a `C`.
const IS_SUFFIX: &str = "__mcfcIs";

fn void_expr() -> TypedExpr {
    TypedExpr {
        kind: TypedExprKind::Int(0),
        ty: Type::Void,
        ref_kind: RefKind::Unknown,
    }
}

/// Checks each class's `extends` and `implements`, gives each class its
/// parent's fields, and returns every class's supertypes, nearest first.
fn class_hierarchy(
    program: &Program,
    struct_defs: &mut BTreeMap<String, StructTypeDef>,
    diagnostics: &mut Diagnostics,
) -> HashMap<String, Vec<String>> {
    let classes: HashMap<&str, &ClassDef> = program
        .classes
        .iter()
        .map(|class| (class.name.as_str(), class))
        .collect();
    let display = |name: &str| name.replace("::", ".");
    for class in &program.classes {
        let parent = class.parent.iter().map(|name| (name, false));
        let interfaces = class.interfaces.iter().map(|name| (name, true));
        for (name, want_interface) in parent.chain(interfaces) {
            let Some(supertype) = classes.get(name.as_str()) else {
                diagnostics.push(Diagnostic::new(
                    format!("unknown class or interface '{}'", display(name)),
                    class.span.clone(),
                ));
                continue;
            };
            let message = if want_interface && !supertype.is_interface {
                if class.is_interface {
                    format!(
                        "an interface extends only interfaces, and '{}' is a class",
                        display(name)
                    )
                } else {
                    format!("'{}' is a class; extend it with 'extends'", display(name))
                }
            } else if !want_interface && supertype.is_interface {
                format!("'{}' is an interface; use 'implements'", display(name))
            } else if supertype.is_final {
                format!("'{}' is final, so nothing can extend it", display(name))
            } else if let Some(permits) = &supertype.permits
                && !permits.contains(&class.name)
            {
                format!(
                    "'{}' is sealed and doesn't permit '{}'",
                    display(name),
                    display(&class.name)
                )
            } else {
                continue;
            };
            diagnostics.push(Diagnostic::new(message, class.span.clone()));
        }
    }

    let mut supertypes = HashMap::new();
    for class in &program.classes {
        let cycle = || {
            Diagnostic::new(
                format!("'{}' extends itself", display(&class.name)),
                class.span.clone(),
            )
        };
        let mut found: Vec<String> = Vec::new();
        let mut current = class.parent.clone();
        while let Some(name) = current {
            if name == class.name || found.contains(&name) {
                diagnostics.push(cycle());
                break;
            }
            current = classes.get(name.as_str()).and_then(|c| c.parent.clone());
            found.push(name);
        }
        // Then the interfaces of the class and its parents, and theirs.
        let mut pending: Vec<String> = std::iter::once(&class.name)
            .chain(&found)
            .filter_map(|name| classes.get(name.as_str()))
            .flat_map(|c| c.interfaces.clone())
            .collect();
        let mut index = 0;
        while let Some(name) = pending.get(index).cloned() {
            index += 1;
            if name == class.name {
                diagnostics.push(cycle());
                continue;
            }
            if found.contains(&name) {
                continue;
            }
            if let Some(interface) = classes.get(name.as_str()) {
                pending.extend(interface.interfaces.clone());
            }
            found.push(name);
        }
        supertypes.insert(class.name.clone(), found);
    }

    // A subclass's objects hold its parent's fields too, so parents go first.
    let depth = |name: &str| {
        supertypes.get(name).map_or(0, |all: &Vec<String>| {
            all.iter()
                .filter(|s| classes.get(s.as_str()).is_some_and(|c| !c.is_interface))
                .count()
        })
    };
    let mut ordered: Vec<&ClassDef> = program.classes.iter().collect();
    ordered.sort_by_key(|class| depth(&class.name));
    for class in ordered {
        let Some(parent) = &class.parent else {
            continue;
        };
        let Some(parent_def) = struct_defs.get(parent).cloned() else {
            continue;
        };
        let Some(parent_info) = parent_def.class.clone() else {
            continue;
        };
        for key in [class.name.clone(), format!("{HEAP_SLOT}{}", class.name)] {
            let Some(def) = struct_defs.get_mut(&key) else {
                continue;
            };
            for (field, ty) in &parent_def.fields {
                if def.fields.contains_key(field) {
                    if key == class.name {
                        diagnostics.push(Diagnostic::new(
                            format!(
                                "field '{field}' is already declared in '{}'",
                                display(&parent_info.declared_in[field])
                            ),
                            class.span.clone(),
                        ));
                    }
                    continue;
                }
                def.fields.insert(field.clone(), ty.clone());
            }
            let Some(info) = &mut def.class else {
                continue;
            };
            let inherited = |fields: &BTreeSet<String>| {
                fields
                    .iter()
                    .filter(|field| parent_info.declared_in.contains_key(*field))
                    .cloned()
                    .collect::<Vec<_>>()
            };
            info.private_fields
                .extend(inherited(&parent_info.private_fields));
            info.final_fields
                .extend(inherited(&parent_info.final_fields));
            for (field, declarer) in &parent_info.declared_in {
                info.declared_in
                    .entry(field.clone())
                    .or_insert_with(|| declarer.clone());
            }
        }
    }
    supertypes
}

/// A method as overriding sees it: its name and parameter types after `this`.
type MethodKey = (String, Vec<Type>);

/// Checks overrides and generates what virtual calls need: a dispatcher
/// `F__mcfcVirtual` for each method `F` that a subclass overrides or that is
/// abstract, and `C__mcfcIs(classId)` for `instanceof C`. `unmangled` holds
/// each function's name before overloads were renamed.
fn class_functions(
    program: &Program,
    unmangled: &[String],
    struct_defs: &BTreeMap<String, StructTypeDef>,
    diagnostics: &mut Diagnostics,
) -> Vec<Function> {
    let info = |name: &str| struct_defs.get(name).and_then(|def| def.class.as_ref());
    let display = |name: &str| name.replace("::", ".");
    let mut methods: BTreeMap<(String, MethodKey), usize> = BTreeMap::new();
    for (index, function) in program.functions.iter().enumerate() {
        let Some(owner) = &function.owner else {
            continue;
        };
        if info(owner).is_none() || function.params.first().is_none_or(|p| p.name != "this") {
            continue;
        }
        let Some(bare) = unmangled[index].strip_prefix(&format!("{owner}__")) else {
            continue;
        };
        if bare.starts_with("mcfc") {
            continue;
        }
        let types = function.params[1..].iter().map(|p| p.ty.clone()).collect();
        methods.insert((owner.clone(), (bare.to_string(), types)), index);
    }
    // What an object of `class` runs for `key`: the nearest body, else the
    // nearest abstract declaration.
    let resolve = |class: &str, key: &MethodKey| {
        let found: Vec<usize> = supertypes(class)
            .into_iter()
            .filter_map(|owner| methods.get(&(owner, key.clone())).copied())
            .collect();
        found
            .iter()
            .copied()
            .find(|&index| !program.functions[index].is_abstract)
            .or(found.first().copied())
    };
    let concrete: Vec<(&ClassDef, usize)> = program
        .classes
        .iter()
        .filter(|class| !class.is_abstract)
        .filter_map(|class| Some((class, info(&class.name)?.id)))
        .collect();
    let objects_of = |name: &str| {
        concrete
            .iter()
            .filter(|(class, _)| is_subclass(&class.name, name))
            .copied()
            .collect::<Vec<_>>()
    };

    for &(class, _) in &concrete {
        let keys: BTreeSet<&MethodKey> = methods
            .keys()
            .filter(|(owner, _)| is_subclass(&class.name, owner))
            .map(|(_, key)| key)
            .collect();
        for key in keys {
            if let Some(index) = resolve(&class.name, key)
                && program.functions[index].is_abstract
            {
                diagnostics.push(Diagnostic::new(
                    format!(
                        "'{}' must implement '{}' from '{}'",
                        display(&class.name),
                        key.0,
                        display(
                            program.functions[index]
                                .owner
                                .as_deref()
                                .unwrap_or_default()
                        )
                    ),
                    class.span.clone(),
                ));
            }
        }
    }
    for ((owner, key), &index) in &methods {
        let function = &program.functions[index];
        let overridden = supertypes(owner)
            .into_iter()
            .skip(1)
            .find_map(|supertype| methods.get(&(supertype, key.clone())));
        match overridden {
            Some(&other) if program.functions[other].return_type != function.return_type => {
                diagnostics.push(Diagnostic::new(
                    format!(
                        "'{}' returns '{}', but the method it overrides returns '{}'",
                        key.0,
                        function.return_type.as_str(),
                        program.functions[other].return_type.as_str()
                    ),
                    function.span.clone(),
                ))
            }
            None if function.is_override && !matches!(key.0.as_str(), "toString" | "equals") => {
                diagnostics.push(Diagnostic::new(
                    format!(
                        "'{}' doesn't override a method of a parent class or interface",
                        key.0
                    ),
                    function.span.clone(),
                ))
            }
            _ => {}
        }
    }

    let mut out = Vec::new();
    let mut virtual_methods = HashSet::new();
    for ((owner, key), &index) in &methods {
        let function = &program.functions[index];
        // Each body an object of this type can run, with the classes that run it.
        let mut groups: Vec<(usize, Vec<usize>)> = Vec::new();
        for (class, id) in objects_of(owner) {
            let Some(found) = resolve(&class.name, key) else {
                continue;
            };
            if program.functions[found].is_abstract {
                continue;
            }
            match groups.iter_mut().find(|(body, _)| *body == found) {
                Some((_, ids)) => ids.push(id),
                None => groups.push((found, vec![id])),
            }
        }
        if !function.is_abstract && groups.iter().all(|(body, _)| *body == index) {
            continue;
        }
        virtual_methods.insert(function.name.clone());
        let span = function.span.clone();
        let at = |kind: ExprKind| Expr {
            kind,
            span: span.clone(),
        };
        let stmt = |kind: StmtKind| Stmt {
            kind,
            span: span.clone(),
        };
        let this = || at(ExprKind::Variable("this".to_string()));
        let class_id = || at(ExprKind::Variable("mcfcClassId".to_string()));
        let mut body = Vec::new();
        if groups.len() > 1 {
            body.push(stmt(StmtKind::Let {
                name: "mcfcClassId".to_string(),
                ty: Some(Type::Int),
                value: at(ExprKind::Call {
                    function: "std::heap::classOf".to_string(),
                    args: vec![at(ExprKind::Call {
                        function: "__mcfc_id".to_string(),
                        args: vec![this()],
                    })],
                }),
            }));
        }
        for (position, (target, ids)) in groups.iter().enumerate() {
            let target = &program.functions[*target];
            let receiver = at(ExprKind::Cast {
                ty: Type::Class(target.owner.clone().unwrap_or_default()),
                expr: Box::new(this()),
            });
            let args = std::iter::once(receiver)
                .chain(
                    function.params[1..]
                        .iter()
                        .map(|param| at(ExprKind::Variable(param.name.clone()))),
                )
                .collect();
            let call = at(ExprKind::Call {
                function: format!("{DIRECT}{}", target.name),
                args,
            });
            let branch = if function.return_type == Type::Void {
                vec![stmt(StmtKind::Expr(call)), stmt(StmtKind::Return(None))]
            } else {
                vec![stmt(StmtKind::Return(Some(call)))]
            };
            if position + 1 == groups.len() {
                body.extend(branch);
            } else {
                body.push(stmt(StmtKind::If {
                    condition: class_test(ids, class_id, &at),
                    then_body: branch,
                    else_body: Vec::new(),
                }));
            }
        }
        if groups.is_empty() && function.return_type != Type::Void {
            body.push(stmt(StmtKind::Return(Some(at(ExprKind::Variable(
                "__mcfc_default".to_string(),
            ))))));
        }
        out.push(Function {
            name: format!("{}{VIRTUAL_SUFFIX}", function.name),
            body,
            is_abstract: false,
            is_override: false,
            ..function.clone()
        });
    }
    VIRTUAL.with(|set| *set.borrow_mut() = virtual_methods);

    for class in &program.classes {
        let span = class.span.clone();
        let at = |kind: ExprKind| Expr {
            kind,
            span: span.clone(),
        };
        let ids: Vec<usize> = objects_of(&class.name)
            .into_iter()
            .map(|(_, id)| id)
            .collect();
        let test = if ids.is_empty() {
            at(ExprKind::Bool(false))
        } else {
            class_test(&ids, || at(ExprKind::Variable("classId".to_string())), &at)
        };
        out.push(Function {
            name: format!("{}{IS_SUFFIX}", class.name),
            is_pub: true,
            type_params: Vec::new(),
            params: vec![Param {
                name: "classId".to_string(),
                ty: Type::Int,
                span: span.clone(),
            }],
            return_type: Type::Bool,
            body: vec![Stmt {
                kind: StmtKind::Return(Some(test)),
                span: span.clone(),
            }],
            span: span.clone(),
            end: 0,
            owner: None,
            module: class
                .name
                .rsplit_once("::")
                .map_or(String::new(), |(module, _)| module.to_string()),
            is_abstract: false,
            is_override: false,
        });
    }
    out
}

/// `classId == 3 || classId == 5`.
fn class_test(ids: &[usize], class_id: impl Fn() -> Expr, at: &impl Fn(ExprKind) -> Expr) -> Expr {
    ids.iter()
        .map(|id| {
            at(ExprKind::Binary {
                op: BinaryOp::Eq,
                left: Box::new(class_id()),
                right: Box::new(at(ExprKind::Int(*id as i64))),
            })
        })
        .reduce(|left, right| {
            at(ExprKind::Binary {
                op: BinaryOp::Or,
                left: Box::new(left),
                right: Box::new(right),
            })
        })
        .expect("at least one class id")
}

/// A local or a field path, which reads the same every time.
fn is_plain_place(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Variable(_) => true,
        ExprKind::Path(path) => {
            matches!(path.base.kind, ExprKind::Variable(_))
                && path
                    .segments
                    .iter()
                    .all(|segment| matches!(segment, PathSegment::Field(_)))
        }
        _ => false,
    }
}

fn has_binding(condition: &Expr) -> bool {
    match &condition.kind {
        ExprKind::Binary {
            op: BinaryOp::And,
            left,
            right,
        } => has_binding(left) || has_binding(right),
        ExprKind::InstanceOf { binding, .. } => binding.is_some(),
        _ => false,
    }
}

/// Moves each `x instanceof C c` binding in an `&&` chain into `C c = (C) x;`.
fn take_bindings(condition: &mut Expr, block: &mut Vec<Stmt>, diagnostics: &mut Diagnostics) {
    match &mut condition.kind {
        ExprKind::Binary {
            op: BinaryOp::And,
            left,
            right,
        } => {
            take_bindings(left, block, diagnostics);
            take_bindings(right, block, diagnostics);
        }
        ExprKind::InstanceOf { expr, ty, binding } => {
            let Some(name) = binding.take() else {
                return;
            };
            if !is_plain_place(expr) {
                diagnostics.push(Diagnostic::new(
                    "a pattern variable matches a variable or field; put the value in a local first",
                    condition.span.clone(),
                ));
            }
            block.push(Stmt {
                kind: StmtKind::Let {
                    name,
                    ty: Some(ty.clone()),
                    value: Expr {
                        kind: ExprKind::Cast {
                            ty: ty.clone(),
                            expr: expr.clone(),
                        },
                        span: condition.span.clone(),
                    },
                },
                span: condition.span.clone(),
            });
        }
        _ => {}
    }
}

/// `switch (shape) { case Circle c -> ...; }` as a value, when its cases are types.
#[allow(clippy::type_complexity)]
fn type_switch_expr(value: &Expr) -> Option<(&Expr, &[(Expr, Expr)], Option<&Expr>)> {
    match &value.kind {
        ExprKind::Switch {
            value,
            arms,
            default,
        } if arms
            .iter()
            .any(|(pattern, _)| matches!(pattern.kind, ExprKind::InstanceOf { .. })) =>
        {
            Some((value, arms, default.as_deref()))
        }
        _ => None,
    }
}

/// Plain statements for a statement with type patterns, and whether they
/// form their own scope. `None` for any other statement.
fn lower_patterns(
    statement: &Stmt,
    struct_defs: &BTreeMap<String, StructTypeDef>,
    signatures: &BTreeMap<String, FunctionSignature>,
    env: &HashMap<String, Type>,
    ref_env: &HashMap<String, RefKind>,
    diagnostics: &mut Diagnostics,
) -> Option<(Vec<Stmt>, bool)> {
    let span = statement.span.clone();
    let stmt = |kind: StmtKind| Stmt {
        kind,
        span: span.clone(),
    };
    // `x = switch (...) { case C c -> e; }` sets `x` in each case instead.
    let set_in_cases = |value: &Expr, each: &dyn Fn(&Expr) -> Vec<Stmt>| {
        let (subject, arms, default) = type_switch_expr(value)?;
        Some(stmt(StmtKind::Switch {
            value: subject.clone(),
            arms: arms
                .iter()
                .map(|(pattern, result)| SwitchArm {
                    pattern: pattern.clone(),
                    body: each(result),
                })
                .collect(),
            default_body: default.map(each).unwrap_or_default(),
        }))
    };
    match &statement.kind {
        StmtKind::If {
            condition,
            then_body,
            else_body,
        } if has_binding(condition) => {
            let mut condition = condition.clone();
            let mut block = Vec::new();
            take_bindings(&mut condition, &mut block, diagnostics);
            block.push(stmt(StmtKind::If {
                condition,
                then_body: then_body.clone(),
                else_body: else_body.clone(),
            }));
            Some((block, true))
        }
        StmtKind::Switch {
            value,
            arms,
            default_body,
        } if arms
            .iter()
            .any(|arm| matches!(arm.pattern.kind, ExprKind::InstanceOf { .. })) =>
        {
            let block = type_switch(
                value,
                arms,
                default_body,
                &span,
                struct_defs,
                signatures,
                env,
                ref_env,
                diagnostics,
            );
            Some((block, true))
        }
        StmtKind::Let {
            name,
            ty: Some(ty),
            value,
        } => {
            let switch = set_in_cases(value, &|result| {
                vec![stmt(StmtKind::Assign {
                    target: AssignTarget::Variable(name.clone()),
                    value: result.clone(),
                })]
            })?;
            let declare = stmt(StmtKind::Let {
                name: name.clone(),
                ty: Some(ty.clone()),
                value: Expr {
                    kind: ExprKind::Variable("__mcfc_default".to_string()),
                    span: span.clone(),
                },
            });
            Some((vec![declare, switch], false))
        }
        StmtKind::Assign { target, value } => {
            let switch = set_in_cases(value, &|result| {
                vec![stmt(StmtKind::Assign {
                    target: target.clone(),
                    value: result.clone(),
                })]
            })?;
            Some((vec![switch], false))
        }
        StmtKind::Return(Some(value)) => {
            let switch = set_in_cases(value, &|result| {
                vec![stmt(StmtKind::Return(Some(result.clone())))]
            })?;
            Some((vec![switch], false))
        }
        _ => None,
    }
}

/// `switch (shape) { case Circle c -> body; ... default -> body; }` as
/// `if (shape instanceof Circle) { Circle c = (Circle) shape; body } else ...`.
/// Without a `default`, the cases must cover every class the value can be.
#[allow(clippy::too_many_arguments)]
fn type_switch(
    value: &Expr,
    arms: &[SwitchArm],
    default_body: &[Stmt],
    span: &Span,
    struct_defs: &BTreeMap<String, StructTypeDef>,
    signatures: &BTreeMap<String, FunctionSignature>,
    env: &HashMap<String, Type>,
    ref_env: &HashMap<String, RefKind>,
    diagnostics: &mut Diagnostics,
) -> Vec<Stmt> {
    let stmt = |kind: StmtKind| Stmt {
        kind,
        span: span.clone(),
    };
    let at = |kind: ExprKind| Expr {
        kind,
        span: span.clone(),
    };
    let value_ty = type_check_expr(
        value,
        struct_defs,
        signatures,
        env,
        ref_env,
        &mut BTreeSet::new(),
        &mut Diagnostics::new(),
    )
    .ty;
    let Type::Class(owner) = &value_ty else {
        diagnostics.push(Diagnostic::new(
            format!(
                "a 'case' with a type needs an object to switch on, not '{}'",
                value_ty.as_str()
            ),
            span.clone(),
        ));
        return Vec::new();
    };
    let mut block = Vec::new();
    let subject = if is_plain_place(value) {
        value.clone()
    } else {
        let name = format!("mcfcSwitch{}", span.range.start);
        block.push(stmt(StmtKind::Let {
            name: name.clone(),
            ty: Some(value_ty.clone()),
            value: value.clone(),
        }));
        at(ExprKind::Variable(name))
    };
    let mut covered = Vec::new();
    let mut chain = default_body.to_vec();
    for arm in arms.iter().rev() {
        let ExprKind::InstanceOf {
            ty,
            binding: Some(binding),
            ..
        } = &arm.pattern.kind
        else {
            diagnostics.push(Diagnostic::new(
                "a switch can't mix type patterns with constants",
                arm.pattern.span.clone(),
            ));
            continue;
        };
        let mut resolved = ty.clone();
        resolve_enum_type(&mut resolved, struct_defs);
        if let Type::Class(name) = resolved {
            covered.push(name);
        }
        let mut then_body = vec![stmt(StmtKind::Let {
            name: binding.clone(),
            ty: Some(ty.clone()),
            value: at(ExprKind::Cast {
                ty: ty.clone(),
                expr: Box::new(subject.clone()),
            }),
        })];
        then_body.extend(arm.body.iter().cloned());
        chain = vec![stmt(StmtKind::If {
            condition: at(ExprKind::InstanceOf {
                expr: Box::new(subject.clone()),
                ty: ty.clone(),
                binding: None,
            }),
            then_body,
            else_body: chain,
        })];
    }
    if default_body.is_empty() {
        for (name, def) in struct_defs {
            let Some(info) = &def.class else {
                continue;
            };
            if !info.is_abstract
                && is_subclass(name, owner)
                && !covered.iter().any(|case| is_subclass(name, case))
            {
                diagnostics.push(Diagnostic::new(
                    format!(
                        "the switch doesn't cover '{}'; add a 'case' for it or a 'default'",
                        name.replace("::", ".")
                    ),
                    span.clone(),
                ));
            }
        }
    }
    block.extend(chain);
    block
}
