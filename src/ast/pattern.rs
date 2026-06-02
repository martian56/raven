//! Match and binding patterns.
//!
//! Patterns are used in `match` arms, `for ... in ...` heads, and `let`
//! bindings (the latter restricted at parse time to a simple identifier
//! in this release; richer destructuring lands later). The parser does
//! not resolve names, so an identifier pattern may bind a fresh variable
//! or refer to an enum constructor; that decision lives in the resolver.

use crate::span::Span;

/// Kinds of pattern.
#[derive(Debug, Clone, PartialEq)]
pub enum PatternKind {
    /// The wildcard `_` pattern: matches anything, binds nothing.
    Wildcard,
    /// A literal value pattern: `42`, `"hi"`, `'c'`, `true`.
    Literal(LiteralPattern),
    /// A bare identifier: binds a name, or names an enum constructor
    /// when resolved.
    Ident(String),
    /// `Name(p1, p2, ...)`: an enum tuple variant or a parenthesized
    /// pattern list.
    Tuple {
        name: Option<String>,
        elements: Vec<Pattern>,
    },
    /// `Name { f1, f2: p2, ... }`: a struct or struct enum variant.
    Struct {
        name: String,
        fields: Vec<FieldPattern>,
    },
    /// `lo..hi` or `lo..=hi` integer range pattern.
    Range { lo: i64, hi: i64, inclusive: bool },
}

/// A pattern with its source span.
#[derive(Debug, Clone, PartialEq)]
pub struct Pattern {
    pub kind: PatternKind,
    pub span: Span,
}

/// A literal pattern's parsed value.
#[derive(Debug, Clone, PartialEq)]
pub enum LiteralPattern {
    Int(i64),
    Float(f64),
    Bool(bool),
    String(String),
    Char(char),
}

/// One field in a struct pattern. `pattern` is `None` for shorthand
/// `{ name }`, meaning `{ name: name }`.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldPattern {
    pub name: String,
    pub pattern: Option<Pattern>,
    pub span: Span,
}
