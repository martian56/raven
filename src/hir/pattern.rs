//! Patterns used in HIR `match` arms and `let` bindings.
//!
//! Patterns are a flat subset of the AST patterns: wildcard, literal,
//! identifier binding, named constructor with positional or named
//! fields, and integer range. Or-patterns are not supported yet; the
//! parser does not produce them.

use crate::span::Span;

/// A literal value usable in a pattern.
#[derive(Debug, Clone, PartialEq)]
pub enum HirLiteralPat {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    Char(char),
}

/// One named field inside a struct pattern. `pattern` is `None` for
/// shorthand `{ name }` style fields.
#[derive(Debug, Clone)]
pub struct HirFieldPat {
    pub name: String,
    pub pattern: Option<HirPattern>,
    pub span: Span,
}

/// A pattern node.
#[derive(Debug, Clone)]
pub struct HirPattern {
    pub kind: HirPatternKind,
    pub span: Span,
}

/// Pattern node kinds.
#[derive(Debug, Clone)]
pub enum HirPatternKind {
    /// `_`. Matches anything, binds nothing.
    Wildcard,
    /// A literal value pattern.
    Literal(HirLiteralPat),
    /// A bare identifier. After lowering this is always a fresh binding
    /// (constructor identifiers are lifted into `Constructor` form).
    Binding(String),
    /// `Name(p1, p2, ...)`: a constructor with positional arguments. The
    /// `name` is `None` for a bare parenthesized tuple pattern.
    Constructor {
        name: Option<String>,
        elements: Vec<HirPattern>,
    },
    /// `Name { f1, f2: p2, ... }`: a struct or struct-shaped enum
    /// variant.
    Struct {
        name: String,
        fields: Vec<HirFieldPat>,
    },
    /// `lo..hi` or `lo..=hi` integer range.
    Range { lo: i64, hi: i64, inclusive: bool },
}
