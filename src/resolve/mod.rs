//! Name resolution for the v2 compiler.
//!
//! The resolver walks an [`crate::ast::File`], binds every identifier
//! use to the declaration it refers to, and resolves every import to
//! its target. The output is a [`ResolvedFile`] that pairs the
//! original AST with a [`ResolutionMap`] and the resolved import list.
//!
//! See `docs/v2/specs/resolver.md` for the full design.

pub mod bindings;
pub mod imports;
pub mod items;
pub mod scope;
pub mod walk;

#[cfg(test)]
mod tests;

use std::collections::HashSet;

use crate::ast::File;
use crate::error::RavenError;

pub use bindings::{
    Binding, DeclId, ImportId, ImportTarget, ResolutionMap, ResolvedImport, UseKey,
};
pub use imports::{FsLoader, LoadedSource, SourceLoader, STDLIB_MODULES};
pub use scope::{Scope, ScopeKind, ScopeStack};

/// The resolver output for a single file.
///
/// The resolver borrows the AST during walking but does not mutate it
/// or take ownership; the [`File`] is re exposed here for convenience.
#[derive(Debug, Clone)]
pub struct ResolvedFile<'a> {
    pub file: &'a File,
    pub map: ResolutionMap,
    pub module_scope: ScopeStack,
}

/// Resolve `file` using `loader` for any local imports it contains.
///
/// Returns a [`ResolvedFile`] on success or the first
/// [`RavenError::Resolve`] encountered. The function is the canonical
/// entry point for the resolver; downstream stages should call it once
/// per source file.
pub fn resolve_file<'a>(
    file: &'a File,
    loader: &mut dyn SourceLoader,
) -> Result<ResolvedFile<'a>, RavenError> {
    let mut scope = ScopeStack::new();
    let mut map = ResolutionMap::new();
    let mut imports_out = Vec::new();
    let mut in_progress: HashSet<std::path::PathBuf> = HashSet::new();
    in_progress.insert((*file.span.file).clone());

    // Pass 1a: collect every top level item into the module scope.
    items::collect_items(file, &mut scope)?;

    // Pass 1b: resolve imports and merge their bindings into the same
    // module scope.
    imports::resolve_imports(file, &mut scope, loader, &mut imports_out, &mut in_progress)?;
    map.imports = imports_out;

    // Pass 2: walk every body, binding identifier uses.
    walk::walk_file(file, &mut scope, &mut map)?;

    Ok(ResolvedFile {
        file,
        map,
        module_scope: scope,
    })
}
