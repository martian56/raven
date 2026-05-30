//! Compiler errors with colored source pointers.

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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    UnexpectedToken {
        expected: String,
        found: String,
    },
    UnexpectedEof {
        expected: String,
    },
    InvalidAssignmentTarget,
    /// Comparison operators are not chainable: `a < b < c` is rejected.
    ChainedComparison,
    DuplicateField(String),
    InvalidImportPath,
    /// Tuple syntax is parsed but not yet supported in v2.0.
    UnsupportedTuple,
    InvalidPattern(String),
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

/// Name resolution errors. See `docs/v2/specs/resolver.md` for the full catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    UnresolvedName(String),
    /// `first_span` points at the original declaration so the renderer can
    /// surface both locations.
    DuplicateDeclaration {
        name: String,
        first_span: Span,
    },
    UnresolvedImport(String),
    CyclicImport(String),
    AmbiguousName {
        name: String,
        candidates: Vec<Span>,
    },
    SelfOutsideImpl,
    /// A resolve-stage diagnostic that does not fit the structured variants,
    /// carrying its own message (for example a `@derive(...)` rejection).
    Other(String),
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
            ResolveError::Other(msg) => write!(f, "{}", msg),
        }
    }
}

/// Type checking errors. See `docs/v2/specs/tycheck.md` for the full catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeError {
    TypeMismatch {
        expected: String,
        actual: String,
    },
    UndefinedField {
        struct_name: String,
        field: String,
    },
    UndefinedMethod {
        receiver_ty: String,
        method: String,
    },
    AmbiguousMethod {
        receiver_ty: String,
        method: String,
        candidates: Vec<String>,
    },
    WrongArity {
        func: String,
        expected: usize,
        actual: usize,
    },
    NonExhaustiveMatch {
        missing: Vec<String>,
    },
    RedundantPattern,
    UnknownType(String),
    CannotInferType,
    /// Occurs check: a variable cannot unify with a type that contains it
    /// (`?T` with `List<?T>`).
    OccursCheck {
        var: String,
        ty: String,
    },
    BoundNotSatisfied {
        ty: String,
        trait_name: String,
    },
    GenericArityMismatch {
        decl: String,
        expected: usize,
        actual: usize,
    },
    OverlappingImpls {
        ty: String,
        trait_name: String,
        candidates: Vec<String>,
    },
    NotCallable(String),
    Custom(String),
}

impl fmt::Display for TypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TypeError::TypeMismatch { expected, actual } => {
                write!(
                    f,
                    "type mismatch: expected `{}`, found `{}`",
                    expected, actual
                )
            }
            TypeError::UndefinedField { struct_name, field } => {
                write!(f, "struct `{}` has no field `{}`", struct_name, field)
            }
            TypeError::UndefinedMethod {
                receiver_ty,
                method,
            } => write!(f, "no method `{}` found for type `{}`", method, receiver_ty),
            TypeError::AmbiguousMethod {
                receiver_ty,
                method,
                candidates,
            } => write!(
                f,
                "ambiguous method `{}` on `{}` (candidates: {})",
                method,
                receiver_ty,
                candidates.join(", ")
            ),
            TypeError::WrongArity {
                func,
                expected,
                actual,
            } => write!(
                f,
                "wrong number of arguments to `{}`: expected {}, found {}",
                func, expected, actual
            ),
            TypeError::NonExhaustiveMatch { missing } => write!(
                f,
                "non exhaustive match: missing {}",
                missing
                    .iter()
                    .map(|s| format!("`{}`", s))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            TypeError::RedundantPattern => {
                write!(f, "unreachable pattern, shadowed by an earlier arm")
            }
            TypeError::UnknownType(name) => write!(f, "unknown type `{}`", name),
            TypeError::CannotInferType => write!(
                f,
                "cannot infer the type of this expression; an annotation is needed"
            ),
            TypeError::OccursCheck { var, ty } => write!(
                f,
                "occurs check failed: `{}` cannot equal `{}` (it contains itself)",
                var, ty
            ),
            TypeError::BoundNotSatisfied { ty, trait_name } => write!(
                f,
                "the trait bound `{}: {}` is not satisfied",
                ty, trait_name
            ),
            TypeError::GenericArityMismatch {
                decl,
                expected,
                actual,
            } => write!(
                f,
                "`{}` takes {} type argument(s), but {} were supplied",
                decl, expected, actual
            ),
            TypeError::OverlappingImpls {
                ty,
                trait_name,
                candidates,
            } => write!(
                f,
                "overlapping impls of `{}` for `{}` (candidates: {})",
                trait_name,
                ty,
                candidates.join(", ")
            ),
            TypeError::NotCallable(actual) => {
                write!(f, "values of type `{}` are not callable", actual)
            }
            TypeError::Custom(msg) => f.write_str(msg),
        }
    }
}

/// Top level compiler error.
///
/// The type error variant is boxed because `TypeError` carries richer
/// payloads than the other stages (lists of candidate names, structured
/// fields). Boxing keeps `RavenError` itself small so that pervasive
/// `Result<_, RavenError>` returns elsewhere in the compiler do not
/// inflate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RavenError {
    /// A lexer error with the offending span and an optional hint.
    Lex(LexError, Span, Option<String>),
    /// A parser error with the offending span and an optional hint.
    Parse(ParseError, Span, Option<String>),
    /// A resolver error with the offending span and an optional hint.
    Resolve(ResolveError, Span, Option<String>),
    /// A type checker error with the offending span and an optional
    /// hint. Boxed to keep the enum compact.
    Type(Box<TypeError>, Span, Option<String>),
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

    /// Construct a type error.
    pub fn ty(kind: TypeError, span: Span) -> Self {
        RavenError::Type(Box::new(kind), span, None)
    }

    /// Attach a hint string to this error.
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        match &mut self {
            RavenError::Lex(_, _, h) => *h = Some(hint.into()),
            RavenError::Parse(_, _, h) => *h = Some(hint.into()),
            RavenError::Resolve(_, _, h) => *h = Some(hint.into()),
            RavenError::Type(_, _, h) => *h = Some(hint.into()),
        }
        self
    }

    /// The span associated with this error.
    pub fn span(&self) -> &Span {
        match self {
            RavenError::Lex(_, sp, _) => sp,
            RavenError::Parse(_, sp, _) => sp,
            RavenError::Resolve(_, sp, _) => sp,
            RavenError::Type(_, sp, _) => sp,
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
            RavenError::Type(k, s, h) => (format!("error: {}", k), s, h.as_deref()),
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
            RavenError::Type(k, s, _) => write!(f, "{}: {}", s, k),
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

    #[test]
    fn type_error_renders_with_carets_and_hint() {
        let src = "let x: Int = 1.5\n";
        // The `1.5` literal sits at byte 13, line 1, col 14.
        let span = Span::new(file(), 13, 16, 1, 14);
        let err = RavenError::ty(
            TypeError::TypeMismatch {
                expected: "Int".into(),
                actual: "Float".into(),
            },
            span,
        )
        .with_hint("did you mean to call `.to_int()`?");
        let rendered = err.display(src);
        assert!(rendered.contains("expected `Int`, found `Float`"));
        assert!(rendered.contains("let x: Int = 1.5"));
        assert!(rendered.contains('^'));
        assert!(rendered.contains("hint:"));
    }

    #[test]
    fn type_error_display_formats_location_then_kind() {
        let span = Span::new(file(), 0, 1, 7, 2);
        let err = RavenError::ty(
            TypeError::UndefinedField {
                struct_name: "Point".into(),
                field: "z".into(),
            },
            span,
        );
        let s = format!("{}", err);
        assert_eq!(s, "test.rv:7:2: struct `Point` has no field `z`");
    }
}
