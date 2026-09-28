use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};

use crate::diagnostics::Span;

thread_local! {
    /// How a copy of a generic class is written: `util::Box__int` is `Box<Integer>`.
    pub static CLASS_DISPLAY: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
}

/// Marks a method name that is also a builtin's internal name, such as
/// `push`, as written in source. Only user types' methods may use them.
pub const WRITTEN_METHOD: &str = "@written:";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub structs: Vec<StructDef>,
    pub classes: Vec<ClassDef>,
    pub enums: Vec<EnumDef>,
    pub player_states: Vec<PlayerStateDef>,
    /// `@WorldState` fields: one value for the whole world, read by name.
    pub world_states: Vec<PlayerStateDef>,
    pub functions: Vec<Function>,
    pub uses: Vec<UseDecl>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumDef {
    pub name: String,
    pub is_pub: bool,
    pub variants: Vec<String>,
    /// What each constant passes to the constructor, in `variants` order.
    pub args: Vec<Vec<Expr>>,
    pub constructor: Vec<Param>,
    pub fields: Vec<EnumField>,
    pub span: Span,
}

/// `class Counter { ... }`. Constructors and methods are in `Program::functions`
/// with this class as their owner; `new Counter(...)` calls `Counter__new`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassDef {
    pub name: String,
    pub is_pub: bool,
    pub fields: Vec<ClassField>,
    /// `extends Parent`; an interface's parents are in `interfaces`.
    pub parent: Option<String>,
    /// `implements A, B`, or an interface's `extends A, B`.
    pub interfaces: Vec<String>,
    pub is_interface: bool,
    pub is_abstract: bool,
    pub is_final: bool,
    /// `sealed ... permits A, B`: only these may extend or implement it.
    pub permits: Option<Vec<String>>,
    /// `class Box<T extends Bound>`: each use such as `Box<Integer>` becomes
    /// its own class (see `generics`).
    pub type_params: Vec<String>,
    pub bounds: Vec<(String, Type)>,
    /// Type arguments given to a supertype: `implements Comparator<Player>`.
    pub super_args: BTreeMap<String, Vec<Type>>,
    pub span: Span,
}

/// A field in a class, enum or record body, such as `private int count = 0;`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassField {
    pub name: String,
    pub ty: Type,
    pub is_pub: bool,
    pub is_static: bool,
    pub is_final: bool,
    pub init: Option<Expr>,
    pub span: Span,
}

/// `private final float mass;`, set by the constructor from parameter `param`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumField {
    pub name: String,
    pub ty: Type,
    pub param: Option<usize>,
    pub span: Span,
}

/// `import a.b.c;` imports `c` under its own name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UseDecl {
    pub path: Vec<String>,
    pub alias: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructDef {
    pub name: String,
    pub is_pub: bool,
    pub fields: Vec<StructField>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructField {
    pub name: String,
    pub ty: Type,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerStateDef {
    pub owner: StateOwner,
    pub path: Vec<String>,
    pub ty: Type,
    pub display_name: String,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateOwner {
    Player,
    Entity,
    World,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Function {
    pub name: String,
    pub is_pub: bool,
    /// `<T, U> R name(...)`; each call compiles a copy with the types filled in.
    pub type_params: Vec<String>,
    /// `<T extends Animal>`: what a type argument must be.
    pub bounds: Vec<(String, Type)>,
    pub params: Vec<Param>,
    pub return_type: Type,
    pub body: Vec<Stmt>,
    pub span: Span,
    /// Byte offset just past the closing `}`.
    pub end: usize,
    /// The record or enum a method is declared in. The function is named
    /// `Type__method`, and an instance method's first parameter is `this`.
    pub owner: Option<String>,
    /// The module's path joined with `::`, filled in by the module resolver.
    pub module: String,
    /// An abstract or interface method: a signature without a body.
    pub is_abstract: bool,
    /// Marked `@Override`; the type checker checks that it overrides something.
    pub is_override: bool,
    /// `int sum(int... values)`: the last parameter is a list the call's
    /// extra arguments fill.
    pub varargs: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Param {
    pub name: String,
    pub ty: Type,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Type {
    Int,
    Float,
    Bool,
    String,
    Array(Box<Type>),
    Dict(Box<Type>),
    Optional(Box<Type>),
    Struct(String),
    Enum(String),
    /// A class instance: the id of its heap slot, or 0 for `null`.
    Class(String),
    /// `Box<T>` inside generic code; concrete uses become the class copy's `Struct`.
    Generic(String, Vec<Type>),
    Bossbar,
    EntitySet,
    EntityRef,
    PlayerRef,
    BlockRef,
    EntityDef,
    BlockDef,
    ItemDef,
    TextDef,
    ItemSlot,
    Nbt,
    Void,
}

impl Type {
    pub fn as_str(&self) -> String {
        match self {
            Type::Int => "int".to_string(),
            Type::Float => "float".to_string(),
            Type::Bool => "boolean".to_string(),
            Type::String => "String".to_string(),
            Type::Array(element) => format!("List<{}>", element.as_type_arg()),
            Type::Dict(value) => format!("Map<String, {}>", value.as_type_arg()),
            Type::Optional(value) => format!("Optional<{}>", value.as_type_arg()),
            Type::Struct(name) => CLASS_DISPLAY
                .with(|map| map.borrow().get(name).cloned())
                .unwrap_or_else(|| name.replace("::", ".")),
            Type::Class(name) if name.is_empty() => "null".to_string(),
            Type::Enum(name) | Type::Class(name) => CLASS_DISPLAY
                .with(|map| map.borrow().get(name).cloned())
                .unwrap_or_else(|| name.replace("::", ".")),
            Type::Generic(name, args) => {
                let args: Vec<String> = args.iter().map(Type::as_type_arg).collect();
                format!("{}<{}>", name.replace("::", "."), args.join(", "))
            }
            Type::Bossbar => "BossBar".to_string(),
            Type::EntitySet => "Selector".to_string(),
            Type::EntityRef => "Entity".to_string(),
            Type::PlayerRef => "Player".to_string(),
            Type::BlockRef => "Block".to_string(),
            Type::EntityDef => "EntityData".to_string(),
            Type::BlockDef => "BlockData".to_string(),
            Type::ItemDef => "ItemStack".to_string(),
            Type::TextDef => "Component".to_string(),
            Type::ItemSlot => "ItemSlot".to_string(),
            Type::Nbt => "Nbt".to_string(),
            Type::Void => "void".to_string(),
        }
    }

    /// The name inside `<...>`, where primitives are boxed: `List<Integer>`.
    pub fn as_type_arg(&self) -> String {
        match self {
            Type::Int => "Integer".to_string(),
            Type::Float => "Float".to_string(),
            Type::Bool => "Boolean".to_string(),
            other => other.as_str(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StmtKind {
    /// `var name = value;` (no `ty`) or `T name = value;`.
    Let {
        name: String,
        ty: Option<Type>,
        value: Expr,
    },
    Assign {
        target: AssignTarget,
        value: Expr,
    },
    If {
        condition: Expr,
        then_body: Vec<Stmt>,
        else_body: Vec<Stmt>,
    },
    /// `step` runs after each iteration and on `continue` (a C-style `for` update).
    While {
        condition: Expr,
        body: Vec<Stmt>,
        step: Vec<Stmt>,
    },
    /// `for (T name : iterable)`; `ty` is `None` for `var`.
    For {
        name: String,
        ty: Option<Type>,
        iterable: Expr,
        body: Vec<Stmt>,
    },
    /// A `{ ... }` scope, also what a C-style `for` desugars to.
    Block(Vec<Stmt>),
    Switch {
        value: Expr,
        arms: Vec<SwitchArm>,
        default_body: Vec<Stmt>,
    },
    Context {
        kind: ContextKind,
        anchor: Expr,
        body: Vec<Stmt>,
    },
    Async {
        body: Vec<Stmt>,
    },
    Break,
    Continue,
    Return(Option<Expr>),
    /// `throw value;`. Lowered by `exceptions::lower` before type checking.
    Throw(Expr),
    /// `try { ... } catch (A | B name) { ... } finally { ... }`, lowered with `Throw`.
    Try {
        body: Vec<Stmt>,
        catches: Vec<Catch>,
        finally: Vec<Stmt>,
    },
    RawCommand(String),
    MacroCommand(String),
    Expr(Expr),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextKind {
    As,
    At,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SleepUnit {
    Seconds,
    Ticks,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssignTarget {
    Variable(String),
    Path(PathExpr),
}

/// `catch (A | B name) { body }`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Catch {
    pub types: Vec<Type>,
    pub name: String,
    pub body: Vec<Stmt>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwitchArm {
    pub pattern: Expr,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathExpr {
    pub base: Box<Expr>,
    pub segments: Vec<PathSegment>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathSegment {
    Field(String),
    Index(Box<Expr>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExprKind {
    Int(i64),
    Float(String),
    Bool(bool),
    String(String),
    ArrayLiteral(Vec<Expr>),
    DictLiteral(Vec<(String, Expr)>),
    StructLiteral {
        name: String,
        fields: Vec<(String, Expr)>,
    },
    /// `new Name(args)`: a record (positional fields) or a builtin builder type.
    /// `type_args` is `Some` for `new Box<Integer>(...)`, empty for `new Box<>(...)`.
    New {
        name: String,
        args: Vec<Expr>,
        type_args: Option<Vec<Type>>,
    },
    Variable(String),
    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
    },
    Binary {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Call {
        function: String,
        args: Vec<Expr>,
        /// `f<Integer>(x)`: type arguments written out, empty when inferred.
        type_args: Vec<Type>,
    },
    MethodCall {
        receiver: Box<Expr>,
        method: String,
        args: Vec<Expr>,
        /// `Util.<Integer>f(x)`: type arguments written out, empty when inferred.
        type_args: Vec<Type>,
    },
    Path(PathExpr),
    /// `condition ? then_expr : else_expr`
    Conditional {
        condition: Box<Expr>,
        then_expr: Box<Expr>,
        else_expr: Box<Expr>,
    },
    /// `value instanceof Type`, or `value instanceof Type name`, which declares
    /// `name`. As a `case` pattern, `value` is an empty variable.
    InstanceOf {
        expr: Box<Expr>,
        ty: Type,
        binding: Option<String>,
    },
    /// `(Circle) shape`: a class cast. Primitive casts are builtin calls.
    Cast {
        ty: Type,
        expr: Box<Expr>,
    },
    /// `switch (value) { case p -> result; ... default -> result; }`, one entry per pattern.
    Switch {
        value: Box<Expr>,
        arms: Vec<(Expr, Expr)>,
        default: Option<Box<Expr>>,
    },
    /// `x -> x * 2` or `(int a, int b) -> { ... }`. A parameter's type is
    /// `None` when not written. An expression body is one `return`, marked
    /// by `expression`, since it is a statement when the method is `void`.
    Lambda {
        params: Vec<(String, Option<Type>)>,
        body: Vec<Stmt>,
        expression: bool,
    },
    /// `Type::method`, `Type::new`, `this::method` or `variable::method`.
    MethodRef {
        target: String,
        method: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Not,
    Neg,
    /// `~x`
    BitNot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    NotEq,
    Lt,
    Lte,
    Gt,
    Gte,
    And,
    Or,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
}
