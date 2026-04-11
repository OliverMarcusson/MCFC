use crate::ast::*;
use crate::diagnostics::{Diagnostic, Diagnostics, Span};
use crate::lexer::{Token, TokenKind, lex};

pub fn parse(source: &str) -> Result<Program, Diagnostics> {
    let tokens = lex(source)?;
    Parser::new(tokens).parse_program()
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
        let mut functions = Vec::new();
        self.skip_newlines();

        while !self.at(&TokenKind::Eof) {
            if self.at(&TokenKind::Fn) || self.at(&TokenKind::BookAnnotation) {
                functions.push(self.parse_function());
            } else {
                self.error_here("expected function definition");
                self.recover_top_level();
            }
            self.skip_newlines();
        }

        self.diagnostics.into_result(Program { functions })
    }

    fn parse_function(&mut self) -> Function {
        let book_exposed = self.eat(&TokenKind::BookAnnotation);
        if book_exposed {
            self.skip_newlines();
        }

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
        self.expect_statement_break("expected newline after function signature");
        let body = self.parse_block_until(|kind| matches!(kind, TokenKind::End));
        self.expect(TokenKind::End, "expected 'end'");

        Function {
            name,
            params,
            return_type,
            body,
            book_exposed,
            span: start,
        }
    }

    fn parse_block_until(&mut self, terminator: impl Fn(&TokenKind) -> bool) -> Vec<Stmt> {
        let mut statements = Vec::new();
        self.skip_newlines();

        while !terminator(&self.peek().kind) && !self.at(&TokenKind::Eof) {
            statements.push(self.parse_stmt());
            self.skip_newlines();
        }
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
                self.expect(TokenKind::End, "expected 'end'");
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
                let body = self.parse_block_until(|kind| matches!(kind, TokenKind::End));
                self.expect(TokenKind::End, "expected 'end'");
                StmtKind::While { condition, body }
            }
            TokenKind::For => {
                self.bump();
                let name = self.expect_identifier("expected loop variable name after 'for'");
                self.expect(TokenKind::In, "expected 'in' after loop variable");
                let start = self.parse_expr_bp(0);
                let inclusive = if self.eat(&TokenKind::DotDotEq) {
                    true
                } else {
                    self.expect(TokenKind::DotDot, "expected '..' or '..=' in for range");
                    false
                };
                let end = self.parse_expr_bp(0);
                self.expect(TokenKind::Colon, "expected ':' after for range");
                self.expect_statement_break("expected newline after for range");
                let body = self.parse_block_until(|kind| matches!(kind, TokenKind::End));
                self.expect(TokenKind::End, "expected 'end'");
                StmtKind::For {
                    name,
                    start,
                    end,
                    inclusive,
                    body,
                }
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
                    || self.at(&TokenKind::End)
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
            TokenKind::Identifier(_) if self.peek_n_is(1, &TokenKind::Assign) => {
                let name = self.expect_identifier("expected variable name");
                self.expect(TokenKind::Assign, "expected '=' in assignment");
                let value = self.parse_expr();
                self.expect_statement_break("expected newline after assignment");
                StmtKind::Assign { name, value }
            }
            _ => {
                let expr = self.parse_expr();
                self.expect_statement_break("expected newline after expression");
                StmtKind::Expr(expr)
            }
        };

        Stmt { kind, span }
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
        match token.kind {
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
            TokenKind::Identifier(name) => {
                if self.eat(&TokenKind::LeftParen) {
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
                    Expr {
                        kind: ExprKind::Call {
                            function: name,
                            args,
                        },
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
        let then_body =
            self.parse_block_until(|kind| matches!(kind, TokenKind::Else | TokenKind::End));
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
                self.parse_block_until(|kind| matches!(kind, TokenKind::End))
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
                "void" => Type::Void,
                _ => {
                    self.diagnostics.push(Diagnostic::new(
                        format!("unknown type '{}'", name),
                        token.span,
                    ));
                    Type::Void
                }
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
        } else if !self.at(&TokenKind::End) && !self.at(&TokenKind::Eof) {
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

    fn peek_n_is(&self, offset: usize, expected: &TokenKind) -> bool {
        same_variant(&self.tokens.peek_n(offset).kind, expected)
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
            if self.at(&TokenKind::Fn) || self.at(&TokenKind::BookAnnotation) {
                break;
            }
            self.bump();
            if self.at(&TokenKind::Fn) || self.at(&TokenKind::BookAnnotation) {
                break;
            }
        }
    }

    fn recover_statement(&mut self) {
        while !self.at(&TokenKind::Eof)
            && !self.at(&TokenKind::Newline)
            && !self.at(&TokenKind::End)
        {
            self.bump();
        }
    }

    fn recover_expression(&mut self) {
        while !self.at(&TokenKind::Eof)
            && !self.at(&TokenKind::Comma)
            && !self.at(&TokenKind::RightParen)
            && !self.at(&TokenKind::DotDot)
            && !self.at(&TokenKind::DotDotEq)
            && !self.at(&TokenKind::Newline)
            && !self.at(&TokenKind::End)
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

    fn peek_n(&self, offset: usize) -> &Token {
        let index = (self.index + offset).min(self.tokens.len().saturating_sub(1));
        &self.tokens[index]
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
    use crate::ast::{BinaryOp, ExprKind, StmtKind, UnaryOp};

    #[test]
    fn parses_comments_and_function_annotations() {
        let program = parse(
            r#"
# leading
@book
fn fibb(n: int) -> void # trailing
    return
end
"#,
        )
        .unwrap();

        assert_eq!(program.functions.len(), 1);
        assert!(program.functions[0].book_exposed);
    }

    #[test]
    fn preserves_expression_precedence() {
        let program = parse(
            r#"
fn main() -> void
    let value = 1 + 2 * 3
    return
end
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
fn main() -> void
    for i in 0..=10:
        if true and not false:
            return
        else if i == 2:
            continue
        else:
            break
        end
    end
end
"#,
        )
        .unwrap();

        let StmtKind::For {
            inclusive, body, ..
        } = &program.functions[0].body[0].kind
        else {
            panic!("expected for statement");
        };
        assert!(*inclusive);

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
fn main() -> void
    let x =
    let y =
end
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
fn main() -> void
    if true:
        return
    else
        return
    end
    for i in 0 10:
        return
    end
end
"#,
        )
        .unwrap_err();

        let rendered = error.to_string();
        assert!(rendered.contains("expected ':' after else"));
        assert!(rendered.contains("expected '..' or '..=' in for range"));
    }
}
