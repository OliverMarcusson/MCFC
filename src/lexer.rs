use crate::diagnostics::{Diagnostic, Diagnostics, SourceFile, Span, TextRange};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
    pub range: TextRange,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenKind {
    Fn,
    Struct,
    PlayerState,
    Let,
    Return,
    End,
    If,
    Match,
    Else,
    While,
    For,
    In,
    Break,
    Continue,
    Async,
    Mc,
    Mcf,
    True,
    False,
    And,
    Or,
    Not,
    Arrow,
    FatArrow,
    DotDot,
    DotDotEq,
    Colon,
    Comma,
    Dot,
    LeftParen,
    RightParen,
    LeftBracket,
    RightBracket,
    LeftBrace,
    RightBrace,
    Assign,
    Plus,
    Minus,
    Star,
    Slash,
    EqEq,
    BangEq,
    Lt,
    Lte,
    Gt,
    Gte,
    Identifier(String),
    Integer(i64),
    String(String),
    Newline,
    Indent,
    Dedent,
    Eof,
}

pub fn lex(source: &str) -> Result<Vec<Token>, Diagnostics> {
    let source_file = SourceFile::new(source);
    let mut cursor = Cursor::new(source);
    let mut diagnostics = Diagnostics::new();
    let mut tokens = Vec::new();
    let mut indents = vec![0usize];
    let mut at_line_start = true;

    while cursor.peek().is_some() {
        if at_line_start {
            let line_start = cursor.position();
            let mut indent = 0usize;
            while let Some(next) = cursor.peek() {
                match next {
                    ' ' => {
                        indent += 1;
                        cursor.bump();
                    }
                    '\t' => {
                        let start = cursor.position();
                        cursor.bump();
                        diagnostics.push(Diagnostic::new(
                            "tabs are not allowed for indentation; use spaces",
                            Span::from_range(
                                &source_file,
                                TextRange::new(start, cursor.position()),
                            ),
                        ));
                        indent += 4;
                    }
                    '\r' => {
                        cursor.bump();
                    }
                    _ => break,
                }
            }

            match cursor.peek() {
                Some('\n') => {
                    let start = cursor.position();
                    cursor.bump();
                    push_token(
                        &mut tokens,
                        &source_file,
                        TokenKind::Newline,
                        TextRange::new(start, cursor.position()),
                    );
                    at_line_start = true;
                    continue;
                }
                Some('#') => {
                    cursor.skip_comment();
                    at_line_start = true;
                    continue;
                }
                None => break,
                _ => {
                    let current = *indents.last().unwrap();
                    if indent > current {
                        indents.push(indent);
                        push_token(
                            &mut tokens,
                            &source_file,
                            TokenKind::Indent,
                            TextRange::new(line_start, cursor.position()),
                        );
                    } else if indent < current {
                        while indent < *indents.last().unwrap() {
                            indents.pop();
                            push_token(
                                &mut tokens,
                                &source_file,
                                TokenKind::Dedent,
                                TextRange::new(line_start, cursor.position()),
                            );
                        }
                        if indent != *indents.last().unwrap() {
                            diagnostics.push(Diagnostic::new(
                                "inconsistent indentation",
                                Span::from_range(
                                    &source_file,
                                    TextRange::new(line_start, cursor.position()),
                                ),
                            ));
                        }
                    }
                    at_line_start = false;
                }
            }
        }
        let Some(ch) = cursor.peek() else {
            break;
        };
        match ch {
            ' ' | '\t' | '\r' => {
                if ch == '\t' {
                    let start = cursor.position();
                    cursor.bump();
                    diagnostics.push(Diagnostic::new(
                        "tabs are not allowed; use spaces",
                        Span::from_range(&source_file, TextRange::new(start, cursor.position())),
                    ));
                    continue;
                }
                cursor.bump();
            }
            '#' => cursor.skip_comment(),
            '\n' => {
                let start = cursor.position();
                cursor.bump();
                push_token(
                    &mut tokens,
                    &source_file,
                    TokenKind::Newline,
                    TextRange::new(start, cursor.position()),
                );
                at_line_start = true;
            }
            '(' => push_simple(&mut cursor, &mut tokens, &source_file, TokenKind::LeftParen),
            ')' => push_simple(
                &mut cursor,
                &mut tokens,
                &source_file,
                TokenKind::RightParen,
            ),
            '[' => push_simple(
                &mut cursor,
                &mut tokens,
                &source_file,
                TokenKind::LeftBracket,
            ),
            ']' => push_simple(
                &mut cursor,
                &mut tokens,
                &source_file,
                TokenKind::RightBracket,
            ),
            '{' => push_simple(&mut cursor, &mut tokens, &source_file, TokenKind::LeftBrace),
            '}' => push_simple(
                &mut cursor,
                &mut tokens,
                &source_file,
                TokenKind::RightBrace,
            ),
            ':' => push_simple(&mut cursor, &mut tokens, &source_file, TokenKind::Colon),
            ',' => push_simple(&mut cursor, &mut tokens, &source_file, TokenKind::Comma),
            '+' => push_simple(&mut cursor, &mut tokens, &source_file, TokenKind::Plus),
            '*' => push_simple(&mut cursor, &mut tokens, &source_file, TokenKind::Star),
            '/' => push_simple(&mut cursor, &mut tokens, &source_file, TokenKind::Slash),
            '.' => {
                let start = cursor.position();
                cursor.bump();
                if cursor.peek() == Some('.') {
                    cursor.bump();
                    let kind = if cursor.peek() == Some('=') {
                        cursor.bump();
                        TokenKind::DotDotEq
                    } else {
                        TokenKind::DotDot
                    };
                    push_token(
                        &mut tokens,
                        &source_file,
                        kind,
                        TextRange::new(start, cursor.position()),
                    );
                } else {
                    push_token(
                        &mut tokens,
                        &source_file,
                        TokenKind::Dot,
                        TextRange::new(start, cursor.position()),
                    );
                }
            }
            '-' => {
                let start = cursor.position();
                cursor.bump();
                if cursor.peek() == Some('>') {
                    cursor.bump();
                    push_token(
                        &mut tokens,
                        &source_file,
                        TokenKind::Arrow,
                        TextRange::new(start, cursor.position()),
                    );
                } else {
                    push_token(
                        &mut tokens,
                        &source_file,
                        TokenKind::Minus,
                        TextRange::new(start, cursor.position()),
                    );
                }
            }
            '=' => {
                let start = cursor.position();
                cursor.bump();
                if cursor.peek() == Some('=') {
                    cursor.bump();
                    push_token(
                        &mut tokens,
                        &source_file,
                        TokenKind::EqEq,
                        TextRange::new(start, cursor.position()),
                    );
                } else if cursor.peek() == Some('>') {
                    cursor.bump();
                    push_token(
                        &mut tokens,
                        &source_file,
                        TokenKind::FatArrow,
                        TextRange::new(start, cursor.position()),
                    );
                } else {
                    push_token(
                        &mut tokens,
                        &source_file,
                        TokenKind::Assign,
                        TextRange::new(start, cursor.position()),
                    );
                }
            }
            '!' => {
                let start = cursor.position();
                cursor.bump();
                if cursor.peek() == Some('=') {
                    cursor.bump();
                    push_token(
                        &mut tokens,
                        &source_file,
                        TokenKind::BangEq,
                        TextRange::new(start, cursor.position()),
                    );
                } else {
                    diagnostics.push(Diagnostic::new(
                        "unexpected '!'",
                        Span::from_range(&source_file, TextRange::new(start, cursor.position())),
                    ));
                }
            }
            '<' => {
                let start = cursor.position();
                cursor.bump();
                if cursor.peek() == Some('=') {
                    cursor.bump();
                    push_token(
                        &mut tokens,
                        &source_file,
                        TokenKind::Lte,
                        TextRange::new(start, cursor.position()),
                    );
                } else {
                    push_token(
                        &mut tokens,
                        &source_file,
                        TokenKind::Lt,
                        TextRange::new(start, cursor.position()),
                    );
                }
            }
            '>' => {
                let start = cursor.position();
                cursor.bump();
                if cursor.peek() == Some('=') {
                    cursor.bump();
                    push_token(
                        &mut tokens,
                        &source_file,
                        TokenKind::Gte,
                        TextRange::new(start, cursor.position()),
                    );
                } else {
                    push_token(
                        &mut tokens,
                        &source_file,
                        TokenKind::Gt,
                        TextRange::new(start, cursor.position()),
                    );
                }
            }
            '"' | '\'' => {
                let start = cursor.position();
                let delimiter = ch;
                cursor.bump();
                let mut value = String::new();
                let mut terminated = false;
                while let Some(next) = cursor.peek() {
                    match next {
                        quote if quote == delimiter => {
                            cursor.bump();
                            terminated = true;
                            break;
                        }
                        '\\' => {
                            cursor.bump();
                            match cursor.peek() {
                                Some(quote) if quote == delimiter => {
                                    cursor.bump();
                                    value.push(delimiter);
                                }
                                Some('"') => {
                                    cursor.bump();
                                    value.push('"');
                                }
                                Some('\'') => {
                                    cursor.bump();
                                    value.push('\'');
                                }
                                Some('\\') => {
                                    cursor.bump();
                                    value.push('\\');
                                }
                                Some('n') => {
                                    cursor.bump();
                                    value.push('\n');
                                }
                                Some('t') => {
                                    cursor.bump();
                                    value.push('\t');
                                }
                                Some(other) => {
                                    cursor.bump();
                                    value.push(other);
                                }
                                None => break,
                            }
                        }
                        other => {
                            cursor.bump();
                            value.push(other);
                        }
                    }
                }

                let range = TextRange::new(start, cursor.position());
                if !terminated {
                    diagnostics.push(Diagnostic::new(
                        "unterminated string literal",
                        Span::from_range(&source_file, range),
                    ));
                } else {
                    push_token(&mut tokens, &source_file, TokenKind::String(value), range);
                }
            }
            '@' => {
                let start = cursor.position();
                cursor.bump();
                let ident_start = cursor.position();
                cursor.consume_while(is_ident_continue);
                let ident = &source[ident_start..cursor.position()];
                let range = TextRange::new(start, cursor.position());
                diagnostics.push(Diagnostic::new(
                    format!("unknown annotation '@{}'", ident),
                    Span::from_range(&source_file, range),
                ));
            }
            ch if ch.is_ascii_digit() => {
                let start = cursor.position();
                cursor.consume_while(|next| next.is_ascii_digit());
                let range = TextRange::new(start, cursor.position());
                let raw = &source[range.start..range.end];
                let value = raw.parse().unwrap_or(0);
                push_token(&mut tokens, &source_file, TokenKind::Integer(value), range);
            }
            ch if is_ident_start(ch) => {
                let start = cursor.position();
                cursor.consume_while(is_ident_continue);
                let range = TextRange::new(start, cursor.position());
                let raw = &source[range.start..range.end];
                let kind = match raw {
                    "fn" => TokenKind::Fn,
                    "struct" => TokenKind::Struct,
                    "player_state" => TokenKind::PlayerState,
                    "let" => TokenKind::Let,
                    "return" => TokenKind::Return,
                    "end" => TokenKind::End,
                    "if" => TokenKind::If,
                    "match" => TokenKind::Match,
                    "else" => TokenKind::Else,
                    "while" => TokenKind::While,
                    "for" => TokenKind::For,
                    "in" => TokenKind::In,
                    "break" => TokenKind::Break,
                    "continue" => TokenKind::Continue,
                    "async" => TokenKind::Async,
                    "mc" => TokenKind::Mc,
                    "mcf" => TokenKind::Mcf,
                    "true" => TokenKind::True,
                    "false" => TokenKind::False,
                    "and" => TokenKind::And,
                    "or" => TokenKind::Or,
                    "not" => TokenKind::Not,
                    _ => TokenKind::Identifier(raw.to_string()),
                };
                push_token(&mut tokens, &source_file, kind, range);
            }
            other => {
                let start = cursor.position();
                cursor.bump();
                diagnostics.push(Diagnostic::new(
                    format!("unexpected character '{}'", other),
                    Span::from_range(&source_file, TextRange::new(start, cursor.position())),
                ));
            }
        }
    }

    if !matches!(
        tokens.last().map(|token| &token.kind),
        Some(TokenKind::Newline) | None
    ) {
        let newline_range = TextRange::new(cursor.position(), cursor.position());
        push_token(&mut tokens, &source_file, TokenKind::Newline, newline_range);
    }
    while indents.len() > 1 {
        indents.pop();
        let range = TextRange::new(cursor.position(), cursor.position());
        push_token(&mut tokens, &source_file, TokenKind::Dedent, range);
    }
    let eof_range = TextRange::new(cursor.position(), cursor.position());
    push_token(&mut tokens, &source_file, TokenKind::Eof, eof_range);
    diagnostics.into_result(tokens)
}

fn push_simple(
    cursor: &mut Cursor<'_>,
    tokens: &mut Vec<Token>,
    source_file: &SourceFile<'_>,
    kind: TokenKind,
) {
    let start = cursor.position();
    cursor.bump();
    push_token(
        tokens,
        source_file,
        kind,
        TextRange::new(start, cursor.position()),
    );
}

fn push_token(
    tokens: &mut Vec<Token>,
    source_file: &SourceFile<'_>,
    kind: TokenKind,
    range: TextRange,
) {
    tokens.push(Token {
        span: Span::from_range(source_file, range),
        kind,
        range,
    });
}

fn is_ident_start(ch: char) -> bool {
    ch.is_ascii_alphabetic() || ch == '_'
}

fn is_ident_continue(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

struct Cursor<'a> {
    source: &'a str,
    position: usize,
}

impl<'a> Cursor<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source,
            position: 0,
        }
    }

    fn position(&self) -> usize {
        self.position
    }

    fn peek(&self) -> Option<char> {
        self.source[self.position..].chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.position += ch.len_utf8();
        Some(ch)
    }

    fn consume_while(&mut self, predicate: impl Fn(char) -> bool) {
        while let Some(ch) = self.peek() {
            if predicate(ch) {
                self.bump();
            } else {
                break;
            }
        }
    }

    fn skip_comment(&mut self) {
        while let Some(ch) = self.peek() {
            if ch == '\n' {
                break;
            }
            self.bump();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{TokenKind, lex};

    #[test]
    fn lexes_comments_and_newlines() {
        let tokens = lex("fn main() -> void # trailing\n# own line\nend\n").unwrap();
        let kinds: Vec<_> = tokens.into_iter().map(|token| token.kind).collect();
        assert!(matches!(kinds[0], TokenKind::Fn));
        assert!(
            kinds
                .iter()
                .filter(|kind| matches!(kind, TokenKind::Newline))
                .count()
                >= 2
        );
        assert!(matches!(kinds.last(), Some(TokenKind::Eof)));
    }

    #[test]
    fn lexes_async_and_ranges() {
        let tokens = lex("async\n0..10\n0..=10\n").unwrap();
        assert!(matches!(tokens[0].kind, TokenKind::Async));
        assert_eq!(tokens[0].range.start, 0);
        assert_eq!(tokens[0].range.end, 5);
        assert!(
            tokens
                .iter()
                .any(|token| matches!(token.kind, TokenKind::DotDot))
        );
        assert!(
            tokens
                .iter()
                .any(|token| matches!(token.kind, TokenKind::DotDotEq))
        );
    }

    #[test]
    fn reports_malformed_tokens_without_stopping() {
        let error = lex("!\n\"\n").unwrap_err();
        let rendered = error.to_string();
        assert!(rendered.contains("unexpected '!'"));
        assert!(rendered.contains("unterminated string literal"));
    }

    #[test]
    fn lexes_single_quoted_strings() {
        let tokens = lex("'hello' '\"\"' 'it\\'s'").unwrap();
        let strings: Vec<_> = tokens
            .into_iter()
            .filter_map(|token| match token.kind {
                TokenKind::String(value) => Some(value),
                _ => None,
            })
            .collect();

        assert_eq!(strings, vec!["hello", "\"\"", "it's"]);
    }

    #[test]
    fn lexes_control_flow_keywords() {
        let tokens = lex("else for in break continue and or not\n").unwrap();
        let kinds: Vec<_> = tokens.into_iter().map(|token| token.kind).collect();
        assert!(kinds.iter().any(|kind| matches!(kind, TokenKind::Else)));
        assert!(kinds.iter().any(|kind| matches!(kind, TokenKind::For)));
        assert!(kinds.iter().any(|kind| matches!(kind, TokenKind::In)));
        assert!(kinds.iter().any(|kind| matches!(kind, TokenKind::Break)));
        assert!(kinds.iter().any(|kind| matches!(kind, TokenKind::Continue)));
        assert!(kinds.iter().any(|kind| matches!(kind, TokenKind::And)));
        assert!(kinds.iter().any(|kind| matches!(kind, TokenKind::Or)));
        assert!(kinds.iter().any(|kind| matches!(kind, TokenKind::Not)));
    }
}
