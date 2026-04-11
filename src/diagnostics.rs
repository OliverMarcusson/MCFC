use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextRange {
    pub start: usize,
    pub end: usize,
}

impl TextRange {
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }
}

#[derive(Debug, Clone)]
pub struct SourceFile<'a> {
    source: &'a str,
    line_starts: Vec<usize>,
}

impl<'a> SourceFile<'a> {
    pub fn new(source: &'a str) -> Self {
        let mut line_starts = vec![0];
        for (index, ch) in source.char_indices() {
            if ch == '\n' {
                line_starts.push(index + ch.len_utf8());
            }
        }
        Self {
            source,
            line_starts,
        }
    }

    pub fn position(&self, offset: usize) -> (usize, usize) {
        let offset = offset.min(self.source.len());
        let line_index = match self.line_starts.binary_search(&offset) {
            Ok(index) => index,
            Err(index) => index.saturating_sub(1),
        };
        let line_start = self.line_starts[line_index];
        let column = self.source[line_start..offset].chars().count() + 1;
        (line_index + 1, column)
    }

    pub fn line_text(&self, line: usize) -> &'a str {
        let Some(&start) = self.line_starts.get(line.saturating_sub(1)) else {
            return "";
        };
        let end = self
            .line_starts
            .get(line)
            .copied()
            .unwrap_or(self.source.len());
        self.source[start..end].trim_end_matches(['\r', '\n'])
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub line: usize,
    pub column: usize,
    pub range: TextRange,
}

impl Span {
    pub fn new(line: usize, column: usize) -> Self {
        Self {
            line,
            column,
            range: TextRange::new(0, 0),
        }
    }

    pub fn with_range(line: usize, column: usize, start: usize, end: usize) -> Self {
        Self {
            line,
            column,
            range: TextRange::new(start, end),
        }
    }

    pub fn from_range(source: &SourceFile<'_>, range: TextRange) -> Self {
        let (line, column) = source.position(range.start);
        Self {
            line,
            column,
            range,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub message: String,
    pub span: Span,
}

impl Diagnostic {
    pub fn new(message: impl Into<String>, span: Span) -> Self {
        Self {
            message: message.into(),
            span,
        }
    }

    pub fn render(&self, source: &str) -> String {
        let source_file = SourceFile::new(source);
        let (line, column) = if self.span.range.start == self.span.range.end {
            (self.span.line, self.span.column)
        } else {
            source_file.position(self.span.range.start)
        };
        let source_line = source_file.line_text(line);
        format!(
            "error:{}:{}: {}\n{:>6} | {}\n       | {}^",
            line,
            column,
            self.message,
            line,
            source_line,
            " ".repeat(column.saturating_sub(1))
        )
    }
}

#[derive(Debug, Clone)]
pub struct Diagnostics(pub Vec<Diagnostic>);

impl Diagnostics {
    pub fn new() -> Self {
        Self(Vec::new())
    }

    pub fn push(&mut self, diagnostic: Diagnostic) {
        self.0.push(diagnostic);
    }

    pub fn into_result<T>(self, value: T) -> Result<T, Self> {
        if self.0.is_empty() {
            Ok(value)
        } else {
            Err(self)
        }
    }
}

impl fmt::Display for Diagnostics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, diagnostic) in self.0.iter().enumerate() {
            if index > 0 {
                writeln!(f)?;
            }
            write!(
                f,
                "error:{}:{}: {}",
                diagnostic.span.line, diagnostic.span.column, diagnostic.message
            )?;
        }
        Ok(())
    }
}

impl std::error::Error for Diagnostics {}
