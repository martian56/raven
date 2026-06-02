//! Type expressions.
//!
//! Types appear in parameter annotations, return types, struct fields,
//! variant payloads, `let` annotations, and `const` declarations. The
//! parser produces this tree exactly as written; later passes (resolver,
//! type checker) normalize and check it.

use crate::span::Span;

/// A type expression.
#[derive(Debug, Clone, PartialEq)]
pub enum TypeKind {
    /// A qualified path with optional generic arguments at each segment:
    /// `Map<K, V>`, `std.collections.Map<String, Int>`.
    Path(TypePath),
    /// `T?` sugar for `Option<T>`.
    Optional(Box<Type>),
    /// `dyn Trait`.
    Dyn(TypePath),
    /// The unit type `()`.
    Unit,
    /// `fun(A, B) -> C` function type.
    Function { params: Vec<Type>, ret: Box<Type> },
}

/// Top level `Type` wrapper with span.
#[derive(Debug, Clone, PartialEq)]
pub struct Type {
    pub kind: TypeKind,
    pub span: Span,
}

/// A dot separated identifier path with optional generic args at each
/// segment. The segments vector is always non empty.
#[derive(Debug, Clone, PartialEq)]
pub struct TypePath {
    pub segments: Vec<TypePathSegment>,
    pub span: Span,
}

/// One segment of a qualified type path, with its identifier name and
/// any generic arguments.
#[derive(Debug, Clone, PartialEq)]
pub struct TypePathSegment {
    pub name: String,
    pub generics: Vec<Type>,
    pub span: Span,
}
