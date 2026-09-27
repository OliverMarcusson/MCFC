use std::collections::{BTreeMap, BTreeSet};

use crate::ast::*;
use crate::diagnostics::{Diagnostic, Diagnostics, Span};
use crate::language_catalog::{
    VANILLA_EVENTS, event_kind_for_type, event_type_name, internal_function_name,
    internal_method_name, java_name_for, vanilla_event_has_block, vanilla_event_has_entity,
};
use crate::lexer::{Token, TokenKind, lex};

pub fn parse(source: &str) -> Result<Program, Diagnostics> {
    let tokens = lex(source)?;
    Parser::new(tokens).parse_program()
}

pub fn parse_expression(source: &str) -> Result<Expr, Diagnostics> {
    let tokens = lex(source)?;
    Parser::new(tokens).parse_expression_only()
}

/// The builtin types std gives methods with `final class Name { ... }`, and
/// the type their `this` has. An `Entity` method also runs on a `Player` or a
/// `Selector`, so its `this` is a type parameter, filled in by each call.
fn native_this(name: &str) -> Option<Type> {
    match name {
        "Component" => Some(Type::TextDef),
        "Player" => Some(Type::PlayerRef),
        "Selector" => Some(Type::EntitySet),
        "Entity" => Some(Type::Struct("mcfcSelf".to_string())),
        // Only static methods.
        "HoverEvent" | "TextColor" => Some(Type::Void),
        _ => None,
    }
}

/// `(int) x` and friends lower to these conversion builtins.
const CASTS: &[(&str, &str)] = &[
    ("int", "int"),
    ("float", "float"),
    ("boolean", "bool"),
    ("String", "string"),
    ("Player", "player_ref"),
];

/// `new ItemStack(id)` and friends lower to these builder builtins.
const BUILTIN_CONSTRUCTORS: &[(&str, &str)] = &[
    ("ItemStack", "item"),
    ("EntityData", "entity"),
    ("BlockData", "block_type"),
    ("Component", "text"),
    ("BossBar", "bossbar"),
];

/// Classes whose static methods MCFC provides.
const STATIC_CLASSES: &[&str] = &[
    "Math",
    "Integer",
    "Float",
    "String",
    "Component",
    "ClickEvent",
    "HoverEvent",
    "TextColor",
    "MiniMessage",
];

/// `Math` methods. Each becomes a method on the first argument named `Math.<name>`.
const MATH_METHODS: &[&str] = &[
    "pow", "sqrt", "hypot", "sin", "cos", "tan", "floor", "ceil", "round", "trunc",
];

/// `Math` methods with no `/compute` provider. They call the `std.math` function.
const MATH_STD_FUNCTIONS: &[&str] = &["atan", "atan2", "asin", "acos"];

/// `Selector.of("@a")` and `Block.of("~ ~ ~")` lower to these builtin calls.
const TEXT_DECORATIONS: &[&str] = &[
    "bold",
    "italic",
    "underlined",
    "strikethrough",
    "obfuscated",
];

const STATIC_FACTORIES: &[(&str, &str)] = &[("Selector", "selector"), ("Block", "block")];

/// Static classes whose methods are builtins: `Sidebar.setLine(1, "Kills")`
/// is the `sidebar_line` builtin.
pub(crate) const STATIC_METHODS: &[(&str, &[(&str, &str)])] = &[
    (
        "Sidebar",
        &[
            ("setTitle", "sidebar_title"),
            ("setLine", "sidebar_line"),
            ("removeLine", "sidebar_remove_line"),
            ("clear", "sidebar_clear"),
        ],
    ),
    (
        "Log",
        &[
            ("debug", "log_debug"),
            ("info", "log_info"),
            ("warn", "log_warn"),
            ("error", "log_error"),
            ("setLevel", "log_level"),
            ("dump", "log_dump"),
        ],
    ),
];

struct Annotation {
    name: String,
    args: Vec<(Option<String>, Expr)>,
    span: Span,
}

/// Modifiers written before `class` or `interface`.
#[derive(Default)]
struct ClassModifiers {
    is_abstract: bool,
    is_final: bool,
    is_sealed: bool,
}

enum TypeMember {
    Method(Function),
    Field(ClassField),
    /// A constructor, named after its type, and whether it is `public`.
    Constructor(Function, bool),
    Skipped,
}

struct Parser {
    tokens: Vec<Token>,
    index: usize,
    diagnostics: Diagnostics,
    /// `final` locals and parameters, one list per open block.
    final_scopes: Vec<Vec<String>>,
    /// Set while parsing a class (`Some(false)`) or interface (`Some(true)`) body.
    class_body: Option<bool>,
    /// Set while parsing a `case` label, whose `->` isn't a lambda's.
    in_case_label: bool,
    /// The loops around the statement being parsed, innermost last.
    loops: Vec<LoopFrame>,
}

/// A loop being parsed, for `break label;` and `continue label;`. A jump out
/// of inner loops sets a flag and breaks; after each loop in between, a check
/// of the flag breaks again, until the labeled loop breaks or continues.
struct LoopFrame {
    label: Option<String>,
    /// The flags jumps to this loop set: `(break, continue)`, and whether used.
    flags: (String, String),
    used: (bool, bool),
    /// Outer loops a jump from inside this one goes to, by index in `loops`.
    jumps: BTreeSet<usize>,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self {
            tokens,
            index: 0,
            diagnostics: Diagnostics::new(),
            final_scopes: Vec::new(),
            class_body: None,
            in_case_label: false,
            loops: Vec::new(),
        }
    }

    fn parse_program(mut self) -> Result<Program, Diagnostics> {
        let mut program = Program {
            structs: Vec::new(),
            classes: Vec::new(),
            enums: Vec::new(),
            player_states: Vec::new(),
            world_states: Vec::new(),
            functions: Vec::new(),
            uses: Vec::new(),
        };

        while !self.at(&TokenKind::Eof) {
            let start = self.index;
            if self.at_word("import") {
                self.parse_import(&mut program.uses);
                continue;
            }
            let annotations = self.parse_annotations();
            let is_pub = self.eat_word("public");
            // Every function is static already, so Java's `static` changes nothing.
            self.eat_word("static");
            if self.at_word("record") {
                self.reject_annotations(&annotations, "a record");
                program
                    .structs
                    .push(self.parse_record(is_pub, &mut program.functions));
            } else if let Some(modifiers) = self.class_modifiers() {
                self.reject_annotations(&annotations, "a class");
                let class = self.parse_class(is_pub, modifiers, &mut program);
                program.classes.extend(class);
            } else if self.at_word("enum") {
                self.reject_annotations(&annotations, "an enum");
                program
                    .enums
                    .push(self.parse_enum(is_pub, &mut program.functions));
            } else {
                self.parse_member(annotations, is_pub, &mut program);
            }
            if self.index == start {
                self.recover_top_level();
            }
        }

        self.diagnostics.into_result(program)
    }

    fn parse_import(&mut self, uses: &mut Vec<UseDecl>) {
        let span = self.bump().span;
        let mut path = vec![self.expect_identifier("expected module path after 'import'")];
        let mut alias = None;
        while self.eat(&TokenKind::Dot) {
            // `import a.b.*;` imports every public name of module `a.b`.
            if self.eat(&TokenKind::Star) {
                alias = Some("*".to_string());
                break;
            }
            path.push(self.expect_identifier("expected name after '.'"));
        }
        self.expect_semicolon("import");
        uses.push(UseDecl {
            alias: alias.unwrap_or_else(|| path.last().cloned().unwrap_or_default()),
            path,
            span,
        });
    }

    fn parse_annotations(&mut self) -> Vec<Annotation> {
        let mut annotations = Vec::new();
        while self.at(&TokenKind::At) {
            let span = self.bump().span;
            let name = self.expect_identifier("expected annotation name after '@'");
            let mut args = Vec::new();
            if self.eat(&TokenKind::LeftParen) {
                while !self.at(&TokenKind::RightParen) && !self.at(&TokenKind::Eof) {
                    let key = if matches!(self.peek().kind, TokenKind::Identifier(_))
                        && matches!(self.peek_at(1), TokenKind::Assign)
                    {
                        let key = self.expect_identifier("expected annotation element");
                        self.bump();
                        Some(key)
                    } else {
                        None
                    };
                    args.push((key, self.parse_expr()));
                    if !self.eat(&TokenKind::Comma) {
                        break;
                    }
                }
                self.expect(TokenKind::RightParen, "expected ')' after annotation");
            }
            annotations.push(Annotation { name, args, span });
        }
        annotations
    }

    fn reject_annotations(&mut self, annotations: &[Annotation], what: &str) {
        for annotation in annotations {
            self.diagnostics.push(Diagnostic::new(
                format!("@{} can't be used on {what}", annotation.name),
                annotation.span.clone(),
            ));
        }
    }

    fn parse_record(&mut self, is_pub: bool, functions: &mut Vec<Function>) -> StructDef {
        self.bump();
        let span = self.current_span();
        let name = self.expect_identifier("expected record name");
        self.expect(TokenKind::LeftParen, "expected '(' after record name");
        let mut fields = Vec::new();
        while !self.at(&TokenKind::RightParen) && !self.at(&TokenKind::Eof) {
            let span = self.current_span();
            let ty = self.parse_type();
            let name = self.expect_identifier("expected field name");
            fields.push(StructField { name, ty, span });
            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }
        self.expect(TokenKind::RightParen, "expected ')' after record fields");
        self.expect(TokenKind::LeftBrace, "expected '{' after record header");
        let component_names: Vec<String> = fields.iter().map(|field| field.name.clone()).collect();
        while !self.at(&TokenKind::RightBrace) && !self.at(&TokenKind::Eof) {
            let start = self.index;
            let member = self.parse_type_member(&name, is_pub);
            match member {
                TypeMember::Method(function) => {
                    if let Some(method) = function.name.strip_prefix(&format!("{name}__"))
                        && component_names.iter().any(|field| field == method)
                        && function.params.len() == 1
                    {
                        self.diagnostics.push(Diagnostic::new(
                            format!("'{method}()' is already the accessor of component '{method}'"),
                            function.span.clone(),
                        ));
                    }
                    functions.push(function);
                }
                TypeMember::Field(field) => self.diagnostics.push(Diagnostic::new(
                    "records can't declare fields; add a component to the record header",
                    field.span,
                )),
                TypeMember::Constructor(ctor, _) => self.diagnostics.push(Diagnostic::new(
                    "records can't declare constructors; 'new' takes one argument per component",
                    ctor.span,
                )),
                TypeMember::Skipped => {}
            }
            if self.index == start {
                self.recover_statement();
            }
        }
        self.expect(TokenKind::RightBrace, "expected '}' after record body");
        StructDef {
            name,
            is_pub,
            fields,
            span,
        }
    }

    fn parse_enum(&mut self, is_pub: bool, functions: &mut Vec<Function>) -> EnumDef {
        self.bump();
        let span = self.current_span();
        let name = self.expect_identifier("expected enum name");
        self.expect(TokenKind::LeftBrace, "expected '{' after enum name");
        let mut variants = Vec::new();
        let mut args = Vec::new();
        while matches!(self.peek().kind, TokenKind::Identifier(_)) {
            variants.push(self.expect_identifier("expected enum constant"));
            args.push(if self.eat(&TokenKind::LeftParen) {
                self.parse_call_args()
            } else {
                Vec::new()
            });
            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }
        if variants.is_empty() {
            self.diagnostics.push(Diagnostic::new(
                "enum requires at least one constant",
                span.clone(),
            ));
        }
        let mut constructor = None;
        let mut fields: Vec<EnumField> = Vec::new();
        if self.eat(&TokenKind::Semicolon) {
            while !self.at(&TokenKind::RightBrace) && !self.at(&TokenKind::Eof) {
                let start = self.index;
                match self.parse_type_member(&name, is_pub) {
                    TypeMember::Method(function) => functions.push(function),
                    TypeMember::Field(field) => {
                        if field.is_static || !field.is_final || field.init.is_some() {
                            self.error_at(
                                "enum fields must be 'final', not 'static', and set by the constructor",
                                field.span.clone(),
                            );
                        }
                        fields.push(EnumField {
                            name: field.name,
                            ty: field.ty,
                            param: None,
                            span: field.span,
                        });
                    }
                    TypeMember::Constructor(ctor, _) => {
                        let span = ctor.span.clone();
                        let ctor = self.enum_constructor(ctor);
                        if constructor.replace(ctor).is_some() {
                            self.error_at("an enum can have only one constructor", span);
                        }
                    }
                    TypeMember::Skipped => {}
                }
                if self.index == start {
                    self.recover_statement();
                }
            }
        }
        self.expect(TokenKind::RightBrace, "expected '}' after enum body");
        let (params, assigned) = constructor.unwrap_or_default();
        for (field, param) in assigned {
            match fields.iter_mut().find(|f| f.name == field) {
                Some(found) => found.param = params.iter().position(|p| p.name == param),
                None => self.error_at(
                    &format!("enum '{name}' has no field '{field}'"),
                    span.clone(),
                ),
            }
        }
        for field in &fields {
            if field.param.is_none() {
                self.error_at(
                    &format!("the constructor must set field '{}'", field.name),
                    field.span.clone(),
                );
            }
        }
        EnumDef {
            name,
            is_pub,
            variants,
            args,
            constructor: params,
            fields,
            span,
        }
    }

    /// An enum constructor's parameters, and each `this.field = param;` in it.
    fn enum_constructor(&mut self, function: Function) -> (Vec<Param>, Vec<(String, String)>) {
        let mut assigned = Vec::new();
        for stmt in &function.body {
            if let StmtKind::Assign {
                target: AssignTarget::Path(path),
                value:
                    Expr {
                        kind: ExprKind::Variable(param),
                        ..
                    },
            } = &stmt.kind
                && matches!(&path.base.kind, ExprKind::Variable(base) if base == "this")
                && let [PathSegment::Field(field)] = path.segments.as_slice()
                && function.params.iter().any(|p| &p.name == param)
            {
                assigned.push((field.clone(), param.clone()));
            } else {
                self.error_at(
                    "an enum constructor can only set fields from its parameters, like 'this.mass = mass;'",
                    stmt.span.clone(),
                );
            }
        }
        (function.params, assigned)
    }

    /// `abstract`, `final`, `sealed` and `non-sealed` before `class` or
    /// `interface`; `None`, with nothing consumed, when no class starts here.
    fn class_modifiers(&mut self) -> Option<ClassModifiers> {
        let start = self.index;
        let mut modifiers = ClassModifiers::default();
        loop {
            if self.eat_word("abstract") {
                modifiers.is_abstract = true;
            } else if self.eat_word("final") {
                modifiers.is_final = true;
            } else if self.eat_word("sealed") {
                modifiers.is_sealed = true;
            } else if self.at_word("non")
                && matches!(self.peek_at(1), TokenKind::Minus)
                && matches!(self.peek_at(2), TokenKind::Identifier(word) if word == "sealed")
            {
                // `non-sealed` only reopens a sealed parent, which is the default.
                self.bump();
                self.bump();
                self.bump();
            } else {
                break;
            }
        }
        if self.at_word("class") || self.at_word("interface") {
            return Some(modifiers);
        }
        self.index = start;
        None
    }

    /// `<T, U extends Bound>`, or nothing. Returns the names and their bounds.
    fn parse_type_params(&mut self) -> (Vec<String>, Vec<(String, Type)>) {
        let mut names = Vec::new();
        let mut bounds = Vec::new();
        if !self.eat(&TokenKind::Lt) {
            return (names, bounds);
        }
        loop {
            let name = self.expect_identifier("expected type parameter name");
            if self.eat_word("extends") {
                loop {
                    bounds.push((name.clone(), self.parse_type()));
                    if !self.eat(&TokenKind::Amp) {
                        break;
                    }
                }
            }
            names.push(name);
            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }
        self.expect(TokenKind::Gt, "expected '>' after type parameters");
        (names, bounds)
    }

    /// `A, B<T>` after `implements`, `extends` (interfaces) or `permits`.
    /// Type arguments go into `args` by name.
    fn parse_type_names(&mut self, args: &mut BTreeMap<String, Vec<Type>>) -> Vec<String> {
        let mut names = Vec::new();
        loop {
            match self.parse_type() {
                Type::Struct(name) => names.push(name),
                Type::Generic(name, types) => {
                    args.insert(name.clone(), types);
                    names.push(name);
                }
                _ => self.error_here("expected a class or interface name"),
            }
            if !self.eat(&TokenKind::Comma) {
                return names;
            }
        }
    }

    /// `class Name extends Parent implements A { members }`, or an interface.
    ///
    /// Each constructor becomes `Name__mcfcInit(this, ...)`, which runs the
    /// parent's constructor (`super(...)`), the field initializers and then
    /// the body, and a factory `Name__new(...)` that allocates the object and
    /// calls it. Abstract classes and interfaces get no factory. Each instance
    /// field gets a getter for accesses on something other than a variable.
    fn parse_class(
        &mut self,
        is_pub: bool,
        modifiers: ClassModifiers,
        program: &mut Program,
    ) -> Option<ClassDef> {
        let first_method = program.functions.len();
        let is_interface = self.at_word("interface");
        self.bump();
        let span = self.current_span();
        let name = self.expect_identifier("expected class name");
        let (type_params, bounds) = self.parse_type_params();
        let mut parent = None;
        let mut interfaces = Vec::new();
        let mut super_args = BTreeMap::new();
        if self.eat_word("extends") {
            if is_interface {
                interfaces = self.parse_type_names(&mut super_args);
            } else {
                match self.parse_type() {
                    Type::Struct(name) => parent = Some(name),
                    Type::Generic(name, types) => {
                        super_args.insert(name.clone(), types);
                        parent = Some(name);
                    }
                    _ => self.error_here("expected a class name after 'extends'"),
                }
            }
        }
        if !is_interface && self.eat_word("implements") {
            interfaces = self.parse_type_names(&mut super_args);
        }
        let permits = self
            .eat_word("permits")
            .then(|| self.parse_type_names(&mut BTreeMap::new()));
        if modifiers.is_sealed && permits.is_none() {
            self.error_at(
                "a sealed type lists what may extend it: 'permits A, B'",
                span.clone(),
            );
        }
        self.expect(TokenKind::LeftBrace, "expected '{' after class name");
        let saved_body = self.class_body.replace(is_interface);
        let mut fields: Vec<ClassField> = Vec::new();
        let mut constructors = Vec::new();
        while !self.at(&TokenKind::RightBrace) && !self.at(&TokenKind::Eof) {
            let start = self.index;
            match self.parse_type_member(&name, is_pub) {
                TypeMember::Method(function) => program.functions.push(function),
                TypeMember::Field(mut field) => {
                    // Interface fields are constants, like Java's.
                    if is_interface {
                        field.is_static = true;
                        field.is_final = true;
                        field.is_pub = true;
                    }
                    fields.push(field);
                }
                TypeMember::Constructor(function, ctor_pub) => {
                    if is_interface {
                        self.error_at("an interface has no constructor", function.span.clone());
                    }
                    constructors.push((function, ctor_pub))
                }
                TypeMember::Skipped => {}
            }
            if self.index == start {
                self.recover_statement();
            }
        }
        self.class_body = saved_body;
        self.expect(TokenKind::RightBrace, "expected '}' after class body");
        if let Some(this_type) = native_this(&name) {
            self.native_class(&name, &fields, &constructors, &span);
            for function in &mut program.functions[first_method..] {
                match function.params.first_mut() {
                    Some(param) if param.name == "this" => {
                        param.ty = this_type.clone();
                        if let Type::Struct(self_type) = &this_type {
                            function.type_params.insert(0, self_type.clone());
                        }
                    }
                    _ => {}
                }
            }
            return None;
        }
        let at = |kind: ExprKind| Expr {
            kind,
            span: span.clone(),
        };
        let stmt = |kind: StmtKind| Stmt {
            kind,
            span: span.clone(),
        };
        let this_field = |field: &str| {
            AssignTarget::Path(PathExpr {
                base: Box::new(at(ExprKind::Variable("this".to_string()))),
                segments: vec![PathSegment::Field(field.to_string())],
            })
        };
        let this_param = |span: &Span| Param {
            name: "this".to_string(),
            ty: Type::Struct(name.clone()),
            span: span.clone(),
        };
        if constructors.is_empty() && !is_interface {
            constructors.push((self.synthetic_function(&name, "new", &span), true));
        }
        for (ctor, ctor_pub) in constructors {
            let mut rest = ctor.body;
            let starts_with_super = matches!(
                rest.first().map(|stmt| &stmt.kind),
                Some(StmtKind::Expr(Expr {
                    kind: ExprKind::Call { function, .. },
                    ..
                })) if function == "super"
            );
            let mut body = Vec::new();
            if starts_with_super {
                body.push(rest.remove(0));
            } else if parent.is_some() {
                body.push(stmt(StmtKind::Expr(at(ExprKind::Call {
                    function: "super".to_string(),
                    args: Vec::new(),
                }))));
            }
            for field in fields.iter().filter(|field| !field.is_static) {
                body.push(stmt(StmtKind::Assign {
                    target: this_field(&field.name),
                    // The type checker fills in the type's empty value.
                    value: field
                        .init
                        .clone()
                        .unwrap_or_else(|| at(ExprKind::Variable("__mcfc_default".to_string()))),
                }));
            }
            body.extend(rest);
            let mut init = self.synthetic_function(&name, "mcfcInit", &ctor.span);
            init.params = std::iter::once(this_param(&ctor.span))
                .chain(ctor.params.iter().cloned())
                .collect();
            init.body = body;
            init.is_pub = ctor_pub && is_pub;
            init.end = ctor.end;
            if !modifiers.is_abstract {
                let mut args = vec![at(ExprKind::Variable("this".to_string()))];
                args.extend(
                    ctor.params
                        .iter()
                        .map(|param| at(ExprKind::Variable(param.name.clone()))),
                );
                let mut factory = self.synthetic_function(&name, "new", &ctor.span);
                factory.params = ctor.params;
                factory.return_type = Type::Struct(name.clone());
                factory.is_pub = ctor_pub && is_pub;
                factory.end = ctor.end;
                factory.body = vec![stmt(StmtKind::Let {
                    name: "this".to_string(),
                    ty: Some(Type::Struct(name.clone())),
                    value: at(ExprKind::Call {
                        function: "__mcfc_alloc".to_string(),
                        args: Vec::new(),
                    }),
                })];
                // The init body is copied in, saving a call per `new`, unless
                // a `return` in it would end the factory early.
                if has_return(&init.body) {
                    factory.body.push(stmt(StmtKind::Expr(at(ExprKind::Call {
                        function: format!("{name}__mcfcInit"),
                        args,
                    }))));
                } else {
                    factory.body.extend(init.body.iter().cloned());
                }
                factory
                    .body
                    .push(stmt(StmtKind::Return(Some(at(ExprKind::Variable(
                        "this".to_string(),
                    ))))));
                program.functions.push(factory);
            }
            program.functions.push(init);
        }
        for field in fields.iter().filter(|field| !field.is_static) {
            let read = at(ExprKind::Path(PathExpr {
                base: Box::new(at(ExprKind::Variable("this".to_string()))),
                segments: vec![PathSegment::Field(field.name.clone())],
            }));
            let mut getter =
                self.synthetic_function(&name, &format!("mcfcGet_{}", field.name), &field.span);
            getter.params = vec![this_param(&field.span)];
            getter.return_type = field.ty.clone();
            getter.body = vec![stmt(StmtKind::Return(Some(read)))];
            getter.is_pub = field.is_pub && is_pub;
            program.functions.push(getter);
        }
        // Static fields are world state (see the type checker). `Name__clinit`,
        // run from the load tag, sets their initial values once per world.
        if fields.iter().any(|field| field.is_static) {
            let ready = "mcfcClinitDone".to_string();
            let mut body = vec![stmt(StmtKind::Assign {
                target: AssignTarget::Variable(ready.clone()),
                value: at(ExprKind::Bool(true)),
            })];
            for field in fields.iter().filter(|field| field.is_static) {
                body.push(stmt(StmtKind::Assign {
                    target: AssignTarget::Variable(field.name.clone()),
                    value: field
                        .init
                        .clone()
                        .unwrap_or_else(|| at(ExprKind::Variable("__mcfc_default".to_string()))),
                }));
            }
            fields.push(ClassField {
                name: ready.clone(),
                ty: Type::Bool,
                is_pub: false,
                is_static: true,
                is_final: false,
                init: None,
                span: span.clone(),
            });
            let mut clinit = self.synthetic_function(&name, "clinit", &span);
            clinit.body = vec![stmt(StmtKind::If {
                condition: at(ExprKind::Unary {
                    op: UnaryOp::Not,
                    expr: Box::new(at(ExprKind::Variable(ready))),
                }),
                then_body: body,
                else_body: Vec::new(),
            })];
            program.functions.push(clinit);
        }
        Some(ClassDef {
            name,
            is_pub,
            fields,
            parent,
            interfaces,
            is_interface,
            is_abstract: modifiers.is_abstract || is_interface,
            is_final: modifiers.is_final,
            permits,
            type_params,
            bounds,
            super_args,
            span,
        })
    }

    /// `final class Component { ... }` names a builtin type, so it only adds
    /// methods, whose `this` is that builtin value.
    fn native_class(
        &mut self,
        name: &str,
        fields: &[ClassField],
        constructors: &[(Function, bool)],
        span: &Span,
    ) {
        if !fields.is_empty() || !constructors.is_empty() {
            self.error_at(
                &format!("'{name}' is a builtin type, so its class only has methods"),
                span.clone(),
            );
        }
    }

    fn synthetic_function(&self, owner: &str, method: &str, span: &Span) -> Function {
        Function {
            name: format!("{owner}__{method}"),
            is_pub: true,
            type_params: Vec::new(),
            bounds: Vec::new(),
            params: Vec::new(),
            return_type: Type::Void,
            body: Vec::new(),
            span: span.clone(),
            end: 0,
            owner: Some(owner.to_string()),
            module: String::new(),
            is_abstract: false,
            is_override: false,
            varargs: false,
        }
    }

    /// One member of a record or enum body: a method, and for enums a field or
    /// the constructor. Methods become top-level functions named `Type__method`.
    fn parse_type_member(&mut self, owner: &str, owner_is_pub: bool) -> TypeMember {
        let annotations = self.parse_annotations();
        let in_interface = self.class_body == Some(true);
        let mut is_pub = self.eat_word("public");
        let is_private = !is_pub && self.eat_word("private");
        if !is_pub && !is_private {
            self.eat_word("protected");
        }
        // Interface members are public unless marked private.
        is_pub |= in_interface && !is_private;
        let marked_abstract = self.eat_word("abstract");
        let is_default = self.eat_word("default");
        let is_static = self.eat_word("static");
        let is_final = self.eat_word("final");
        if is_default && !in_interface {
            self.error_here("only interface methods can be 'default'");
        }
        let span = self.current_span();
        // A constructor: `Planet(float mass) { this.mass = mass; }`.
        if self.at_word(owner) && matches!(self.peek_at(1), TokenKind::LeftParen) {
            self.reject_annotations(&annotations, "a constructor");
            self.bump();
            let function =
                self.parse_function_rest(owner.to_string(), span.clone(), Vec::new(), Type::Void);
            return TypeMember::Constructor(function, is_pub);
        }
        let (type_params, bounds) = self.parse_type_params();
        if !matches!(self.peek().kind, TokenKind::Identifier(_)) {
            self.error_here("expected a method");
            return TypeMember::Skipped;
        }
        let ty = self.parse_type();
        let span = self.current_span();
        let name = self.expect_identifier("expected a name after the type");
        if !self.at(&TokenKind::LeftParen) {
            self.reject_annotations(&annotations, "a field");
            let init = self.eat(&TokenKind::Assign).then(|| self.parse_expr());
            self.expect_semicolon("field declaration");
            return TypeMember::Field(ClassField {
                name,
                ty,
                is_pub,
                is_static,
                is_final,
                init,
                span,
            });
        }
        let mut function =
            self.parse_function_rest(format!("{owner}__{name}"), span.clone(), type_params, ty);
        function.bounds = bounds;
        function.owner = Some(owner.to_string());
        if function.is_abstract && !(marked_abstract || in_interface && !is_default && !is_static) {
            self.error_at("a method needs a body unless it is abstract", span.clone());
        } else if !function.is_abstract && marked_abstract {
            self.error_at(
                "an abstract method has no body; end it with ';'",
                span.clone(),
            );
        } else if in_interface && !function.is_abstract && !is_default && !is_static && !is_private
        {
            self.error_at(
                "an interface method with a body must be 'default', 'static' or 'private'",
                span.clone(),
            );
        }
        // Methods of a private type are only reachable where the type is.
        function.is_pub = is_pub && owner_is_pub;
        if !is_static {
            function.params.insert(
                0,
                Param {
                    name: "this".to_string(),
                    ty: Type::Struct(owner.to_string()),
                    span: span.clone(),
                },
            );
        }
        for annotation in &annotations {
            let overridable = matches!(
                (name.as_str(), function.params.len()),
                ("toString", 1) | ("equals", 2)
            );
            if annotation.name != "Override" {
                self.error_at(
                    &format!("@{} can't be used on a method", annotation.name),
                    annotation.span.clone(),
                );
            } else if self.class_body.is_some() {
                // The type checker checks that a class method overrides something.
                function.is_override = true;
                if is_static {
                    self.error_at(
                        "a static method doesn't override anything",
                        annotation.span.clone(),
                    );
                }
            } else if is_static || !overridable {
                self.error_at(
                    "method doesn't override anything; only 'toString()' and 'equals(other)' can be overridden",
                    annotation.span.clone(),
                );
            }
        }
        TypeMember::Method(function)
    }

    /// A function (`<T> R name(...) { }`) or an annotated state field (`@PlayerState int coins;`).
    fn parse_member(&mut self, annotations: Vec<Annotation>, is_pub: bool, program: &mut Program) {
        let (type_params, bounds) = self.parse_type_params();
        if !matches!(self.peek().kind, TokenKind::Identifier(_)) {
            self.error_here("expected a function, record, enum, or import declaration");
            return;
        }
        let ty = self.parse_type();
        let span = self.current_span();
        let mut path = vec![self.expect_identifier("expected a name after the type")];
        if self.at(&TokenKind::LeftParen) {
            let mut function = self.parse_function_rest(path.remove(0), span, type_params, ty);
            function.bounds = bounds;
            if function.is_abstract {
                self.error_at("a function needs a body", function.span.clone());
            }
            function.is_pub = is_pub;
            self.apply_function_annotations(&annotations, &mut function);
            program.functions.push(function);
            return;
        }
        while self.eat(&TokenKind::Dot) {
            path.push(self.expect_identifier("expected state path segment"));
        }
        self.expect_semicolon("state declaration");
        let owner = match annotations.first().map(|a| a.name.as_str()) {
            Some("PlayerState") => StateOwner::Player,
            Some("EntityState") => StateOwner::Entity,
            Some("WorldState") => StateOwner::World,
            _ => {
                self.diagnostics.push(Diagnostic::new(
                    "top-level variables need @PlayerState, @EntityState or @WorldState",
                    span,
                ));
                return;
            }
        };
        let display_name = match annotations[0].args.as_slice() {
            [] => path.join("."),
            [
                (
                    None,
                    Expr {
                        kind: ExprKind::String(name),
                        ..
                    },
                ),
            ] if owner == StateOwner::Player => name.clone(),
            _ => {
                self.diagnostics.push(Diagnostic::new(
                    match owner {
                        StateOwner::Player => "@PlayerState takes an optional display name string",
                        StateOwner::Entity => "@EntityState takes no arguments",
                        StateOwner::World => "@WorldState takes no arguments",
                    },
                    annotations[0].span.clone(),
                ));
                String::new()
            }
        };
        self.reject_annotations(&annotations[1..], "a state declaration");
        if owner == StateOwner::World {
            if path.len() > 1 {
                self.diagnostics.push(Diagnostic::new(
                    "a @WorldState name can't have dots",
                    span.clone(),
                ));
            }
            program.world_states.push(PlayerStateDef {
                owner,
                path,
                ty,
                display_name,
                span,
            });
            return;
        }
        program.player_states.push(PlayerStateDef {
            owner,
            path,
            ty,
            display_name,
            span,
        });
    }

    fn parse_function_rest(
        &mut self,
        name: String,
        span: Span,
        type_params: Vec<String>,
        return_type: Type,
    ) -> Function {
        self.expect(TokenKind::LeftParen, "expected '(' after function name");
        let mut params = Vec::new();
        let mut finals = Vec::new();
        let mut varargs = false;
        while !self.at(&TokenKind::RightParen) && !self.at(&TokenKind::Eof) {
            let span = self.current_span();
            let is_final = self.eat_word("final");
            let mut ty = self.parse_type();
            if varargs {
                self.error_at("only the last parameter can be '...'", span.clone());
            }
            if self.at(&TokenKind::Dot) && matches!(self.peek_at(1), TokenKind::Dot) {
                self.bump();
                self.bump();
                self.expect(TokenKind::Dot, "expected '...' after the parameter type");
                ty = Type::Array(Box::new(ty));
                varargs = true;
            }
            let name = self.expect_identifier("expected parameter name");
            if is_final {
                finals.push(name.clone());
            }
            params.push(Param { name, ty, span });
            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }
        self.expect(TokenKind::RightParen, "expected ')' after parameters");
        // `throws A, B` only documents: every exception is unchecked.
        if self.eat_word("throws") {
            self.parse_type();
            while self.eat(&TokenKind::Comma) {
                self.parse_type();
            }
        }
        let is_abstract = self.eat(&TokenKind::Semicolon);
        self.final_scopes.push(finals);
        let body = if is_abstract {
            Vec::new()
        } else {
            self.parse_block("function body")
        };
        self.final_scopes.pop();
        Function {
            name,
            is_pub: false,
            type_params,
            bounds: Vec::new(),
            params,
            return_type,
            body,
            span,
            end: self.previous_end(),
            owner: None,
            module: String::new(),
            is_abstract,
            is_override: false,
            varargs,
        }
    }

    /// `@EventHandler`, `@Command`, `@Every` and `@After` turn a function into a handler.
    /// It is renamed to the hook name the backend installs; parameters that stand
    /// for the running player become a prologue.
    fn apply_function_annotations(&mut self, annotations: &[Annotation], function: &mut Function) {
        let Some((annotation, rest)) = annotations.split_first() else {
            return;
        };
        self.reject_annotations(rest, "a function that already has a handler annotation");
        if function.return_type != Type::Void || !function.type_params.is_empty() {
            self.diagnostics.push(Diagnostic::new(
                format!(
                    "@{} handlers must be non-generic 'void' functions",
                    annotation.name
                ),
                annotation.span.clone(),
            ));
        }
        let span = annotation.span.clone();
        match annotation.name.as_str() {
            "EventHandler" => {
                if !annotation.args.is_empty() {
                    self.error_at(
                        "@EventHandler takes no arguments; the parameter type picks the event",
                        span.clone(),
                    );
                }
                let kind = match function.params.as_slice() {
                    [param] => match &param.ty {
                        Type::Struct(ty) => event_kind_for_type(ty),
                        _ => None,
                    },
                    _ => None,
                };
                let Some(kind) = kind else {
                    self.error_at(
                        "@EventHandler handlers take one event parameter, like 'PlayerJoinEvent event'",
                        span,
                    );
                    return;
                };
                if VANILLA_EVENTS.contains(&kind) {
                    // Vanilla handlers run as the player; the event is built from `@s`.
                    let param = function.params.remove(0);
                    let at_s = call(
                        "selector",
                        vec![string_expr("@s", &param.span)],
                        &param.span,
                    );
                    let player = call(
                        "player_ref",
                        vec![call("single", vec![at_s], &param.span)],
                        &param.span,
                    );
                    let mut fields = vec![("player".to_string(), player)];
                    if vanilla_event_has_entity(kind) {
                        // The generated reward function tags the other entity.
                        let target = call(
                            "selector",
                            vec![string_expr(
                                "@e[tag=mcfc_event_target,limit=1]",
                                &param.span,
                            )],
                            &param.span,
                        );
                        fields.push((
                            "entity".to_string(),
                            call("single", vec![target], &param.span),
                        ));
                    }
                    if vanilla_event_has_block(kind) {
                        // The generated reward function stores the ray hit.
                        fields.push((
                            "block".to_string(),
                            call("__mcfc_event_block", Vec::new(), &param.span),
                        ));
                    }
                    let event = Expr {
                        kind: ExprKind::StructLiteral {
                            name: event_type_name(kind),
                            fields,
                        },
                        span: param.span.clone(),
                    };
                    function.body.insert(
                        0,
                        Stmt {
                            kind: StmtKind::Let {
                                name: param.name,
                                ty: None,
                                value: event,
                            },
                            span: param.span,
                        },
                    );
                    function.name = format!("__mcfc_event_{kind}");
                } else {
                    function.name = format!("__mcfc_agent_event_{kind}");
                }
            }
            "Command" | "Menu" => {
                let menu = annotation.name == "Menu";
                let name = match annotation.args.as_slice() {
                    [] if !menu => resource_name(&function.name),
                    [
                        (
                            None,
                            Expr {
                                kind: ExprKind::String(label),
                                ..
                            },
                        ),
                    ] if menu => {
                        // The button label rides in the name, hex-encoded so it
                        // stays a valid function path; the backend decodes it.
                        let hex: String = label.bytes().map(|b| format!("{b:02x}")).collect();
                        format!("{}__menu_{hex}", resource_name(&function.name))
                    }
                    [
                        (
                            None,
                            Expr {
                                kind: ExprKind::String(name),
                                ..
                            },
                        ),
                    ] => name.clone(),
                    _ if menu => {
                        self.error_at("@Menu takes a button label string", span);
                        return;
                    }
                    _ => {
                        self.error_at("@Command takes an optional command name string", span);
                        return;
                    }
                };
                if !is_command_name(&name) {
                    self.error_at(&format!("'{name}' is not a valid command name"), span);
                }
                let takes_sender = matches!(function.params.as_slice(),
                    [param] if param.ty == Type::Struct("CommandSender".to_string()));
                if !takes_sender {
                    self.bind_player_param(function, &annotation.name);
                }
                function.name = format!("__mcfc_command_{name}");
            }
            "Every" | "After" => {
                let ticks = match annotation.args.as_slice() {
                    [
                        (
                            Some(unit),
                            Expr {
                                kind: ExprKind::Int(n),
                                ..
                            },
                        ),
                    ] if *n > 0 => match unit.as_str() {
                        "ticks" => Some(*n),
                        "seconds" => Some(*n * 20),
                        _ => None,
                    },
                    _ => None,
                };
                let Some(ticks) = ticks else {
                    self.error_at(
                        &format!(
                            "@{} takes a positive 'ticks = n' or 'seconds = n'",
                            annotation.name
                        ),
                        span,
                    );
                    return;
                };
                if !function.params.is_empty() {
                    self.error_at("task handlers take no parameters", span);
                }
                let schedule = if annotation.name == "Every" {
                    "every"
                } else {
                    "after"
                };
                function.name = format!(
                    "__mcfc_task_{}_{schedule}_ticks_{ticks}",
                    resource_name(&function.name)
                );
            }
            "Test" => {
                if !annotation.args.is_empty() || !function.params.is_empty() {
                    self.error_at("@Test functions take no arguments or parameters", span);
                }
                function.name = format!("__mcfc_test_{}", resource_name(&function.name));
            }
            other => self.error_at(&format!("unknown annotation '@{other}'"), span),
        }
    }

    /// A handler that runs as the player may name them: `void onJoin(Player player)`.
    fn bind_player_param(&mut self, function: &mut Function, annotation: &str) {
        match function.params.as_slice() {
            [] => {}
            [param] if param.ty == Type::PlayerRef => {
                let param = function.params.remove(0);
                let at_s = call(
                    "selector",
                    vec![string_expr("@s", &param.span)],
                    &param.span,
                );
                let player = call(
                    "player_ref",
                    vec![call("single", vec![at_s], &param.span)],
                    &param.span,
                );
                function.body.insert(
                    0,
                    Stmt {
                        kind: StmtKind::Let {
                            name: param.name,
                            ty: None,
                            value: player,
                        },
                        span: param.span,
                    },
                );
            }
            _ => self.error_at(
                &format!("@{annotation} handlers take no parameters or one 'Player'"),
                function.span.clone(),
            ),
        }
    }

    fn parse_expression_only(mut self) -> Result<Expr, Diagnostics> {
        let expr = self.parse_expr();
        if !self.at(&TokenKind::Eof) {
            self.error_here("expected end of placeholder expression");
        }
        self.diagnostics.into_result(expr)
    }

    fn parse_block(&mut self, what: &str) -> Vec<Stmt> {
        if !self.eat(&TokenKind::LeftBrace) {
            self.error_here(&format!("expected '{{' to start the {what}"));
            return Vec::new();
        }
        let mut statements = Vec::new();
        self.final_scopes.push(Vec::new());
        while !self.at(&TokenKind::RightBrace) && !self.at(&TokenKind::Eof) {
            let start = self.index;
            statements.push(self.parse_stmt());
            if self.index == start {
                self.bump();
            }
        }
        self.final_scopes.pop();
        self.expect(
            TokenKind::RightBrace,
            &format!("expected '}}' to end the {what}"),
        );
        statements
    }

    /// A braced block or a single statement, as after `if (...)`.
    fn parse_body(&mut self, what: &str) -> Vec<Stmt> {
        if self.at(&TokenKind::LeftBrace) {
            self.parse_block(what)
        } else {
            vec![self.parse_stmt()]
        }
    }

    fn parse_stmt(&mut self) -> Stmt {
        let span = self.current_span();
        let label = match (self.peek().kind.clone(), self.peek_at(1)) {
            (TokenKind::Identifier(name), TokenKind::Colon)
                if !matches!(self.peek_at(2), TokenKind::Colon) =>
            {
                self.bump();
                self.bump();
                Some(name)
            }
            _ => None,
        };
        if matches!(
            self.peek().kind,
            TokenKind::While | TokenKind::For | TokenKind::Do
        ) {
            return self.parse_loop(label, span);
        }
        if let Some(label) = label {
            self.error_at(
                &format!("only loops can have a label like '{label}:'"),
                span,
            );
        }
        self.parse_stmt_inner()
    }

    /// A loop, with the flags and checks its labeled jumps need around it.
    fn parse_loop(&mut self, label: Option<String>, span: Span) -> Stmt {
        let name = label.clone().unwrap_or_default();
        let flag = |kind: &str| format!("__{kind}_{name}_{}_{}", span.line, span.column);
        self.loops.push(LoopFrame {
            label,
            flags: (flag("break"), flag("continue")),
            used: (false, false),
            jumps: BTreeSet::new(),
        });
        let stmt = self.parse_stmt_inner();
        let frame = self.loops.pop().expect("pushed above");
        let at = |kind: ExprKind| Expr {
            kind,
            span: span.clone(),
        };
        let stmt_at = |kind: StmtKind| Stmt {
            kind,
            span: span.clone(),
        };
        let set = |flag: &str, value: bool| {
            stmt_at(StmtKind::Assign {
                target: AssignTarget::Variable(flag.to_string()),
                value: at(ExprKind::Bool(value)),
            })
        };
        let when = |flag: &str, then_body: Vec<Stmt>| {
            stmt_at(StmtKind::If {
                condition: at(ExprKind::Variable(flag.to_string())),
                then_body,
                else_body: Vec::new(),
            })
        };
        let mut out = Vec::new();
        for (used, flag) in [
            (frame.used.0, &frame.flags.0),
            (frame.used.1, &frame.flags.1),
        ] {
            if used {
                out.push(stmt_at(StmtKind::Let {
                    name: flag.clone(),
                    ty: None,
                    value: at(ExprKind::Bool(false)),
                }));
            }
        }
        out.push(stmt);
        let enclosing = self.loops.len().checked_sub(1);
        for target in frame.jumps {
            let (used, (break_flag, continue_flag)) =
                (self.loops[target].used, self.loops[target].flags.clone());
            if used.0 {
                out.push(when(&break_flag, vec![stmt_at(StmtKind::Break)]));
            }
            if used.1 && Some(target) == enclosing {
                out.push(when(
                    &continue_flag,
                    vec![set(&continue_flag, false), stmt_at(StmtKind::Continue)],
                ));
            } else if used.1 {
                out.push(when(&continue_flag, vec![stmt_at(StmtKind::Break)]));
            }
        }
        if out.len() == 1 {
            return out.pop().expect("one statement");
        }
        stmt_at(StmtKind::Block(out))
    }

    /// `break label;` or `continue label;`.
    fn labeled_jump(&mut self, label: &str, is_break: bool, span: Span) -> StmtKind {
        let jump = if is_break {
            StmtKind::Break
        } else {
            StmtKind::Continue
        };
        let Some(target) = self
            .loops
            .iter()
            .rposition(|frame| frame.label.as_deref() == Some(label))
        else {
            self.error_at(&format!("no loop labeled '{label}' is around this"), span);
            return jump;
        };
        if target + 1 == self.loops.len() {
            return jump;
        }
        let frame = &mut self.loops[target];
        let flag = if is_break {
            frame.used.0 = true;
            frame.flags.0.clone()
        } else {
            frame.used.1 = true;
            frame.flags.1.clone()
        };
        for frame in &mut self.loops[target + 1..] {
            frame.jumps.insert(target);
        }
        let at = |kind| Stmt {
            kind,
            span: span.clone(),
        };
        StmtKind::Block(vec![
            at(StmtKind::Assign {
                target: AssignTarget::Variable(flag),
                value: Expr {
                    kind: ExprKind::Bool(true),
                    span: span.clone(),
                },
            }),
            at(StmtKind::Break),
        ])
    }

    fn parse_stmt_inner(&mut self) -> Stmt {
        let span = self.current_span();
        let kind = match self.peek().kind.clone() {
            TokenKind::LeftBrace => StmtKind::Block(self.parse_block("block")),
            TokenKind::If => {
                self.bump();
                self.parse_if_rest()
            }
            TokenKind::While => {
                self.bump();
                let condition = self.parse_paren_expr("while");
                let body = self.parse_body("while body");
                StmtKind::While {
                    condition,
                    body,
                    step: Vec::new(),
                }
            }
            TokenKind::For => {
                self.bump();
                self.parse_for_rest(&span)
            }
            // `do { body } while (c);` is
            // `{ var first = true; while (first || c) { first = false; body } }`,
            // so the body runs once before the check and `continue` still checks.
            TokenKind::Do => {
                self.bump();
                let mut body = self.parse_block("do body");
                self.expect(TokenKind::While, "expected 'while' after the do body");
                let condition = self.parse_paren_expr("while");
                self.expect_semicolon("do-while");
                let first = format!("__do_{}_{}", span.line, span.column);
                let flag = |kind: ExprKind| Expr {
                    kind,
                    span: span.clone(),
                };
                body.insert(
                    0,
                    Stmt {
                        kind: StmtKind::Assign {
                            target: AssignTarget::Variable(first.clone()),
                            value: flag(ExprKind::Bool(false)),
                        },
                        span: span.clone(),
                    },
                );
                StmtKind::Block(vec![
                    Stmt {
                        kind: StmtKind::Let {
                            name: first.clone(),
                            ty: None,
                            value: flag(ExprKind::Bool(true)),
                        },
                        span: span.clone(),
                    },
                    Stmt {
                        kind: StmtKind::While {
                            condition: flag(ExprKind::Binary {
                                op: BinaryOp::Or,
                                left: Box::new(flag(ExprKind::Variable(first))),
                                right: Box::new(condition),
                            }),
                            body,
                            step: Vec::new(),
                        },
                        span: span.clone(),
                    },
                ])
            }
            TokenKind::Async => {
                self.bump();
                StmtKind::Async {
                    body: self.parse_block("async block"),
                }
            }
            TokenKind::Break | TokenKind::Continue => {
                let is_break = self.bump().kind == TokenKind::Break;
                let what = if is_break { "break" } else { "continue" };
                let label = match self.peek().kind.clone() {
                    TokenKind::Identifier(label) => {
                        self.bump();
                        Some(label)
                    }
                    _ => None,
                };
                self.expect_semicolon(what);
                match label {
                    Some(label) => self.labeled_jump(&label, is_break, span.clone()),
                    None if is_break => StmtKind::Break,
                    None => StmtKind::Continue,
                }
            }
            TokenKind::Return => {
                self.bump();
                let value = (!self.at(&TokenKind::Semicolon)).then(|| self.parse_expr());
                self.expect_semicolon("return");
                StmtKind::Return(value)
            }
            // `assert c : msg;` is `if (!c) { assert_fail("line N: " + msg); }`.
            TokenKind::Identifier(word)
                if word == "assert"
                    && !matches!(
                        self.peek_at(1),
                        TokenKind::Assign | TokenKind::Dot | TokenKind::Semicolon
                    ) =>
            {
                self.bump();
                let condition = self.parse_expr();
                let at = string_expr(&format!("line {}", span.line), &span);
                let message = if self.eat(&TokenKind::Colon) {
                    let message = self.parse_expr();
                    Expr {
                        kind: ExprKind::Binary {
                            op: BinaryOp::Add,
                            left: Box::new(string_expr(&format!("line {}: ", span.line), &span)),
                            right: Box::new(message),
                        },
                        span: span.clone(),
                    }
                } else {
                    at
                };
                self.expect_semicolon("assert");
                StmtKind::If {
                    condition: Expr {
                        kind: ExprKind::Unary {
                            op: UnaryOp::Not,
                            expr: Box::new(condition),
                        },
                        span: span.clone(),
                    },
                    then_body: vec![Stmt {
                        kind: StmtKind::Expr(call("assert_fail", vec![message], &span)),
                        span: span.clone(),
                    }],
                    else_body: Vec::new(),
                }
            }
            TokenKind::Identifier(word)
                if word == "throw"
                    && !matches!(
                        self.peek_at(1),
                        TokenKind::Assign
                            | TokenKind::Semicolon
                            | TokenKind::Dot
                            | TokenKind::LeftParen
                    ) =>
            {
                self.bump();
                let value = self.parse_expr();
                self.expect_semicolon("throw");
                StmtKind::Throw(value)
            }
            TokenKind::Identifier(word)
                if word == "try" && matches!(self.peek_at(1), TokenKind::LeftBrace) =>
            {
                self.bump();
                let body = self.parse_block("try body");
                let mut catches = Vec::new();
                while self.at_word("catch") {
                    let span = self.current_span();
                    self.bump();
                    self.expect(TokenKind::LeftParen, "expected '(' after 'catch'");
                    let mut types = vec![self.parse_type()];
                    while self.eat(&TokenKind::Pipe) {
                        types.push(self.parse_type());
                    }
                    let name = self.expect_identifier("expected the caught exception's name");
                    self.expect(
                        TokenKind::RightParen,
                        "expected ')' after the catch parameter",
                    );
                    let body = self.parse_block("catch body");
                    catches.push(Catch {
                        types,
                        name,
                        body,
                        span,
                    });
                }
                let finally = if self.eat_word("finally") {
                    self.parse_block("finally body")
                } else {
                    Vec::new()
                };
                if catches.is_empty() && finally.is_empty() {
                    self.error_at("a 'try' needs a 'catch' or a 'finally'", span.clone());
                }
                StmtKind::Try {
                    body,
                    catches,
                    finally,
                }
            }
            TokenKind::Identifier(word)
                if word == "switch" && matches!(self.peek_at(1), TokenKind::LeftParen) =>
            {
                self.bump();
                self.parse_switch_rest(span.clone())
            }
            TokenKind::Identifier(word)
                if (word == "mc" || word == "mcf")
                    && matches!(self.peek_at(1), TokenKind::LeftParen) =>
            {
                self.bump();
                self.bump();
                let command = self.expect_string(&format!("{word}(...) takes a string literal"));
                self.expect(
                    TokenKind::RightParen,
                    &format!("expected ')' after {word}(...)"),
                );
                self.expect_semicolon("command");
                if word == "mc" {
                    StmtKind::RawCommand(command)
                } else {
                    StmtKind::MacroCommand(command)
                }
            }
            _ => {
                let kind = self.parse_simple_stmt();
                if let StmtKind::Expr(Expr {
                    kind: ExprKind::Call { function, args },
                    span: call_span,
                }) = &kind
                    && (function == "as" || function == "at")
                    && self.at(&TokenKind::LeftBrace)
                {
                    let context = if function == "as" {
                        ContextKind::As
                    } else {
                        ContextKind::At
                    };
                    if args.len() != 1 {
                        self.error_at(
                            &format!("a {function} block takes exactly one anchor"),
                            call_span.clone(),
                        );
                    }
                    let anchor = args
                        .first()
                        .cloned()
                        .unwrap_or_else(|| int_expr(0, call_span));
                    let body = self.parse_block(&format!("{function} block"));
                    return Stmt {
                        kind: StmtKind::Context {
                            kind: context,
                            anchor,
                            body,
                        },
                        span,
                    };
                }
                self.expect_semicolon("statement");
                kind
            }
        };
        Stmt { kind, span }
    }

    /// A declaration, assignment, `i++`, or expression, without the trailing `;`.
    fn parse_simple_stmt(&mut self) -> StmtKind {
        let is_final = self.eat_word("final");
        if let Some(after_type) = self.scan_type(self.index)
            && matches!(self.tokens[after_type].kind, TokenKind::Identifier(_))
            && matches!(
                self.tokens.get(after_type + 1).map(|t| &t.kind),
                Some(TokenKind::Assign | TokenKind::Semicolon)
            )
        {
            let ty = self.parse_declared_type();
            let name = self.expect_identifier("expected variable name");
            if !self.eat(&TokenKind::Assign) {
                self.error_here("variables must be initialized, like 'int x = 0;'");
                return StmtKind::Expr(int_expr(0, &self.current_span()));
            }
            let value = self.parse_expr();
            if is_final && let Some(scope) = self.final_scopes.last_mut() {
                scope.push(name.clone());
            }
            return StmtKind::Let { name, ty, value };
        }
        if is_final {
            self.error_here("'final' goes on a variable declaration, like 'final int x = 0;'");
        }

        let expr = self.parse_expr();
        let shift_assign = match self.adjacent_pair() {
            Some((TokenKind::Lt, TokenKind::Lte)) => Some(BinaryOp::Shl),
            Some((TokenKind::Gt, TokenKind::Gte)) => Some(BinaryOp::Shr),
            _ => None,
        };
        if shift_assign.is_some() {
            self.bump();
        }
        let op = match self.peek().kind {
            _ if shift_assign.is_some() => shift_assign,
            TokenKind::Assign => None,
            TokenKind::AmpAssign => Some(BinaryOp::BitAnd),
            TokenKind::PipeAssign => Some(BinaryOp::BitOr),
            TokenKind::CaretAssign => Some(BinaryOp::BitXor),
            TokenKind::PlusAssign | TokenKind::PlusPlus => Some(BinaryOp::Add),
            TokenKind::MinusAssign | TokenKind::MinusMinus => Some(BinaryOp::Sub),
            TokenKind::StarAssign => Some(BinaryOp::Mul),
            TokenKind::SlashAssign => Some(BinaryOp::Div),
            TokenKind::PercentAssign => Some(BinaryOp::Rem),
            _ => return StmtKind::Expr(expr),
        };
        let token = self.bump();
        let value = match token.kind {
            TokenKind::PlusPlus | TokenKind::MinusMinus => int_expr(1, &token.span),
            _ => self.parse_expr(),
        };
        let value = match op {
            Some(op) => Expr {
                kind: ExprKind::Binary {
                    op,
                    left: Box::new(expr.clone()),
                    right: Box::new(value),
                },
                span: token.span.clone(),
            },
            None => value,
        };
        if let ExprKind::Variable(name) = &expr.kind
            && self
                .final_scopes
                .iter()
                .flatten()
                .any(|final_name| final_name == name)
        {
            self.error_at(
                &format!("cannot assign to final variable '{name}'"),
                token.span.clone(),
            );
        }
        StmtKind::Assign {
            target: self.into_assign_target(expr),
            value,
        }
    }

    /// `var` gives `None`; anything else is a written-out type.
    fn parse_declared_type(&mut self) -> Option<Type> {
        if self.at_word("var") {
            self.bump();
            None
        } else {
            Some(self.parse_type())
        }
    }

    /// Where a type starting at `index` ends: `Name`, `a.b.Name`, `Name<T, U>`.
    fn scan_type(&self, mut index: usize) -> Option<usize> {
        let ident = |index: usize| {
            matches!(
                self.tokens.get(index).map(|t| &t.kind),
                Some(TokenKind::Identifier(_))
            )
        };
        let kind = |index: usize| self.tokens.get(index).map(|t| &t.kind);
        if !ident(index) {
            return None;
        }
        index += 1;
        while matches!(kind(index), Some(TokenKind::Dot)) && ident(index + 1) {
            index += 2;
        }
        if matches!(kind(index), Some(TokenKind::Lt)) {
            index += 1;
            loop {
                index = self.scan_type(index)?;
                if !matches!(kind(index), Some(TokenKind::Comma)) {
                    break;
                }
                index += 1;
            }
            if !matches!(kind(index), Some(TokenKind::Gt)) {
                return None;
            }
            index += 1;
        }
        Some(index)
    }

    fn parse_if_rest(&mut self) -> StmtKind {
        let condition = self.parse_paren_expr("if");
        let then_body = self.parse_body("if body");
        let else_body = if self.eat(&TokenKind::Else) {
            if self.at(&TokenKind::If) {
                vec![self.parse_stmt()]
            } else {
                self.parse_body("else body")
            }
        } else {
            Vec::new()
        };
        StmtKind::If {
            condition,
            then_body,
            else_body,
        }
    }

    /// `for (T x : items)` or `for (init; condition; update)`, which becomes
    /// `{ init; while (condition) { body } step { update } }`.
    fn parse_for_rest(&mut self, span: &Span) -> StmtKind {
        self.expect(TokenKind::LeftParen, "expected '(' after 'for'");
        if let Some(after_type) = self.scan_type(self.index)
            && matches!(self.tokens[after_type].kind, TokenKind::Identifier(_))
            && matches!(
                self.tokens.get(after_type + 1).map(|t| &t.kind),
                Some(TokenKind::Colon)
            )
        {
            let ty = self.parse_declared_type();
            let name = self.expect_identifier("expected loop variable name");
            self.bump();
            let iterable = self.parse_expr();
            self.expect(TokenKind::RightParen, "expected ')' after for-each header");
            let body = self.parse_body("for body");
            return StmtKind::For {
                name,
                ty,
                iterable,
                body,
            };
        }

        let simple = |parser: &mut Self, end: TokenKind| {
            (!parser.at(&end)).then(|| Stmt {
                span: parser.current_span(),
                kind: parser.parse_simple_stmt(),
            })
        };
        let init = simple(self, TokenKind::Semicolon);
        self.expect(TokenKind::Semicolon, "expected ';' after for initializer");
        let condition = if self.at(&TokenKind::Semicolon) {
            Expr {
                kind: ExprKind::Bool(true),
                span: span.clone(),
            }
        } else {
            self.parse_expr()
        };
        self.expect(TokenKind::Semicolon, "expected ';' after for condition");
        let step = simple(self, TokenKind::RightParen);
        self.expect(TokenKind::RightParen, "expected ')' after for header");
        let body = self.parse_body("for body");
        let mut block: Vec<Stmt> = init.into_iter().collect();
        block.push(Stmt {
            kind: StmtKind::While {
                condition,
                body,
                step: step.into_iter().collect(),
            },
            span: span.clone(),
        });
        StmtKind::Block(block)
    }

    /// `switch (v) { case A, B -> stmt; case C -> { ... } default -> ...; }`
    fn parse_switch_rest(&mut self, span: Span) -> StmtKind {
        let value = self.parse_paren_expr("switch");
        self.expect(TokenKind::LeftBrace, "expected '{' after switch value");
        let mut arms = Vec::new();
        let mut default_body = None;
        while !self.at(&TokenKind::RightBrace) && !self.at(&TokenKind::Eof) {
            let start = self.index;
            if self.eat_word("case") {
                let mut patterns = vec![self.parse_case_pattern()];
                while self.eat(&TokenKind::Comma) {
                    patterns.push(self.parse_case_pattern());
                }
                self.expect_case_arrow();
                let body = self.parse_body("case body");
                arms.extend(patterns.into_iter().map(|pattern| SwitchArm {
                    pattern,
                    body: body.clone(),
                }));
            } else if self.eat_word("default") {
                self.expect_case_arrow();
                let body = self.parse_body("default body");
                if default_body.replace(body).is_some() {
                    self.error_here("duplicate default arm");
                }
            } else {
                self.error_here("expected 'case' or 'default' in switch");
                self.recover_statement();
            }
            if self.index == start {
                self.bump();
            }
        }
        self.expect(TokenKind::RightBrace, "expected '}' after switch body");
        if arms.is_empty() && default_body.is_none() {
            self.diagnostics
                .push(Diagnostic::new("switch requires at least one arm", span));
        }
        StmtKind::Switch {
            value,
            arms,
            default_body: default_body.unwrap_or_default(),
        }
    }

    /// A `case` label: a constant, or a type pattern like `Circle c`, which is
    /// an `instanceof` of an empty variable.
    fn parse_case_pattern(&mut self) -> Expr {
        let saved = std::mem::replace(&mut self.in_case_label, true);
        let pattern = self.parse_case_pattern_inner();
        self.in_case_label = saved;
        pattern
    }

    fn parse_case_pattern_inner(&mut self) -> Expr {
        if let (TokenKind::Identifier(_), TokenKind::Identifier(binding)) =
            (self.peek().kind.clone(), self.peek_at(1).clone())
        {
            let span = self.current_span();
            let ty = self.parse_type();
            self.bump();
            return Expr {
                kind: ExprKind::InstanceOf {
                    expr: Box::new(Expr {
                        kind: ExprKind::Variable(String::new()),
                        span: span.clone(),
                    }),
                    ty,
                    binding: Some(binding),
                },
                span,
            };
        }
        self.parse_expr()
    }

    fn expect_case_arrow(&mut self) {
        if self.at(&TokenKind::Colon) {
            self.error_here("use 'case X ->'; switch cases don't fall through");
            self.bump();
        } else {
            self.expect(TokenKind::Arrow, "expected '->' after case label");
        }
    }

    fn parse_paren_expr(&mut self, keyword: &str) -> Expr {
        self.expect(
            TokenKind::LeftParen,
            &format!("expected '(' after '{keyword}'"),
        );
        let expr = self.parse_expr();
        self.expect(
            TokenKind::RightParen,
            &format!("expected ')' after {keyword} condition"),
        );
        expr
    }

    /// An expression, including `c ? a : b`, which binds looser than `||`.
    fn parse_expr(&mut self) -> Expr {
        let condition = self.parse_expr_bp(0);
        if !self.at(&TokenKind::Question) {
            return condition;
        }
        let span = self.bump().span;
        let then_expr = self.parse_expr();
        self.expect(TokenKind::Colon, "expected ':' in 'a ? b : c'");
        let else_expr = self.parse_expr();
        Expr {
            kind: ExprKind::Conditional {
                condition: Box::new(condition),
                then_expr: Box::new(then_expr),
                else_expr: Box::new(else_expr),
            },
            span,
        }
    }

    /// `switch (v) { case A, B -> a; default -> b; }` as an expression.
    fn parse_switch_expr(&mut self, span: Span) -> Expr {
        let value = self.parse_paren_expr("switch");
        self.expect(TokenKind::LeftBrace, "expected '{' after switch value");
        let mut arms = Vec::new();
        let mut default = None;
        while !self.at(&TokenKind::RightBrace) && !self.at(&TokenKind::Eof) {
            let start = self.index;
            let patterns = if self.eat_word("case") {
                let mut patterns = vec![self.parse_case_pattern()];
                while self.eat(&TokenKind::Comma) {
                    patterns.push(self.parse_case_pattern());
                }
                Some(patterns)
            } else if self.eat_word("default") {
                None
            } else {
                self.error_here("expected 'case' or 'default' in switch");
                self.recover_statement();
                if self.index == start {
                    self.bump();
                }
                continue;
            };
            self.expect_case_arrow();
            let result = if self.at(&TokenKind::LeftBrace) {
                self.parse_yield_block()
            } else {
                let result = self.parse_expr();
                self.expect_semicolon("case value");
                result
            };
            match patterns {
                Some(patterns) => arms.extend(
                    patterns
                        .into_iter()
                        .map(|pattern| (pattern, result.clone())),
                ),
                None => {
                    if default.replace(Box::new(result)).is_some() {
                        self.error_here("duplicate default arm");
                    }
                }
            }
            if self.index == start {
                self.bump();
            }
        }
        self.expect(TokenKind::RightBrace, "expected '}' after switch body");
        Expr {
            kind: ExprKind::Switch {
                value: Box::new(value),
                arms,
                default,
            },
            span,
        }
    }

    /// `{ yield e; }`. A case that needs more statements belongs in a switch statement.
    fn parse_yield_block(&mut self) -> Expr {
        let span = self.bump().span;
        let result = if self.eat_word("yield") {
            let result = self.parse_expr();
            self.expect_semicolon("yield");
            Some(result)
        } else {
            None
        };
        if result.is_none() || !self.at(&TokenKind::RightBrace) {
            self.error_at(
                "a switch expression case block can only hold 'yield value;'; use a switch statement for more",
                span.clone(),
            );
            self.skip_balanced_until_close();
        }
        self.expect(TokenKind::RightBrace, "expected '}' after yield");
        result.unwrap_or_else(|| int_expr(0, &span))
    }

    fn parse_expr_bp(&mut self, min_bp: u8) -> Expr {
        let mut left = self.parse_prefix();
        loop {
            // `x instanceof Circle c` binds like `<`.
            if self.at_word("instanceof") && INSTANCEOF_BP >= min_bp {
                let span = self.bump().span;
                let ty = self.parse_type();
                let binding = match self.peek().kind.clone() {
                    TokenKind::Identifier(name) if name != "instanceof" => {
                        self.bump();
                        Some(name)
                    }
                    _ => None,
                };
                left = Expr {
                    kind: ExprKind::InstanceOf {
                        expr: Box::new(left),
                        ty,
                        binding,
                    },
                    span,
                };
                continue;
            }
            let Some((op, left_bp, right_bp)) = self.infix_operator() else {
                break;
            };
            if left_bp < min_bp {
                break;
            }
            let span = self.bump().span;
            let mut unsigned = false;
            if matches!(op, BinaryOp::Shl | BinaryOp::Shr) {
                let second = self.bump();
                // `>>>` is a third adjacent `>`.
                unsigned = op == BinaryOp::Shr
                    && self.at(&TokenKind::Gt)
                    && self.peek().range.start == second.range.end;
                if unsigned {
                    self.bump();
                }
            }
            let right = self.parse_expr_bp(right_bp);
            if unsigned {
                left = self.unsigned_shift(left, right, span);
                continue;
            }
            left = Expr {
                kind: ExprKind::Binary {
                    op,
                    left: Box::new(left),
                    right: Box::new(right),
                },
                span,
            };
        }
        left
    }

    /// `x >>> n` is `(x >> n) & mask`, clearing the bits the sign filled in.
    /// ponytail: needs a literal `n`; a variable count needs a runtime mask.
    fn unsigned_shift(&mut self, value: Expr, count: Expr, span: Span) -> Expr {
        let ExprKind::Int(bits) = count.kind else {
            self.error_at(
                "'>>>' needs a number of bits written as a literal, like 'x >>> 4'",
                span.clone(),
            );
            return value;
        };
        if !(0..32).contains(&bits) {
            self.error_at("'>>>' shifts by 0 to 31 bits", span.clone());
            return value;
        }
        if bits == 0 {
            return value;
        }
        let at = |kind| Expr {
            kind,
            span: span.clone(),
        };
        let shifted = at(ExprKind::Binary {
            op: BinaryOp::Shr,
            left: Box::new(value),
            right: Box::new(count),
        });
        at(ExprKind::Binary {
            op: BinaryOp::BitAnd,
            left: Box::new(shifted),
            right: Box::new(at(ExprKind::Int((1i64 << (32 - bits)) - 1))),
        })
    }

    /// `<<` and `>>` are two adjacent `<` or `>` tokens, so `List<List<int>>`
    /// still closes two generic lists.
    fn infix_operator(&self) -> Option<(BinaryOp, u8, u8)> {
        match self.adjacent_pair() {
            Some((TokenKind::Lt, TokenKind::Lt)) => Some((BinaryOp::Shl, 15, 16)),
            Some((TokenKind::Gt, TokenKind::Gt)) => Some((BinaryOp::Shr, 15, 16)),
            // `<<=` and `>>=` end the expression; the statement parser takes them.
            Some((TokenKind::Lt, TokenKind::Lte) | (TokenKind::Gt, TokenKind::Gte)) => None,
            _ => infix_binding_power(&self.peek().kind),
        }
    }

    fn adjacent_pair(&self) -> Option<(TokenKind, TokenKind)> {
        let next = self.tokens.get(self.index + 1)?;
        (self.peek().range.end == next.range.start)
            .then(|| (self.peek().kind.clone(), next.kind.clone()))
    }

    fn parse_prefix(&mut self) -> Expr {
        let op = match self.peek().kind {
            TokenKind::Bang => Some(UnaryOp::Not),
            TokenKind::Minus => Some(UnaryOp::Neg),
            TokenKind::Tilde => Some(UnaryOp::BitNot),
            _ => None,
        };
        if let Some(op) = op {
            let token = self.bump();
            let expr = self.parse_expr_bp(PREFIX_BP);
            return Expr {
                kind: ExprKind::Unary {
                    op,
                    expr: Box::new(expr),
                },
                span: token.span,
            };
        }
        if let Some(builtin) = self.cast_at_cursor() {
            let span = self.bump().span;
            self.bump();
            self.bump();
            let expr = self.parse_expr_bp(PREFIX_BP);
            return call(builtin, vec![expr], &span);
        }
        if self.class_cast_at_cursor() {
            let span = self.bump().span;
            let ty = self.parse_type();
            self.bump();
            let expr = self.parse_expr_bp(PREFIX_BP);
            return Expr {
                kind: ExprKind::Cast {
                    ty,
                    expr: Box::new(expr),
                },
                span,
            };
        }
        self.parse_primary()
    }

    /// `(Circle) shape`: a capitalized type name in parentheses before an operand
    /// that can't follow a parenthesized value, so `(X) - 1` stays arithmetic.
    fn class_cast_at_cursor(&self) -> bool {
        let TokenKind::LeftParen = self.peek().kind else {
            return false;
        };
        let TokenKind::Identifier(name) = self.peek_at(1) else {
            return false;
        };
        name.starts_with(|c: char| c.is_ascii_uppercase())
            && !CASTS.iter().any(|(ty, _)| ty == name)
            && matches!(self.peek_at(2), TokenKind::RightParen)
            && matches!(
                self.peek_at(3),
                TokenKind::Identifier(_) | TokenKind::LeftParen | TokenKind::New
            )
    }

    /// `(int)` followed by the start of an operand.
    fn cast_at_cursor(&self) -> Option<&'static str> {
        let TokenKind::LeftParen = self.peek().kind else {
            return None;
        };
        let TokenKind::Identifier(name) = self.peek_at(1) else {
            return None;
        };
        let (_, builtin) = CASTS.iter().find(|(ty, _)| ty == name)?;
        let starts_operand = matches!(
            self.peek_at(3),
            TokenKind::Identifier(_)
                | TokenKind::Integer(_)
                | TokenKind::Float(_)
                | TokenKind::String(_)
                | TokenKind::True
                | TokenKind::False
                | TokenKind::LeftParen
                | TokenKind::New
                | TokenKind::Minus
                | TokenKind::Bang
        );
        (matches!(self.peek_at(2), TokenKind::RightParen) && starts_operand).then_some(builtin)
    }

    fn parse_primary(&mut self) -> Expr {
        let token = self.bump();
        let span = token.span.clone();
        let expr = match token.kind {
            TokenKind::Integer(value) => int_expr(value, &span),
            TokenKind::Float(value) => Expr {
                kind: ExprKind::Float(value),
                span,
            },
            TokenKind::True | TokenKind::False => Expr {
                kind: ExprKind::Bool(matches!(token.kind, TokenKind::True)),
                span,
            },
            TokenKind::String(value) => string_expr(&value, &span),
            TokenKind::Identifier(word)
                if word == "switch" && matches!(self.peek().kind, TokenKind::LeftParen) =>
            {
                return self.parse_switch_expr(span);
            }
            TokenKind::New => {
                let mut name = self.expect_identifier("expected a type after 'new'");
                while self.eat(&TokenKind::Dot) {
                    name.push_str("::");
                    name.push_str(&self.expect_identifier("expected name after '.'"));
                }
                let type_args = self.at(&TokenKind::Lt).then(|| self.parse_type_args());
                self.expect(TokenKind::LeftParen, "expected '(' after the type in 'new'");
                let args = self.parse_call_args();
                match BUILTIN_CONSTRUCTORS.iter().find(|(ty, _)| *ty == name) {
                    // Marked, so a method named like the builtin can't take the call.
                    Some((_, builtin)) => call(&format!("@new:{builtin}"), args, &span),
                    None => Expr {
                        kind: ExprKind::New {
                            name,
                            args,
                            type_args,
                        },
                        span,
                    },
                }
            }
            TokenKind::Identifier(name)
                if (name == "List"
                    || name == "Map"
                    || STATIC_FACTORIES.iter().any(|(ty, _)| *ty == name))
                    && matches!(self.peek().kind, TokenKind::Dot)
                    && matches!(self.peek_at(1), TokenKind::Identifier(of) if of == "of") =>
            {
                self.bump();
                self.bump();
                self.expect(
                    TokenKind::LeftParen,
                    &format!("expected '(' after {name}.of"),
                );
                let args = self.parse_call_args();
                if let Some((_, builtin)) = STATIC_FACTORIES.iter().find(|(ty, _)| *ty == name) {
                    call(builtin, args, &span)
                } else if name == "List" {
                    Expr {
                        kind: ExprKind::ArrayLiteral(args),
                        span,
                    }
                } else {
                    self.map_literal(args, span)
                }
            }
            TokenKind::Identifier(name)
                if matches!(self.peek().kind, TokenKind::Dot)
                    && STATIC_METHODS.iter().any(|(class, _)| *class == name) =>
            {
                let (_, methods) = STATIC_METHODS
                    .iter()
                    .find(|(class, _)| *class == name)
                    .unwrap();
                self.bump();
                let method = match &self.peek().kind {
                    TokenKind::Identifier(method) => method.clone(),
                    _ => String::new(),
                };
                self.bump();
                let builtin = methods
                    .iter()
                    .find(|(java, _)| *java == method)
                    .map(|(_, builtin)| *builtin);
                if builtin.is_none() {
                    let names: Vec<&str> = methods.iter().map(|(java, _)| *java).collect();
                    self.error_at(&format!("{name} has {}", names.join(", ")), span.clone());
                }
                self.expect(
                    TokenKind::LeftParen,
                    &format!("expected '(' after {name}.{method}"),
                );
                let args = self.parse_call_args();
                call(builtin.unwrap_or(methods[0].1), args, &span)
            }
            TokenKind::Identifier(name)
                if !self.in_case_label && matches!(self.peek().kind, TokenKind::Arrow) =>
            {
                self.bump();
                return self.parse_lambda_body(vec![(name, None)], span);
            }
            TokenKind::LeftParen if !self.in_case_label && self.lambda_params_ahead() => {
                let mut params = Vec::new();
                while !self.at(&TokenKind::RightParen) && !self.at(&TokenKind::Eof) {
                    let untyped =
                        matches!(self.peek_at(1), TokenKind::Comma | TokenKind::RightParen);
                    let ty = (!untyped).then(|| self.parse_type());
                    params.push((self.expect_identifier("expected a parameter name"), ty));
                    if !self.eat(&TokenKind::Comma) {
                        break;
                    }
                }
                self.expect(
                    TokenKind::RightParen,
                    "expected ')' after lambda parameters",
                );
                self.expect(TokenKind::Arrow, "expected '->' after lambda parameters");
                return self.parse_lambda_body(params, span);
            }
            TokenKind::Identifier(mut name) => {
                // `util::twice(x)` is a resolved full name; the resolver writes
                // these into `$(...)` placeholders, which are parsed again later.
                while matches!(self.peek().kind, TokenKind::Colon)
                    && matches!(self.peek_at(1), TokenKind::Colon)
                    && let TokenKind::Identifier(next) = self.peek_at(2).clone()
                {
                    self.bump();
                    self.bump();
                    self.bump();
                    name = format!("{name}::{next}");
                }
                // `Type::new`, or `Type::method` when no call follows.
                if matches!(self.peek().kind, TokenKind::Colon)
                    && matches!(self.peek_at(1), TokenKind::Colon)
                    && matches!(self.peek_at(2), TokenKind::New)
                {
                    self.bump();
                    self.bump();
                    self.bump();
                    name.push_str("::new");
                }
                if let Some((target, method)) = name.rsplit_once("::")
                    && !self.at(&TokenKind::LeftParen)
                {
                    return Expr {
                        kind: ExprKind::MethodRef {
                            target: target.to_string(),
                            method: method.to_string(),
                        },
                        span,
                    };
                }
                if self.eat(&TokenKind::LeftParen) {
                    if let Some((ty, _)) = CASTS.iter().find(|(_, builtin)| *builtin == name) {
                        self.error_at(&format!("use a cast: '({ty}) value'"), span.clone());
                    }
                    if let Some((ty, _)) = BUILTIN_CONSTRUCTORS
                        .iter()
                        .find(|(_, builtin)| *builtin == name)
                    {
                        self.error_at(&format!("use 'new {ty}(...)'"), span.clone());
                    }
                    if let Some((ty, _)) = STATIC_FACTORIES
                        .iter()
                        .find(|(_, builtin)| *builtin == name)
                    {
                        self.error_at(&format!("use '{ty}.of(...)'"), span.clone());
                    }
                    if let Some(java) = java_name_for(&name, false) {
                        self.error_at(&format!("use '{java}(...)'"), span.clone());
                    }
                    if let Some(method) = match name.as_str() {
                        "single" => Some("selector.getFirst()"),
                        "findFirst" | "find_first" => Some("selector.findFirst()"),
                        "exists" => Some("entity.isValid()"),
                        _ => None,
                    } {
                        self.error_at(&format!("use '{method}'"), span.clone());
                    }
                    let args = self.parse_call_args();
                    Expr {
                        kind: ExprKind::Call {
                            function: internal_function_name(&name).to_string(),
                            args,
                        },
                        span,
                    }
                } else {
                    Expr {
                        kind: ExprKind::Variable(name),
                        span,
                    }
                }
            }
            TokenKind::LeftParen => {
                let expr = self.parse_expr();
                self.expect(TokenKind::RightParen, "expected ')' after expression");
                expr
            }
            _ => {
                self.diagnostics
                    .push(Diagnostic::new("expected expression", span.clone()));
                self.index -= usize::from(!matches!(token.kind, TokenKind::Eof));
                self.recover_expression();
                int_expr(0, &span)
            }
        };
        self.parse_postfix(expr)
    }

    /// After a `(`: a lambda's parameter list, `(a, b) ->` or `(int a) ->`.
    fn lambda_params_ahead(&self) -> bool {
        let mut depth = 1;
        let mut index = self.index;
        while let Some(token) = self.tokens.get(index) {
            match token.kind {
                TokenKind::LeftParen => depth += 1,
                TokenKind::RightParen => {
                    depth -= 1;
                    if depth == 0 {
                        return matches!(
                            self.tokens.get(index + 1).map(|t| &t.kind),
                            Some(TokenKind::Arrow)
                        );
                    }
                }
                TokenKind::Eof | TokenKind::Semicolon | TokenKind::LeftBrace => return false,
                _ => {}
            }
            index += 1;
        }
        false
    }

    /// What follows a lambda's `->`: a block, or an expression it returns.
    fn parse_lambda_body(&mut self, params: Vec<(String, Option<Type>)>, span: Span) -> Expr {
        let saved = std::mem::replace(&mut self.in_case_label, false);
        let (body, expression) = if self.at(&TokenKind::LeftBrace) {
            (self.parse_block("lambda body"), false)
        } else if let TokenKind::Identifier(word) = self.peek().kind.clone()
            && (word == "mc" || word == "mcf")
            && matches!(self.peek_at(1), TokenKind::LeftParen)
        {
            // `x -> mcf("say $(x)")` runs the command.
            let start = self.current_span();
            self.bump();
            self.bump();
            let command = self.expect_string(&format!("{word}(...) takes a string literal"));
            self.expect(
                TokenKind::RightParen,
                &format!("expected ')' after {word}(...)"),
            );
            let kind = if word == "mc" {
                StmtKind::RawCommand(command)
            } else {
                StmtKind::MacroCommand(command)
            };
            (vec![Stmt { kind, span: start }], false)
        } else {
            // An expression, or an assignment such as `() -> count += 1`.
            let start = self.current_span();
            match self.parse_simple_stmt() {
                StmtKind::Expr(value) => {
                    let stmt = Stmt {
                        span: value.span.clone(),
                        kind: StmtKind::Return(Some(value)),
                    };
                    (vec![stmt], true)
                }
                kind => (vec![Stmt { kind, span: start }], false),
            }
        };
        self.in_case_label = saved;
        Expr {
            kind: ExprKind::Lambda {
                params,
                body,
                expression,
            },
            span,
        }
    }

    /// `Map.of("a", 1, "b", 2)`: keys are string literals.
    fn map_literal(&mut self, args: Vec<Expr>, span: Span) -> Expr {
        if !args.len().is_multiple_of(2) {
            self.error_at("Map.of takes key, value pairs", span.clone());
        }
        let mut entries = Vec::new();
        let mut args = args.into_iter();
        while let (Some(key), Some(value)) = (args.next(), args.next()) {
            match key.kind {
                ExprKind::String(key) => entries.push((key, value)),
                _ => self.error_at("Map.of keys must be string literals", key.span),
            }
        }
        Expr {
            kind: ExprKind::DictLiteral(entries),
            span,
        }
    }

    fn parse_postfix(&mut self, mut expr: Expr) -> Expr {
        loop {
            if self.at(&TokenKind::LeftParen) {
                let span = self.current_span();
                let ExprKind::Path(path) = &expr.kind else {
                    break;
                };
                let mut path = path.clone();
                let Some(PathSegment::Field(method)) = path.segments.pop() else {
                    self.error_here("only member access may be called like a method");
                    break;
                };
                let receiver = if path.segments.is_empty() {
                    *path.base
                } else {
                    Expr {
                        kind: ExprKind::Path(path),
                        span: expr.span.clone(),
                    }
                };
                self.bump();
                if let ExprKind::Variable(class) = &receiver.kind
                    && STATIC_CLASSES.contains(&class.as_str())
                {
                    let args = self.parse_call_args();
                    expr = self.static_call(class.clone(), &method, args, span);
                    continue;
                }
                let args = self.parse_call_args();
                // Module calls such as `vec.add(a, b)` are mapped back in modules.rs.
                // A builtin's internal name written in source, like `push`, is
                // marked: a class may have a method `push`, a List may not.
                let method = if java_name_for(&method, true).is_some() {
                    format!("{WRITTEN_METHOD}{method}")
                } else {
                    internal_method_name(&method, args.len()).to_string()
                };
                expr = Expr {
                    kind: ExprKind::MethodCall {
                        receiver: Box::new(receiver),
                        method,
                        args,
                    },
                    span,
                };
            } else if self.eat(&TokenKind::Dot) {
                let span = self.current_span();
                let field = self.expect_identifier("expected field name after '.'");
                // `NamedTextColor.RED` is "red", `TextDecoration.BOLD` is "bold".
                if let ExprKind::Variable(class) = &expr.kind
                    && let Some(names) = match class.as_str() {
                        "NamedTextColor" => Some(crate::minimessage::NAMED_COLORS),
                        "TextDecoration" => Some(TEXT_DECORATIONS),
                        _ => None,
                    }
                {
                    let name = field.to_lowercase();
                    if !names.contains(&name.as_str()) {
                        self.error_at(&format!("unknown constant '{class}.{field}'"), span.clone());
                    }
                    expr = string_expr(&name, &span);
                    continue;
                }
                expr = append_path_segment(expr, PathSegment::Field(field), span);
            } else if self.eat(&TokenKind::LeftBracket) {
                let span = self.current_span();
                let index = self.parse_expr();
                self.expect(TokenKind::RightBracket, "expected ']' after index");
                expr = append_path_segment(expr, PathSegment::Index(Box::new(index)), span);
            } else {
                break;
            }
        }
        expr
    }

    /// `String.format("%s has %d", name, kills)` is `"" + name + " has " + kills`.
    /// ponytail: `%s`, `%d` and `%%` only, and the format is a literal; widths
    /// and precision need runtime padding.
    fn format_call(&mut self, args: Vec<Expr>, span: Span) -> Expr {
        let mut args = args.into_iter();
        let Some(Expr {
            kind: ExprKind::String(format),
            ..
        }) = args.next()
        else {
            self.error_at(
                "String.format(...) needs a string literal first",
                span.clone(),
            );
            return string_expr("", &span);
        };
        let mut out = string_expr("", &span);
        let mut text = String::new();
        let mut chars = format.chars();
        while let Some(c) = chars.next() {
            if c != '%' {
                text.push(c);
                continue;
            }
            match chars.next() {
                Some('%') => text.push('%'),
                Some('s' | 'd') => {
                    let Some(arg) = args.next() else {
                        self.error_at(
                            "String.format(...) has more '%' fields than values",
                            span.clone(),
                        );
                        break;
                    };
                    for part in [string_expr(&std::mem::take(&mut text), &span), arg] {
                        out = Expr {
                            kind: ExprKind::Binary {
                                op: BinaryOp::Add,
                                left: Box::new(out),
                                right: Box::new(part),
                            },
                            span: span.clone(),
                        };
                    }
                }
                other => {
                    let field = other.map(String::from).unwrap_or_default();
                    self.error_at(
                        &format!("String.format(...) supports '%s', '%d' and '%%', not '%{field}'"),
                        span.clone(),
                    );
                }
            }
        }
        if args.next().is_some() {
            self.error_at(
                "String.format(...) has more values than '%' fields",
                span.clone(),
            );
        }
        Expr {
            kind: ExprKind::Binary {
                op: BinaryOp::Add,
                left: Box::new(out),
                right: Box::new(string_expr(&text, &span)),
            },
            span,
        }
    }

    /// `Math.pow(a, b)` becomes the method `"Math.pow"` on `a`, `String.valueOf(x)`
    /// becomes `"" + x`, and `String.join(sep, parts)` calls `std.str.join`. The
    /// dotted names can't be written as methods in source.
    fn static_call(
        &mut self,
        class: String,
        method: &str,
        mut args: Vec<Expr>,
        span: Span,
    ) -> Expr {
        if let Some(expr) = self.adventure_call(&class, method, &mut args, &span) {
            return expr;
        }
        let known = match class.as_str() {
            "Math" => MATH_METHODS.contains(&method) || MATH_STD_FUNCTIONS.contains(&method),
            "Integer" => matches!(method, "parseInt" | "toString"),
            "Float" => method == "toString",
            "String" => matches!(method, "valueOf" | "join" | "format"),
            _ => false,
        };
        if !known {
            self.error_at(&format!("unknown method '{class}.{method}'"), span.clone());
        }
        if class == "String" && method == "format" {
            return self.format_call(args, span);
        }
        if class == "String" && method == "join" {
            return Expr {
                kind: ExprKind::Call {
                    function: "std::str::join".to_string(),
                    args,
                },
                span,
            };
        }
        if class == "Math" && MATH_STD_FUNCTIONS.contains(&method) {
            return Expr {
                kind: ExprKind::Call {
                    function: format!("std::math::{method}"),
                    args,
                },
                span,
            };
        }
        if args.is_empty() {
            self.error_at(
                &format!("'{class}.{method}' needs an argument"),
                span.clone(),
            );
            return int_expr(0, &span);
        }
        let first = args.remove(0);
        if matches!(method, "toString" | "valueOf") {
            if !args.is_empty() {
                self.error_at(
                    &format!("'{class}.{method}' takes one argument"),
                    span.clone(),
                );
            }
            return Expr {
                kind: ExprKind::Binary {
                    op: BinaryOp::Add,
                    left: Box::new(string_expr("", &span)),
                    right: Box::new(first),
                },
                span,
            };
        }
        Expr {
            kind: ExprKind::MethodCall {
                receiver: Box::new(first),
                method: format!("{class}.{method}"),
                args,
            },
            span,
        }
    }

    /// Adventure's text API (`Component.text`, `ClickEvent.runCommand`,
    /// `MiniMessage.miniMessage().deserialize`) calls `std.text`.
    fn adventure_call(
        &mut self,
        class: &str,
        method: &str,
        args: &mut Vec<Expr>,
        span: &Span,
    ) -> Option<Expr> {
        if !matches!(
            class,
            "Component" | "ClickEvent" | "HoverEvent" | "TextColor" | "MiniMessage"
        ) {
            return None;
        }
        // Methods std declares on the builtin types, such as `Component.text(...)`.
        if class != "MiniMessage" {
            return Some(Expr {
                kind: ExprKind::Call {
                    function: format!("@native:{class}__{method}"),
                    args: std::mem::take(args),
                },
                span: span.clone(),
            });
        }
        let function = match (class, method, args.len()) {
            // `.deserialize(...)` on this comes back here as `MiniMessage.deserialize`.
            ("MiniMessage", "miniMessage", 0) => {
                return Some(Expr {
                    kind: ExprKind::Variable("MiniMessage".to_string()),
                    span: span.clone(),
                });
            }
            ("MiniMessage", "deserialize", 1) => match &args[0].kind {
                ExprKind::String(source) if source.contains("$(") => {
                    self.error_at(
                        "a MiniMessage literal can't use $(...); append the value with .append(Component.text(x))",
                        span.clone(),
                    );
                    "parseMiniMessage"
                }
                ExprKind::String(source) => {
                    return Some(call(
                        "text_snbt",
                        vec![string_expr(&crate::minimessage::to_snbt(source), span)],
                        span,
                    ));
                }
                _ => "parseMiniMessage",
            },
            _ => {
                self.error_at(
                    &format!(
                        "unknown method '{class}.{method}' with {} arguments",
                        args.len()
                    ),
                    span.clone(),
                );
                "empty"
            }
        };
        Some(Expr {
            kind: ExprKind::Call {
                function: format!("std::text::{function}"),
                args: std::mem::take(args),
            },
            span: span.clone(),
        })
    }

    fn parse_call_args(&mut self) -> Vec<Expr> {
        let mut args = Vec::new();
        while !self.at(&TokenKind::RightParen) && !self.at(&TokenKind::Eof) {
            args.push(self.parse_expr());
            if !self.eat(&TokenKind::Comma) {
                break;
            }
            if self.at(&TokenKind::RightParen) {
                self.error_here("expected expression");
            }
        }
        self.expect(TokenKind::RightParen, "expected ')' after call arguments");
        args
    }

    fn parse_type(&mut self) -> Type {
        let span = self.current_span();
        let TokenKind::Identifier(name) = self.bump().kind else {
            self.error_at("expected type", span);
            return Type::Void;
        };
        let generic = |parser: &mut Self, count: usize| {
            parser.expect(TokenKind::Lt, &format!("expected '<' after '{name}'"));
            let mut args = vec![parser.parse_type_arg()];
            while args.len() < count && parser.eat(&TokenKind::Comma) {
                args.push(parser.parse_type_arg());
            }
            parser.expect(
                TokenKind::Gt,
                &format!("expected '>' after {name} type arguments"),
            );
            args
        };
        match name.as_str() {
            "int" | "Integer" => Type::Int,
            "float" | "Float" => Type::Float,
            "boolean" | "Boolean" => Type::Bool,
            "String" => Type::String,
            "void" => Type::Void,
            // Scores are 32-bit ints, so Java's other number types have no match.
            "long" | "short" | "byte" | "char" | "Long" | "Short" | "Byte" | "Character" => {
                self.error_at(
                    &format!("MCFC has no '{name}'; numbers are 'int' (32-bit) or 'float'"),
                    span,
                );
                Type::Int
            }
            "double" | "Double" => {
                self.error_at("MCFC has no 'double'; use 'float'", span);
                Type::Float
            }
            "Selector" => Type::EntitySet,
            "Entity" => Type::EntityRef,
            "Player" => Type::PlayerRef,
            "Block" => Type::BlockRef,
            "EntityData" => Type::EntityDef,
            "BlockData" => Type::BlockDef,
            "ItemStack" => Type::ItemDef,
            "Component" => Type::TextDef,
            "ItemSlot" => Type::ItemSlot,
            "BossBar" => Type::Bossbar,
            "Nbt" => Type::Nbt,
            "List" => Type::Array(Box::new(generic(self, 1).remove(0))),
            "Optional" => Type::Optional(Box::new(generic(self, 1).remove(0))),
            "Map" => {
                let mut args = generic(self, 2);
                if args.len() != 2 || args[0] != Type::String {
                    self.error_at("Map keys are always 'String': write Map<String, T>", span);
                }
                Type::Dict(Box::new(args.pop().unwrap_or(Type::Void)))
            }
            _ => {
                let mut path = name;
                while self.at(&TokenKind::Dot)
                    && matches!(self.peek_at(1), TokenKind::Identifier(_))
                {
                    self.bump();
                    path.push_str("::");
                    path.push_str(&self.expect_identifier("expected type name"));
                }
                if self.at(&TokenKind::Lt) {
                    Type::Generic(path, self.parse_type_args())
                } else {
                    Type::Struct(path)
                }
            }
        }
    }

    /// `<A, B>` after a class name; `<>` gives no types.
    fn parse_type_args(&mut self) -> Vec<Type> {
        self.expect(TokenKind::Lt, "expected '<'");
        let mut args = Vec::new();
        while !self.at(&TokenKind::Gt) && !self.at(&TokenKind::Eof) {
            args.push(self.parse_type_arg());
            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }
        self.expect(TokenKind::Gt, "expected '>' after type arguments");
        args
    }

    /// A type inside `<...>`: like Java, `List<Integer>`, not `List<int>`.
    fn parse_type_arg(&mut self) -> Type {
        let span = self.current_span();
        if let TokenKind::Identifier(name) = &self.peek().kind
            && let Some(boxed) = match name.as_str() {
                "int" => Some("Integer"),
                "float" => Some("Float"),
                "boolean" => Some("Boolean"),
                _ => None,
            }
        {
            let message = format!("use '{boxed}' inside '<...>', not '{name}'");
            self.error_at(&message, span);
        }
        self.parse_type()
    }

    fn into_assign_target(&mut self, expr: Expr) -> AssignTarget {
        match expr.kind {
            ExprKind::Variable(name) => AssignTarget::Variable(name),
            ExprKind::Path(path) => AssignTarget::Path(path),
            _ => {
                self.error_at("invalid assignment target", expr.span);
                AssignTarget::Variable("_error".to_string())
            }
        }
    }

    fn expect_semicolon(&mut self, after: &str) {
        if !self.eat(&TokenKind::Semicolon) {
            let span = self.tokens[self.index.saturating_sub(1)].span.clone();
            self.error_at(&format!("expected ';' after {after}"), span);
            self.recover_statement();
        }
    }

    fn expect_identifier(&mut self, message: &str) -> String {
        if let TokenKind::Identifier(name) = &self.peek().kind {
            let name = name.clone();
            self.bump();
            name
        } else {
            self.error_here(message);
            "_error".to_string()
        }
    }

    fn expect_string(&mut self, message: &str) -> String {
        if let TokenKind::String(value) = &self.peek().kind {
            let value = value.clone();
            self.bump();
            value
        } else {
            self.error_here(message);
            String::new()
        }
    }

    fn expect(&mut self, expected: TokenKind, message: &str) -> Token {
        if self.at(&expected) {
            self.bump()
        } else {
            self.error_here(message);
            Token {
                span: self.current_span(),
                range: self.peek().range,
                kind: expected,
            }
        }
    }

    fn eat(&mut self, expected: &TokenKind) -> bool {
        let found = self.at(expected);
        if found {
            self.bump();
        }
        found
    }

    fn at(&self, expected: &TokenKind) -> bool {
        std::mem::discriminant(&self.peek().kind) == std::mem::discriminant(expected)
    }

    fn at_word(&self, word: &str) -> bool {
        matches!(&self.peek().kind, TokenKind::Identifier(name) if name == word)
    }

    fn eat_word(&mut self, word: &str) -> bool {
        let found = self.at_word(word);
        if found {
            self.bump();
        }
        found
    }

    fn peek(&self) -> &Token {
        &self.tokens[self.index]
    }

    fn peek_at(&self, offset: usize) -> &TokenKind {
        let last = self.tokens.len() - 1;
        &self.tokens[(self.index + offset).min(last)].kind
    }

    fn bump(&mut self) -> Token {
        let token = self.peek().clone();
        if !matches!(token.kind, TokenKind::Eof) {
            self.index += 1;
        }
        token
    }

    fn previous_end(&self) -> usize {
        self.tokens[self.index.saturating_sub(1)].range.end
    }

    fn current_span(&self) -> Span {
        self.peek().span.clone()
    }

    fn error_here(&mut self, message: &str) {
        self.diagnostics
            .push(Diagnostic::new(message, self.current_span()));
    }

    fn error_at(&mut self, message: &str, span: Span) {
        self.diagnostics.push(Diagnostic::new(message, span));
    }

    /// Skips to just past the next `;` or balanced `{ }` at depth 0.
    fn recover_top_level(&mut self) {
        let mut depth = 0usize;
        while !self.at(&TokenKind::Eof) {
            match self.bump().kind {
                TokenKind::LeftBrace => depth += 1,
                TokenKind::RightBrace => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return;
                    }
                }
                TokenKind::Semicolon if depth == 0 => return,
                _ => {}
            }
        }
    }

    /// Skips to just past the next `;`, or up to a `}` that closes the block.
    fn recover_statement(&mut self) {
        let mut depth = 0usize;
        while !self.at(&TokenKind::Eof) {
            match self.peek().kind {
                TokenKind::Semicolon if depth == 0 => {
                    self.bump();
                    return;
                }
                TokenKind::RightBrace if depth == 0 => return,
                TokenKind::LeftBrace => depth += 1,
                TokenKind::RightBrace => depth -= 1,
                _ => {}
            }
            self.bump();
        }
    }

    fn recover_expression(&mut self) {
        while !matches!(
            self.peek().kind,
            TokenKind::Eof
                | TokenKind::Comma
                | TokenKind::Semicolon
                | TokenKind::RightParen
                | TokenKind::RightBracket
                | TokenKind::RightBrace
                | TokenKind::LeftBrace
        ) {
            self.bump();
        }
    }

    fn skip_balanced_until_close(&mut self) {
        let mut depth = 0usize;
        while !self.at(&TokenKind::Eof) {
            match self.peek().kind {
                TokenKind::RightBrace if depth == 0 => return,
                TokenKind::LeftBrace => depth += 1,
                TokenKind::RightBrace => depth -= 1,
                _ => {}
            }
            self.bump();
        }
    }
}

const PREFIX_BP: u8 = 21;
const INSTANCEOF_BP: u8 = 13;

/// Java precedence; shifts (15, 16) are matched in `infix_operator`.
fn infix_binding_power(kind: &TokenKind) -> Option<(BinaryOp, u8, u8)> {
    Some(match kind {
        TokenKind::OrOr => (BinaryOp::Or, 1, 2),
        TokenKind::AndAnd => (BinaryOp::And, 3, 4),
        TokenKind::Pipe => (BinaryOp::BitOr, 5, 6),
        TokenKind::Caret => (BinaryOp::BitXor, 7, 8),
        TokenKind::Amp => (BinaryOp::BitAnd, 9, 10),
        TokenKind::EqEq => (BinaryOp::Eq, 11, 12),
        TokenKind::BangEq => (BinaryOp::NotEq, 11, 12),
        TokenKind::Lt => (BinaryOp::Lt, 13, 14),
        TokenKind::Lte => (BinaryOp::Lte, 13, 14),
        TokenKind::Gt => (BinaryOp::Gt, 13, 14),
        TokenKind::Gte => (BinaryOp::Gte, 13, 14),
        TokenKind::Plus => (BinaryOp::Add, 17, 18),
        TokenKind::Minus => (BinaryOp::Sub, 17, 18),
        TokenKind::Star => (BinaryOp::Mul, 19, 20),
        TokenKind::Slash => (BinaryOp::Div, 19, 20),
        TokenKind::Percent => (BinaryOp::Rem, 19, 20),
        _ => return None,
    })
}

fn append_path_segment(expr: Expr, segment: PathSegment, span: Span) -> Expr {
    let mut path = match expr.kind {
        ExprKind::Path(path) => path,
        _ => PathExpr {
            base: Box::new(expr),
            segments: Vec::new(),
        },
    };
    path.segments.push(segment);
    Expr {
        kind: ExprKind::Path(path),
        span,
    }
}

fn is_command_name(name: &str) -> bool {
    name.starts_with(|ch: char| ch.is_ascii_alphabetic() || ch == '_')
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

/// `onJoin` -> `on_join`: Minecraft resource paths and objectives are lowercase.
pub fn resource_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    for (index, ch) in name.char_indices() {
        if ch.is_ascii_uppercase() {
            let previous = name[..index].chars().last();
            if previous.is_some_and(|prev| prev.is_ascii_lowercase() || prev.is_ascii_digit()) {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

fn call(function: &str, args: Vec<Expr>, span: &Span) -> Expr {
    Expr {
        kind: ExprKind::Call {
            function: function.to_string(),
            args,
        },
        span: span.clone(),
    }
}

fn int_expr(value: i64, span: &Span) -> Expr {
    Expr {
        kind: ExprKind::Int(value),
        span: span.clone(),
    }
}

fn string_expr(value: &str, span: &Span) -> Expr {
    Expr {
        kind: ExprKind::String(value.to_string()),
        span: span.clone(),
    }
}

fn has_return(body: &[Stmt]) -> bool {
    body.iter().any(|stmt| match &stmt.kind {
        StmtKind::Return(_) => true,
        StmtKind::If {
            then_body,
            else_body,
            ..
        } => has_return(then_body) || has_return(else_body),
        StmtKind::While { body, step, .. } => has_return(body) || has_return(step),
        StmtKind::Switch {
            arms, default_body, ..
        } => arms.iter().any(|arm| has_return(&arm.body)) || has_return(default_body),
        StmtKind::For { body, .. }
        | StmtKind::Block(body)
        | StmtKind::Context { body, .. }
        | StmtKind::Async { body } => has_return(body),
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::{parse, resource_name};
    use crate::ast::{BinaryOp, ExprKind, StmtKind, Type, UnaryOp};

    #[test]
    fn parses_precedence_like_java() {
        let program = parse("void main() { var v = 1 + 2 * 3 < 10 && !false; }").unwrap();
        let StmtKind::Let {
            value, ty: None, ..
        } = &program.functions[0].body[0].kind
        else {
            panic!("expected var declaration");
        };
        let ExprKind::Binary {
            op: BinaryOp::And,
            left,
            right,
        } = &value.kind
        else {
            panic!("expected &&");
        };
        assert!(matches!(
            left.kind,
            ExprKind::Binary {
                op: BinaryOp::Lt,
                ..
            }
        ));
        assert!(matches!(
            right.kind,
            ExprKind::Unary {
                op: UnaryOp::Not,
                ..
            }
        ));
    }

    #[test]
    fn desugars_c_style_for_into_block_with_step() {
        let program =
            parse("void main() { for (int i = 0; i < 3; i++) { if (i == 1) continue; } }").unwrap();
        let StmtKind::Block(block) = &program.functions[0].body[0].kind else {
            panic!("expected block");
        };
        assert!(matches!(
            &block[0].kind,
            StmtKind::Let {
                ty: Some(Type::Int),
                ..
            }
        ));
        let StmtKind::While { step, .. } = &block[1].kind else {
            panic!("expected while");
        };
        assert!(matches!(step[0].kind, StmtKind::Assign { .. }));
    }

    #[test]
    fn parses_for_each_switch_and_casts() {
        let program = parse(
            r#"
void main() {
    for (Player p : Selector.of("@a")) { p.sendMessage("hi"); }
    switch (mode) {
        case A, B -> debug("ab");
        default -> { debug("other"); }
    }
    float f = (float) 3 * 2.0;
}
"#,
        )
        .unwrap();
        let body = &program.functions[0].body;
        assert!(matches!(
            &body[0].kind,
            StmtKind::For {
                ty: Some(Type::PlayerRef),
                ..
            }
        ));
        let StmtKind::Switch {
            arms, default_body, ..
        } = &body[1].kind
        else {
            panic!("expected switch");
        };
        assert_eq!(arms.len(), 2);
        assert_eq!(default_body.len(), 1);
        let StmtKind::Let { value, .. } = &body[2].kind else {
            panic!("expected declaration");
        };
        let ExprKind::Binary { left, .. } = &value.kind else {
            panic!("cast binds tighter than *");
        };
        assert!(matches!(&left.kind, ExprKind::Call { function, .. } if function == "float"));
    }

    #[test]
    fn annotations_rename_handlers_and_bind_player() {
        let program = parse(
            r#"
@PlayerState("Coins") int coins;
@EventHandler void onJoin(PlayerJoinEvent event) { Player player = event.player(); player.sendMessage("hi"); }
@Command("spawn") void spawn() {}
@Every(seconds = 2) void heartBeat() {}
"#,
        )
        .unwrap();
        assert_eq!(program.player_states[0].display_name, "Coins");
        let names: Vec<_> = program.functions.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "__mcfc_event_player_join",
                "__mcfc_command_spawn",
                "__mcfc_task_heart_beat_every_ticks_40"
            ]
        );
        assert!(program.functions[0].params.is_empty());
        assert!(matches!(
            program.functions[0].body[0].kind,
            StmtKind::Let { .. }
        ));
    }

    #[test]
    fn rejects_old_syntax_with_useful_errors() {
        let error = parse("void main() { var x = int(2.5); var s = item(\"a\") }").unwrap_err();
        let rendered = error.to_string();
        assert!(rendered.contains("use a cast: '(int) value'"));
        assert!(rendered.contains("use 'new ItemStack(...)'"));
        assert!(rendered.contains("expected ';' after statement"));
    }

    #[test]
    fn converts_names_to_resource_paths() {
        assert_eq!(resource_name("onPlayerJoin"), "on_player_join");
        assert_eq!(resource_name("already_snake"), "already_snake");
        assert_eq!(resource_name("HTTPThing"), "httpthing");
    }
}
