//! Abstract Syntax Tree for the Raven v2 grammar.
//!
//! Each submodule covers one category of node:
//!
//! * `ty`: type expressions
//! * `pattern`: match and let patterns
//! * `expr`: value expressions
//! * `stmt`: statements (assignments, lets, control flow effects)
//! * `decl`: top level items (functions, structs, traits, etc.)
//!
//! Every node carries a `Span` so downstream passes can render errors
//! anchored at the offending source range. The lexer's `Span` type is
//! re exported here for convenience.
//!
//! The grammar this AST encodes is documented in
//! `docs/v2/specs/parser.md`.

pub mod decl;
pub mod expr;
pub mod pattern;
pub mod stmt;
pub mod ty;

pub use decl::*;
pub use expr::*;
pub use pattern::*;
pub use stmt::*;
pub use ty::*;

use crate::span::Span;

/// A parsed source file: the sequence of top level items.
#[derive(Debug, Clone, PartialEq)]
pub struct File {
    pub items: Vec<Decl>,
    pub span: Span,
}
