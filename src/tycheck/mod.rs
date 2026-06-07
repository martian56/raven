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
pub mod wf;

#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};

use crate::ast::File;
use crate::error::RavenError;
use crate::resolve::{ResolvedFile, UseKey};
use crate::span::Span;

pub use env::{
    EnumSig, FieldSig, FnSig, GenericParamSig, ImplSig, StructSig, TraitSig, TypeEnv,
    VariantPayloadSig, VariantSig,
};
pub use ty::{FfiTy, Ty};

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
    /// Resolved explicit type arguments of a generic call, keyed by the
    /// callee's span. Recorded only when a call writes them (`f<Int>()`).
    /// MIR uses them to bind a callee's generic parameters that the value
    /// arguments and result type do not pin down (for example a
    /// `type_name<T>()` inside a generic body, or any `f<T>()` with no
    /// argument carrying `T`).
    pub type_args: HashMap<UseKey, Vec<Ty>>,
    /// Spans of top-level function names passed where a `CFnPtr` is
    /// expected (a C-FFI callback). Such a name lowers to the function's
    /// raw C address rather than a Raven closure object. Every other
    /// function-typed name used as a value is a Raven closure value.
    pub callback_fns: HashSet<UseKey>,
    /// Spans of closure values passed where a `CFnPtr` is expected. The site
    /// lowers to a generated trampoline (userdata-last) the closure is
    /// invoked through, rather than the closure object pointer.
    pub closure_callbacks: HashSet<UseKey>,
    /// Spans of closure values passed where a `CPtr` is expected: the closure
    /// object pointer is handed to C as the trampoline's `userdata`.
    pub closure_userdata: HashSet<UseKey>,
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

    /// Record the resolved explicit type arguments of a call at the
    /// callee's `span`.
    pub fn record_type_args(&mut self, span: &Span, args: Vec<Ty>) {
        self.type_args.insert(UseKey::from_span(span), args);
    }

    /// Look up the explicit type arguments recorded at `span`, if any.
    pub fn lookup_type_args(&self, span: &Span) -> Option<&Vec<Ty>> {
        self.type_args.get(&UseKey::from_span(span))
    }

    /// Record a `dyn Trait` coercion at `span` (the coerced expression).
    pub fn record_coercion(&mut self, span: &Span, coercion: DynCoercion) {
        self.coercions.insert(UseKey::from_span(span), coercion);
    }

    /// Look up a `dyn Trait` coercion recorded at `span`, if any.
    pub fn lookup_coercion(&self, span: &Span) -> Option<&DynCoercion> {
        self.coercions.get(&UseKey::from_span(span))
    }

    /// Record that the function name at `span` is a C-FFI callback (passed
    /// where a `CFnPtr` is expected), so it lowers to a raw C address.
    pub fn record_callback_fn(&mut self, span: &Span) {
        self.callback_fns.insert(UseKey::from_span(span));
    }

    /// True when the function name at `span` was recorded as a C-FFI
    /// callback.
    pub fn is_callback_fn(&self, span: &Span) -> bool {
        self.callback_fns.contains(&UseKey::from_span(span))
    }

    /// Record that the closure at `span` is passed where a `CFnPtr` is
    /// expected, so it lowers to a trampoline.
    pub fn record_closure_callback(&mut self, span: &Span) {
        self.closure_callbacks.insert(UseKey::from_span(span));
    }

    /// True when the closure at `span` is a `CFnPtr` callback (a trampoline).
    pub fn is_closure_callback(&self, span: &Span) -> bool {
        self.closure_callbacks.contains(&UseKey::from_span(span))
    }

    /// Record that the closure at `span` is passed as a callback's `userdata`
    /// pointer.
    pub fn record_closure_userdata(&mut self, span: &Span) {
        self.closure_userdata.insert(UseKey::from_span(span));
    }

    /// True when the closure at `span` is a callback's `userdata` pointer.
    pub fn is_closure_userdata(&self, span: &Span) -> bool {
        self.closure_userdata.contains(&UseKey::from_span(span))
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
/// or every [`RavenError::Type`] the body pass recovered from.
///
/// The function runs two passes: a declared type collection pass that
/// populates `TypeEnv` and a body check pass that fills the `TypeMap`. The
/// collection pass is fail-fast (a malformed signature stops it, since later
/// items depend on collected signatures); the body pass recovers at item and
/// statement boundaries so one compile reports many independent errors.
pub fn check_file_all<'a>(
    resolved: &'a ResolvedFile<'a>,
) -> Result<TypedFile<'a>, Vec<RavenError>> {
    let mut env = TypeEnv::new();
    collect::collect_declarations(resolved, &mut env).map_err(|e| vec![e])?;
    // Reject declared types whose generic arguments do not satisfy the
    // declaration's bounds (for example a `Map<K, V>` key without `Hash`), with
    // a clear error here rather than an unresolved callee at codegen.
    let wf_errors = wf::check_declared_types(&env);
    if !wf_errors.is_empty() {
        return Err(wf_errors);
    }
    let mut types = TypeMap::new();
    expr::check_bodies(resolved, &env, &mut types)?;
    Ok(TypedFile {
        file: resolved.file,
        resolved,
        env,
        types,
    })
}

/// Run the type checker and return either a `TypedFile` or the first
/// [`RavenError::Type`] encountered. A thin wrapper over [`check_file_all`]
/// for callers that only surface one error (tests, golden harnesses).
pub fn check_file<'a>(resolved: &'a ResolvedFile<'a>) -> Result<TypedFile<'a>, RavenError> {
    check_file_all(resolved).map_err(|mut es| es.remove(0))
}
