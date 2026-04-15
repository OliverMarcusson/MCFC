use crate::diagnostics::Span;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub structs: Vec<StructDef>,
    pub player_states: Vec<PlayerStateDef>,
    pub functions: Vec<Function>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructDef {
    pub name: String,
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
    pub path: Vec<String>,
    pub ty: Type,
    pub display_name: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Function {
    pub name: String,
    pub params: Vec<Param>,
    pub return_type: Type,
    pub body: Vec<Stmt>,
    pub span: Span,
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
    Bool,
    String,
    Array(Box<Type>),
    Dict(Box<Type>),
    Struct(String),
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
            Type::Bool => "bool".to_string(),
            Type::String => "string".to_string(),
            Type::Array(element) => format!("array<{}>", element.as_str()),
            Type::Dict(value) => format!("dict<{}>", value.as_str()),
            Type::Struct(name) => name.clone(),
            Type::Bossbar => "bossbar".to_string(),
            Type::EntitySet => "entity_set".to_string(),
            Type::EntityRef => "entity_ref".to_string(),
            Type::PlayerRef => "player_ref".to_string(),
            Type::BlockRef => "block_ref".to_string(),
            Type::EntityDef => "entity_def".to_string(),
            Type::BlockDef => "block_def".to_string(),
            Type::ItemDef => "item_def".to_string(),
            Type::TextDef => "text_def".to_string(),
            Type::ItemSlot => "item_slot".to_string(),
            Type::Nbt => "nbt".to_string(),
            Type::Void => "void".to_string(),
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
    Let {
        name: String,
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
    While {
        condition: Expr,
        body: Vec<Stmt>,
    },
    For {
        name: String,
        kind: ForKind,
        body: Vec<Stmt>,
    },
    Match {
        value: Expr,
        arms: Vec<MatchArm>,
        else_body: Vec<Stmt>,
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
pub enum ForKind {
    Range {
        start: Expr,
        end: Expr,
        inclusive: bool,
    },
    Each {
        iterable: Expr,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchArm {
    pub pattern: String,
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
    Bool(bool),
    String(String),
    ArrayLiteral(Vec<Expr>),
    DictLiteral(Vec<(String, Expr)>),
    StructLiteral {
        name: String,
        fields: Vec<(String, Expr)>,
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
    Eq,
    NotEq,
    Lt,
    Lte,
    Gt,
    Gte,
    And,
    Or,
}
