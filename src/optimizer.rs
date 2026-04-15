use crate::ast::{BinaryOp, UnaryOp};
use crate::ir::{IrAssignTarget, IrExpr, IrExprKind, IrForKind, IrFunction, IrProgram, IrStmt};

pub fn optimize(mut program: IrProgram) -> IrProgram {
    for function in &mut program.functions {
        optimize_function(function);
    }
    program
}

fn optimize_function(function: &mut IrFunction) {
    function.body = optimize_stmts(std::mem::take(&mut function.body));
}

fn optimize_stmts(stmts: Vec<IrStmt>) -> Vec<IrStmt> {
    let mut optimized = Vec::new();
    for stmt in stmts {
        match optimize_stmt(stmt) {
            Some(IrStmt::If {
                condition,
                then_body,
                else_body,
            }) => match condition.kind {
                IrExprKind::Bool(true) if !contains_control_flow(&then_body) => {
                    optimized.extend(then_body)
                }
                IrExprKind::Bool(false) if !contains_control_flow(&else_body) => {
                    optimized.extend(else_body)
                }
                _ => optimized.push(IrStmt::If {
                    condition,
                    then_body,
                    else_body,
                }),
            },
            Some(stmt) => optimized.push(stmt),
            None => {}
        }
    }
    optimized
}

fn contains_control_flow(stmts: &[IrStmt]) -> bool {
    stmts.iter().any(|stmt| match stmt {
        IrStmt::Break | IrStmt::Continue | IrStmt::Return(_) | IrStmt::Sleep { .. } => true,
        IrStmt::If {
            then_body,
            else_body,
            ..
        } => contains_control_flow(then_body) || contains_control_flow(else_body),
        IrStmt::While { body, .. } | IrStmt::For { body, .. } | IrStmt::Context { body, .. } => {
            contains_control_flow(body)
        }
        IrStmt::Async { .. }
        | IrStmt::Let { .. }
        | IrStmt::Assign { .. }
        | IrStmt::RawCommand(_)
        | IrStmt::MacroCommand { .. }
        | IrStmt::Expr(_) => false,
    })
}

fn optimize_stmt(stmt: IrStmt) -> Option<IrStmt> {
    match stmt {
        IrStmt::Let { name, ty, value } => Some(IrStmt::Let {
            name,
            ty,
            value: fold_expr(value),
        }),
        IrStmt::Assign { target, value } => {
            let value = fold_expr(value);
            if matches!(
                (&target, &value.kind),
                (IrAssignTarget::Variable(left), IrExprKind::Variable(right)) if left == right
            ) {
                return None;
            }
            Some(IrStmt::Assign { target, value })
        }
        IrStmt::If {
            condition,
            then_body,
            else_body,
        } => Some(IrStmt::If {
            condition: fold_expr(condition),
            then_body: optimize_stmts(then_body),
            else_body: optimize_stmts(else_body),
        }),
        IrStmt::While { condition, body } => {
            let condition = fold_expr(condition);
            if matches!(condition.kind, IrExprKind::Bool(false)) {
                return None;
            }
            Some(IrStmt::While {
                condition,
                body: optimize_stmts(body),
            })
        }
        IrStmt::For { name, kind, body } => Some(IrStmt::For {
            name,
            kind: optimize_for_kind(kind),
            body: optimize_stmts(body),
        }),
        IrStmt::Context { kind, anchor, body } => Some(IrStmt::Context {
            kind,
            anchor: fold_expr(anchor),
            body: optimize_stmts(body),
        }),
        IrStmt::Async {
            mut function,
            captures,
        } => {
            optimize_function(&mut function);
            Some(IrStmt::Async { function, captures })
        }
        IrStmt::Return(Some(value)) => Some(IrStmt::Return(Some(fold_expr(value)))),
        IrStmt::Sleep { duration, unit } => Some(IrStmt::Sleep {
            duration: fold_expr(duration),
            unit,
        }),
        IrStmt::Expr(value) => Some(IrStmt::Expr(fold_expr(value))),
        IrStmt::MacroCommand {
            template,
            placeholders,
        } => Some(IrStmt::MacroCommand {
            template,
            placeholders,
        }),
        IrStmt::Break | IrStmt::Continue | IrStmt::Return(None) | IrStmt::RawCommand(_) => {
            Some(stmt)
        }
    }
}

fn optimize_for_kind(kind: IrForKind) -> IrForKind {
    match kind {
        IrForKind::Range {
            start,
            end,
            inclusive,
        } => IrForKind::Range {
            start: fold_expr(start),
            end: fold_expr(end),
            inclusive,
        },
        IrForKind::Each { iterable } => IrForKind::Each {
            iterable: fold_expr(iterable),
        },
    }
}

fn fold_expr(expr: IrExpr) -> IrExpr {
    let ty = expr.ty.clone();
    let ref_kind = expr.ref_kind;
    let kind = match expr.kind {
        IrExprKind::Unary { op, expr } => {
            let expr = fold_expr(*expr);
            match (op, &expr.kind) {
                (UnaryOp::Not, IrExprKind::Bool(value)) => IrExprKind::Bool(!value),
                (UnaryOp::Neg, IrExprKind::Int(value)) => IrExprKind::Int(-value),
                _ => IrExprKind::Unary {
                    op,
                    expr: Box::new(expr),
                },
            }
        }
        IrExprKind::Binary { op, left, right } => {
            let left = fold_expr(*left);
            let right = fold_expr(*right);
            fold_binary(op, &left, &right).unwrap_or(IrExprKind::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            })
        }
        IrExprKind::ArrayLiteral(values) => {
            IrExprKind::ArrayLiteral(values.into_iter().map(fold_expr).collect())
        }
        IrExprKind::DictLiteral(entries) => IrExprKind::DictLiteral(
            entries
                .into_iter()
                .map(|(key, value)| (key, fold_expr(value)))
                .collect(),
        ),
        IrExprKind::StructLiteral { name, fields } => IrExprKind::StructLiteral {
            name,
            fields: fields
                .into_iter()
                .map(|(name, value)| (name, fold_expr(value)))
                .collect(),
        },
        IrExprKind::Call { function, args } => IrExprKind::Call {
            function,
            args: args.into_iter().map(fold_expr).collect(),
        },
        IrExprKind::MethodCall {
            receiver,
            method,
            args,
        } => IrExprKind::MethodCall {
            receiver: Box::new(fold_expr(*receiver)),
            method,
            args: args.into_iter().map(fold_expr).collect(),
        },
        IrExprKind::Single(value) => IrExprKind::Single(Box::new(fold_expr(*value))),
        IrExprKind::Exists(value) => IrExprKind::Exists(Box::new(fold_expr(*value))),
        IrExprKind::HasData(value) => IrExprKind::HasData(Box::new(fold_expr(*value))),
        IrExprKind::At { anchor, value } => IrExprKind::At {
            anchor: Box::new(fold_expr(*anchor)),
            value: Box::new(fold_expr(*value)),
        },
        IrExprKind::As { anchor, value } => IrExprKind::As {
            anchor: Box::new(fold_expr(*anchor)),
            value: Box::new(fold_expr(*value)),
        },
        IrExprKind::Path(mut path) => {
            path.base = Box::new(fold_expr(*path.base));
            IrExprKind::Path(path)
        }
        IrExprKind::Cast { kind, expr } => IrExprKind::Cast {
            kind,
            expr: Box::new(fold_expr(*expr)),
        },
        IrExprKind::InterpolatedString {
            template,
            placeholders,
        } => IrExprKind::InterpolatedString {
            template,
            placeholders,
        },
        kind => kind,
    };
    IrExpr { ty, ref_kind, kind }
}

fn fold_binary(op: BinaryOp, left: &IrExpr, right: &IrExpr) -> Option<IrExprKind> {
    match (&left.kind, &right.kind) {
        (IrExprKind::Int(left), IrExprKind::Int(right)) => match op {
            BinaryOp::Add => Some(IrExprKind::Int(left + right)),
            BinaryOp::Sub => Some(IrExprKind::Int(left - right)),
            BinaryOp::Mul => Some(IrExprKind::Int(left * right)),
            BinaryOp::Div if *right != 0 => Some(IrExprKind::Int(left / right)),
            BinaryOp::Eq => Some(IrExprKind::Bool(left == right)),
            BinaryOp::NotEq => Some(IrExprKind::Bool(left != right)),
            BinaryOp::Lt => Some(IrExprKind::Bool(left < right)),
            BinaryOp::Lte => Some(IrExprKind::Bool(left <= right)),
            BinaryOp::Gt => Some(IrExprKind::Bool(left > right)),
            BinaryOp::Gte => Some(IrExprKind::Bool(left >= right)),
            _ => None,
        },
        (IrExprKind::Bool(left), IrExprKind::Bool(right)) => match op {
            BinaryOp::And => Some(IrExprKind::Bool(*left && *right)),
            BinaryOp::Or => Some(IrExprKind::Bool(*left || *right)),
            BinaryOp::Eq => Some(IrExprKind::Bool(left == right)),
            BinaryOp::NotEq => Some(IrExprKind::Bool(left != right)),
            _ => None,
        },
        (IrExprKind::String(left), IrExprKind::String(right)) => match op {
            BinaryOp::Eq => Some(IrExprKind::Bool(left == right)),
            BinaryOp::NotEq => Some(IrExprKind::Bool(left != right)),
            _ => None,
        },
        _ => None,
    }
}
