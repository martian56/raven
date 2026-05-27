//! Compiler errors with colored source pointers.
//!
//! `RavenError` is the top level error enum for every compiler stage. In
//! Phase 1 only `LexError` is populated; parse and type errors will be added
//! in their respective phases as new variants.
//!
//! `RavenError::display(source)` renders a multi line message: a red header,
//! the offending source line, a row of red carets under the bad span, and an
//! optional dim hint line.

use std::fmt;

use crate::span::Span;

/// Lexical analysis errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LexError {
    /// A character that does not begin any valid token.
    UnexpectedChar(char),
    /// A `"..."` string ran off the end of the source or the line.
    UnterminatedString,
    /// A `"""..."""` block string ran off the end of the source.
    UnterminatedBlockString,
    /// A `/* ... */` block comment was not closed.
    UnterminatedBlockComment,
    /// An unknown escape sequence like `\q`.
    InvalidEscape(char),
    /// A malformed `\u{...}` or `\x..` escape.
    InvalidUnicodeEscape,
    /// A numeric literal could not be parsed (overflow, empty digits, etc.).
    /// The string carries the failing lexeme for diagnostics.
    InvalidNumber(String),
    /// A `'...'` char literal was empty, multi character, or unterminated.
    InvalidCharLit(String),
}

impl fmt::Display for LexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LexError::UnexpectedChar(c) => write!(f, "unexpected character {:?}", c),
            LexError::UnterminatedString => write!(f, "unterminated string literal"),
            LexError::UnterminatedBlockString => write!(f, "unterminated block string literal"),
            LexError::UnterminatedBlockComment => write!(f, "unterminated block comment"),
            LexError::InvalidEscape(c) => write!(f, "invalid escape sequence '\\{}'", c),
            LexError::InvalidUnicodeEscape => write!(f, "invalid unicode escape"),
            LexError::InvalidNumber(s) => write!(f, "invalid numeric literal '{}'", s),
            LexError::InvalidCharLit(s) => write!(f, "invalid character literal: {}", s),
        }
    }
}

/// Top level compiler error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RavenError {
    /// A lexer error with the offending span and an optional hint.
    Lex(LexError, Span, Option<String>),
}

impl RavenError {
    /// Construct a lex error.
    pub fn lex(kind: LexError, span: Span) -> Self {
        RavenError::Lex(kind, span, None)
    }

    /// Attach a hint string to this error.
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        match &mut self {
            RavenError::Lex(_, _, h) => *h = Some(hint.into()),
        }
        self
    }

    /// The span associated with this error.
    pub fn span(&self) -> &Span {
        match self {
            RavenError::Lex(_, sp, _) => sp,
        }
    }

    /// Render this error as a colored, multi line message anchored at the
    /// offending span. ANSI escapes are used for color: red for the header
    /// and carets, dim for the hint and gutter.
    pub fn display(&self, source: &str) -> String {
        const RED: &str = "\x1b[31;1m";
        const DIM: &str = "\x1b[2m";
        const RESET: &str = "\x1b[0m";

        let (kind_str, span, hint) = match self {
            RavenError::Lex(k, s, h) => (format!("error: {}", k), s, h.as_deref()),
        };

        let mut out = String::new();
        out.push_str(RED);
        out.push_str(&kind_str);
        out.push_str(RESET);
        out.push('\n');
        out.push_str(DIM);
        out.push_str("  --> ");
        out.push_str(RESET);
        out.push_str(&format!("{}", span));
        out.push('\n');

        // Find the source line containing `span.start`.
        let line_text = source.lines().nth((span.line.saturating_sub(1)) as usize);
        if let Some(text) = line_text {
            let gutter = format!("{:>4} | ", span.line);
            out.push_str(DIM);
            out.push_str(&gutter);
            out.push_str(RESET);
            out.push_str(text);
            out.push('\n');

            // Caret row. col is 1 indexed and counted in chars; we use the
            // same char count for the underline so wide ASCII is handled
            // correctly for the common case.
            let pad_chars = (span.col.saturating_sub(1)) as usize;
            let caret_count = std::cmp::max(1, span.len());
            let mut caret_line = String::new();
            caret_line.push_str(DIM);
            caret_line.push_str("     | ");
            caret_line.push_str(RESET);
            for _ in 0..pad_chars {
                caret_line.push(' ');
            }
            caret_line.push_str(RED);
            for _ in 0..caret_count {
                caret_line.push('^');
            }
            caret_line.push_str(RESET);
            out.push_str(&caret_line);
            out.push('\n');
        }

        if let Some(h) = hint {
            out.push_str(DIM);
            out.push_str("  = hint: ");
            out.push_str(h);
            out.push_str(RESET);
            out.push('\n');
        }

        out
    }
}

impl fmt::Display for RavenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RavenError::Lex(k, s, _) => write!(f, "{}: {}", s, k),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn file() -> Arc<PathBuf> {
        Arc::new(PathBuf::from("test.rv"))
    }

    #[test]
    fn unexpected_char_renders_pointer() {
        let src = "let x = @\nlet y = 1\n";
        // The `@` is at byte 8, line 1, col 9.
        let span = Span::new(file(), 8, 9, 1, 9);
        let err = RavenError::lex(LexError::UnexpectedChar('@'), span);
        let rendered = err.display(src);

        // Header has the error text.
        assert!(rendered.contains("unexpected character"));
        // Source line is included.
        assert!(rendered.contains("let x = @"));
        // Caret row is present.
        assert!(rendered.contains('^'));
        // File location is included.
        assert!(rendered.contains("test.rv:1:9"));
    }

    #[test]
    fn hint_is_rendered_when_present() {
        let src = "let x = 0xZ\n";
        let span = Span::new(file(), 8, 11, 1, 9);
        let err = RavenError::lex(LexError::InvalidNumber("0xZ".into()), span)
            .with_hint("hexadecimal literals must contain only 0-9 and a-f");
        let rendered = err.display(src);
        assert!(rendered.contains("hint:"));
        assert!(rendered.contains("hexadecimal literals"));
    }

    #[test]
    fn display_formats_location_then_kind() {
        let span = Span::new(file(), 0, 1, 3, 5);
        let err = RavenError::lex(LexError::UnterminatedString, span);
        let s = format!("{}", err);
        assert_eq!(s, "test.rv:3:5: unterminated string literal");
    }
}
