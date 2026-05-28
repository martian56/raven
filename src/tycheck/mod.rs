//! Type checker for the Raven v2 monomorphic core.
//!
//! Given a [`ResolvedFile`](crate::resolve::ResolvedFile) produced by
//! `src/resolve`, the type checker validates every expression and
//! declaration, assigns a [`Ty`] to each expression site, and
//! produces a [`TypedFile`].
//!
//! The implementation is split into sub modules:
//!
//! * `ty` defines the internal type representation.
//! * `env` defines the declaration signature environment.
//! * `unify` defines type equality and assignability.
//! * `builtin` defines the built in `Option`, `Result`, `List`
//!   signatures and their inherent methods.
//! * `collect` runs the first pass that populates the `TypeEnv`.
//! * `expr` and `stmt` run the body checking pass.
//! * `pattern` and `match_check` validate pattern matching and
//!   exhaustiveness.
//!
//! See `docs/v2/specs/tycheck.md` for the design.

pub mod builtin;
pub mod collect;
pub mod env;
pub mod expr;
pub mod infer;
pub mod match_check;
pub mod pattern;
pub mod stmt;
pub mod ty;
pub mod unify;

#[cfg(test)]
mod tests;

use std::collections::HashMap;

use crate::ast::File;
use crate::error::RavenError;
use crate::resolve::{ResolvedFile, UseKey};
use crate::span::Span;

pub use env::{
    EnumSig, FieldSig, FnSig, GenericParamSig, ImplSig, StructSig, TraitSig, TypeEnv,
    VariantPayloadSig, VariantSig,
};
pub use ty::Ty;

/// One recorded `dyn Trait` unsizing coercion.
///
/// The type checker inserts an entry whenever a concrete value is used
/// where a `dyn Trait` is expected (a `let` with a `dyn` annotation, an
/// argument to a `dyn` parameter, or a `return` of a `dyn` value). The
/// HIR lowering reads the entry, keyed by the coerced expression's span,
/// and materializes the fat pointer construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DynCoercion {
    /// The target trait's short name.
    pub trait_name: String,
    /// The trait's method names in declaration order (the vtable slot
    /// order). Carried so the back end can lay out and dispatch through
    /// the vtable without re-reading the trait declaration.
    pub methods: Vec<String>,
    /// The source concrete type being coerced. Used by the back end to
    /// select the `(concrete_type, trait)` vtable.
    pub concrete_ty: Ty,
}

/// Per file type checking output.
#[derive(Debug, Clone, Default)]
pub struct TypeMap {
    /// Inferred type for each expression site, keyed by the resolver's
    /// `UseKey` (file path plus byte range). Statements that introduce
    /// a binding store the binding's type under the introducing span as
    /// well so downstream passes can look it up.
    pub types: HashMap<UseKey, Ty>,
    /// `dyn Trait` coercions keyed by the coerced expression's span.
    pub coercions: HashMap<UseKey, DynCoercion>,
}

impl TypeMap {
    /// Construct an empty type map.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record `ty` as the inferred type of the expression at `span`.
    pub fn record(&mut self, span: &Span, ty: Ty) {
        self.types.insert(UseKey::from_span(span), ty);
    }

    /// Look up the type recorded at `span`, if any.
    pub fn lookup(&self, span: &Span) -> Option<&Ty> {
        self.types.get(&UseKey::from_span(span))
    }

    /// Record a `dyn Trait` coercion at `span` (the coerced expression).
    pub fn record_coercion(&mut self, span: &Span, coercion: DynCoercion) {
        self.coercions.insert(UseKey::from_span(span), coercion);
    }

    /// Look up a `dyn Trait` coercion recorded at `span`, if any.
    pub fn lookup_coercion(&self, span: &Span) -> Option<&DynCoercion> {
        self.coercions.get(&UseKey::from_span(span))
    }

    /// Iterator yielding `(key, ty)` pairs sorted by file and offset
    /// for stable diagnostic output.
    pub fn sorted_iter(&self) -> Vec<(&UseKey, &Ty)> {
        let mut pairs: Vec<_> = self.types.iter().collect();
        pairs.sort_by(|a, b| {
            let pa = a.0.file.display().to_string();
            let pb = b.0.file.display().to_string();
            (pa, a.0.start, a.0.end).cmp(&(pb, b.0.start, b.0.end))
        });
        pairs
    }
}

/// The result of type checking one file.
#[derive(Debug, Clone)]
pub struct TypedFile<'a> {
    pub file: &'a File,
    pub resolved: &'a ResolvedFile<'a>,
    pub env: TypeEnv,
    pub types: TypeMap,
}

/// Run the type checker on `resolved` and return either a `TypedFile`
/// or the first [`RavenError::Type`] encountered.
///
/// The function runs two passes: a declared type collection pass that
/// populates `TypeEnv` and a body check pass that fills the `TypeMap`.
pub fn check_file<'a>(resolved: &'a ResolvedFile<'a>) -> Result<TypedFile<'a>, RavenError> {
    let mut env = TypeEnv::new();
    collect::collect_declarations(resolved, &mut env)?;
    let mut types = TypeMap::new();
    expr::check_bodies(resolved, &env, &mut types)?;
    Ok(TypedFile {
        file: resolved.file,
        resolved,
        env,
        types,
    })
}
