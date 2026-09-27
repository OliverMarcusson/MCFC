use crate::ast::{BinaryOp, UnaryOp};
use crate::ir::{IrAssignTarget, IrExpr, IrExprKind, IrFunction, IrProgram, IrStmt};

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
        IrStmt::Break
        | IrStmt::Continue
        | IrStmt::Return(_)
        | IrStmt::Sleep { .. }
        | IrStmt::HostCall { .. } => true,
        IrStmt::If {
            then_body,
            else_body,
            ..
        } => contains_control_flow(then_body) || contains_control_flow(else_body),
        IrStmt::While { body, step, .. } => {
            contains_control_flow(body) || contains_control_flow(step)
        }
        IrStmt::For { body, .. } | IrStmt::Context { body, .. } => contains_control_flow(body),
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
        IrStmt::While {
            condition,
            body,
            step,
        } => {
            let condition = fold_expr(condition);
            if matches!(condition.kind, IrExprKind::Bool(false)) {
                return None;
            }
            Some(IrStmt::While {
                condition,
                body: optimize_stmts(body),
                step: optimize_stmts(step),
            })
        }
        IrStmt::For {
            name,
            iterable,
            body,
        } => Some(IrStmt::For {
            name,
            iterable: fold_expr(iterable),
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
        IrStmt::HostCall {
            module,
            function,
            args,
            dest,
            return_type,
        } => Some(IrStmt::HostCall {
            module,
            function,
            args: args.into_iter().map(fold_expr).collect(),
            dest,
            return_type,
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

fn fold_expr(expr: IrExpr) -> IrExpr {
    let ty = expr.ty.clone();
    let ref_kind = expr.ref_kind;
    let kind = match expr.kind {
        IrExprKind::Unary { op, expr } => {
            let expr = fold_expr(*expr);
            match (op, &expr.kind) {
                (UnaryOp::Not, IrExprKind::Bool(value)) => IrExprKind::Bool(!value),
                (UnaryOp::Neg, IrExprKind::Int(value)) => IrExprKind::Int(-value),
                (UnaryOp::BitNot, IrExprKind::Int(value)) => IrExprKind::Int(!value),
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
        IrExprKind::Conditional {
            condition,
            then_expr,
            else_expr,
        } => {
            let condition = fold_expr(*condition);
            match condition.kind {
                IrExprKind::Bool(true) => return fold_expr(*then_expr),
                IrExprKind::Bool(false) => return fold_expr(*else_expr),
                _ => IrExprKind::Conditional {
                    condition: Box::new(condition),
                    then_expr: Box::new(fold_expr(*then_expr)),
                    else_expr: Box::new(fold_expr(*else_expr)),
                },
            }
        }
        IrExprKind::Bind { name, value, body } => IrExprKind::Bind {
            name,
            value: Box::new(fold_expr(*value)),
            body: Box::new(fold_expr(*body)),
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

/// Scoreboard `/=` rounds toward negative infinity (Java's `floorDiv`), so
/// folding must too: `-7 / 2` is `-4`, not Rust's truncated `-3`.
fn floor_div(left: i64, right: i64) -> Option<i64> {
    let quotient = left.checked_div(right)?;
    if left % right != 0 && (left < 0) != (right < 0) {
        Some(quotient - 1)
    } else {
        Some(quotient)
    }
}

fn fold_binary(op: BinaryOp, left: &IrExpr, right: &IrExpr) -> Option<IrExprKind> {
    match (&left.kind, &right.kind) {
        (IrExprKind::Int(left), IrExprKind::Int(right)) => match op {
            BinaryOp::Add => Some(IrExprKind::Int(left + right)),
            BinaryOp::Sub => Some(IrExprKind::Int(left - right)),
            BinaryOp::Mul => Some(IrExprKind::Int(left * right)),
            BinaryOp::Div if *right != 0 => floor_div(*left, *right).map(IrExprKind::Int),
            // Scoreboard `%=` is Math.floorMod, which is rem_euclid only for positive divisors.
            BinaryOp::Rem if *right != 0 => {
                floor_div(*left, *right).map(|q| IrExprKind::Int(left - q * right))
            }
            // Java int semantics: 32 bits, shift counts masked to 0..31.
            BinaryOp::BitAnd => Some(IrExprKind::Int(left & right)),
            BinaryOp::BitOr => Some(IrExprKind::Int(left | right)),
            BinaryOp::BitXor => Some(IrExprKind::Int(left ^ right)),
            BinaryOp::Shl => Some(IrExprKind::Int(
                ((*left as i32).wrapping_shl(*right as u32 & 31)) as i64,
            )),
            BinaryOp::Shr => Some(IrExprKind::Int(
                ((*left as i32) >> (*right as u32 & 31)) as i64,
            )),
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
