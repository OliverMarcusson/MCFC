//! Lambdas and method references. Each one becomes a class implementing the
//! functional interface it is assigned to, with a field for every variable it
//! captures, and the lambda's value is a new object of that class:
//! `Function<Integer, Integer> f = x -> x + step;` is `new Lambda(step)`, and
//! `f.apply(2)` is an ordinary interface call.
//!
//! The class can only be written once the lambda's types are known, so the
//! type checker collects the classes it finds and `type_check` runs again with
//! them added, like copies of generic classes.

use super::*;

thread_local! {
    /// Each interface with exactly one abstract method, and that method.
    static FUNCTIONAL: RefCell<HashMap<String, FunctionalMethod>> = RefCell::new(HashMap::new());
    /// Lambda classes found in this run that the program doesn't have yet.
    static FOUND: RefCell<BTreeMap<String, LambdaClass>> = const { RefCell::new(BTreeMap::new()) };
    /// Set while a lambda's body is checked only to learn its type.
    static TRIAL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The class a lambda argument has until the call's parameter types are known.
const PENDING: &str = "@lambda";

/// Stands in for a lambda argument while the call is picked.
pub(super) fn pending() -> TypedExpr {
    TypedExpr {
        kind: TypedExprKind::Int(0),
        ty: Type::Class(PENDING.to_string()),
        ref_kind: RefKind::Unknown,
    }
}

pub(super) fn is_pending(arg: &TypedExpr) -> bool {
    matches!(&arg.ty, Type::Class(name) if name == PENDING)
}

/// Mentions one of `params`.
fn mentions(ty: &Type, params: &[String]) -> bool {
    match ty {
        Type::Struct(name) => params.contains(name),
        Type::Array(inner) | Type::Dict(inner) | Type::Optional(inner) => mentions(inner, params),
        Type::Generic(_, args) => args.iter().any(|arg| mentions(arg, params)),
        _ => false,
    }
}

/// Binds the type parameters of a generic call that a lambda argument shows:
/// for `<T, R> List<R> map(List<T> xs, Function<T, R> f)` and `x -> x + 1`
/// with `T` known, the body's type is `R`. Only an expression lambda (or a
/// method reference) shows its result type.
#[allow(clippy::too_many_arguments)]
pub(super) fn infer_type_params(
    expr: &Expr,
    param: &Type,
    type_params: &[String],
    bindings: &mut BTreeMap<String, Type>,
    struct_defs: &BTreeMap<String, StructTypeDef>,
    signatures: &BTreeMap<String, FunctionSignature>,
    env: &HashMap<String, Type>,
    ref_env: &HashMap<String, RefKind>,
) {
    let param = crate::generics::substitute_plain(param, bindings);
    let Type::Generic(name, args) = &param else {
        return;
    };
    if !mentions(&param, type_params) {
        return;
    }
    let Some((own, method_params, method_return)) = crate::generics::generic_functional(name)
    else {
        return;
    };
    let own: BTreeMap<String, Type> = own.into_iter().zip(args.iter().cloned()).collect();
    // `comparing(Player::kills)`: the object the method is called on is a `Player`.
    if let ExprKind::MethodRef { target, method } = &expr.kind
        && method != "new"
        && !env.contains_key(target.as_str())
        && struct_defs.contains_key(target.as_str())
        && signatures
            .get(&format!("{target}__{method}"))
            .is_none_or(|signature| signature.instance)
        && let Some(first) = method_params.first()
    {
        let mut receiver = Type::Struct(target.clone());
        resolve_enum_type(&mut receiver, struct_defs);
        let first = crate::generics::substitute_plain(first, &own);
        bind_type_params(&first, &receiver, type_params, bindings, false);
    }
    let own: BTreeMap<String, Type> = own
        .into_iter()
        .map(|(name, ty)| (name, crate::generics::substitute_plain(&ty, bindings)))
        .collect();
    let Some(lambda_params) = method_params
        .iter()
        .map(|ty| {
            let ty = crate::generics::substitute_plain(ty, &own);
            (!mentions(&ty, type_params)).then(|| substitute(&ty, &BTreeMap::new()))
        })
        .collect::<Option<Vec<Type>>>()
    else {
        return;
    };
    let (names, body) = match &expr.kind {
        ExprKind::Lambda { params, body, .. } => (
            params.iter().map(|(name, _)| name.clone()).collect(),
            body.clone(),
        ),
        ExprKind::MethodRef { target, method } => {
            match method_ref_lambda(
                target,
                method,
                lambda_params.len(),
                env,
                signatures,
                &expr.span,
            ) {
                Ok((params, body, _)) => (params.into_iter().map(|(name, _)| name).collect(), body),
                Err(_) => return,
            }
        }
        _ => return,
    };
    let names: Vec<String> = names;
    let [
        Stmt {
            kind: StmtKind::Return(Some(value)),
            ..
        },
    ] = body.as_slice()
    else {
        return;
    };
    if names.len() != lambda_params.len() {
        return;
    }
    let mut env = env.clone();
    let mut ref_env = ref_env.clone();
    for (name, ty) in names.iter().zip(lambda_params) {
        env.insert(name.clone(), ty);
        ref_env.insert(name.clone(), RefKind::Unknown);
    }
    let was_trial = TRIAL.with(|trial| trial.replace(true));
    let found = type_check_expr(
        value,
        struct_defs,
        signatures,
        &env,
        &ref_env,
        &mut BTreeSet::new(),
        &mut Diagnostics::new(),
    )
    .ty;
    TRIAL.with(|trial| trial.set(was_trial));
    let result = crate::generics::substitute_plain(&method_return, &own);
    bind_type_params(&result, &found, type_params, bindings, false);
}

/// A lambda's class: its declaration and its functions.
pub type LambdaClass = (ClassDef, Vec<Function>);
/// A lambda's parameters, body, and whether the body is one expression.
type LambdaParts = (Vec<(String, Option<Type>)>, Vec<Stmt>, bool);

/// The field holding the object a lambda in a method was made in.
const OUTER: &str = "mcfcOuter";

#[derive(Debug, Clone)]
struct FunctionalMethod {
    /// The method's name as written, like `apply`.
    name: String,
    params: Vec<Type>,
    return_type: Type,
}

/// Lambda classes asked for since the last call.
pub fn take_found() -> BTreeMap<String, LambdaClass> {
    FOUND.with(|found| std::mem::take(&mut *found.borrow_mut()))
}

/// Records which interfaces are functional. `unmangled` holds each function's
/// name before overloads were renamed; overloaded methods never count.
pub(super) fn find_functional(program: &Program, unmangled: &[String]) {
    FOUND.with(|found| found.borrow_mut().clear());
    let mut methods: HashMap<&str, Vec<(String, &Function)>> = HashMap::new();
    for (function, name) in program.functions.iter().zip(unmangled) {
        let Some(owner) = &function.owner else {
            continue;
        };
        if function
            .params
            .first()
            .is_none_or(|param| param.name != "this")
        {
            continue;
        }
        if let Some(bare) = name.strip_prefix(&format!("{owner}__"))
            && !bare.starts_with("mcfc")
        {
            methods
                .entry(owner.as_str())
                .or_default()
                .push((bare.to_string(), function));
        }
    }
    let mut functional = HashMap::new();
    for class in program.classes.iter().filter(|class| class.is_interface) {
        let chain = supertypes(&class.name);
        let all: Vec<&(String, &Function)> = chain
            .iter()
            .filter_map(|owner| methods.get(owner.as_str()))
            .flatten()
            .collect();
        let key = |(name, function): &&(String, &Function)| {
            let types: Vec<Type> = function.params[1..].iter().map(|p| p.ty.clone()).collect();
            (name.clone(), types)
        };
        let implemented: BTreeSet<(String, Vec<Type>)> = all
            .iter()
            .filter(|(_, function)| !function.is_abstract)
            .map(key)
            .collect();
        let mut open: Vec<&&(String, &Function)> = all
            .iter()
            .filter(|method| method.1.is_abstract && !implemented.contains(&key(method)))
            .filter(|(name, _)| name != "toString" && name != "equals")
            .collect();
        open.dedup_by_key(|method| key(method));
        if let [(name, function)] = open.as_slice() {
            functional.insert(
                class.name.clone(),
                FunctionalMethod {
                    name: name.clone(),
                    params: function.params[1..].iter().map(|p| p.ty.clone()).collect(),
                    return_type: function.return_type.clone(),
                },
            );
        }
    }
    FUNCTIONAL.with(|map| *map.borrow_mut() = functional);
}

/// A lambda or method reference, which only has a type where one is expected.
pub(super) fn is_function_value(expr: &Expr) -> bool {
    matches!(
        expr.kind,
        ExprKind::Lambda { .. } | ExprKind::MethodRef { .. }
    )
}

/// Checks a lambda or method reference as a value of `expected`, which must be
/// a functional interface.
#[allow(clippy::too_many_arguments)]
pub(super) fn type_check_function_value(
    expr: &Expr,
    expected: &Type,
    struct_defs: &BTreeMap<String, StructTypeDef>,
    signatures: &BTreeMap<String, FunctionSignature>,
    env: &HashMap<String, Type>,
    ref_env: &HashMap<String, RefKind>,
    called_functions: &mut BTreeSet<String>,
    diagnostics: &mut Diagnostics,
) -> TypedExpr {
    let placeholder = TypedExpr {
        kind: TypedExprKind::Int(0),
        ty: expected.clone(),
        ref_kind: RefKind::Unknown,
    };
    let method = match expected {
        Type::Class(interface) => FUNCTIONAL.with(|map| map.borrow().get(interface).cloned()),
        _ => None,
    };
    let (Type::Class(interface), Some(method)) = (expected, method) else {
        diagnostics.push(Diagnostic::new(
            format!(
                "a lambda needs a functional interface (one with exactly one abstract method), not '{}'",
                expected.as_str()
            ),
            expr.span.clone(),
        ));
        return placeholder;
    };
    let (params, mut body, expression) = match &expr.kind {
        ExprKind::Lambda {
            params,
            body,
            expression,
        } => (params.clone(), body.clone(), *expression),
        ExprKind::MethodRef {
            target,
            method: name,
        } => {
            match method_ref_lambda(
                target,
                name,
                method.params.len(),
                env,
                signatures,
                &expr.span,
            ) {
                Ok(lambda) => lambda,
                Err(message) => {
                    diagnostics.push(Diagnostic::new(message, expr.span.clone()));
                    return placeholder;
                }
            }
        }
        _ => unreachable!("only lambdas and method references are function values"),
    };
    if params.len() != method.params.len() {
        diagnostics.push(Diagnostic::new(
            format!(
                "'{}' takes {} arguments, but the lambda has {} parameters",
                display_function(&format!("{interface}__{}", method.name)).replacen("__", ".", 1),
                method.params.len(),
                params.len()
            ),
            expr.span.clone(),
        ));
        return placeholder;
    }
    for ((name, written), ty) in params.iter().zip(&method.params) {
        if let Some(written) = written {
            let mut written = written.clone();
            resolve_enum_type(&mut written, struct_defs);
            if &written != ty {
                diagnostics.push(Diagnostic::new(
                    format!(
                        "lambda parameter '{name}' is '{}', but '{}' gives it '{}'",
                        written.as_str(),
                        expected.as_str(),
                        ty.as_str()
                    ),
                    expr.span.clone(),
                ));
            }
        }
    }
    if expression && method.return_type == Type::Void {
        // `x -> list.add(x)` for a `void` method runs the expression.
        if let Some(Stmt {
            kind: StmtKind::Return(Some(value)),
            span,
        }) = body.pop()
        {
            body.push(Stmt {
                kind: StmtKind::Expr(value),
                span,
            });
        }
    }

    let param_names: HashSet<String> = params.iter().map(|(name, _)| name.clone()).collect();
    let mut declared = param_names.clone();
    declared_names(&body, &mut declared);
    let mut used = BTreeSet::new();
    used_names(&body, &mut used);
    let owner = env_tag(env, OWNER_TAG).map(str::to_string);
    let member = |name: &str| {
        !env.contains_key(name) && this_member(env, struct_defs, name, &expr.span).is_some()
    };
    let method_of_owner = |name: &str| {
        owner
            .as_deref()
            .and_then(|owner| find_method(signatures, owner, name))
    };
    let mut captured: Vec<(String, Type)> = used
        .iter()
        .filter(|name| !declared.contains(*name) && name.as_str() != "this")
        .filter_map(|name| {
            let ty = env.get(name)?;
            (*ty != Type::Void || !name.starts_with('@')).then(|| (name.clone(), ty.clone()))
        })
        .filter(|(name, _)| !name.starts_with('@'))
        .collect();
    let needs_this = env.contains_key("this")
        && used.iter().any(|name| {
            !declared.contains(name)
                && (name == "this"
                    || member(name)
                    || method_of_owner(name).is_some_and(|(_, instance)| instance))
        });
    for stmt in &body {
        if let Some(name) = assigned_capture(stmt, &captured) {
            diagnostics.push(Diagnostic::new(
                format!(
                    "a lambda can't change '{name}'; it has its own copy, made when the lambda is"
                ),
                stmt.span.clone(),
            ));
        }
    }

    // Outer `this`, its fields and its methods go through the captured object.
    if needs_this || owner.is_some() {
        let outer = OuterRewrite {
            declared: &declared,
            captured: &captured,
            member: &|name: &str| {
                if env.contains_key(name) {
                    return None;
                }
                this_member(env, struct_defs, name, &expr.span)
            },
            method: &method_of_owner,
        };
        outer.stmts(&mut body);
    }
    if needs_this {
        captured.push((OUTER.to_string(), env["this"].clone()));
    }

    let enclosing = env_tag(env, FUNCTION_TAG).unwrap_or("lambda");
    let class = format!("{enclosing}__mcfcLambda{}", expr.span.range.start);
    let args: Vec<Expr> = captured
        .iter()
        .map(|(name, _)| Expr {
            kind: ExprKind::Variable(if name == OUTER {
                "this".to_string()
            } else {
                name.clone()
            }),
            span: expr.span.clone(),
        })
        .collect();
    if struct_defs.contains_key(&class) {
        let mut typed = type_check_expr(
            &Expr {
                kind: ExprKind::Call {
                    type_args: Vec::new(),
                    function: format!("{class}__new"),
                    args,
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
        typed.ty = expected.clone();
        return typed;
    }
    let module = env_tag(env, MODULE_TAG).unwrap_or_default().to_string();
    let params: Vec<(String, Type)> = params
        .into_iter()
        .map(|(name, _)| name)
        .zip(method.params.iter().cloned())
        .collect();
    let lambda = lambda_class(
        &class, interface, &method, params, body, &captured, &module, &expr.span,
    );
    if !TRIAL.with(|trial| trial.get()) {
        FOUND.with(|found| {
            found.borrow_mut().entry(class).or_insert(lambda);
        });
    }
    placeholder
}

/// A method reference as the lambda it stands for, given the interface
/// method's parameter count.
fn method_ref_lambda(
    target: &str,
    method: &str,
    arity: usize,
    env: &HashMap<String, Type>,
    signatures: &BTreeMap<String, FunctionSignature>,
    span: &Span,
) -> Result<LambdaParts, String> {
    let at = |kind: ExprKind| Expr {
        kind,
        span: span.clone(),
    };
    let names: Vec<String> = (0..arity).map(|index| format!("mcfcArg{index}")).collect();
    let vars = |names: &[String]| -> Vec<Expr> {
        names
            .iter()
            .map(|name| at(ExprKind::Variable(name.clone())))
            .collect()
    };
    let method_call = |receiver: Expr, args: Vec<Expr>| {
        at(ExprKind::MethodCall {
            type_args: Vec::new(),
            receiver: Box::new(receiver),
            method: crate::language_catalog::internal_method_name(method, args.len()).to_string(),
            args,
        })
    };
    let value = if target == "this" || env.contains_key(target) {
        // `this::speed` and `list::add` call the method on that object.
        method_call(at(ExprKind::Variable(target.to_string())), vars(&names))
    } else if method == "new" {
        at(ExprKind::Call {
            type_args: Vec::new(),
            function: format!("{target}__new"),
            args: vars(&names),
        })
    } else {
        let function = format!("{target}__{method}");
        let static_fits = |name: &str| {
            signatures
                .get(name)
                .is_some_and(|sig| !sig.instance && sig.params.len() == arity)
        };
        let overloads = signatures
            .get(&overload_key(&function))
            .map(|entry| entry.overloads.clone())
            .unwrap_or_default();
        if static_fits(&function) || overloads.iter().any(|name| static_fits(name)) {
            // `Math2::twice` calls a static method with the arguments.
            at(ExprKind::Call {
                type_args: Vec::new(),
                function,
                args: vars(&names),
            })
        } else if arity == 0 {
            return Err(format!(
                "'{}::{method}' needs an object to call '{method}' on",
                target.replace("::", ".")
            ));
        } else {
            // `String::length` calls the method on the first argument.
            let receiver = at(ExprKind::Variable(names[0].clone()));
            method_call(receiver, vars(&names[1..]))
        }
    };
    let body = vec![Stmt {
        kind: StmtKind::Return(Some(value)),
        span: span.clone(),
    }];
    Ok((
        names.into_iter().map(|name| (name, None)).collect(),
        body,
        true,
    ))
}

/// The class for one lambda: a field per capture, a factory taking them, a
/// getter per field, and the interface method holding the lambda's body.
#[allow(clippy::too_many_arguments)]
fn lambda_class(
    class: &str,
    interface: &str,
    method: &FunctionalMethod,
    params: Vec<(String, Type)>,
    body: Vec<Stmt>,
    captured: &[(String, Type)],
    module: &str,
    span: &Span,
) -> LambdaClass {
    let at = |kind: ExprKind| Expr {
        kind,
        span: span.clone(),
    };
    let stmt = |kind: StmtKind| Stmt {
        kind,
        span: span.clone(),
    };
    let this = || at(ExprKind::Variable("this".to_string()));
    let field_of_this = |field: &str| PathExpr {
        base: Box::new(this()),
        segments: vec![PathSegment::Field(field.to_string())],
    };
    let function = |name: &str, params: Vec<Param>, return_type: Type, body: Vec<Stmt>| Function {
        name: format!("{class}__{name}"),
        is_pub: true,
        type_params: Vec::new(),
        bounds: Vec::new(),
        params,
        return_type,
        body,
        span: span.clone(),
        end: span.range.end,
        owner: Some(class.to_string()),
        module: module.to_string(),
        is_abstract: false,
        is_override: false,
        varargs: false,
    };
    let param = |name: &str, ty: Type| Param {
        name: name.to_string(),
        ty,
        span: span.clone(),
    };
    let this_param = || param("this", Type::Struct(class.to_string()));

    let mut factory_body = vec![stmt(StmtKind::Let {
        name: "this".to_string(),
        ty: Some(Type::Struct(class.to_string())),
        value: at(ExprKind::Call {
            type_args: Vec::new(),
            function: "__mcfc_alloc".to_string(),
            args: Vec::new(),
        }),
    })];
    for (name, _) in captured {
        factory_body.push(stmt(StmtKind::Assign {
            target: AssignTarget::Path(field_of_this(name)),
            value: at(ExprKind::Variable(name.clone())),
        }));
    }
    factory_body.push(stmt(StmtKind::Return(Some(this()))));
    let mut functions = vec![function(
        "new",
        captured
            .iter()
            .map(|(name, ty)| param(name, ty.clone()))
            .collect(),
        Type::Struct(class.to_string()),
        factory_body,
    )];
    for (name, ty) in captured {
        functions.push(function(
            &format!("mcfcGet_{name}"),
            vec![this_param()],
            ty.clone(),
            vec![stmt(StmtKind::Return(Some(at(ExprKind::Path(
                field_of_this(name),
            )))))],
        ));
    }
    functions.push(function(
        &method.name,
        std::iter::once(this_param())
            .chain(params.into_iter().map(|(name, ty)| param(&name, ty)))
            .collect(),
        method.return_type.clone(),
        body,
    ));
    let def = ClassDef {
        name: class.to_string(),
        is_pub: true,
        fields: captured
            .iter()
            .map(|(name, ty)| ClassField {
                name: name.clone(),
                ty: ty.clone(),
                is_pub: false,
                is_static: false,
                is_final: true,
                init: None,
                span: span.clone(),
            })
            .collect(),
        parent: None,
        interfaces: vec![interface.to_string()],
        is_interface: false,
        is_abstract: false,
        is_final: true,
        permits: None,
        type_params: Vec::new(),
        bounds: Vec::new(),
        super_args: BTreeMap::new(),
        span: span.clone(),
    };
    (def, functions)
}

/// Every name `body` declares: locals, loop variables and nested lambdas'
/// parameters. They are never captures.
fn declared_names(body: &[Stmt], names: &mut HashSet<String>) {
    fn expr(e: &Expr, names: &mut HashSet<String>) {
        if let ExprKind::Lambda { params, body, .. } = &e.kind {
            names.extend(params.iter().map(|(name, _)| name.clone()));
            declared_names(body, names);
        }
        let mut e = e.clone();
        crate::generics::each_child(&mut e, &mut |child| expr(child, names));
    }
    for stmt in body {
        match &stmt.kind {
            StmtKind::Let { name, .. } | StmtKind::For { name, .. } => {
                names.insert(name.clone());
            }
            _ => {}
        }
        each_part(stmt, &mut |part| match part {
            Part::Expr(e) => expr(e, names),
            Part::Stmts(stmts) => declared_names(stmts, names),
        });
    }
}

/// Every name `body` reads, writes or calls without a receiver, including
/// inside nested lambdas and `$(...)` placeholders.
fn used_names(body: &[Stmt], names: &mut BTreeSet<String>) {
    fn text(value: &str, names: &mut BTreeSet<String>) {
        if !value.contains("$(") {
            return;
        }
        if let Ok(tokens) = crate::lexer::lex(value) {
            for token in tokens {
                if let crate::lexer::TokenKind::Identifier(name) = token.kind {
                    names.insert(name);
                }
            }
        }
        // A string's text is one token; look inside each placeholder too.
        for (start, _) in value.match_indices("$(") {
            let rest = &value[start + 2..];
            if let Ok(tokens) = crate::lexer::lex(rest.split(')').next().unwrap_or_default()) {
                for token in tokens {
                    if let crate::lexer::TokenKind::Identifier(name) = token.kind {
                        names.insert(name);
                    }
                }
            }
        }
    }
    fn expr(e: &Expr, names: &mut BTreeSet<String>) {
        match &e.kind {
            ExprKind::Variable(name) => {
                names.insert(name.clone());
            }
            ExprKind::Call { function, .. } if !function.contains("::") => {
                names.insert(function.clone());
            }
            ExprKind::MethodRef { target, .. } => {
                names.insert(target.clone());
            }
            ExprKind::String(value) => text(value, names),
            ExprKind::Lambda { body, .. } => used_names(body, names),
            _ => {}
        }
        let mut e = e.clone();
        crate::generics::each_child(&mut e, &mut |child| expr(child, names));
    }
    for stmt in body {
        match &stmt.kind {
            StmtKind::Assign {
                target: AssignTarget::Variable(name),
                ..
            } => {
                names.insert(name.clone());
            }
            StmtKind::MacroCommand(command) => text(command, names),
            _ => {}
        }
        each_part(stmt, &mut |part| match part {
            Part::Expr(e) => expr(e, names),
            Part::Stmts(stmts) => used_names(stmts, names),
        });
    }
}

/// A captured variable that `stmt` assigns to, if any.
fn assigned_capture(stmt: &Stmt, captured: &[(String, Type)]) -> Option<String> {
    if let StmtKind::Assign {
        target: AssignTarget::Variable(name),
        ..
    } = &stmt.kind
        && captured.iter().any(|(captured, _)| captured == name)
    {
        return Some(name.clone());
    }
    let mut found = None;
    each_part(stmt, &mut |part| {
        if let Part::Stmts(stmts) = part {
            found = found
                .take()
                .or_else(|| stmts.iter().find_map(|s| assigned_capture(s, captured)));
        }
    });
    found
}

/// Rewrites a lambda body for its class: the enclosing method's `this`
/// becomes the captured `mcfcOuter`, and so do its bare fields and methods.
struct OuterRewrite<'a> {
    declared: &'a HashSet<String>,
    captured: &'a [(String, Type)],
    member: &'a dyn Fn(&str) -> Option<ExprKind>,
    method: &'a dyn Fn(&str) -> Option<(String, bool)>,
}

impl OuterRewrite<'_> {
    fn is_local(&self, name: &str) -> bool {
        self.declared.contains(name) || self.captured.iter().any(|(c, _)| c == name)
    }

    fn outer(span: &Span) -> Expr {
        Expr {
            kind: ExprKind::Variable(OUTER.to_string()),
            span: span.clone(),
        }
    }

    /// `this` in `kind` becomes `mcfcOuter`.
    fn retarget(kind: ExprKind, span: &Span) -> ExprKind {
        let mut expr = Expr {
            kind,
            span: span.clone(),
        };
        fn walk(e: &mut Expr) {
            if matches!(&e.kind, ExprKind::Variable(name) if name == "this") {
                e.kind = ExprKind::Variable(OUTER.to_string());
            }
            crate::generics::each_child(e, &mut walk);
        }
        walk(&mut expr);
        expr.kind
    }

    fn stmts(&self, body: &mut [Stmt]) {
        for stmt in body {
            if let StmtKind::Assign {
                target: target @ AssignTarget::Variable(_),
                ..
            } = &mut stmt.kind
                && let AssignTarget::Variable(name) = &*target
                && !self.is_local(name)
                && let Some(member) = (self.member)(name)
            {
                match Self::retarget(member, &stmt.span) {
                    ExprKind::Path(path) => *target = AssignTarget::Path(path),
                    ExprKind::Variable(state) => *target = AssignTarget::Variable(state),
                    _ => {}
                }
            }
            if let StmtKind::Assign {
                target: AssignTarget::Path(path),
                ..
            } = &mut stmt.kind
            {
                self.expr(&mut path.base);
            }
            each_part_mut(stmt, &mut |part| match part {
                PartMut::Expr(e) => self.expr(e),
                PartMut::Stmts(stmts) => self.stmts(stmts),
            });
        }
    }

    fn expr(&self, e: &mut Expr) {
        match &mut e.kind {
            ExprKind::Variable(name) if name == "this" => {
                e.kind = ExprKind::Variable(OUTER.to_string());
                return;
            }
            ExprKind::Variable(name) if !self.is_local(name) => {
                if let Some(member) = (self.member)(name) {
                    e.kind = Self::retarget(member, &e.span);
                }
                return;
            }
            ExprKind::Call { function, args, .. }
                if !function.contains("::") && !self.is_local(function) =>
            {
                if let Some((resolved, instance)) = (self.method)(function) {
                    let mut args = std::mem::take(args);
                    for arg in &mut args {
                        self.expr(arg);
                    }
                    e.kind = if instance {
                        ExprKind::MethodCall {
                            type_args: Vec::new(),
                            receiver: Box::new(Self::outer(&e.span)),
                            method: function.clone(),
                            args,
                        }
                    } else {
                        ExprKind::Call {
                            type_args: Vec::new(),
                            function: resolved,
                            args,
                        }
                    };
                    return;
                }
            }
            ExprKind::MethodRef { target, .. } if target == "this" => {
                *target = OUTER.to_string();
            }
            ExprKind::Lambda { body, .. } => {
                self.stmts(body);
                return;
            }
            _ => {}
        }
        crate::generics::each_child(e, &mut |child| self.expr(child));
    }
}

/// One part of a statement: an expression or a nested block.
enum Part<'a> {
    Expr(&'a Expr),
    Stmts(&'a [Stmt]),
}

enum PartMut<'a> {
    Expr(&'a mut Expr),
    Stmts(&'a mut [Stmt]),
}

fn each_part(stmt: &Stmt, visit: &mut dyn FnMut(Part)) {
    let mut stmt = stmt.clone();
    each_part_mut(&mut stmt, &mut |part| match part {
        PartMut::Expr(e) => visit(Part::Expr(e)),
        PartMut::Stmts(stmts) => visit(Part::Stmts(stmts)),
    });
}

/// Calls `visit` for each expression and nested block directly in `stmt`.
/// An assignment's target path is left to the caller.
fn each_part_mut(stmt: &mut Stmt, visit: &mut dyn FnMut(PartMut)) {
    match &mut stmt.kind {
        StmtKind::Let { value, .. } => visit(PartMut::Expr(value)),
        StmtKind::Assign { target, value } => {
            if let AssignTarget::Path(path) = target {
                for segment in &mut path.segments {
                    if let PathSegment::Index(index) = segment {
                        visit(PartMut::Expr(index));
                    }
                }
            }
            visit(PartMut::Expr(value));
        }
        StmtKind::If {
            condition,
            then_body,
            else_body,
        } => {
            visit(PartMut::Expr(condition));
            visit(PartMut::Stmts(then_body));
            visit(PartMut::Stmts(else_body));
        }
        StmtKind::While {
            condition,
            body,
            step,
        } => {
            visit(PartMut::Expr(condition));
            visit(PartMut::Stmts(body));
            visit(PartMut::Stmts(step));
        }
        StmtKind::For { iterable, body, .. } => {
            visit(PartMut::Expr(iterable));
            visit(PartMut::Stmts(body));
        }
        StmtKind::Switch {
            value,
            arms,
            default_body,
        } => {
            visit(PartMut::Expr(value));
            for arm in arms {
                visit(PartMut::Expr(&mut arm.pattern));
                visit(PartMut::Stmts(&mut arm.body));
            }
            visit(PartMut::Stmts(default_body));
        }
        StmtKind::Context { anchor, body, .. } => {
            visit(PartMut::Expr(anchor));
            visit(PartMut::Stmts(body));
        }
        StmtKind::Block(body) | StmtKind::Async { body } => visit(PartMut::Stmts(body)),
        StmtKind::Try {
            body,
            catches,
            finally,
        } => {
            visit(PartMut::Stmts(body));
            for catch in catches {
                visit(PartMut::Stmts(&mut catch.body));
            }
            visit(PartMut::Stmts(finally));
        }
        StmtKind::Return(Some(value)) | StmtKind::Expr(value) | StmtKind::Throw(value) => {
            visit(PartMut::Expr(value))
        }
        StmtKind::Return(None)
        | StmtKind::Break
        | StmtKind::Continue
        | StmtKind::RawCommand(_)
        | StmtKind::MacroCommand(_) => {}
    }
}
