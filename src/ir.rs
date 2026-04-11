use std::collections::BTreeMap;

use crate::ast::{BinaryOp, Type};
use crate::types::{
    BookCommand, MacroPlaceholder, TypedExpr, TypedExprKind, TypedFunction, TypedProgram,
    TypedStmt, TypedStmtKind,
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
        name: String,
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
        start: IrExpr,
        end: IrExpr,
        inclusive: bool,
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
pub struct IrMacroPlaceholder {
    pub name: String,
    pub ty: Type,
}

#[derive(Debug, Clone)]
pub struct IrExpr {
    pub ty: Type,
    pub kind: IrExprKind,
}

#[derive(Debug, Clone)]
pub enum IrExprKind {
    Int(i64),
    Bool(bool),
    String(String),
    Variable(String),
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
        TypedStmtKind::Assign { name, value } => IrStmt::Assign {
            name: name.clone(),
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
        TypedStmtKind::For {
            name,
            start,
            end,
            inclusive,
            body,
        } => IrStmt::For {
            name: name.clone(),
            start: lower_expr(start),
            end: lower_expr(end),
            inclusive: *inclusive,
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

fn lower_macro_placeholder(placeholder: &MacroPlaceholder) -> IrMacroPlaceholder {
    IrMacroPlaceholder {
        name: placeholder.name.clone(),
        ty: placeholder.ty.clone(),
    }
}

fn lower_expr(expr: &TypedExpr) -> IrExpr {
    IrExpr {
        ty: expr.ty.clone(),
        kind: match &expr.kind {
            TypedExprKind::Int(value) => IrExprKind::Int(*value),
            TypedExprKind::Bool(value) => IrExprKind::Bool(*value),
            TypedExprKind::String(value) => IrExprKind::String(value.clone()),
            TypedExprKind::Variable(name) => IrExprKind::Variable(name.clone()),
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
        },
    }
}
