//! Binding kinds and the resolution map.
//!
//! Every identifier site in a resolved file maps to a [`Binding`], which
//! is a tagged reference to the declaration that introduces the name.
//! Bindings are intentionally loose references (indices and spans) rather
//! than direct pointers so the resolved file can be passed across module
//! boundaries without lifetime knots.
//!
//! See `docs/v2/specs/resolver.md` for the full catalog of variants.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::span::Span;

/// Opaque index into a [`crate::ast::File`]'s `items` vector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DeclId(pub usize);

/// Opaque index into the file's import list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ImportId(pub usize);

/// A hashable key for identifier use sites. We do not derive `Hash`
/// on `Span` itself (it carries an `Arc<PathBuf>` and line/col fields
/// that are derived from `start`); instead a use site is uniquely
/// identified by its file path and byte range.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UseKey {
    pub file: Arc<PathBuf>,
    pub start: usize,
    pub end: usize,
}

impl UseKey {
    /// Build a key from a span.
    pub fn from_span(span: &Span) -> Self {
        UseKey {
            file: span.file.clone(),
            start: span.start,
            end: span.end,
        }
    }
}

/// What an identifier resolves to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Binding {
    /// Top level function.
    Function(DeclId),
    /// Top level struct type.
    Struct(DeclId),
    /// Top level trait.
    Trait(DeclId),
    /// Top level enum.
    Enum(DeclId),
    /// One variant inside an enum (parent enum is at `enum_id`).
    Variant {
        enum_id: DeclId,
        variant_index: usize,
    },
    /// One foreign function inside an extern block.
    Extern { decl_id: DeclId, item_index: usize },
    /// Top level `const`.
    Const(DeclId),
    /// Top level mutable `let`.
    Static(DeclId),
    /// A function parameter, identified by its declaration span.
    Param(Span),
    /// A `let` binding inside a function body.
    Local(Span),
    /// A name introduced by a pattern (match arm, for head, let pattern).
    PatternBinding(Span),
    /// A generic parameter declared on the enclosing item.
    GenericParam { owner: Span, name: String },
    /// `Self` keyword referring to the enclosing impl target type.
    SelfType,
    /// `self` value parameter inside an `impl` method.
    SelfValue,
    /// An `import path as alias` brings in this single name. The actual
    /// import target lives in [`ResolutionMap::imports`].
    ImportAlias(ImportId),
    /// A specific selector from an import like `import std/io { println }`.
    /// Member resolution against the source module is deferred to the
    /// type checker.
    ImportedItem { import_id: ImportId, name: String },
}

/// The concrete target an import points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportTarget {
    /// A built in stdlib module (e.g. `std/io`). The segments are kept
    /// verbatim so the type checker can route member lookups.
    StdlibModule { segments: Vec<String> },
    /// A github package referenced by URL. Fetching is deferred to rvpm.
    ExternalPackage {
        host: String,
        user: String,
        repo: String,
        subpath: Vec<String>,
    },
    /// A locally resolved file. The contents have been parsed and
    /// resolved; member lookups happen against `module_names`.
    LocalModule {
        canonical_path: PathBuf,
        /// Names exported by the loaded file's module scope.
        module_names: Vec<String>,
    },
}

/// One entry in the resolved imports list, keyed by [`ImportId`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedImport {
    pub id: ImportId,
    /// Source spelling of the import path, for diagnostics.
    pub path: String,
    pub target: ImportTarget,
    pub span: Span,
}

/// Per file resolution output.
///
/// Identifier sites are keyed by [`UseKey`] (file plus byte range), so
/// the map round trips through serialization and across worker boundaries
/// without depending on internal span hashing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolutionMap {
    /// Map from identifier use site key to its bound declaration.
    pub uses: HashMap<UseKey, Binding>,
    /// Resolved imports in declaration order. Indexed by [`ImportId`].
    pub imports: Vec<ResolvedImport>,
}

impl ResolutionMap {
    /// Construct an empty map.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record `binding` as the resolution of the identifier site at
    /// `span`.
    pub fn bind_use(&mut self, span: &Span, binding: Binding) {
        let key = UseKey::from_span(span);
        debug_assert!(
            !self.uses.contains_key(&key),
            "double binding at span {:?}",
            span
        );
        self.uses.insert(key, binding);
    }

    /// Look up the binding recorded at `span`, if any.
    pub fn lookup(&self, span: &Span) -> Option<&Binding> {
        self.uses.get(&UseKey::from_span(span))
    }

    /// Iterate over bindings sorted by file then byte range, for stable
    /// diagnostic output.
    pub fn sorted_iter(&self) -> Vec<(&UseKey, &Binding)> {
        let mut pairs: Vec<_> = self.uses.iter().collect();
        pairs.sort_by(|a, b| {
            let pa = a.0.file.display().to_string();
            let pb = b.0.file.display().to_string();
            (pa, a.0.start, a.0.end).cmp(&(pb, b.0.start, b.0.end))
        });
        pairs
    }
}
