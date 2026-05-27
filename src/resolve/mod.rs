//! Name resolution for the v2 compiler.
//!
//! The resolver walks an [`ast::File`], binds every identifier use to
//! the declaration it refers to, and resolves every import to its
//! target. The output is a [`ResolvedFile`] that pairs the original
//! AST with a [`ResolutionMap`].
//!
//! See `docs/v2/specs/resolver.md` for the full design.

pub mod bindings;
pub mod items;
pub mod scope;

pub use bindings::{
    Binding, DeclId, ImportId, ImportTarget, ResolutionMap, ResolvedImport, UseKey,
};
pub use scope::{Scope, ScopeKind, ScopeStack};
