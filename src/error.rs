//! Compiler errors with colored source pointers.
//!
//! `RavenError` is the top level error enum for every compiler stage. Lex
//! and parse errors are populated; resolve, type, and runtime errors land
//! as new variants when the corresponding stages are built.
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

/// Parsing errors raised by the recursive descent parser.
///
/// Most errors render as "expected X, found Y" with the span pointing at
/// the offending token. Specific variants exist for common shapes so that
/// downstream tooling can match on them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// The next token did not match what the parser expected. `expected`
    /// describes the syntactic category (e.g. "`{`", "type", "identifier")
    /// and `found` quotes the actual token kind.
    UnexpectedToken { expected: String, found: String },
    /// End of file reached while expecting more input.
    UnexpectedEof { expected: String },
    /// The left hand side of an assignment is not a valid place
    /// expression.
    InvalidAssignmentTarget,
    /// Comparison operators are not chainable: `a < b < c` is rejected.
    ChainedComparison,
    /// A struct or enum literal repeats a field name.
    DuplicateField(String),
    /// An `import` directive that the parser could not interpret.
    InvalidImportPath,
    /// Tuple syntax is parsed but not yet supported in v2.0.
    UnsupportedTuple,
    /// A pattern fragment that cannot be parsed.
    InvalidPattern(String),
    /// A bespoke error message for situations that do not fit the
    /// dedicated variants above.
    Custom(String),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::UnexpectedToken { expected, found } => {
                write!(f, "expected {}, found {}", expected, found)
            }
            ParseError::UnexpectedEof { expected } => {
                write!(f, "expected {}, found end of file", expected)
            }
            ParseError::InvalidAssignmentTarget => {
                write!(f, "invalid assignment target")
            }
            ParseError::ChainedComparison => {
                write!(f, "comparison operators cannot be chained")
            }
            ParseError::DuplicateField(name) => {
                write!(f, "duplicate field '{}'", name)
            }
            ParseError::InvalidImportPath => {
                write!(f, "invalid import path")
            }
            ParseError::UnsupportedTuple => {
                write!(f, "tuple expressions are not yet supported")
            }
            ParseError::InvalidPattern(msg) => {
                write!(f, "invalid pattern: {}", msg)
            }
            ParseError::Custom(msg) => f.write_str(msg),
        }
    }
}

/// Name resolution errors raised by the resolver.
///
/// All variants carry the offending span on the outer `RavenError::Resolve`
/// wrapper; payload data here is just enough to format a human readable
/// message. See `docs/v2/specs/resolver.md` for the full catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    /// An identifier could not be found in any enclosing scope.
    UnresolvedName(String),
    /// Two declarations with the same name appear in the same scope.
    /// `first_span` points at the original declaration so the renderer can
    /// surface both locations if it chooses.
    DuplicateDeclaration { name: String, first_span: Span },
    /// An import path could not be resolved to a target.
    UnresolvedImport(String),
    /// The import graph contains a cycle that reaches the given path.
    CyclicImport(String),
    /// A name is visible from multiple import sources at the same scope.
    AmbiguousName { name: String, candidates: Vec<Span> },
    /// `self` or `Self` used outside an `impl` block.
    SelfOutsideImpl,
}

impl fmt::Display for ResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResolveError::UnresolvedName(name) => {
                write!(f, "cannot find `{}` in scope", name)
            }
            ResolveError::DuplicateDeclaration { name, .. } => {
                write!(f, "the name `{}` is declared multiple times", name)
            }
            ResolveError::UnresolvedImport(path) => {
                write!(f, "cannot resolve import `{}`", path)
            }
            ResolveError::CyclicImport(path) => {
                write!(f, "cyclic import detected involving `{}`", path)
            }
            ResolveError::AmbiguousName { name, .. } => {
                write!(f, "the name `{}` is ambiguous", name)
            }
            ResolveError::SelfOutsideImpl => {
                write!(f, "`self` or `Self` used outside an `impl` block")
            }
        }
    }
}

/// Top level compiler error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RavenError {
    /// A lexer error with the offending span and an optional hint.
    Lex(LexError, Span, Option<String>),
    /// A parser error with the offending span and an optional hint.
    Parse(ParseError, Span, Option<String>),
    /// A resolver error with the offending span and an optional hint.
    Resolve(ResolveError, Span, Option<String>),
}

impl RavenError {
    /// Construct a lex error.
    pub fn lex(kind: LexError, span: Span) -> Self {
        RavenError::Lex(kind, span, None)
    }

    /// Construct a parse error.
    pub fn parse(kind: ParseError, span: Span) -> Self {
        RavenError::Parse(kind, span, None)
    }

    /// Construct a resolve error.
    pub fn resolve(kind: ResolveError, span: Span) -> Self {
        RavenError::Resolve(kind, span, None)
    }

    /// Attach a hint string to this error.
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        match &mut self {
            RavenError::Lex(_, _, h) => *h = Some(hint.into()),
            RavenError::Parse(_, _, h) => *h = Some(hint.into()),
            RavenError::Resolve(_, _, h) => *h = Some(hint.into()),
        }
        self
    }

    /// The span associated with this error.
    pub fn span(&self) -> &Span {
        match self {
            RavenError::Lex(_, sp, _) => sp,
            RavenError::Parse(_, sp, _) => sp,
            RavenError::Resolve(_, sp, _) => sp,
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
            RavenError::Parse(k, s, h) => (format!("error: {}", k), s, h.as_deref()),
            RavenError::Resolve(k, s, h) => (format!("error: {}", k), s, h.as_deref()),
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
            RavenError::Parse(k, s, _) => write!(f, "{}: {}", s, k),
            RavenError::Resolve(k, s, _) => write!(f, "{}: {}", s, k),
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

    #[test]
    fn parse_error_renders_with_carets_and_hint() {
        let src = "fun add(a Int)\n";
        // The bad spot is the missing colon between `a` and `Int`.
        let span = Span::new(file(), 10, 13, 1, 11);
        let err = RavenError::parse(
            ParseError::UnexpectedToken {
                expected: "`:`".into(),
                found: "`Int`".into(),
            },
            span,
        )
        .with_hint("parameters need a type annotation: `a: Int`");
        let rendered = err.display(src);
        assert!(rendered.contains("expected `:`, found `Int`"));
        assert!(rendered.contains("fun add(a Int)"));
        assert!(rendered.contains('^'));
        assert!(rendered.contains("hint:"));
        assert!(rendered.contains("type annotation"));
    }

    #[test]
    fn parse_error_display_formats_location_then_kind() {
        let span = Span::new(file(), 0, 1, 2, 3);
        let err = RavenError::parse(ParseError::ChainedComparison, span);
        let s = format!("{}", err);
        assert_eq!(s, "test.rv:2:3: comparison operators cannot be chained");
    }

    #[test]
    fn resolve_error_renders_with_carets_and_hint() {
        let src = "fun main() { println(\"hi\") }\n";
        // `println` starts at byte 13, col 14.
        let span = Span::new(file(), 13, 20, 1, 14);
        let err = RavenError::resolve(ResolveError::UnresolvedName("println".into()), span)
            .with_hint("did you mean to `import std/io { println }`?");
        let rendered = err.display(src);
        assert!(rendered.contains("cannot find `println` in scope"));
        assert!(rendered.contains("fun main() { println"));
        assert!(rendered.contains('^'));
        assert!(rendered.contains("hint:"));
    }

    #[test]
    fn resolve_error_display_formats_location_then_kind() {
        let span = Span::new(file(), 0, 1, 4, 7);
        let err = RavenError::resolve(ResolveError::SelfOutsideImpl, span);
        let s = format!("{}", err);
        assert_eq!(
            s,
            "test.rv:4:7: `self` or `Self` used outside an `impl` block"
        );
    }
}
