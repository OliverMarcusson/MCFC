use crate::diagnostics::{Diagnostic, Diagnostics, SourceFile, Span, TextRange};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
    pub range: TextRange,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenKind {
    Return,
    If,
    Else,
    While,
    For,
    Break,
    Continue,
    Async,
    New,
    Do,
    True,
    False,
    AndAnd,
    OrOr,
    Bang,
    Arrow,
    Colon,
    Question,
    Semicolon,
    Comma,
    Dot,
    At,
    LeftParen,
    RightParen,
    LeftBracket,
    RightBracket,
    LeftBrace,
    RightBrace,
    Assign,
    PlusAssign,
    MinusAssign,
    StarAssign,
    SlashAssign,
    PercentAssign,
    PlusPlus,
    MinusMinus,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    EqEq,
    BangEq,
    Lt,
    Lte,
    Gt,
    Gte,
    Identifier(String),
    Integer(i64),
    Float(String),
    String(String),
    Eof,
}

pub fn lex(source: &str) -> Result<Vec<Token>, Diagnostics> {
    let source_file = SourceFile::new(source);
    let mut cursor = Cursor::new(source);
    let mut diagnostics = Diagnostics::new();
    let mut tokens = Vec::new();

    while let Some(ch) = cursor.peek() {
        let start = cursor.position();
        if ch.is_whitespace() {
            cursor.bump();
            continue;
        }
        if cursor.rest().starts_with("//") {
            cursor.consume_while(|next| next != '\n');
            continue;
        }
        if cursor.rest().starts_with("/*") {
            match cursor.rest()[2..].find("*/") {
                Some(end) => cursor.position += end + 4,
                None => {
                    cursor.position = source.len();
                    diagnostics.push(Diagnostic::new(
                        "unterminated block comment",
                        Span::from_range(&source_file, TextRange::new(start, start + 2)),
                    ));
                }
            }
            continue;
        }

        let kind = if ch == '"' {
            match lex_string(&mut cursor) {
                Some(value) => TokenKind::String(value),
                None => {
                    diagnostics.push(Diagnostic::new(
                        "unterminated string literal",
                        Span::from_range(&source_file, TextRange::new(start, cursor.position())),
                    ));
                    continue;
                }
            }
        } else if ch.is_ascii_digit() {
            match lex_number(&mut cursor) {
                Some(kind) => kind,
                None => {
                    diagnostics.push(Diagnostic::new(
                        "invalid number literal",
                        Span::from_range(&source_file, TextRange::new(start, cursor.position())),
                    ));
                    continue;
                }
            }
        } else if is_ident_start(ch) {
            cursor.consume_while(is_ident_continue);
            match &source[start..cursor.position()] {
                "return" => TokenKind::Return,
                "if" => TokenKind::If,
                "else" => TokenKind::Else,
                "while" => TokenKind::While,
                "for" => TokenKind::For,
                "break" => TokenKind::Break,
                "continue" => TokenKind::Continue,
                "async" => TokenKind::Async,
                "new" => TokenKind::New,
                "do" => TokenKind::Do,
                "true" => TokenKind::True,
                "false" => TokenKind::False,
                raw => TokenKind::Identifier(raw.to_string()),
            }
        } else {
            match lex_punct(&mut cursor) {
                Some(kind) => kind,
                None => {
                    cursor.bump();
                    diagnostics.push(Diagnostic::new(
                        format!("unexpected character '{}'", ch),
                        Span::from_range(&source_file, TextRange::new(start, cursor.position())),
                    ));
                    continue;
                }
            }
        };
        push_token(
            &mut tokens,
            &source_file,
            kind,
            TextRange::new(start, cursor.position()),
        );
    }

    let eof_range = TextRange::new(cursor.position(), cursor.position());
    push_token(&mut tokens, &source_file, TokenKind::Eof, eof_range);
    diagnostics.into_result(tokens)
}

/// Longest match first, so `+=` wins over `+`.
const PUNCTUATION: &[(&str, TokenKind)] = &[
    ("&&", TokenKind::AndAnd),
    ("||", TokenKind::OrOr),
    ("->", TokenKind::Arrow),
    ("==", TokenKind::EqEq),
    ("!=", TokenKind::BangEq),
    ("<=", TokenKind::Lte),
    (">=", TokenKind::Gte),
    ("+=", TokenKind::PlusAssign),
    ("-=", TokenKind::MinusAssign),
    ("*=", TokenKind::StarAssign),
    ("/=", TokenKind::SlashAssign),
    ("%=", TokenKind::PercentAssign),
    ("++", TokenKind::PlusPlus),
    ("--", TokenKind::MinusMinus),
    ("!", TokenKind::Bang),
    ("?", TokenKind::Question),
    (":", TokenKind::Colon),
    (";", TokenKind::Semicolon),
    (",", TokenKind::Comma),
    (".", TokenKind::Dot),
    ("@", TokenKind::At),
    ("(", TokenKind::LeftParen),
    (")", TokenKind::RightParen),
    ("[", TokenKind::LeftBracket),
    ("]", TokenKind::RightBracket),
    ("{", TokenKind::LeftBrace),
    ("}", TokenKind::RightBrace),
    ("=", TokenKind::Assign),
    ("+", TokenKind::Plus),
    ("-", TokenKind::Minus),
    ("*", TokenKind::Star),
    ("/", TokenKind::Slash),
    ("%", TokenKind::Percent),
    ("<", TokenKind::Lt),
    (">", TokenKind::Gt),
];

fn lex_punct(cursor: &mut Cursor<'_>) -> Option<TokenKind> {
    let (text, kind) = PUNCTUATION
        .iter()
        .find(|(text, _)| cursor.rest().starts_with(text))?;
    cursor.position += text.len();
    Some(kind.clone())
}

/// Java number literals: `0xFF`, `1_000`, `1.5`, `1.5f`, `2f`. Floats keep their
/// source text without `_` or the suffix.
fn lex_number(cursor: &mut Cursor<'_>) -> Option<TokenKind> {
    let start = cursor.position();
    if cursor.rest().starts_with("0x") || cursor.rest().starts_with("0X") {
        cursor.position += 2;
        cursor.consume_while(|next| next.is_ascii_hexdigit() || next == '_');
        let digits = cursor.source[start + 2..cursor.position()].replace('_', "");
        return i64::from_str_radix(&digits, 16)
            .ok()
            .map(TokenKind::Integer);
    }
    let digit_or_underscore = |next: char| next.is_ascii_digit() || next == '_';
    cursor.consume_while(digit_or_underscore);
    let rest = cursor.rest();
    let mut is_float =
        rest.starts_with('.') && rest[1..].starts_with(|next: char| next.is_ascii_digit());
    if is_float {
        cursor.bump();
        cursor.consume_while(digit_or_underscore);
    }
    let raw = cursor.source[start..cursor.position()].replace('_', "");
    if cursor.rest().starts_with(['f', 'F']) {
        cursor.bump();
        is_float = true;
    }
    if cursor.rest().starts_with(is_ident_continue) {
        cursor.consume_while(is_ident_continue);
        return None;
    }
    if raw.ends_with('_') {
        return None;
    }
    if is_float {
        let raw = if raw.contains('.') {
            raw
        } else {
            format!("{raw}.0")
        };
        Some(TokenKind::Float(raw))
    } else {
        raw.parse().ok().map(TokenKind::Integer)
    }
}

/// Reads a `"..."` literal; `None` when it is unterminated.
fn lex_string(cursor: &mut Cursor<'_>) -> Option<String> {
    cursor.bump();
    let mut value = String::new();
    while let Some(next) = cursor.bump() {
        match next {
            '"' => return Some(value),
            '\n' => return None,
            '\\' => match cursor.bump()? {
                'n' => value.push('\n'),
                't' => value.push('\t'),
                other => value.push(other),
            },
            other => value.push(other),
        }
    }
    None
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

    fn rest(&self) -> &'a str {
        &self.source[self.position..]
    }

    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
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
}

#[cfg(test)]
mod tests {
    use super::{TokenKind, lex};

    fn kinds(source: &str) -> Vec<TokenKind> {
        lex(source)
            .unwrap()
            .into_iter()
            .map(|token| token.kind)
            .collect()
    }

    #[test]
    fn skips_line_and_block_comments() {
        let kinds = kinds("void main() { // trailing\n/* block\n comment */ }\n");
        assert_eq!(kinds.len(), 7);
        assert!(matches!(kinds.last(), Some(TokenKind::Eof)));
    }

    #[test]
    fn lexes_compound_operators_longest_first() {
        let kinds = kinds("i++ x += 1 a && !b || c -> d != e");
        assert!(kinds.contains(&TokenKind::PlusPlus));
        assert!(kinds.contains(&TokenKind::PlusAssign));
        assert!(kinds.contains(&TokenKind::AndAnd));
        assert!(kinds.contains(&TokenKind::Bang));
        assert!(kinds.contains(&TokenKind::OrOr));
        assert!(kinds.contains(&TokenKind::Arrow));
        assert!(kinds.contains(&TokenKind::BangEq));
    }

    #[test]
    fn reports_malformed_tokens_without_stopping() {
        let error = lex("#\n\"abc\n/* open").unwrap_err();
        let rendered = error.to_string();
        assert!(rendered.contains("unexpected character '#'"));
        assert!(rendered.contains("unterminated string literal"));
        assert!(rendered.contains("unterminated block comment"));
    }

    #[test]
    fn lexes_string_escapes() {
        let strings: Vec<_> = kinds(r#""say \"hi\"" "a\nb""#)
            .into_iter()
            .filter_map(|kind| match kind {
                TokenKind::String(value) => Some(value),
                _ => None,
            })
            .collect();
        assert_eq!(strings, vec!["say \"hi\"", "a\nb"]);
    }
}
