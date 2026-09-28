//! Generic classes. Like generic functions, each use such as `Box<Integer>`
//! compiles its own copy of the class, `Box__int`, with the type arguments
//! filled in, so a copy is an ordinary class to the rest of the compiler.
//!
//! `expand` runs between module resolution and type checking. Copies that only
//! generic code asks for, such as `Box<T>` in `<T> Box<T> wrap(T x)`, are found
//! while type checking; the type checker then runs `expand` again with them.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};

use crate::ast::*;
use crate::diagnostics::{Diagnostic, Diagnostics, Span};

/// A generic class and its type arguments, such as `("Box", [Int])`.
pub type Instance = (String, Vec<Type>);

/// Each instance asked for, with the first place that asked.
pub type Wanted = BTreeMap<Instance, Span>;

/// More copies than this means a class keeps asking for bigger copies of
/// itself, like `class A<T> { A<List<T>> next; }`.
const MAX_INSTANCES: usize = 256;

thread_local! {
    /// Each copy's name and what it is a copy of, for the current type check.
    static INSTANCES: RefCell<HashMap<String, Instance>> = RefCell::new(HashMap::new());
    /// Copies the type checker asked for that don't exist yet.
    static LATE: RefCell<Wanted> = const { RefCell::new(BTreeMap::new()) };
    /// The generic classes, which only exist as copies after `expand`.
    static GENERIC: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
    /// Generic interfaces with one abstract method: their type parameters,
    /// and the method's parameter and return types written with them.
    static FUNCTIONAL: RefCell<HashMap<String, GenericMethod>> = RefCell::new(HashMap::new());
}

/// A generic interface's type parameters and its one abstract method's
/// parameter and return types, like `([T, R], [T], R)` for `Function<T, R>`.
pub type GenericMethod = (Vec<String>, Vec<Type>, Type);

/// The one abstract method of generic interface `name`, if it has one.
pub fn generic_functional(name: &str) -> Option<GenericMethod> {
    FUNCTIONAL.with(|map| map.borrow().get(name).cloned())
}

/// Names a generic class, which has to be written with type arguments.
pub fn is_generic(name: &str) -> bool {
    GENERIC.with(|set| set.borrow().contains(name))
}

/// The name of one copy, such as `max__int` or `util::Box__list_float`. It
/// must stay a valid function path.
pub fn mangle(name: &str, types: &[Type]) -> String {
    let types: Vec<String> = types.iter().map(raw_name).collect();
    format!("{name}__{}", types.join("__"))
}

/// A type's part of a mangled name. Unlike `Type::as_str`, it never uses a
/// copy's display name, so names stay the same between runs of `expand`.
fn raw_name(ty: &Type) -> String {
    match ty {
        Type::Array(inner) => format!("list_{}", raw_name(inner)),
        Type::Dict(inner) => format!("map_{}", raw_name(inner)),
        Type::Optional(inner) => format!("optional_{}", raw_name(inner)),
        Type::Struct(name) | Type::Enum(name) | Type::Class(name) => {
            name.replace("::", "_").to_lowercase()
        }
        Type::Generic(name, args) => mangle(name, args).replace("::", "_").to_lowercase(),
        other => other.as_str().to_lowercase(),
    }
}

/// The generic class and type arguments that copy `name` was made from.
pub fn instance_of(name: &str) -> Option<Instance> {
    INSTANCES.with(|map| map.borrow().get(name).cloned())
}

/// Copies asked for since the last `expand`, which the type checker needs to
/// run again with.
pub fn take_late() -> Wanted {
    LATE.with(|late| std::mem::take(&mut *late.borrow_mut()))
}

/// Replace type parameters with their types, and turn a `Generic` whose
/// arguments are all known into its copy's class, asking for the copy when it
/// doesn't exist yet.
pub fn substitute(ty: &Type, bindings: &BTreeMap<String, Type>, span: &Span) -> Type {
    match ty {
        Type::Struct(name) => bindings.get(name).cloned().unwrap_or_else(|| ty.clone()),
        Type::Array(inner) => Type::Array(Box::new(substitute(inner, bindings, span))),
        Type::Dict(inner) => Type::Dict(Box::new(substitute(inner, bindings, span))),
        Type::Optional(inner) => Type::Optional(Box::new(substitute(inner, bindings, span))),
        Type::Generic(name, args) => {
            let args: Vec<Type> = args
                .iter()
                .map(|arg| substitute(arg, bindings, span))
                .collect();
            if args.iter().all(|arg| is_concrete(arg, &[])) {
                let copy = mangle(name, &args);
                let known = INSTANCES.with(|map| map.borrow().contains_key(&copy));
                if !known {
                    LATE.with(|late| {
                        late.borrow_mut()
                            .entry((name.clone(), args))
                            .or_insert_with(|| span.clone());
                    });
                }
                Type::Class(copy)
            } else {
                Type::Generic(name.clone(), args)
            }
        }
        _ => ty.clone(),
    }
}

/// No type parameter (one of `params`) and no generic use of one left in it.
fn is_concrete(ty: &Type, params: &[String]) -> bool {
    match ty {
        Type::Generic(..) => false,
        Type::Struct(name) => !params.contains(name),
        Type::Array(inner) | Type::Dict(inner) | Type::Optional(inner) => {
            is_concrete(inner, params)
        }
        _ => true,
    }
}

/// Fill in the type parameters of one copy of a generic function's body.
/// Generic classes it uses become their copies, asked for if needed.
pub fn substitute_body(body: &mut [Stmt], bindings: &BTreeMap<String, Type>, return_type: &Type) {
    let mut rewriter = Rewriter {
        bindings,
        renames: &BTreeMap::new(),
        return_type: Some(return_type.clone()),
    };
    rewriter.stmts(body);
    visit_stmts(body, &mut |ty, span| {
        *ty = substitute(ty, &BTreeMap::new(), span)
    });
    new_calls(body, &[], &mut |name, types, span| {
        substitute(
            &Type::Generic(name.to_string(), types.to_vec()),
            &BTreeMap::new(),
            span,
        );
    });
}

/// Expands generic classes: removes each one, and adds a copy for every set
/// of type arguments the program uses (and those in `wanted`).
pub fn expand(program: &Program, wanted: &mut Wanted, diagnostics: &mut Diagnostics) -> Program {
    let generic: BTreeMap<&str, &ClassDef> = program
        .classes
        .iter()
        .filter(|class| !class.type_params.is_empty())
        .map(|class| (class.name.as_str(), class))
        .collect();
    INSTANCES.with(|map| map.borrow_mut().clear());
    LATE.with(|late| late.borrow_mut().clear());
    GENERIC.with(|set| *set.borrow_mut() = generic.keys().map(|name| name.to_string()).collect());
    let functional = generic_functional_methods(program, &generic);
    FUNCTIONAL.with(|map| *map.borrow_mut() = functional);
    if generic.is_empty() && wanted.is_empty() {
        return program.clone();
    }
    // A generic class's own functions are copied; its static methods stay.
    let member_of = |function: &Function| {
        let owner = function.owner.as_deref()?;
        let class = generic.get(owner)?;
        let is_static = function.params.first().is_none_or(|p| p.name != "this")
            && !function.name.ends_with("__new");
        (!is_static).then_some(*class)
    };
    for class in generic.values() {
        if let Some(field) = class.fields.iter().find(|field| field.is_static) {
            diagnostics.push(Diagnostic::new(
                "a generic class can't have static fields",
                field.span.clone(),
            ));
        }
    }

    let mut out = program.clone();
    out.classes.retain(|class| class.type_params.is_empty());
    out.functions
        .retain(|function| member_of(function).is_none());
    let mut found = Wanted::new();
    concretize_program(&mut out, &mut found);

    // Copies can use more generic classes, so repeat until nothing is new.
    let mut made: BTreeMap<String, Instance> = BTreeMap::new();
    let mut bound_checks = Vec::new();
    loop {
        for (instance, span) in std::mem::take(&mut found) {
            wanted.entry(instance).or_insert(span);
        }
        let pending: Vec<(Instance, Span)> = wanted
            .iter()
            .filter(|((name, args), _)| !made.contains_key(&mangle(name, args)))
            .map(|(instance, span)| (instance.clone(), span.clone()))
            .collect();
        if pending.is_empty() {
            break;
        }
        for ((name, args), span) in pending {
            let copy = mangle(&name, &args);
            made.insert(copy.clone(), (name.clone(), args.clone()));
            if made.len() > MAX_INSTANCES {
                diagnostics.push(Diagnostic::new(
                    format!(
                        "'{}' keeps using bigger copies of itself",
                        name.replace("::", ".")
                    ),
                    span,
                ));
                return out;
            }
            let Some(class) = generic.get(name.as_str()) else {
                diagnostics.push(Diagnostic::new(
                    format!("'{}' has no type parameters", name.replace("::", ".")),
                    span,
                ));
                continue;
            };
            if class.type_params.len() != args.len() {
                diagnostics.push(Diagnostic::new(
                    format!(
                        "'{}' takes {} type arguments, found {}",
                        name.replace("::", "."),
                        class.type_params.len(),
                        args.len()
                    ),
                    span,
                ));
                continue;
            }
            let mut bindings: BTreeMap<String, Type> = class
                .type_params
                .iter()
                .cloned()
                .zip(args.iter().cloned())
                .collect();
            // Inside its body, the class's bare name means this copy.
            bindings.insert(name.clone(), Type::Class(copy.clone()));
            for (param, bound) in &class.bounds {
                bound_checks.push((
                    bindings[param].clone(),
                    concretize(substitute_plain(bound, &bindings), &[], &mut found, &span),
                    param.clone(),
                    span.clone(),
                ));
            }
            let members: Vec<&Function> = program
                .functions
                .iter()
                .filter(|function| member_of(function).is_some_and(|c| c.name == name))
                .collect();
            let renames: BTreeMap<String, String> = members
                .iter()
                .map(|function| {
                    let suffix = &function.name[name.len()..];
                    (function.name.clone(), format!("{copy}{suffix}"))
                })
                .collect();
            let mut def = (*class).clone();
            def.name = copy.clone();
            def.type_params.clear();
            def.bounds.clear();
            for field in &mut def.fields {
                field.ty = concretize(
                    substitute_plain(&field.ty, &bindings),
                    &[],
                    &mut found,
                    &span,
                );
            }
            let super_args = std::mem::take(&mut def.super_args);
            for super_name in def.parent.iter_mut().chain(&mut def.interfaces) {
                if let Some(args) = super_args.get(super_name.as_str()) {
                    let ty = Type::Generic(
                        super_name.clone(),
                        args.iter()
                            .map(|arg| substitute_plain(arg, &bindings))
                            .collect(),
                    );
                    if let Type::Class(resolved) = concretize(ty, &[], &mut found, &span) {
                        *super_name = resolved;
                    }
                }
            }
            out.classes.push(def);
            for member in members {
                let mut function = member.clone();
                function.name = renames[&member.name].clone();
                function.owner = Some(copy.clone());
                // A generic method keeps its own type parameters.
                let own = function.type_params.clone();
                let bindings: BTreeMap<String, Type> = bindings
                    .iter()
                    .filter(|(param, _)| !own.contains(param))
                    .map(|(param, ty)| (param.clone(), ty.clone()))
                    .collect();
                // Diamonds read the written form, `Pair<B, A>`, not the copy.
                let written_return = substitute_plain(&function.return_type, &bindings);
                let concrete = |ty: &Type, found: &mut Wanted| {
                    concretize(substitute_plain(ty, &bindings), &own, found, &span)
                };
                for param in &mut function.params {
                    param.ty = concrete(&param.ty, &mut found);
                }
                function.return_type = concrete(&function.return_type, &mut found);
                for (_, bound) in &mut function.bounds {
                    *bound = concrete(bound, &mut found);
                }
                let mut rewriter = Rewriter {
                    bindings: &bindings,
                    renames: &renames,
                    return_type: Some(written_return),
                };
                rewriter.stmts(&mut function.body);
                concretize_body(&mut function.body, &own, &mut found);
                out.functions.push(function);
            }
        }
    }

    // `as_str` reads the map, so name every copy before storing any.
    let names: Vec<(String, String)> = made
        .iter()
        .map(|(copy, (name, args))| {
            (
                copy.clone(),
                Type::Generic(name.clone(), args.clone()).as_str(),
            )
        })
        .collect();
    CLASS_DISPLAY.with(|display| display.borrow_mut().extend(names));
    INSTANCES.with(|map| *map.borrow_mut() = made.into_iter().collect());
    for (arg, bound, param, span) in bound_checks {
        if !extends(&out, &arg, &bound) {
            diagnostics.push(Diagnostic::new(
                format!(
                    "'{param}' must be a '{}', but it is '{}'",
                    bound.as_str(),
                    arg.as_type_arg()
                ),
                span,
            ));
        }
    }
    out
}

/// The one abstract method of each generic interface that has one, found in
/// the interface or, when it declares none, in the one interface it extends
/// (`UnaryOperator<T> extends Function<T, T>`).
fn generic_functional_methods(
    program: &Program,
    generic: &BTreeMap<&str, &ClassDef>,
) -> HashMap<String, GenericMethod> {
    fn find(
        name: &str,
        program: &Program,
        generic: &BTreeMap<&str, &ClassDef>,
        depth: usize,
    ) -> Option<GenericMethod> {
        let class = generic.get(name)?;
        if !class.is_interface || depth > 8 {
            return None;
        }
        let abstract_methods: Vec<&Function> = program
            .functions
            .iter()
            .filter(|f| f.owner.as_deref() == Some(name) && f.is_abstract)
            .filter(|f| f.params.first().is_some_and(|p| p.name == "this"))
            .collect();
        match abstract_methods.as_slice() {
            [method] => Some((
                class.type_params.clone(),
                method.params[1..].iter().map(|p| p.ty.clone()).collect(),
                method.return_type.clone(),
            )),
            [] if class.interfaces.len() == 1 => {
                let parent = &class.interfaces[0];
                let args = class.super_args.get(parent)?;
                let (params, method_params, return_type) =
                    find(parent, program, generic, depth + 1)?;
                let bindings: BTreeMap<String, Type> =
                    params.into_iter().zip(args.iter().cloned()).collect();
                Some((
                    class.type_params.clone(),
                    method_params
                        .iter()
                        .map(|ty| substitute_plain(ty, &bindings))
                        .collect(),
                    substitute_plain(&return_type, &bindings),
                ))
            }
            _ => None,
        }
    }
    generic
        .keys()
        .filter_map(|name| Some((name.to_string(), find(name, program, generic, 0)?)))
        .collect()
}

/// Whether `ty` is `bound` or a subtype of it, from the class declarations.
fn extends(program: &Program, ty: &Type, bound: &Type) -> bool {
    if ty == bound {
        return true;
    }
    // Before type checking, a written class name is still a `Struct`.
    let (Type::Class(name) | Type::Struct(name), Type::Class(bound) | Type::Struct(bound)) =
        (ty, bound)
    else {
        return false;
    };
    let mut pending = vec![name.clone()];
    while let Some(current) = pending.pop() {
        if &current == bound {
            return true;
        }
        if let Some(class) = program.classes.iter().find(|class| class.name == current) {
            pending.extend(class.parent.iter().cloned());
            pending.extend(class.interfaces.iter().cloned());
        }
    }
    false
}

/// `substitute` without asking for copies: `expand` asks through `concretize`.
pub fn substitute_plain(ty: &Type, bindings: &BTreeMap<String, Type>) -> Type {
    match ty {
        Type::Struct(name) => bindings.get(name).cloned().unwrap_or_else(|| ty.clone()),
        Type::Array(inner) => Type::Array(Box::new(substitute_plain(inner, bindings))),
        Type::Dict(inner) => Type::Dict(Box::new(substitute_plain(inner, bindings))),
        Type::Optional(inner) => Type::Optional(Box::new(substitute_plain(inner, bindings))),
        Type::Generic(name, args) => Type::Generic(
            name.clone(),
            args.iter()
                .map(|arg| substitute_plain(arg, bindings))
                .collect(),
        ),
        _ => ty.clone(),
    }
}

/// Turn a `Generic` with only known arguments into its copy's class, noting
/// the copy in `found`. `Box<T>` in generic code stays as it is.
fn concretize(ty: Type, params: &[String], found: &mut Wanted, span: &Span) -> Type {
    match ty {
        Type::Array(inner) => Type::Array(Box::new(concretize(*inner, params, found, span))),
        Type::Dict(inner) => Type::Dict(Box::new(concretize(*inner, params, found, span))),
        Type::Optional(inner) => Type::Optional(Box::new(concretize(*inner, params, found, span))),
        Type::Generic(name, args) => {
            let args: Vec<Type> = args
                .into_iter()
                .map(|arg| concretize(arg, params, found, span))
                .collect();
            if args.iter().all(|arg| is_concrete(arg, params)) {
                let copy = mangle(&name, &args);
                found.entry((name, args)).or_insert_with(|| span.clone());
                Type::Class(copy)
            } else {
                Type::Generic(name, args)
            }
        }
        other => other,
    }
}

fn concretize_program(program: &mut Program, found: &mut Wanted) {
    for class in &mut program.classes {
        for field in &mut class.fields {
            field.ty = concretize(field.ty.clone(), &[], found, &field.span);
        }
        let super_args = std::mem::take(&mut class.super_args);
        for super_name in class.parent.iter_mut().chain(&mut class.interfaces) {
            if let Some(args) = super_args.get(super_name.as_str()) {
                let ty = Type::Generic(super_name.clone(), args.clone());
                if let Type::Class(resolved) = concretize(ty, &[], found, &class.span) {
                    *super_name = resolved;
                }
            }
        }
    }
    for def in &mut program.structs {
        for field in &mut def.fields {
            field.ty = concretize(field.ty.clone(), &[], found, &field.span);
        }
    }
    for state in program
        .player_states
        .iter_mut()
        .chain(program.world_states.iter_mut())
    {
        state.ty = concretize(state.ty.clone(), &[], found, &state.span);
    }
    for def in &mut program.enums {
        for param in &mut def.constructor {
            param.ty = concretize(param.ty.clone(), &[], found, &param.span);
        }
        for field in &mut def.fields {
            field.ty = concretize(field.ty.clone(), &[], found, &field.span);
        }
    }
    for function in &mut program.functions {
        let params = function.type_params.clone();
        for param in &mut function.params {
            param.ty = concretize(param.ty.clone(), &params, found, &param.span);
        }
        let mut rewriter = Rewriter {
            bindings: &BTreeMap::new(),
            renames: &BTreeMap::new(),
            return_type: Some(function.return_type.clone()),
        };
        function.return_type =
            concretize(function.return_type.clone(), &params, found, &function.span);
        rewriter.stmts(&mut function.body);
        concretize_body(&mut function.body, &params, found);
    }
}

/// `concretize` every type written in `body`, and turn `new Box<Integer>(...)`
/// into a call of the copy's factory.
fn concretize_body(body: &mut [Stmt], params: &[String], found: &mut Wanted) {
    visit_stmts(body, &mut |ty, span| {
        *ty = concretize(ty.clone(), params, found, span)
    });
    new_calls(body, params, &mut |name, types, span| {
        found
            .entry((name.to_string(), types.to_vec()))
            .or_insert_with(|| span.clone());
    });
}

/// Replaces `new Box<Integer>(...)` whose type arguments are all known with a
/// call of `Box__int__new`, and calls `want` to ask for the copy.
fn new_calls(body: &mut [Stmt], params: &[String], want: &mut dyn FnMut(&str, &[Type], &Span)) {
    fn new_call(expr: &mut Expr, params: &[String], want: &mut dyn FnMut(&str, &[Type], &Span)) {
        each_child(expr, &mut |child| new_call(child, params, want));
        if let ExprKind::Lambda { body, .. } = &mut expr.kind {
            new_calls(body, params, want);
        }
        if let ExprKind::New {
            name,
            args,
            type_args: Some(types),
        } = &mut expr.kind
            && !types.is_empty()
            && types.iter().all(|ty| is_concrete(ty, params))
        {
            want(name, types, &expr.span);
            expr.kind = ExprKind::Call {
                type_args: Vec::new(),
                function: format!("{}__new", mangle(name, types)),
                args: std::mem::take(args),
            };
        }
    }
    for stmt in body {
        each_stmt_part(stmt, &mut |part| match part {
            Part::Expr(e) => new_call(e, params, want),
            Part::Stmts(stmts) => new_calls(stmts, params, want),
            Part::Type(_) => {}
        });
    }
}

/// Fills in a copy's type parameters, renames calls to the generic class's
/// functions to the copy's, and gives `new Box<>(...)` the declared type.
struct Rewriter<'a> {
    bindings: &'a BTreeMap<String, Type>,
    renames: &'a BTreeMap<String, String>,
    return_type: Option<Type>,
}

impl Rewriter<'_> {
    fn stmts(&mut self, body: &mut [Stmt]) {
        for stmt in body {
            self.stmt(stmt);
        }
    }

    fn stmt(&mut self, stmt: &mut Stmt) {
        // `Box<Integer> b = new Box<>(1);` and `return new Box<>(1);`
        let declared = match &mut stmt.kind {
            StmtKind::Let {
                ty: Some(ty),
                value,
                ..
            } => Some((substitute_plain(ty, self.bindings), value)),
            StmtKind::Return(Some(value)) => self.return_type.clone().map(|ty| (ty, value)),
            _ => None,
        };
        if let Some((ty, value)) = declared {
            fill_diamond(value, &ty);
        }
        each_stmt_part(stmt, &mut |part| match part {
            Part::Expr(expr) => self.expr(expr),
            Part::Stmts(stmts) => self.stmts(stmts),
            Part::Type(ty) => *ty = substitute_plain(ty, self.bindings),
        });
    }

    fn expr(&mut self, expr: &mut Expr) {
        each_child(expr, &mut |child| self.expr(child));
        match &mut expr.kind {
            ExprKind::Call {
                function,
                type_args,
                ..
            } => {
                if let Some(renamed) = self.renames.get(function) {
                    *function = renamed.clone();
                }
                for ty in type_args {
                    *ty = substitute_plain(ty, self.bindings);
                }
            }
            ExprKind::New {
                type_args: Some(types),
                ..
            }
            | ExprKind::MethodCall {
                type_args: types, ..
            } => {
                for ty in types {
                    *ty = substitute_plain(ty, self.bindings);
                }
            }
            ExprKind::InstanceOf { ty, .. } | ExprKind::Cast { ty, .. } => {
                *ty = substitute_plain(ty, self.bindings)
            }
            ExprKind::Lambda { params, body, .. } => {
                for ty in params.iter_mut().filter_map(|(_, ty)| ty.as_mut()) {
                    *ty = substitute_plain(ty, self.bindings);
                }
                let saved = self.return_type.take();
                self.stmts(body);
                self.return_type = saved;
            }
            _ => {}
        }
    }
}

/// `new Box<>(...)` takes its type arguments from the type it is assigned to.
pub fn fill_diamond(value: &mut Expr, declared: &Type) {
    let ExprKind::New {
        name,
        type_args: Some(types),
        ..
    } = &mut value.kind
    else {
        return;
    };
    if !types.is_empty() {
        return;
    }
    match declared {
        Type::Generic(declared_name, args) if declared_name == name => *types = args.clone(),
        Type::Class(copy) => {
            if let Some((declared_name, args)) = instance_of(copy)
                && &declared_name == name
            {
                *types = args;
            }
        }
        _ => {}
    }
}

/// One part of a statement for a walker to visit.
pub enum Part<'a> {
    Expr(&'a mut Expr),
    Stmts(&'a mut [Stmt]),
    Type(&'a mut Type),
}

/// Calls `visit` for each expression, nested block and declared type directly
/// in `stmt`.
pub fn each_stmt_part(stmt: &mut Stmt, visit: &mut dyn FnMut(Part)) {
    match &mut stmt.kind {
        StmtKind::Let { ty, value, .. } => {
            if let Some(ty) = ty {
                visit(Part::Type(ty));
            }
            visit(Part::Expr(value));
        }
        StmtKind::Assign { target, value } => {
            if let AssignTarget::Path(path) = target {
                visit_path(path, visit);
            }
            visit(Part::Expr(value));
        }
        StmtKind::If {
            condition,
            then_body,
            else_body,
        } => {
            visit(Part::Expr(condition));
            visit(Part::Stmts(then_body));
            visit(Part::Stmts(else_body));
        }
        StmtKind::While {
            condition,
            body,
            step,
        } => {
            visit(Part::Expr(condition));
            visit(Part::Stmts(body));
            visit(Part::Stmts(step));
        }
        StmtKind::For {
            ty, iterable, body, ..
        } => {
            if let Some(ty) = ty {
                visit(Part::Type(ty));
            }
            visit(Part::Expr(iterable));
            visit(Part::Stmts(body));
        }
        StmtKind::Switch {
            value,
            arms,
            default_body,
        } => {
            visit(Part::Expr(value));
            for arm in arms {
                visit(Part::Expr(&mut arm.pattern));
                visit(Part::Stmts(&mut arm.body));
            }
            visit(Part::Stmts(default_body));
        }
        StmtKind::Context { anchor, body, .. } => {
            visit(Part::Expr(anchor));
            visit(Part::Stmts(body));
        }
        StmtKind::Block(body) | StmtKind::Async { body } => visit(Part::Stmts(body)),
        StmtKind::Try {
            body,
            catches,
            finally,
        } => {
            visit(Part::Stmts(body));
            for catch in catches {
                catch.types.iter_mut().for_each(|ty| visit(Part::Type(ty)));
                visit(Part::Stmts(&mut catch.body));
            }
            visit(Part::Stmts(finally));
        }
        StmtKind::Return(Some(value)) | StmtKind::Expr(value) | StmtKind::Throw(value) => {
            visit(Part::Expr(value))
        }
        StmtKind::Return(None)
        | StmtKind::Break
        | StmtKind::Continue
        | StmtKind::RawCommand(_)
        | StmtKind::MacroCommand(_) => {}
    }
}

fn visit_path(path: &mut PathExpr, visit: &mut dyn FnMut(Part)) {
    visit(Part::Expr(&mut path.base));
    for segment in &mut path.segments {
        if let PathSegment::Index(index) = segment {
            visit(Part::Expr(index));
        }
    }
}

/// Calls `visit` for each expression directly inside `expr`.
pub fn each_child(expr: &mut Expr, visit: &mut dyn FnMut(&mut Expr)) {
    match &mut expr.kind {
        ExprKind::ArrayLiteral(items) => items.iter_mut().for_each(visit),
        ExprKind::DictLiteral(entries) => entries.iter_mut().for_each(|(_, v)| visit(v)),
        ExprKind::StructLiteral { fields, .. } => fields.iter_mut().for_each(|(_, v)| visit(v)),
        ExprKind::New { args, .. } | ExprKind::Call { args, .. } => args.iter_mut().for_each(visit),
        ExprKind::Unary { expr, .. }
        | ExprKind::InstanceOf { expr, .. }
        | ExprKind::Cast { expr, .. } => visit(expr),
        ExprKind::Binary { left, right, .. } => {
            visit(left);
            visit(right);
        }
        ExprKind::MethodCall { receiver, args, .. } => {
            visit(receiver);
            args.iter_mut().for_each(visit);
        }
        ExprKind::Path(path) => {
            visit(&mut path.base);
            for segment in &mut path.segments {
                if let PathSegment::Index(index) = segment {
                    visit(index);
                }
            }
        }
        ExprKind::Conditional {
            condition,
            then_expr,
            else_expr,
        } => {
            visit(condition);
            visit(then_expr);
            visit(else_expr);
        }
        ExprKind::Switch {
            value,
            arms,
            default,
        } => {
            visit(value);
            for (pattern, result) in arms {
                visit(pattern);
                visit(result);
            }
            if let Some(default) = default {
                visit(default);
            }
        }
        // A lambda's body is statements; walkers that need it visit it themselves.
        ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::Bool(_)
        | ExprKind::String(_)
        | ExprKind::Variable(_)
        | ExprKind::Lambda { .. }
        | ExprKind::MethodRef { .. } => {}
    }
}

/// Calls `visit` for every type written in `body`: declarations, casts,
/// `instanceof`, and `new` and call type arguments.
/// Each comes with the span of the expression or statement that has it.
fn visit_stmts(body: &mut [Stmt], visit: &mut dyn FnMut(&mut Type, &Span)) {
    fn expr(e: &mut Expr, visit: &mut dyn FnMut(&mut Type, &Span)) {
        each_child(e, &mut |child| expr(child, visit));
        match &mut e.kind {
            ExprKind::InstanceOf { ty, .. } | ExprKind::Cast { ty, .. } => visit(ty, &e.span),
            ExprKind::New {
                type_args: Some(types),
                ..
            }
            | ExprKind::Call {
                type_args: types, ..
            }
            | ExprKind::MethodCall {
                type_args: types, ..
            } => types.iter_mut().for_each(|ty| visit(ty, &e.span)),
            ExprKind::Lambda { params, body, .. } => {
                for ty in params.iter_mut().filter_map(|(_, ty)| ty.as_mut()) {
                    visit(ty, &e.span);
                }
                visit_stmts(body, visit);
            }
            _ => {}
        }
    }
    for stmt in body {
        let span = stmt.span.clone();
        each_stmt_part(stmt, &mut |part| match part {
            Part::Expr(e) => expr(e, visit),
            Part::Stmts(stmts) => visit_stmts(stmts, visit),
            Part::Type(ty) => visit(ty, &span),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mangles_nested_type_arguments() {
        assert_eq!(mangle("util::Box", &[Type::Int]), "util::Box__int");
        assert_eq!(
            mangle("Pair", &[Type::String, Type::Array(Box::new(Type::Float))]),
            "Pair__string__list_float"
        );
    }
}
