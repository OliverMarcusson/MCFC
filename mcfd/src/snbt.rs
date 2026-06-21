//! A small, tolerant SNBT reader for the request envelopes Minecraft renders into
//! the log, plus a writer helper for emitting result compounds. It handles the
//! subset we produce: compounds, lists, quoted strings, and numbers.

use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub enum Value {
    Compound(BTreeMap<String, Value>),
    List(Vec<Value>),
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
}

impl Value {
    pub fn as_compound(&self) -> Option<&BTreeMap<String, Value>> {
        match self {
            Value::Compound(map) => Some(map),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_int(&self) -> Option<i64> {
        match self {
            Value::Int(value) => Some(*value),
            Value::Bool(value) => Some(*value as i64),
            Value::Float(value) => Some(*value as i64),
            _ => None,
        }
    }

    /// Render an argument as a plain string for handlers (numbers stringified).
    pub fn to_arg_string(&self) -> String {
        match self {
            Value::Str(value) => value.clone(),
            Value::Int(value) => value.to_string(),
            Value::Float(value) => value.to_string(),
            Value::Bool(value) => value.to_string(),
            _ => String::new(),
        }
    }
}

/// Parse a compound beginning at the leading `{` of `input`.
pub fn parse_compound(input: &str) -> Option<Value> {
    let chars: Vec<char> = input.chars().collect();
    let mut parser = Parser { chars, pos: 0 };
    parser.skip_ws();
    let value = parser.parse_value()?;
    Some(value)
}

struct Parser {
    chars: Vec<char>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let ch = self.peek();
        if ch.is_some() {
            self.pos += 1;
        }
        ch
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(ch) if ch.is_whitespace()) {
            self.pos += 1;
        }
    }

    fn parse_value(&mut self) -> Option<Value> {
        self.skip_ws();
        match self.peek()? {
            '{' => self.parse_compound_value(),
            '[' => self.parse_list(),
            '"' | '\'' => self.parse_quoted().map(Value::Str),
            _ => Some(self.parse_scalar()),
        }
    }

    fn parse_compound_value(&mut self) -> Option<Value> {
        self.bump(); // consume '{'
        let mut map = BTreeMap::new();
        loop {
            self.skip_ws();
            match self.peek()? {
                '}' => {
                    self.bump();
                    break;
                }
                _ => {
                    let key = self.parse_key()?;
                    self.skip_ws();
                    if self.peek()? != ':' {
                        return None;
                    }
                    self.bump(); // ':'
                    let value = self.parse_value()?;
                    map.insert(key, value);
                    self.skip_ws();
                    if self.peek() == Some(',') {
                        self.bump();
                    }
                }
            }
        }
        Some(Value::Compound(map))
    }

    fn parse_list(&mut self) -> Option<Value> {
        self.bump(); // consume '['
                     // Skip a typed-array prefix like `I;` or `B;`.
        let save = self.pos;
        self.skip_ws();
        if matches!(self.peek(), Some(c) if c.is_ascii_alphabetic()) {
            let marker = self.pos;
            self.bump();
            if self.peek() == Some(';') {
                self.bump();
            } else {
                self.pos = marker;
            }
        } else {
            self.pos = save;
        }
        let mut items = Vec::new();
        loop {
            self.skip_ws();
            match self.peek()? {
                ']' => {
                    self.bump();
                    break;
                }
                _ => {
                    let value = self.parse_value()?;
                    items.push(value);
                    self.skip_ws();
                    if self.peek() == Some(',') {
                        self.bump();
                    }
                }
            }
        }
        Some(Value::List(items))
    }

    fn parse_key(&mut self) -> Option<String> {
        self.skip_ws();
        match self.peek()? {
            '"' | '\'' => self.parse_quoted(),
            _ => {
                let mut key = String::new();
                while let Some(ch) = self.peek() {
                    if ch == ':' || ch.is_whitespace() {
                        break;
                    }
                    key.push(ch);
                    self.bump();
                }
                if key.is_empty() {
                    None
                } else {
                    Some(key)
                }
            }
        }
    }

    fn parse_quoted(&mut self) -> Option<String> {
        let quote = self.bump()?; // opening quote
        let mut out = String::new();
        while let Some(ch) = self.bump() {
            match ch {
                '\\' => {
                    if let Some(escaped) = self.bump() {
                        match escaped {
                            'n' => out.push('\n'),
                            't' => out.push('\t'),
                            'r' => out.push('\r'),
                            other => out.push(other),
                        }
                    }
                }
                c if c == quote => return Some(out),
                c => out.push(c),
            }
        }
        Some(out)
    }

    fn parse_scalar(&mut self) -> Value {
        let mut token = String::new();
        while let Some(ch) = self.peek() {
            if ch == ',' || ch == '}' || ch == ']' || ch.is_whitespace() {
                break;
            }
            token.push(ch);
            self.bump();
        }
        classify_scalar(&token)
    }
}

fn classify_scalar(token: &str) -> Value {
    match token {
        "true" => return Value::Bool(true),
        "false" => return Value::Bool(false),
        _ => {}
    }
    // Strip a single trailing numeric type suffix (b, s, l, f, d).
    let trimmed = token.trim_end_matches(['b', 's', 'l', 'L', 'f', 'd', 'B', 'S', 'F', 'D']);
    if token.contains('.') || token.ends_with(['f', 'd', 'F', 'D']) {
        if let Ok(value) = trimmed.parse::<f64>() {
            return Value::Float(value);
        }
    }
    if let Ok(value) = trimmed.parse::<i64>() {
        return Value::Int(value);
    }
    Value::Str(token.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_request_envelope() {
        let input =
            r#"{id: 7, v: 1, mod: "http", fn: "get", args: ["https://api.example.com/data"]}"#;
        let value = parse_compound(input).expect("should parse");
        let map = value.as_compound().expect("compound");
        assert_eq!(map.get("id").and_then(Value::as_int), Some(7));
        assert_eq!(map.get("mod").and_then(Value::as_str), Some("http"));
        assert_eq!(map.get("fn").and_then(Value::as_str), Some("get"));
        match map.get("args") {
            Some(Value::List(items)) => {
                assert_eq!(items[0].as_str(), Some("https://api.example.com/data"));
            }
            other => panic!("expected list, got {:?}", other),
        }
    }

    #[test]
    fn handles_byte_suffix_and_nested() {
        let input = r#"{id:3b, args:[1, 2, 3], flag: true}"#;
        let map = parse_compound(input)
            .unwrap()
            .as_compound()
            .unwrap()
            .clone();
        assert_eq!(map.get("id").and_then(Value::as_int), Some(3));
        assert!(matches!(map.get("flag"), Some(Value::Bool(true))));
    }

    #[test]
    fn escapes_for_single_line_command() {
        assert_eq!(escape_string("a\"b\\c"), "\"a\\\"b\\\\c\"");
        assert_eq!(escape_string("line1\nline2"), "\"line1 line2\"");
    }
}

/// Quote and escape a string for emission inside a single-line `.mcfunction`
/// command. Control characters are flattened to spaces so the command stays valid.
pub fn escape_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' | '\r' | '\t' => out.push(' '),
            c if (c as u32) < 0x20 => out.push(' '),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
