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
pub mod stdlib;
pub mod walk;

#[cfg(test)]
mod tests;

use std::collections::HashSet;

use crate::ast::File;
use crate::error::RavenError;

pub use bindings::{
    Binding, DeclId, ImportId, ImportTarget, ResolutionMap, ResolvedImport, UseKey,
};
pub use imports::{FsLoader, GithubPath, LoadedSource, SourceLoader, STDLIB_MODULES};
pub use scope::{Scope, ScopeKind, ScopeStack};
pub use stdlib::{
    expand_with_stdlib, expand_with_stdlib_ctx, external_module_key, local_module_key,
    mangle_external_fn, mangle_local_fn, mangle_stdlib_fn, PackageContext,
};

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
    resolve_file_ctx(file, loader, None)
}

/// Resolve `file` like [`resolve_file`], additionally binding external
/// (`github.com/...`) import selectors to the `ext.`-namespaced symbols
/// the expander merged from the rvpm cache. When `ctx` is `None`, external
/// imports stay deferred exactly as before.
pub fn resolve_file_ctx<'a>(
    file: &'a File,
    loader: &mut dyn SourceLoader,
    ctx: Option<&PackageContext>,
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
    imports::resolve_imports_ctx(
        file,
        &mut scope,
        loader,
        &mut imports_out,
        &mut in_progress,
        ctx,
    )?;
    map.imports = imports_out;

    // Pass 2: walk every body, binding identifier uses.
    walk::walk_file(file, &mut scope, &mut map)?;

    Ok(ResolvedFile {
        file,
        map,
        module_scope: scope,
    })
}
