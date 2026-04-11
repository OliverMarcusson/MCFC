use std::collections::BTreeMap;

use crate::ast::{BinaryOp, Type};
use crate::types::{
    BookCommand, CastKind, MacroPlaceholder, RefKind, TypedAssignTarget, TypedExpr, TypedExprKind,
    TypedForKind, TypedFunction, TypedPathExpr, TypedProgram, TypedStmt, TypedStmtKind,
};

#[derive(Debug, Clone)]
pub struct IrProgram {
    pub functions: Vec<IrFunction>,
    pub call_depths: BTreeMap<String, usize>,
    pub book_commands: BTreeMap<String, IrBookCommand>,
}

#[derive(Debug, Clone)]
pub struct IrFunction {
    pub name: String,
    pub params: Vec<IrParam>,
    pub return_type: Type,
    pub body: Vec<IrStmt>,
    pub locals: BTreeMap<String, Type>,
    pub book_exposed: bool,
}

#[derive(Debug, Clone)]
pub struct IrBookCommand {
    pub function_name: String,
    pub arg_count: usize,
}

#[derive(Debug, Clone)]
pub struct IrParam {
    pub name: String,
    pub ty: Type,
}

#[derive(Debug, Clone)]
pub enum IrStmt {
    Let {
        name: String,
        ty: Type,
        value: IrExpr,
    },
    Assign {
        target: IrAssignTarget,
        value: IrExpr,
    },
    If {
        condition: IrExpr,
        then_body: Vec<IrStmt>,
        else_body: Vec<IrStmt>,
    },
    While {
        condition: IrExpr,
        body: Vec<IrStmt>,
    },
    For {
        name: String,
        kind: IrForKind,
        body: Vec<IrStmt>,
    },
    Break,
    Continue,
    Return(Option<IrExpr>),
    RawCommand(String),
    MacroCommand {
        template: String,
        placeholders: Vec<IrMacroPlaceholder>,
    },
    Expr(IrExpr),
}

#[derive(Debug, Clone)]
pub enum IrAssignTarget {
    Variable(String),
    Path(IrPathExpr),
}

#[derive(Debug, Clone)]
pub enum IrForKind {
    Range {
        start: IrExpr,
        end: IrExpr,
        inclusive: bool,
    },
    Each {
        iterable: IrExpr,
    },
}

#[derive(Debug, Clone)]
pub struct IrMacroPlaceholder {
    pub name: String,
    pub ty: Type,
}

#[derive(Debug, Clone)]
pub struct IrExpr {
    pub ty: Type,
    pub ref_kind: RefKind,
    pub kind: IrExprKind,
}

#[derive(Debug, Clone)]
pub struct IrPathExpr {
    pub base: Box<IrExpr>,
    pub segments: Vec<crate::ast::PathSegment>,
    pub ty: Type,
}

#[derive(Debug, Clone)]
pub enum IrExprKind {
    Int(i64),
    Bool(bool),
    String(String),
    ArrayLiteral(Vec<IrExpr>),
    DictLiteral(Vec<(String, IrExpr)>),
    Variable(String),
    Selector(String),
    Block(String),
    Unary {
        op: crate::ast::UnaryOp,
        expr: Box<IrExpr>,
    },
    Binary {
        op: BinaryOp,
        left: Box<IrExpr>,
        right: Box<IrExpr>,
    },
    Call {
        function: String,
        args: Vec<IrExpr>,
    },
    MethodCall {
        receiver: Box<IrExpr>,
        method: String,
        args: Vec<IrExpr>,
    },
    Single(Box<IrExpr>),
    Exists(Box<IrExpr>),
    At {
        anchor: Box<IrExpr>,
        value: Box<IrExpr>,
    },
    Path(IrPathExpr),
    Cast {
        kind: crate::types::CastKind,
        expr: Box<IrExpr>,
    },
}

pub fn lower(program: &TypedProgram) -> IrProgram {
    IrProgram {
        functions: program.functions.iter().map(lower_function).collect(),
        call_depths: program.call_depths.clone(),
        book_commands: program
            .book_commands
            .iter()
            .map(|(name, command)| (name.clone(), lower_book_command(command)))
            .collect(),
    }
}

fn lower_function(function: &TypedFunction) -> IrFunction {
    IrFunction {
        name: function.name.clone(),
        params: function
            .params
            .iter()
            .map(|param| IrParam {
                name: param.name.clone(),
                ty: param.ty.clone(),
            })
            .collect(),
        return_type: function.return_type.clone(),
        body: function.body.iter().map(lower_stmt).collect(),
        locals: function.locals.clone(),
        book_exposed: function.book_exposed,
    }
}

fn lower_book_command(command: &BookCommand) -> IrBookCommand {
    IrBookCommand {
        function_name: command.function_name.clone(),
        arg_count: command.arg_count,
    }
}

fn lower_stmt(stmt: &TypedStmt) -> IrStmt {
    match &stmt.kind {
        TypedStmtKind::Let { name, ty, value } => IrStmt::Let {
            name: name.clone(),
            ty: ty.clone(),
            value: lower_expr(value),
        },
        TypedStmtKind::Assign { target, value } => IrStmt::Assign {
            target: lower_assign_target(target),
            value: lower_expr(value),
        },
        TypedStmtKind::If {
            condition,
            then_body,
            else_body,
        } => IrStmt::If {
            condition: lower_expr(condition),
            then_body: then_body.iter().map(lower_stmt).collect(),
            else_body: else_body.iter().map(lower_stmt).collect(),
        },
        TypedStmtKind::While { condition, body } => IrStmt::While {
            condition: lower_expr(condition),
            body: body.iter().map(lower_stmt).collect(),
        },
        TypedStmtKind::For { name, kind, body } => IrStmt::For {
            name: name.clone(),
            kind: lower_for_kind(kind),
            body: body.iter().map(lower_stmt).collect(),
        },
        TypedStmtKind::Break => IrStmt::Break,
        TypedStmtKind::Continue => IrStmt::Continue,
        TypedStmtKind::Return(value) => IrStmt::Return(value.as_ref().map(lower_expr)),
        TypedStmtKind::RawCommand(raw) => IrStmt::RawCommand(raw.clone()),
        TypedStmtKind::MacroCommand {
            template,
            placeholders,
        } => IrStmt::MacroCommand {
            template: template.clone(),
            placeholders: placeholders.iter().map(lower_macro_placeholder).collect(),
        },
        TypedStmtKind::Expr(expr) => IrStmt::Expr(lower_expr(expr)),
    }
}

fn lower_assign_target(target: &TypedAssignTarget) -> IrAssignTarget {
    match target {
        TypedAssignTarget::Variable(name) => IrAssignTarget::Variable(name.clone()),
        TypedAssignTarget::Path(path) => IrAssignTarget::Path(lower_path_expr(path)),
    }
}

fn lower_for_kind(kind: &TypedForKind) -> IrForKind {
    match kind {
        TypedForKind::Range {
            start,
            end,
            inclusive,
        } => IrForKind::Range {
            start: lower_expr(start),
            end: lower_expr(end),
            inclusive: *inclusive,
        },
        TypedForKind::Each { iterable } => IrForKind::Each {
            iterable: lower_expr(iterable),
        },
    }
}

fn lower_path_expr(path: &TypedPathExpr) -> IrPathExpr {
    IrPathExpr {
        base: Box::new(lower_expr(&path.base)),
        segments: path.segments.clone(),
        ty: path.ty.clone(),
    }
}

fn lower_macro_placeholder(placeholder: &MacroPlaceholder) -> IrMacroPlaceholder {
    IrMacroPlaceholder {
        name: placeholder.name.clone(),
        ty: placeholder.ty.clone(),
    }
}

fn lower_expr(expr: &TypedExpr) -> IrExpr {
    IrExpr {
        ty: expr.ty.clone(),
        ref_kind: expr.ref_kind,
        kind: match &expr.kind {
            TypedExprKind::Int(value) => IrExprKind::Int(*value),
            TypedExprKind::Bool(value) => IrExprKind::Bool(*value),
            TypedExprKind::String(value) => IrExprKind::String(value.clone()),
            TypedExprKind::ArrayLiteral(values) => {
                IrExprKind::ArrayLiteral(values.iter().map(lower_expr).collect())
            }
            TypedExprKind::DictLiteral(entries) => IrExprKind::DictLiteral(
                entries
                    .iter()
                    .map(|(key, value)| (key.clone(), lower_expr(value)))
                    .collect(),
            ),
            TypedExprKind::Variable(name) => IrExprKind::Variable(name.clone()),
            TypedExprKind::Selector(value) => IrExprKind::Selector(value.clone()),
            TypedExprKind::Block(value) => IrExprKind::Block(value.clone()),
            TypedExprKind::Unary { op, expr } => IrExprKind::Unary {
                op: *op,
                expr: Box::new(lower_expr(expr)),
            },
            TypedExprKind::Binary { op, left, right } => IrExprKind::Binary {
                op: *op,
                left: Box::new(lower_expr(left)),
                right: Box::new(lower_expr(right)),
            },
            TypedExprKind::Call { function, args } => IrExprKind::Call {
                function: function.clone(),
                args: args.iter().map(lower_expr).collect(),
            },
            TypedExprKind::MethodCall {
                receiver,
                method,
                args,
            } => IrExprKind::MethodCall {
                receiver: Box::new(lower_expr(receiver)),
                method: method.clone(),
                args: args.iter().map(lower_expr).collect(),
            },
            TypedExprKind::Single(expr) => IrExprKind::Single(Box::new(lower_expr(expr))),
            TypedExprKind::Exists(expr) => IrExprKind::Exists(Box::new(lower_expr(expr))),
            TypedExprKind::At { anchor, value } => IrExprKind::At {
                anchor: Box::new(lower_expr(anchor)),
                value: Box::new(lower_expr(value)),
            },
            TypedExprKind::Path(path) => IrExprKind::Path(lower_path_expr(path)),
            TypedExprKind::Cast { kind, expr } => IrExprKind::Cast {
                kind: match kind {
                    CastKind::Int => CastKind::Int,
                    CastKind::Bool => CastKind::Bool,
                    CastKind::String => CastKind::String,
                },
                expr: Box::new(lower_expr(expr)),
            },
        },
    }
}
