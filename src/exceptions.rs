//! `throw` and `try`, lowered to plain statements before type checking.
//!
//! `throw e;` stores `e` in the static field `std.exception.Thrown.value`.
//! After each statement that can throw, a check of that field escapes: a
//! `return` out of the function, a `break` out of a loop inside a `try`, or,
//! directly in a `try` body, skipping the rest of the block. The `try` then
//! picks its `catch` with `instanceof` and runs `finally`. A function no code
//! calls (`main`, `tick`, event handlers, tests) catches everything and logs it.
//!
//! What can throw is found by name, so a call to any function or method
//! sharing a name with one that throws is checked. A program without `throw`
//! compiles as if this pass didn't exist.

use std::collections::BTreeSet;

use crate::ast::*;
use crate::diagnostics::Span;
use crate::generics::{Part, each_child, each_stmt_part};

const MODULE: &str = "std::exception::";
const THROWN_CLASS: &str = "std::exception::Thrown";
const EXCEPTION: &str = "std::exception::Exception";

pub fn lower(program: &mut Program) {
    let throwing = throwing_functions(&program.functions);
    let has_try = program
        .functions
        .iter()
        .any(|function| mentions_try(&function.body));
    if throwing.is_empty() && !has_try {
        // Unused, the classes would cost every pack the object heap, and
        // `Thrown`'s static field a load-time initializer.
        let used = program
            .functions
            .iter()
            .filter(|function| !function.module.starts_with("std"))
            .map(|function| format!("{function:?}"))
            .chain(program.classes.iter().map(|class| format!("{class:?}")))
            .any(|text| text.contains(MODULE) && !text.contains(&format!("name: \"{MODULE}")));
        let doomed =
            |name: &str| name.starts_with(THROWN_CLASS) || (!used && name.starts_with(MODULE));
        program.classes.retain(|class| !doomed(&class.name));
        program.functions.retain(|function| !doomed(&function.name));
        return;
    }
    let called = called_names(&program.functions);
    let names = Names { throwing };
    for function in &mut program.functions {
        let body = std::mem::take(&mut function.body);
        let root = function.return_type == Type::Void
            && !called.iter().any(|call| call.matches(&function.name));
        function.body = if root && names.block_throws(&body) {
            let test = function.name.starts_with("__mcfc_test_");
            let mut body = names.lower_block(body, Escape::Skip, &function.return_type);
            body.push(report_uncaught(test, &function.span));
            body
        } else {
            names.lower_block(body, Escape::Return, &function.return_type)
        };
    }
}

/// How a statement that threw leaves the code around it.
#[derive(Clone, Copy, PartialEq)]
enum Escape {
    /// Return from the function, outside any `try`.
    Return,
    /// Break out of a loop inside a `try`; the check after the loop goes on.
    Break,
    /// In a `try` body: skip the rest of the block.
    Skip,
}

impl Escape {
    fn in_loop(self) -> Self {
        match self {
            Escape::Return => Escape::Return,
            Escape::Break | Escape::Skip => Escape::Break,
        }
    }
}

/// A call as written, matched against function names by what it could reach.
enum Called {
    /// `f(...)`, `Type__f(...)` or a bare method name inside a class.
    Function(String),
    /// `x.f(...)`: any method named `f`.
    Method(String),
    /// `new C(...)`: `C`'s constructor, or a generic copy's.
    New(String),
}

impl Called {
    fn matches(&self, function: &str) -> bool {
        let method = |name: &str| {
            function.ends_with(&format!("__{name}")) || function.contains(&format!("__{name}__"))
        };
        match self {
            Called::Function(name) => {
                function == name || function.starts_with(&format!("{name}__")) || method(name)
            }
            Called::Method(name) => method(name.trim_start_matches(WRITTEN_METHOD)),
            Called::New(name) => function.starts_with(name.as_str()) && function.ends_with("__new"),
        }
    }
}

struct Names {
    throwing: BTreeSet<String>,
}

impl Names {
    fn call_throws(&self, called: &Called) -> bool {
        self.throwing.iter().any(|name| called.matches(name))
    }

    fn expr_throws(&self, expr: &Expr) -> bool {
        let mut throws = false;
        visit_calls(expr, &mut |called| throws |= self.call_throws(&called));
        throws
    }

    fn block_throws(&self, stmts: &[Stmt]) -> bool {
        stmts.iter().any(|stmt| self.stmt_throws(stmt))
    }

    /// `stmt` can leave with an exception set. An `async` body runs later.
    fn stmt_throws(&self, stmt: &Stmt) -> bool {
        match &stmt.kind {
            StmtKind::Throw(_) => true,
            StmtKind::Async { .. } => false,
            _ => {
                let mut throws = false;
                let mut stmt = stmt.clone();
                each_stmt_part(&mut stmt, &mut |part| match part {
                    Part::Expr(expr) => throws |= self.expr_throws(expr),
                    Part::Stmts(stmts) => throws |= self.block_throws(stmts),
                    Part::Type(_) => {}
                });
                throws
            }
        }
    }

    fn lower_block(&self, stmts: Vec<Stmt>, escape: Escape, returns: &Type) -> Vec<Stmt> {
        let mut out = Vec::new();
        let mut rest = stmts.into_iter();
        while let Some(stmt) = rest.next() {
            let span = stmt.span.clone();
            let throws = self.stmt_throws(&stmt);
            if let StmtKind::Throw(value) = stmt.kind {
                out.push(at(
                    StmtKind::Assign {
                        target: AssignTarget::Path(thrown_path(&span)),
                        value,
                    },
                    &span,
                ));
                if escape != Escape::Skip {
                    out.push(escape_stmt(escape, returns, &span));
                }
                // What follows a `throw` never runs.
                break;
            }
            out.push(self.lower_stmt(stmt, escape, returns));
            if !throws {
                continue;
            }
            if escape == Escape::Skip {
                let rest = self.lower_block(rest.collect(), escape, returns);
                if !rest.is_empty() {
                    out.push(if_stmt(is_thrown(false, &span), rest, &span));
                }
                break;
            }
            out.push(if_stmt(
                is_thrown(true, &span),
                vec![escape_stmt(escape, returns, &span)],
                &span,
            ));
        }
        out
    }

    fn lower_stmt(&self, mut stmt: Stmt, escape: Escape, returns: &Type) -> Stmt {
        let span = stmt.span.clone();
        let block = |stmts: &mut Vec<Stmt>, escape: Escape| {
            *stmts = self.lower_block(std::mem::take(stmts), escape, returns);
        };
        match &mut stmt.kind {
            StmtKind::If {
                then_body,
                else_body,
                ..
            } => {
                block(then_body, escape);
                block(else_body, escape);
            }
            StmtKind::While { body, step, .. } => {
                block(body, escape.in_loop());
                block(step, escape.in_loop());
            }
            StmtKind::For { body, .. } => block(body, escape.in_loop()),
            StmtKind::Block(body) => block(body, escape),
            StmtKind::Switch {
                arms, default_body, ..
            } => {
                for arm in arms {
                    block(&mut arm.body, escape);
                }
                block(default_body, escape);
            }
            // An `as`/`at` body runs as its own function, so it can only skip
            // ahead; the check after the whole statement escapes further.
            StmtKind::Context { body, .. } => block(body, Escape::Skip),
            // An `async` body runs later, with nothing to catch its exception.
            StmtKind::Async { body } if self.block_throws(body) => {
                block(body, Escape::Skip);
                body.push(report_uncaught(false, &span));
            }
            StmtKind::Try {
                body,
                catches,
                finally,
            } => {
                return at(
                    StmtKind::Block(self.lower_try(
                        std::mem::take(body),
                        std::mem::take(catches),
                        std::mem::take(finally),
                        escape,
                        returns,
                        &span,
                    )),
                    &span,
                );
            }
            _ => {}
        }
        stmt
    }

    /// `try { body } catch (A e) { ... } finally { ... }` as:
    /// ```text
    /// body, skipping the rest after a throw
    /// if (Thrown.value != null) {
    ///     Exception caught = Thrown.value;
    ///     if (caught instanceof A e) { Thrown.value = null; ... } else if ...
    /// }
    /// Exception pending = Thrown.value; Thrown.value = null;
    /// finally
    /// if (Thrown.value == null) { Thrown.value = pending; }
    /// ```
    fn lower_try(
        &self,
        body: Vec<Stmt>,
        catches: Vec<Catch>,
        finally: Vec<Stmt>,
        escape: Escape,
        returns: &Type,
        span: &Span,
    ) -> Vec<Stmt> {
        let body_throws = self.block_throws(&body);
        let mut out = self.lower_block(body, Escape::Skip, returns);
        let suffix = format!("{}_{}", span.line, span.column);
        let variable = |name: &str| expr(ExprKind::Variable(name.to_string()), span);
        let exception = Some(Type::Struct(EXCEPTION.to_string()));
        if body_throws && !catches.is_empty() {
            let caught = format!("__caught_{suffix}");
            let mut chain: Vec<Stmt> = Vec::new();
            for catch in catches.into_iter().rev() {
                let single = catch.types.len() == 1;
                let condition = catch
                    .types
                    .iter()
                    .map(|ty| {
                        expr(
                            ExprKind::InstanceOf {
                                expr: Box::new(variable(&caught)),
                                ty: ty.clone(),
                                binding: None,
                            },
                            &catch.span,
                        )
                    })
                    .reduce(|left, right| {
                        expr(
                            ExprKind::Binary {
                                op: BinaryOp::Or,
                                left: Box::new(left),
                                right: Box::new(right),
                            },
                            &catch.span,
                        )
                    })
                    .expect("a catch names a type");
                // `catch (A | B e)` makes `e` an `Exception`.
                let ty = if single {
                    catch.types[0].clone()
                } else {
                    Type::Struct(EXCEPTION.to_string())
                };
                let value = expr(
                    ExprKind::Cast {
                        ty: ty.clone(),
                        expr: Box::new(variable(&caught)),
                    },
                    &catch.span,
                );
                let then_body = vec![
                    clear_thrown(&catch.span),
                    at(
                        StmtKind::Let {
                            name: catch.name.clone(),
                            ty: Some(ty),
                            value,
                        },
                        &catch.span,
                    ),
                ];
                let mut then_body = then_body;
                then_body.extend(self.lower_block(catch.body, Escape::Skip, returns));
                chain = vec![at(
                    StmtKind::If {
                        condition,
                        then_body,
                        else_body: chain,
                    },
                    &catch.span,
                )];
            }
            let mut handler = vec![at(
                StmtKind::Let {
                    name: caught,
                    ty: exception.clone(),
                    value: expr(ExprKind::Path(thrown_path(span)), span),
                },
                span,
            )];
            handler.extend(chain);
            out.push(if_stmt(is_thrown(true, span), handler, span));
        }
        if !finally.is_empty() {
            let pending = format!("__pending_{suffix}");
            out.push(at(
                StmtKind::Let {
                    name: pending.clone(),
                    ty: exception,
                    value: expr(ExprKind::Path(thrown_path(span)), span),
                },
                span,
            ));
            out.push(clear_thrown(span));
            out.extend(self.lower_block(finally, escape, returns));
            out.push(if_stmt(
                is_thrown(false, span),
                vec![at(
                    StmtKind::Assign {
                        target: AssignTarget::Path(thrown_path(span)),
                        value: variable(&pending),
                    },
                    span,
                )],
                span,
            ));
        }
        out
    }
}

/// Functions that can leave with an exception set, found by name until no
/// more are.
fn throwing_functions(functions: &[Function]) -> BTreeSet<String> {
    let mut names = Names {
        throwing: BTreeSet::new(),
    };
    loop {
        let found: Vec<String> = functions
            .iter()
            .filter(|function| !names.throwing.contains(&function.name))
            .filter(|function| names.block_throws(&function.body))
            .map(|function| function.name.clone())
            .collect();
        if found.is_empty() {
            return names.throwing;
        }
        names.throwing.extend(found);
    }
}

fn mentions_try(stmts: &[Stmt]) -> bool {
    stmts.iter().any(|stmt| {
        if matches!(stmt.kind, StmtKind::Try { .. } | StmtKind::Throw(_)) {
            return true;
        }
        let mut found = false;
        let mut stmt = stmt.clone();
        each_stmt_part(&mut stmt, &mut |part| {
            if let Part::Stmts(stmts) = part {
                found |= mentions_try(stmts);
            }
        });
        found
    })
}

/// Every call written in any function, lambdas included.
fn called_names(functions: &[Function]) -> Vec<Called> {
    let mut called = Vec::new();
    fn walk(stmts: &[Stmt], called: &mut Vec<Called>) {
        for stmt in stmts {
            let mut stmt = stmt.clone();
            each_stmt_part(&mut stmt, &mut |part| match part {
                Part::Expr(expr) => walk_expr(expr, called),
                Part::Stmts(stmts) => walk(stmts, called),
                Part::Type(_) => {}
            });
        }
    }
    fn walk_expr(expr: &mut Expr, called: &mut Vec<Called>) {
        if let ExprKind::Lambda { body, .. } = &expr.kind {
            walk(body, called);
        }
        if let ExprKind::MethodRef { target, method } = &expr.kind {
            called.push(Called::Method(method.clone()));
            called.push(Called::Function(format!("{target}__{method}")));
        }
        visit_calls(expr, &mut |call| called.push(call));
        each_child(expr, &mut |child| walk_expr(child, called));
    }
    for function in functions {
        walk(&function.body, &mut called);
    }
    called
}

/// Calls `visit` for each call in `expr`, not looking into lambdas, which
/// run when called.
fn visit_calls(expr: &Expr, visit: &mut dyn FnMut(Called)) {
    let mut expr = expr.clone();
    fn walk(expr: &mut Expr, visit: &mut dyn FnMut(Called)) {
        match &expr.kind {
            ExprKind::Call { function, .. } => visit(Called::Function(function.clone())),
            ExprKind::MethodCall { method, .. } => visit(Called::Method(method.clone())),
            ExprKind::New { name, .. } => visit(Called::New(name.clone())),
            _ => {}
        }
        each_child(expr, &mut |child| walk(child, visit));
    }
    walk(&mut expr, visit);
}

/// `if (Thrown.value != null) { Exception e = ...; Thrown.value = null; uncaught(e); }`,
/// or a failed assertion in a test.
fn report_uncaught(test: bool, span: &Span) -> Stmt {
    let name = "__uncaught";
    let variable = expr(ExprKind::Variable(name.to_string()), span);
    let report = if test {
        let message = expr(
            ExprKind::Binary {
                op: BinaryOp::Add,
                left: Box::new(expr(
                    ExprKind::String("uncaught exception: ".to_string()),
                    span,
                )),
                right: Box::new(expr(
                    ExprKind::MethodCall {
                        receiver: Box::new(variable.clone()),
                        method: "getMessage".to_string(),
                        args: Vec::new(),
                    },
                    span,
                )),
            },
            span,
        );
        call("assert_fail", vec![message], span)
    } else {
        call("std::exception::uncaught", vec![variable], span)
    };
    if_stmt(
        is_thrown(true, span),
        vec![
            at(
                StmtKind::Let {
                    name: name.to_string(),
                    ty: Some(Type::Struct(EXCEPTION.to_string())),
                    value: expr(ExprKind::Path(thrown_path(span)), span),
                },
                span,
            ),
            clear_thrown(span),
            at(StmtKind::Expr(report), span),
        ],
        span,
    )
}

/// `return;`, or `return` of the return type's empty value, or `break;`.
fn escape_stmt(escape: Escape, returns: &Type, span: &Span) -> Stmt {
    match escape {
        Escape::Break | Escape::Skip => at(StmtKind::Break, span),
        Escape::Return if *returns == Type::Void => at(StmtKind::Return(None), span),
        Escape::Return => at(
            StmtKind::Block(vec![
                at(
                    StmtKind::Let {
                        name: "__none".to_string(),
                        ty: Some(returns.clone()),
                        value: expr(ExprKind::Variable("__mcfc_default".to_string()), span),
                    },
                    span,
                ),
                at(
                    StmtKind::Return(Some(expr(ExprKind::Variable("__none".to_string()), span))),
                    span,
                ),
            ]),
            span,
        ),
    }
}

fn thrown_path(span: &Span) -> PathExpr {
    PathExpr {
        base: Box::new(expr(ExprKind::Variable(THROWN_CLASS.to_string()), span)),
        segments: vec![PathSegment::Field("value".to_string())],
    }
}

/// `Thrown.value != null`, or `== null`.
fn is_thrown(thrown: bool, span: &Span) -> Expr {
    expr(
        ExprKind::Binary {
            op: if thrown {
                BinaryOp::NotEq
            } else {
                BinaryOp::Eq
            },
            left: Box::new(expr(ExprKind::Path(thrown_path(span)), span)),
            right: Box::new(expr(ExprKind::Variable("null".to_string()), span)),
        },
        span,
    )
}

fn clear_thrown(span: &Span) -> Stmt {
    at(
        StmtKind::Assign {
            target: AssignTarget::Path(thrown_path(span)),
            value: expr(ExprKind::Variable("null".to_string()), span),
        },
        span,
    )
}

fn if_stmt(condition: Expr, then_body: Vec<Stmt>, span: &Span) -> Stmt {
    at(
        StmtKind::If {
            condition,
            then_body,
            else_body: Vec::new(),
        },
        span,
    )
}

fn call(function: &str, args: Vec<Expr>, span: &Span) -> Expr {
    expr(
        ExprKind::Call {
            function: function.to_string(),
            args,
        },
        span,
    )
}

fn at(kind: StmtKind, span: &Span) -> Stmt {
    Stmt {
        kind,
        span: span.clone(),
    }
}

fn expr(kind: ExprKind, span: &Span) -> Expr {
    Expr {
        kind,
        span: span.clone(),
    }
}
