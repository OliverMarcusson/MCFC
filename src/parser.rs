use crate::ast::*;
use crate::diagnostics::{Diagnostic, Diagnostics, Span};
use crate::lexer::{Token, TokenKind, lex};

pub fn parse(source: &str) -> Result<Program, Diagnostics> {
    let tokens = lex(source)?;
    Parser::new(tokens).parse_program()
}

pub fn parse_expression(source: &str) -> Result<Expr, Diagnostics> {
    let tokens = lex(source)?;
    Parser::new(tokens).parse_expression_only()
}

struct Parser {
    tokens: TokenStream,
    diagnostics: Diagnostics,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self {
            tokens: TokenStream::new(tokens),
            diagnostics: Diagnostics::new(),
        }
    }

    fn parse_program(mut self) -> Result<Program, Diagnostics> {
        let mut structs = Vec::new();
        let mut player_states = Vec::new();
        let mut functions = Vec::new();
        self.skip_newlines();

        while !self.at(&TokenKind::Eof) {
            if self.at(&TokenKind::Struct) {
                structs.push(self.parse_struct());
            } else if self.at(&TokenKind::PlayerState) {
                player_states.push(self.parse_player_state());
            } else if self.at(&TokenKind::Fn) {
                functions.push(self.parse_function());
            } else if self.at(&TokenKind::End) {
                self.error_here("'end' is no longer used; close blocks with indentation");
                self.bump();
            } else {
                self.error_here("expected player_state, struct, or function definition");
                self.recover_top_level();
            }
            self.skip_newlines();
        }

        self.diagnostics.into_result(Program {
            structs,
            player_states,
            functions,
        })
    }

    fn parse_expression_only(mut self) -> Result<Expr, Diagnostics> {
        self.skip_newlines();
        let expr = self.parse_expr();
        self.skip_newlines();
        if !self.at(&TokenKind::Eof) {
            self.error_here("expected end of placeholder expression");
        }
        self.diagnostics.into_result(expr)
    }

    fn parse_struct(&mut self) -> StructDef {
        let start = self.expect(TokenKind::Struct, "expected 'struct'").span;
        let name = self.expect_identifier("expected struct name");
        self.expect(TokenKind::Colon, "expected ':' after struct name");
        self.expect_statement_break("expected newline after struct name");
        let mut fields = Vec::new();
        self.skip_newlines();
        self.expect(TokenKind::Indent, "expected indented struct body");
        self.skip_newlines();
        while !self.at(&TokenKind::Dedent) && !self.at(&TokenKind::Eof) {
            let span = self.current_span();
            let name = self.expect_identifier("expected field name");
            self.expect(TokenKind::Colon, "expected ':' after field name");
            let ty = self.parse_type();
            self.expect_statement_break("expected newline after struct field");
            fields.push(StructField { name, ty, span });
            self.skip_newlines();
        }
        self.expect(TokenKind::Dedent, "expected dedent after struct body");
        StructDef {
            name,
            fields,
            span: start,
        }
    }

    fn parse_player_state(&mut self) -> PlayerStateDef {
        let start = self
            .expect(TokenKind::PlayerState, "expected 'player_state'")
            .span;
        let mut path = Vec::new();
        path.push(self.expect_identifier("expected player state name"));
        while self.eat(&TokenKind::Dot) {
            path.push(self.expect_identifier("expected player state path segment"));
        }
        self.expect(TokenKind::Colon, "expected ':' after player state name");
        let ty = self.parse_type();
        self.expect(
            TokenKind::Assign,
            "expected '=' before player state display name",
        );
        let display_name = self.expect_string("expected string display name");
        self.expect_statement_break("expected newline after player state declaration");
        PlayerStateDef {
            path,
            ty,
            display_name,
            span: start,
        }
    }

    fn parse_function(&mut self) -> Function {
        let start = self.expect(TokenKind::Fn, "expected 'fn'").span;
        let name = self.expect_identifier("expected function name");
        self.expect(TokenKind::LeftParen, "expected '(' after function name");

        let mut params = Vec::new();
        if !self.at(&TokenKind::RightParen) {
            loop {
                let span = self.current_span();
                let name = self.expect_identifier("expected parameter name");
                self.expect(TokenKind::Colon, "expected ':' after parameter name");
                let ty = self.parse_type();
                params.push(Param { name, ty, span });
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
        }

        self.expect(TokenKind::RightParen, "expected ')' after parameters");
        self.expect(TokenKind::Arrow, "expected '->' before return type");
        let return_type = self.parse_type();
        self.expect(TokenKind::Colon, "expected ':' after function signature");
        self.expect_statement_break("expected newline after function signature");
        let body = self.parse_indented_block("expected indented function body");

        Function {
            name,
            params,
            return_type,
            body,
            span: start,
        }
    }

    fn parse_indented_block(&mut self, indent_message: &str) -> Vec<Stmt> {
        if !self.eat(&TokenKind::Indent) {
            self.error_here(indent_message);
            return Vec::new();
        }
        let mut statements = Vec::new();
        self.skip_newlines();

        while !self.at(&TokenKind::Dedent) && !self.at(&TokenKind::Eof) {
            statements.push(self.parse_stmt());
            self.skip_newlines();
        }
        self.expect(TokenKind::Dedent, "expected dedent after block");
        statements
    }

    fn parse_stmt(&mut self) -> Stmt {
        let span = self.current_span();
        let kind = match self.peek().kind.clone() {
            TokenKind::Let => {
                self.bump();
                let name = self.expect_identifier("expected variable name after 'let'");
                self.expect(TokenKind::Assign, "expected '=' after variable name");
                let value = self.parse_expr();
                self.expect_statement_break("expected newline after let binding");
                StmtKind::Let { name, value }
            }
            TokenKind::If => {
                self.bump();
                let (condition, then_body, else_body) = self.parse_if_parts(span.clone());
                StmtKind::If {
                    condition,
                    then_body,
                    else_body,
                }
            }
            TokenKind::While => {
                self.bump();
                let condition = self.parse_expr();
                self.expect(TokenKind::Colon, "expected ':' after while condition");
                self.expect_statement_break("expected newline after while condition");
                let body = self.parse_indented_block("expected indented while body");
                StmtKind::While { condition, body }
            }
            TokenKind::Match => {
                self.bump();
                self.parse_match_stmt(span.clone())
            }
            TokenKind::Async => {
                self.bump();
                self.expect(TokenKind::Colon, "expected ':' after async");
                self.expect_statement_break("expected newline after async");
                let body = self.parse_indented_block("expected indented async body");
                StmtKind::Async { body }
            }
            TokenKind::For => {
                self.bump();
                let name = self.expect_identifier("expected loop variable name after 'for'");
                self.expect(TokenKind::In, "expected 'in' after loop variable");
                let first = self.parse_expr_bp(0);
                let kind = if self.eat(&TokenKind::DotDotEq) {
                    let end = self.parse_expr_bp(0);
                    ForKind::Range {
                        start: first,
                        end,
                        inclusive: true,
                    }
                } else if self.eat(&TokenKind::DotDot) {
                    let end = self.parse_expr_bp(0);
                    ForKind::Range {
                        start: first,
                        end,
                        inclusive: false,
                    }
                } else {
                    ForKind::Each { iterable: first }
                };
                self.expect(TokenKind::Colon, "expected ':' after for range");
                self.expect_statement_break("expected newline after for range");
                let body = self.parse_indented_block("expected indented for body");
                StmtKind::For { name, kind, body }
            }
            TokenKind::Break => {
                self.bump();
                self.expect_statement_break("expected newline after break");
                StmtKind::Break
            }
            TokenKind::Continue => {
                self.bump();
                self.expect_statement_break("expected newline after continue");
                StmtKind::Continue
            }
            TokenKind::Return => {
                self.bump();
                let value = if self.at(&TokenKind::Newline)
                    || self.at(&TokenKind::Dedent)
                    || self.at(&TokenKind::Else)
                    || self.at(&TokenKind::Eof)
                {
                    None
                } else {
                    Some(self.parse_expr())
                };
                self.expect_statement_break("expected newline after return");
                StmtKind::Return(value)
            }
            TokenKind::End => {
                self.error_here("'end' is no longer used; close blocks with indentation");
                self.bump();
                StmtKind::Expr(Expr {
                    kind: ExprKind::Variable("_error".to_string()),
                    span: span.clone(),
                })
            }
            TokenKind::Mc => {
                self.bump();
                let value = self.expect_string("expected string literal after 'mc'");
                self.expect_statement_break("expected newline after raw command");
                StmtKind::RawCommand(value)
            }
            TokenKind::Mcf => {
                self.bump();
                let value = self.expect_string("expected string literal after 'mcf'");
                self.expect_statement_break("expected newline after macro command");
                StmtKind::MacroCommand(value)
            }
            _ => {
                let expr = self.parse_expr();
                if self.eat(&TokenKind::Assign) {
                    let target = self.into_assign_target(expr);
                    let value = self.parse_expr();
                    self.expect_statement_break("expected newline after assignment");
                    StmtKind::Assign { target, value }
                } else if self.eat(&TokenKind::Colon) {
                    let context = self.into_context_header(expr);
                    self.expect_statement_break("expected newline after context header");
                    let body = self.parse_indented_block("expected indented context body");
                    match context {
                        Some((kind, anchor)) => StmtKind::Context { kind, anchor, body },
                        None => StmtKind::Expr(Expr {
                            kind: ExprKind::Variable("_error".to_string()),
                            span: span.clone(),
                        }),
                    }
                } else {
                    self.expect_statement_break("expected newline after expression");
                    StmtKind::Expr(expr)
                }
            }
        };

        Stmt { kind, span }
    }

    fn parse_match_stmt(&mut self, span: Span) -> StmtKind {
        let value = self.parse_expr();
        self.expect(TokenKind::Colon, "expected ':' after match value");
        self.expect_statement_break("expected newline after match value");
        self.expect(TokenKind::Indent, "expected indented match body");
        self.skip_newlines();

        let mut arms = Vec::new();
        let mut else_body = Vec::new();
        while !self.at(&TokenKind::Dedent) && !self.at(&TokenKind::Eof) {
            if self.eat(&TokenKind::Else) {
                self.expect(TokenKind::FatArrow, "expected '=>' after else");
                let stmt = self.parse_stmt();
                else_body.push(stmt);
            } else {
                let pattern = self.expect_string("expected string literal match arm");
                self.expect(TokenKind::FatArrow, "expected '=>' after match arm");
                let stmt = self.parse_stmt();
                arms.push(MatchArm {
                    pattern,
                    body: vec![stmt],
                });
            }
            self.skip_newlines();
        }
        self.expect(TokenKind::Dedent, "expected dedent after match body");
        if arms.is_empty() && else_body.is_empty() {
            self.diagnostics.push(Diagnostic::new(
                "match requires at least one arm",
                span.clone(),
            ));
        }
        StmtKind::Match {
            value,
            arms,
            else_body,
        }
    }

    fn parse_expr(&mut self) -> Expr {
        self.parse_expr_bp(0)
    }

    fn parse_expr_bp(&mut self, min_bp: u8) -> Expr {
        let mut left = self.parse_prefix();

        loop {
            let Some((op, left_bp, right_bp)) = self.current_infix_binding_power() else {
                break;
            };
            if left_bp < min_bp {
                break;
            }

            let span = self.bump().span;
            let right = self.parse_expr_bp(right_bp);
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

    fn parse_prefix(&mut self) -> Expr {
        if self.at(&TokenKind::Not) {
            let token = self.bump();
            let expr = self.parse_expr_bp(7);
            Expr {
                kind: ExprKind::Unary {
                    op: UnaryOp::Not,
                    expr: Box::new(expr),
                },
                span: token.span,
            }
        } else {
            self.parse_primary()
        }
    }

    fn parse_primary(&mut self) -> Expr {
        let token = self.bump();
        let expr = match token.kind {
            TokenKind::Integer(value) => Expr {
                kind: ExprKind::Int(value),
                span: token.span,
            },
            TokenKind::True => Expr {
                kind: ExprKind::Bool(true),
                span: token.span,
            },
            TokenKind::False => Expr {
                kind: ExprKind::Bool(false),
                span: token.span,
            },
            TokenKind::String(value) => Expr {
                kind: ExprKind::String(value),
                span: token.span,
            },
            TokenKind::LeftBracket => self.parse_array_literal(token.span),
            TokenKind::LeftBrace => self.parse_dict_literal(token.span),
            TokenKind::Identifier(name) => {
                if self.eat(&TokenKind::LeftParen) {
                    let args = self.parse_call_args();
                    Expr {
                        kind: ExprKind::Call {
                            function: name,
                            args,
                        },
                        span: token.span,
                    }
                } else if self.eat(&TokenKind::LeftBrace) {
                    let fields = self.parse_struct_literal_fields();
                    Expr {
                        kind: ExprKind::StructLiteral { name, fields },
                        span: token.span,
                    }
                } else {
                    Expr {
                        kind: ExprKind::Variable(name),
                        span: token.span,
                    }
                }
            }
            TokenKind::LeftParen => {
                let expr = self.parse_expr_bp(0);
                self.expect(TokenKind::RightParen, "expected ')' after expression");
                expr
            }
            _ => {
                self.diagnostics
                    .push(Diagnostic::new("expected expression", token.span.clone()));
                self.recover_expression();
                Expr {
                    kind: ExprKind::Int(0),
                    span: token.span,
                }
            }
        };

        self.parse_postfix(expr)
    }

    fn parse_postfix(&mut self, mut expr: Expr) -> Expr {
        loop {
            if self.at(&TokenKind::LeftParen) {
                let span = self.current_span();
                let (method, receiver) = match &expr.kind {
                    ExprKind::Path(path) => {
                        let mut path = path.clone();
                        match path.segments.pop() {
                            Some(PathSegment::Field(method)) => {
                                let receiver = if path.segments.is_empty() {
                                    *path.base
                                } else {
                                    Expr {
                                        kind: ExprKind::Path(path),
                                        span: expr.span.clone(),
                                    }
                                };
                                (method, receiver)
                            }
                            _ => {
                                self.diagnostics.push(Diagnostic::new(
                                    "only member access may be called like a method",
                                    span.clone(),
                                ));
                                break;
                            }
                        }
                    }
                    _ => break,
                };
                self.bump();
                let args = self.parse_call_args();
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
                expr = self.append_path_segment(expr, PathSegment::Field(field), span);
            } else if self.eat(&TokenKind::LeftBracket) {
                let span = self.current_span();
                let index_expr = self.parse_expr_bp(0);
                self.expect(TokenKind::RightBracket, "expected ']' after index");
                expr =
                    self.append_path_segment(expr, PathSegment::Index(Box::new(index_expr)), span);
            } else {
                break;
            }
        }

        expr
    }

    fn parse_array_literal(&mut self, span: Span) -> Expr {
        let mut values = Vec::new();
        if !self.at(&TokenKind::RightBracket) {
            loop {
                values.push(self.parse_expr_bp(0));
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
                if self.at(&TokenKind::RightBracket) {
                    break;
                }
            }
        }
        self.expect(TokenKind::RightBracket, "expected ']' after array literal");
        Expr {
            kind: ExprKind::ArrayLiteral(values),
            span,
        }
    }

    fn parse_dict_literal(&mut self, span: Span) -> Expr {
        let mut entries = Vec::new();
        if !self.at(&TokenKind::RightBrace) {
            loop {
                let key = self.expect_string("expected string key in dictionary literal");
                self.expect(TokenKind::Colon, "expected ':' after dictionary key");
                let value = self.parse_expr_bp(0);
                entries.push((key, value));
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
                if self.at(&TokenKind::RightBrace) {
                    break;
                }
            }
        }
        self.expect(
            TokenKind::RightBrace,
            "expected '}' after dictionary literal",
        );
        Expr {
            kind: ExprKind::DictLiteral(entries),
            span,
        }
    }

    fn parse_struct_literal_fields(&mut self) -> Vec<(String, Expr)> {
        let mut fields = Vec::new();
        if !self.at(&TokenKind::RightBrace) {
            loop {
                let name = self.expect_identifier("expected field name in struct literal");
                self.expect(TokenKind::Colon, "expected ':' after struct field");
                let value = self.parse_expr_bp(0);
                fields.push((name, value));
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
                if self.at(&TokenKind::RightBrace) {
                    break;
                }
            }
        }
        self.expect(TokenKind::RightBrace, "expected '}' after struct literal");
        fields
    }

    fn parse_call_args(&mut self) -> Vec<Expr> {
        let mut args = Vec::new();
        if !self.at(&TokenKind::RightParen) {
            loop {
                args.push(self.parse_expr_bp(0));
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
                if self.at(&TokenKind::RightParen) {
                    self.error_here("expected expression");
                    break;
                }
            }
        }
        self.expect(TokenKind::RightParen, "expected ')' after call arguments");
        args
    }

    fn append_path_segment(&mut self, expr: Expr, segment: PathSegment, span: Span) -> Expr {
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

    fn current_infix_binding_power(&self) -> Option<(BinaryOp, u8, u8)> {
        match self.peek().kind {
            TokenKind::Or => Some((BinaryOp::Or, 1, 2)),
            TokenKind::And => Some((BinaryOp::And, 3, 4)),
            TokenKind::EqEq => Some((BinaryOp::Eq, 5, 6)),
            TokenKind::BangEq => Some((BinaryOp::NotEq, 5, 6)),
            TokenKind::Lt => Some((BinaryOp::Lt, 5, 6)),
            TokenKind::Lte => Some((BinaryOp::Lte, 5, 6)),
            TokenKind::Gt => Some((BinaryOp::Gt, 5, 6)),
            TokenKind::Gte => Some((BinaryOp::Gte, 5, 6)),
            TokenKind::Plus => Some((BinaryOp::Add, 7, 8)),
            TokenKind::Minus => Some((BinaryOp::Sub, 7, 8)),
            TokenKind::Star => Some((BinaryOp::Mul, 9, 10)),
            TokenKind::Slash => Some((BinaryOp::Div, 9, 10)),
            _ => None,
        }
    }

    fn parse_if_parts(&mut self, span: Span) -> (Expr, Vec<Stmt>, Vec<Stmt>) {
        let condition = self.parse_expr();
        self.expect(TokenKind::Colon, "expected ':' after if condition");
        self.expect_statement_break("expected newline after if condition");
        let then_body = self.parse_indented_block("expected indented if body");
        let else_body = if self.eat(&TokenKind::Else) {
            if self.eat(&TokenKind::If) {
                let (condition, then_body, else_body) = self.parse_if_parts(span.clone());
                vec![Stmt {
                    kind: StmtKind::If {
                        condition,
                        then_body,
                        else_body,
                    },
                    span: span.clone(),
                }]
            } else {
                self.expect(TokenKind::Colon, "expected ':' after else");
                self.expect_statement_break("expected newline after else");
                self.parse_indented_block("expected indented else body")
            }
        } else {
            Vec::new()
        };
        (condition, then_body, else_body)
    }

    fn parse_type(&mut self) -> Type {
        let token = self.bump();
        match token.kind {
            TokenKind::Identifier(name) => match name.as_str() {
                "int" => Type::Int,
                "bool" => Type::Bool,
                "string" => Type::String,
                "entity_set" => Type::EntitySet,
                "entity_ref" => Type::EntityRef,
                "player_ref" => Type::PlayerRef,
                "block_ref" => Type::BlockRef,
                "entity_def" => Type::EntityDef,
                "block_def" => Type::BlockDef,
                "item_def" => Type::ItemDef,
                "text_def" => Type::TextDef,
                "item_slot" => Type::ItemSlot,
                "nbt" => Type::Nbt,
                "void" => Type::Void,
                "array" => {
                    self.expect(TokenKind::Lt, "expected '<' after 'array'");
                    let element = self.parse_type();
                    self.expect(TokenKind::Gt, "expected '>' after array element type");
                    Type::Array(Box::new(element))
                }
                "dict" => {
                    self.expect(TokenKind::Lt, "expected '<' after 'dict'");
                    let value = self.parse_type();
                    self.expect(TokenKind::Gt, "expected '>' after dictionary value type");
                    Type::Dict(Box::new(value))
                }
                "bossbar" => Type::Bossbar,
                _ => Type::Struct(name),
            },
            _ => {
                self.diagnostics
                    .push(Diagnostic::new("expected type", token.span));
                Type::Void
            }
        }
    }

    fn expect_statement_break(&mut self, message: &str) {
        if self.at(&TokenKind::Newline) {
            self.skip_newlines();
        } else if !self.at(&TokenKind::Dedent) && !self.at(&TokenKind::Eof) {
            self.error_here(message);
            self.recover_statement();
            self.skip_newlines();
        }
    }

    fn expect_identifier(&mut self, message: &str) -> String {
        let token = self.bump();
        match token.kind {
            TokenKind::Identifier(name) => name,
            _ => {
                self.diagnostics
                    .push(Diagnostic::new(message, token.span.clone()));
                "_error".to_string()
            }
        }
    }

    fn expect_string(&mut self, message: &str) -> String {
        let token = self.bump();
        match token.kind {
            TokenKind::String(value) => value,
            _ => {
                self.diagnostics
                    .push(Diagnostic::new(message, token.span.clone()));
                String::new()
            }
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
        if self.at(expected) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn at(&self, expected: &TokenKind) -> bool {
        same_variant(&self.peek().kind, expected)
    }

    fn peek(&self) -> &Token {
        self.tokens.peek()
    }

    fn bump(&mut self) -> Token {
        self.tokens.bump()
    }

    fn current_span(&self) -> Span {
        self.peek().span.clone()
    }

    fn skip_newlines(&mut self) {
        while self.eat(&TokenKind::Newline) {}
    }

    fn recover_top_level(&mut self) {
        while !self.at(&TokenKind::Eof) {
            if self.at(&TokenKind::Fn)
                || self.at(&TokenKind::Struct)
                || self.at(&TokenKind::PlayerState)
            {
                break;
            }
            self.bump();
            if self.at(&TokenKind::Fn)
                || self.at(&TokenKind::Struct)
                || self.at(&TokenKind::PlayerState)
            {
                break;
            }
        }
    }

    fn recover_statement(&mut self) {
        while !self.at(&TokenKind::Eof)
            && !self.at(&TokenKind::Newline)
            && !self.at(&TokenKind::Dedent)
        {
            self.bump();
        }
    }

    fn recover_expression(&mut self) {
        while !self.at(&TokenKind::Eof)
            && !self.at(&TokenKind::Comma)
            && !self.at(&TokenKind::RightParen)
            && !self.at(&TokenKind::RightBracket)
            && !self.at(&TokenKind::RightBrace)
            && !self.at(&TokenKind::DotDot)
            && !self.at(&TokenKind::DotDotEq)
            && !self.at(&TokenKind::Newline)
            && !self.at(&TokenKind::Dedent)
            && !self.at(&TokenKind::Else)
            && !self.at(&TokenKind::Colon)
            && !self.at(&TokenKind::Let)
            && !self.at(&TokenKind::If)
            && !self.at(&TokenKind::For)
            && !self.at(&TokenKind::While)
            && !self.at(&TokenKind::Break)
            && !self.at(&TokenKind::Continue)
            && !self.at(&TokenKind::Return)
            && !self.at(&TokenKind::Mc)
            && !self.at(&TokenKind::Mcf)
        {
            self.bump();
        }
    }

    fn error_here(&mut self, message: &str) {
        self.diagnostics
            .push(Diagnostic::new(message, self.current_span()));
    }

    fn into_assign_target(&mut self, expr: Expr) -> AssignTarget {
        match expr.kind {
            ExprKind::Variable(name) => AssignTarget::Variable(name),
            ExprKind::Path(path) => AssignTarget::Path(path),
            _ => {
                self.diagnostics
                    .push(Diagnostic::new("invalid assignment target", expr.span));
                AssignTarget::Variable("_error".to_string())
            }
        }
    }

    fn into_context_header(&mut self, expr: Expr) -> Option<(ContextKind, Expr)> {
        let span = expr.span.clone();
        match expr.kind {
            ExprKind::Call { function, mut args } if function == "as" || function == "at" => {
                if args.len() != 1 {
                    self.diagnostics.push(Diagnostic::new(
                        format!("{} context block requires exactly one anchor", function),
                        span,
                    ));
                    return None;
                }
                let kind = if function == "as" {
                    ContextKind::As
                } else {
                    ContextKind::At
                };
                Some((kind, args.remove(0)))
            }
            _ => {
                self.diagnostics.push(Diagnostic::new(
                    "only 'as(anchor):' and 'at(anchor):' may introduce context blocks",
                    span,
                ));
                None
            }
        }
    }
}

struct TokenStream {
    tokens: Vec<Token>,
    index: usize,
}

impl TokenStream {
    fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, index: 0 }
    }

    fn peek(&self) -> &Token {
        &self.tokens[self.index]
    }
    fn bump(&mut self) -> Token {
        let token = self.peek().clone();
        if !matches!(token.kind, TokenKind::Eof) {
            self.index += 1;
        }
        token
    }
}

fn same_variant(left: &TokenKind, right: &TokenKind) -> bool {
    std::mem::discriminant(left) == std::mem::discriminant(right)
}

#[cfg(test)]
mod tests {
    use super::parse;
    use crate::ast::{BinaryOp, Expr, ExprKind, ForKind, StmtKind, UnaryOp};

    #[test]
    fn parses_comments_and_async_blocks() {
        let program = parse(
            r#"
# leading
fn fibb(n: int) -> void: # trailing
    async:
        return
"#,
        )
        .unwrap();

        assert_eq!(program.functions.len(), 1);
        assert!(matches!(
            program.functions[0].body[0].kind,
            StmtKind::Async { .. }
        ));
    }

    #[test]
    fn preserves_expression_precedence() {
        let program = parse(
            r#"
fn main() -> void:
    let value = 1 + 2 * 3
    return
"#,
        )
        .unwrap();

        let StmtKind::Let { value, .. } = &program.functions[0].body[0].kind else {
            panic!("expected let statement");
        };
        let ExprKind::Binary { op, right, .. } = &value.kind else {
            panic!("expected binary expression");
        };
        assert!(matches!(op, BinaryOp::Add));
        assert!(matches!(right.kind, ExprKind::Binary { .. }));
    }

    #[test]
    fn parses_else_if_for_and_logic() {
        let program = parse(
            r#"
fn main() -> void:
    for i in 0..=10:
        if true and not false:
            return
        else if i == 2:
            continue
        else:
            break
"#,
        )
        .unwrap();

        let StmtKind::For { kind, body, .. } = &program.functions[0].body[0].kind else {
            panic!("expected for statement");
        };
        assert!(matches!(
            kind,
            ForKind::Range {
                inclusive: true,
                ..
            }
        ));

        let StmtKind::If {
            condition,
            then_body,
            else_body,
        } = &body[0].kind
        else {
            panic!("expected if statement");
        };
        assert!(matches!(
            condition.kind,
            ExprKind::Binary {
                op: BinaryOp::And,
                ..
            }
        ));
        let ExprKind::Binary { right, .. } = &condition.kind else {
            panic!("expected binary condition");
        };
        assert!(matches!(
            right.kind,
            ExprKind::Unary {
                op: UnaryOp::Not,
                ..
            }
        ));
        assert!(matches!(then_body[0].kind, StmtKind::Return(None)));
        assert!(matches!(else_body[0].kind, StmtKind::If { .. }));
    }

    #[test]
    fn recovers_multiple_parser_errors() {
        let error = parse(
            r#"
fn main() -> void:
    let x =
    let y =
"#,
        )
        .unwrap_err();

        assert!(error.0.len() >= 2);
        assert!(error.to_string().contains("expected expression"));
    }

    #[test]
    fn rejects_malformed_else_and_for() {
        let error = parse(
            r#"
fn main() -> void:
    if true:
        return
    else
        return
    for i in 0 10:
        return
"#,
        )
        .unwrap_err();

        let rendered = error.to_string();
        assert!(rendered.contains("expected ':' after else"));
        assert!(rendered.contains("expected ':' after for range"));
    }

    #[test]
    fn parses_for_each_and_path_assignment() {
        let program = parse(
            r#"
fn main() -> void:
    let pigs = selector("@e[type=pig]")
    for pig in pigs:
        pig.CustomName = "Hello"
"#,
        )
        .unwrap();

        let StmtKind::For { kind, body, .. } = &program.functions[0].body[1].kind else {
            panic!("expected for statement");
        };
        assert!(matches!(kind, ForKind::Each { .. }));
        assert!(matches!(body[0].kind, StmtKind::Assign { .. }));
    }

    #[test]
    fn parses_player_method_calls() {
        let program = parse(
            r#"
fn main() -> void:
    let player = single(selector("@p"))
    player.effect("speed", 10, 1)
"#,
        )
        .unwrap();

        assert!(matches!(
            program.functions[0].body[1].kind,
            StmtKind::Expr(Expr {
                kind: ExprKind::MethodCall { .. },
                ..
            })
        ));
    }

    #[test]
    fn parses_gameplay_builtins_and_equipment_paths() {
        let program = parse(
            r#"
fn main() -> void:
    let pig = summon("minecraft:pig")
    pig.add_tag("elite")
    pig.offhand.item = "minecraft:shield"
    teleport(pig, block("~ ~ ~"))
    tellraw(pig, "hello")
    debug_marker(block("~ ~1 ~"), "marker")
    debug_entity(pig, "pig")
    fill(block("~ ~ ~"), block("~1 ~1 ~1"), "minecraft:stone")
"#,
        )
        .unwrap();

        assert!(matches!(
            program.functions[0].body[0].kind,
            StmtKind::Let {
                value: Expr {
                    kind: ExprKind::Call { .. },
                    ..
                },
                ..
            }
        ));
        assert!(matches!(
            program.functions[0].body[1].kind,
            StmtKind::Expr(Expr {
                kind: ExprKind::MethodCall { .. },
                ..
            })
        ));
        assert!(matches!(
            program.functions[0].body[2].kind,
            StmtKind::Assign { .. }
        ));
        assert!(matches!(
            program.functions[0].body[3].kind,
            StmtKind::Expr(Expr {
                kind: ExprKind::Call { .. },
                ..
            })
        ));
    }

    #[test]
    fn parses_entity_and_block_builders() {
        let program = parse(
            r#"
fn main() -> void:
    let pig = entity("minecraft:pig")
    pig.name = "Boss"
    let chest = block_type("minecraft:chest")
    chest.states.facing = "north"
    let spawned = summon(pig)
    block("~ ~ ~").setblock(chest)
"#,
        )
        .unwrap();

        assert!(matches!(
            program.functions[0].body[0].kind,
            StmtKind::Let {
                value: Expr {
                    kind: ExprKind::Call { .. },
                    ..
                },
                ..
            }
        ));
        assert!(matches!(
            program.functions[0].body[1].kind,
            StmtKind::Assign { .. }
        ));
        assert!(matches!(
            program.functions[0].body[3].kind,
            StmtKind::Assign { .. }
        ));
        assert!(matches!(
            program.functions[0].body[4].kind,
            StmtKind::Let {
                value: Expr {
                    kind: ExprKind::Call { .. },
                    ..
                },
                ..
            }
        ));
        assert!(matches!(
            program.functions[0].body[5].kind,
            StmtKind::Expr(Expr {
                kind: ExprKind::MethodCall { .. },
                ..
            })
        ));
    }
}
