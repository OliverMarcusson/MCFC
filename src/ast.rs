use crate::diagnostics::Span;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub structs: Vec<StructDef>,
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
            Type::Struct(name) => name.replace("::", "."),
            Type::Enum(name) => name.replace("::", "."),
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
    New {
        name: String,
        args: Vec<Expr>,
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
    },
    MethodCall {
        receiver: Box<Expr>,
        method: String,
        args: Vec<Expr>,
    },
    Path(PathExpr),
    /// `condition ? then_expr : else_expr`
    Conditional {
        condition: Box<Expr>,
        then_expr: Box<Expr>,
        else_expr: Box<Expr>,
    },
    /// `switch (value) { case p -> result; ... default -> result; }`, one entry per pattern.
    Switch {
        value: Box<Expr>,
        arms: Vec<(Expr, Expr)>,
        default: Option<Box<Expr>>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Not,
    Neg,
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
