//! Bundled standard library loading.
//!
//! Standard library modules are written in Raven (`.rv` source) and
//! bundled into the compiler with `include_str!`. When a program writes
//! `import std/io { ... }`, the compiler parses the embedded source for
//! that module, namespaces its top level functions, and merges them into
//! the program so the rest of the pipeline (type checker, lowering,
//! codegen) sees them as ordinary functions defined alongside the user
//! code.
//!
//! Namespacing: a stdlib function `f` in module `io` is renamed to
//! `std.io.f`. The `.` makes the name unwritable by a user, so a stdlib
//! function never collides with a user declaration. A selective import
//! `import std/io { println }` then binds the bare name `println` to the
//! `std.io.println` function (see `imports.rs`).
//!
//! See `docs/v2/specs/stdlib.md` for the full mechanism.

use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeSet, HashMap};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::ast::{
    Block, Decl, DeclKind, ElseBranch, Expr, ExprKind, File, FunctionBody, ImportSource,
    LambdaBody, MatchArm, Stmt, StmtKind, StrFragment,
};
use crate::error::{RavenError, ResolveError};
use crate::lexer::Lexer;
use crate::parser::parse;

use super::imports::{FsLoader, SourceLoader};

/// The embedded source of one bundled stdlib module, keyed by its module
/// path under `std/`. A `std/io` import maps to the `"io"` entry. The
/// list grows as later modules (issues #72 to #80) land; each adds one
/// `include_str!` row here.
pub const BUNDLED_MODULES: &[(&str, &str)] = &[
    ("core", include_str!("../../stdlib/std/core.rv")),
    ("io", include_str!("../../stdlib/std/io.rv")),
    ("string", include_str!("../../stdlib/std/string.rv")),
    ("iter", include_str!("../../stdlib/std/iter.rv")),
    (
        "collections",
        include_str!("../../stdlib/std/collections.rv"),
    ),
    ("cmp", include_str!("../../stdlib/std/cmp.rv")),
    ("hash", include_str!("../../stdlib/std/hash.rv")),
    ("encoding", include_str!("../../stdlib/std/encoding.rv")),
    ("random", include_str!("../../stdlib/std/random.rv")),
    ("fmt", include_str!("../../stdlib/std/fmt.rv")),
    ("math", include_str!("../../stdlib/std/math.rv")),
    ("path", include_str!("../../stdlib/std/path.rv")),
    ("error", include_str!("../../stdlib/std/error.rv")),
    ("env", include_str!("../../stdlib/std/env.rv")),
    ("fs", include_str!("../../stdlib/std/fs.rv")),
    ("time", include_str!("../../stdlib/std/time.rv")),
    ("net", include_str!("../../stdlib/std/net.rv")),
    ("http", include_str!("../../stdlib/std/http.rv")),
    ("json", include_str!("../../stdlib/std/json.rv")),
    ("regex", include_str!("../../stdlib/std/regex.rv")),
    ("process", include_str!("../../stdlib/std/process.rv")),
    ("ffi", include_str!("../../stdlib/std/ffi.rv")),
    ("test", include_str!("../../stdlib/std/test.rv")),
];

/// The prelude module that is implicitly imported into every program.
/// Its traits (`ToString`, `Eq`, `Ord`, `Hash`, `Iterator`) and their
/// built-in impls are always in scope, so a user writes neither an
/// `import std/core` line nor an explicit `impl` for the built-in types.
/// See `docs/v2/specs/core-traits.md`.
pub const PRELUDE_MODULE: &str = "core";

/// The separator used when namespacing a bundled function name. The
/// resulting name (for example `std.io.println`) is unwritable by a user
/// because Raven identifiers cannot contain `.`.
pub const NAMESPACE_SEP: char = '.';

/// Build the mangled name of a stdlib function: `std.<module>.<name>`.
pub fn mangle_stdlib_fn(module: &str, name: &str) -> String {
    format!("std{sep}{module}{sep}{name}", sep = NAMESPACE_SEP)
}

/// Build a stable namespacing key for a local module from its canonical
/// path: `loc.<hash>`. The hash is computed from the canonical path
/// string with `DefaultHasher`, whose seed is fixed, so the same path
/// always yields the same key within a compile and across runs. The
/// importer's selector binding (`bind_import`) recomputes this key from
/// the same canonical path, so the merged declaration and the bound name
/// agree. The `.` in the key is unwritable by a user, so a namespaced
/// local function never collides with a user declaration.
pub fn local_module_key(canonical_path: &Path) -> String {
    let mut hasher = DefaultHasher::new();
    canonical_path.to_string_lossy().hash(&mut hasher);
    format!("loc{sep}{:016x}", hasher.finish(), sep = NAMESPACE_SEP)
}

/// Build the mangled name of a local module function: `loc.<hash>.<name>`.
pub fn mangle_local_fn(key: &str, name: &str) -> String {
    format!("{key}{sep}{name}", sep = NAMESPACE_SEP)
}

/// Look up the embedded source for a bundled module by its `std/` path.
pub fn bundled_source(module_path: &str) -> Option<&'static str> {
    BUNDLED_MODULES
        .iter()
        .find(|(name, _)| *name == module_path)
        .map(|(_, src)| *src)
}

/// Expand `user` into a combined [`File`] that contains every bundled
/// stdlib module the program imports, followed by the user's own items.
///
/// Each imported `std/<module>` is loaded once (duplicate imports are
/// deduplicated), parsed, and its top level functions are renamed to
/// `std.<module>.<name>`. The returned file is owned by the caller and
/// must outlive the resolution that borrows it.
///
/// An unknown `std/<module>` (one with no bundled source) is left for
/// the import pass to report as `UnresolvedImport`; this function only
/// merges the modules it can load.
pub fn expand_with_stdlib(user: &File) -> Result<File, RavenError> {
    // Modules to merge, collected to a fixed point. A module enters the
    // set from the user file's `std/...` imports or from a bundled
    // module's own `import std/...` line. The set deduplicates, so each
    // module merges exactly once and an import cycle (A imports B imports
    // A) just resolves to both being present once with no infinite loop.
    let mut wanted: BTreeSet<String> = BTreeSet::new();
    // The prelude (`std/core`) is implicitly imported into every program,
    // so its traits and built-in impls are always in scope without an
    // `import std/core` line. It seeds the set; a `BTreeSet` keeps the
    // module order stable and deduplicates against any later import of the
    // same module (so the prelude never merges twice).
    wanted.insert(PRELUDE_MODULE.to_string());
    collect_std_module_imports(user, &mut wanted);

    // Load every local `./`/`../` module reachable from the user file,
    // transitively, before computing the bundled set: a local module may
    // itself `import std/...`, and those bundled modules must merge too so
    // the local module's own calls resolve.
    let mut loader = FsLoader;
    let local_modules = load_local_modules(user, &mut loader)?;
    for module_file in &local_modules {
        collect_std_module_imports(module_file, &mut wanted);
    }

    // Follow each bundled module's own `std/...` imports to a fixed point.
    // A worklist over the not-yet-scanned modules terminates because every
    // discovered module is added to `wanted` (a set) at most once and only
    // unscanned modules are pushed.
    let mut to_scan: Vec<String> = wanted.iter().cloned().collect();
    while let Some(module) = to_scan.pop() {
        let module_file = parse_bundled_module(&module)?;
        let before = wanted.len();
        collect_std_module_imports(&module_file, &mut wanted);
        if wanted.len() != before {
            // New modules appeared; queue only the freshly added ones.
            for m in &wanted {
                if !to_scan.contains(m) {
                    to_scan.push(m.clone());
                }
            }
        }
    }

    let mut combined_items = Vec::new();
    let bundled_path = PathBuf::from("<bundled>");
    for module in &wanted {
        let module_file = parse_bundled_module(module)?;
        // The module's own functions rename to `std.<module>.<name>`, plus
        // any names it selectively imports from another module. A bundled
        // module imports other modules without selectors, so the import
        // part is normally empty, but the same code path serves both.
        let mut rename = import_rename_map(&module_file, &bundled_path, &mut loader);
        for name in top_level_fn_names(&module_file) {
            rename.insert(name.clone(), mangle_stdlib_fn(module, &name));
        }
        merge_module_items(module_file.items, &rename, &mut combined_items);
    }

    // Merge the local modules loaded above through the same merge core,
    // with a path derived namespace instead of the `std.<module>.` one.
    for module_file in local_modules {
        let importing = (*module_file.span.file).clone();
        let key = local_module_key(&importing);
        let mut rename = import_rename_map(&module_file, &importing, &mut loader);
        for name in top_level_fn_names(&module_file) {
            rename.insert(name.clone(), mangle_local_fn(&key, &name));
        }
        merge_module_items(module_file.items, &rename, &mut combined_items);
    }

    // The user's items follow the stdlib items so user DeclIds shift by a
    // fixed, known amount but otherwise keep their relative order. The
    // combined file's span borrows the user's file path for diagnostics
    // that key off the top level file.
    combined_items.extend(user.items.iter().cloned());

    Ok(File {
        items: combined_items,
        span: user.span.clone(),
    })
}

/// Add every bundled `std/<module>` imported by `file` to `wanted`. Only
/// imports that name a known bundled module are added; an unknown module
/// is left for the import pass to report.
fn collect_std_module_imports(file: &File, wanted: &mut BTreeSet<String>) {
    for decl in &file.items {
        if let DeclKind::Import(import) = &decl.kind {
            if let ImportSource::Std(segments) = &import.source {
                if let Some(head) = segments.first() {
                    if bundled_source(head).is_some() {
                        wanted.insert(head.clone());
                    }
                }
            }
        }
    }
}

/// Merge one module's items into `combined`, renaming free functions and
/// rewriting call sites through `rename`. The `rename` map carries the
/// module's own functions (bare name to its `std.<mod>.` or `loc.<hash>.`
/// namespaced name) plus every name the module selectively imports from
/// another merged module (so a transitive call resolves to the dependency's
/// namespaced symbol). Shared by the bundled and local paths; the only
/// difference between them is the namespace the rename map uses and where
/// the source comes from (bundled `include_str!` versus the filesystem). A
/// future external package backend (issue #85) plugs in here by supplying
/// the source from the rvpm cache and reusing this same merge.
fn merge_module_items(
    items: Vec<Decl>,
    rename: &HashMap<String, String>,
    combined: &mut Vec<Decl>,
) {
    for mut decl in items {
        // A module's own `import ...` declarations are consumed by the
        // expander (the imported module is merged separately); they must
        // not leak into the combined file as import items.
        if matches!(&decl.kind, DeclKind::Import(_)) {
            continue;
        }
        match &mut decl.kind {
            DeclKind::Function(f) => {
                rewrite_fn_body_calls(&mut f.body, rename);
                if let Some(replacement) = rename.get(&f.name) {
                    f.name = replacement.clone();
                }
            }
            DeclKind::Impl(i) => {
                // An `impl` on a type keeps its method names: a method is
                // dispatched by the receiver's type, not by a free function
                // name, so it never collides and needs no namespacing. Its
                // body may call sibling free functions of the same module,
                // which were renamed above; rewrite those call sites.
                for m in &mut i.items {
                    rewrite_fn_body_calls(&mut m.body, rename);
                }
            }
            // Struct, enum, and trait types merge under their own names,
            // the same way bundled types like `Map` do. Two local modules
            // that both define a type named `Foo` therefore collide; this
            // mirrors the existing stdlib type behavior (issues #178/#184).
            _ => {}
        }
        combined.push(decl);
    }
}

/// Load every local `./` or `../` module reachable from `user`,
/// transitively, returning each parsed file exactly once. Each returned
/// file's `span.file` is its canonical path, so a later caller can derive
/// the namespacing key from the same path the import binder uses.
///
/// Modules are deduplicated by canonical path, so a module imported from
/// several places loads once. A cycle (a module that imports itself
/// directly or transitively) is broken gracefully: each module is loaded
/// once and the back edge is ignored, mirroring the bundled set's fixed
/// point behavior. A missing file is left for the import resolution pass
/// to report with a precise span; this function only loads what it can.
fn load_local_modules(user: &File, loader: &mut dyn SourceLoader) -> Result<Vec<File>, RavenError> {
    let mut to_load: Vec<(PathBuf, String)> = local_import_targets(user);
    let mut loaded_paths: BTreeSet<PathBuf> = BTreeSet::new();
    let mut out: Vec<File> = Vec::new();

    while let Some((importing, target)) = to_load.pop() {
        let Some(loaded) = loader.load(&importing, &target) else {
            continue;
        };
        if !loaded_paths.insert(loaded.canonical_path.clone()) {
            continue;
        }

        let tokens = Lexer::new(loaded.source.clone(), loaded.canonical_path.clone())
            .tokenize()
            .map_err(|e| local_error(&loaded.canonical_path, format!("lex: {e}")))?;
        let module_file = parse(&tokens)
            .map_err(|e| local_error(&loaded.canonical_path, format!("parse: {e}")))?;

        for (_, dep) in local_import_targets(&module_file) {
            to_load.push((loaded.canonical_path.clone(), dep));
        }
        out.push(module_file);
    }

    Ok(out)
}

/// Build the rename entries a merged module needs for the names it
/// selectively imports from OTHER modules. A `import std/io { println }`
/// maps `println` to `std.io.println`; a `import "./b" { base }` maps
/// `base` to `loc.<hashB>.base`, where the hash is keyed by `./b`'s
/// canonical path resolved relative to `importing`. The resolver does not
/// rebind these names (the import decls were stripped from the merged
/// file), so the call sites inside the module body must be rewritten here.
/// A whole module import (no selectors) introduces no free name to rename.
fn import_rename_map(
    file: &File,
    importing: &Path,
    loader: &mut dyn SourceLoader,
) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for decl in &file.items {
        let DeclKind::Import(import) = &decl.kind else {
            continue;
        };
        if import.selectors.is_empty() {
            continue;
        }
        match &import.source {
            ImportSource::Std(segments) => {
                if let Some(module) = segments.first() {
                    if let Ok(target) = parse_bundled_module(module) {
                        let fns = top_level_fn_names(&target);
                        for name in &import.selectors {
                            // Only functions are namespaced; a type keeps its
                            // own name (see `merge_module_items`), so a type
                            // selector needs no rename.
                            if fns.contains(name) {
                                map.insert(name.clone(), mangle_stdlib_fn(module, name));
                            }
                        }
                    }
                }
            }
            ImportSource::Quoted(path) => {
                if !(path.starts_with("./") || path.starts_with("../")) {
                    continue;
                }
                if let Some(loaded) = loader.load(importing, path) {
                    if let Some(target) = parse_loaded(&loaded.source, &loaded.canonical_path) {
                        let key = local_module_key(&loaded.canonical_path);
                        let fns = top_level_fn_names(&target);
                        for name in &import.selectors {
                            if fns.contains(name) {
                                map.insert(name.clone(), mangle_local_fn(&key, name));
                            }
                        }
                    }
                }
            }
        }
    }
    map
}

/// The set of top level free function names declared in `file`.
fn top_level_fn_names(file: &File) -> BTreeSet<String> {
    file.items
        .iter()
        .filter_map(|d| match &d.kind {
            DeclKind::Function(f) => Some(f.name.clone()),
            _ => None,
        })
        .collect()
}

/// Lex and parse a loaded local source, returning `None` on any error
/// (the import resolution pass reports the precise diagnostic).
fn parse_loaded(source: &str, canonical_path: &Path) -> Option<File> {
    let tokens = Lexer::new(source.to_string(), canonical_path.to_path_buf())
        .tokenize()
        .ok()?;
    parse(&tokens).ok()
}

/// Collect the `(importing_path, target)` pairs for every local `./` or
/// `../` import declared in `file`. The importing path is the file's own
/// path, so the loader resolves the target relative to it.
fn local_import_targets(file: &File) -> Vec<(PathBuf, String)> {
    let importing = (*file.span.file).clone();
    let mut out = Vec::new();
    for decl in &file.items {
        if let DeclKind::Import(import) = &decl.kind {
            if let ImportSource::Quoted(path) = &import.source {
                if path.starts_with("./") || path.starts_with("../") {
                    out.push((importing.clone(), path.clone()));
                }
            }
        }
    }
    out
}

/// Build a resolve error for a local module that failed to load. The lex
/// or parse error is anchored at the start of the offending file.
fn local_error(path: &Path, detail: String) -> RavenError {
    let span = crate::span::Span::point(Arc::new(path.to_path_buf()), 0, 1, 1);
    RavenError::resolve(
        ResolveError::UnresolvedImport(path.display().to_string()),
        span,
    )
    .with_hint(format!("local module failed to load: {detail}"))
}

/// Lex and parse one bundled module's embedded source.
fn parse_bundled_module(module: &str) -> Result<File, RavenError> {
    let source = bundled_source(module).expect("module presence checked by caller");
    let virtual_path = PathBuf::from(format!("<bundled>/std/{module}.rv"));
    let tokens = Lexer::new(source.to_string(), virtual_path)
        .tokenize()
        .map_err(|e| bundled_error(module, format!("lex: {e}")))?;
    parse(&tokens).map_err(|e| bundled_error(module, format!("parse: {e}")))
}

/// Rewrite every reference to a sibling stdlib function inside a bundled
/// module's function body to its namespaced name.
///
/// A bundled module declares free functions that call one another by
/// their bare names (for example `index_of` calls `matches_at`). The
/// declarations are renamed to `std.<module>.<name>`, so a call site must
/// use the same namespaced name to resolve. This walk renames any bare
/// identifier whose name is one of the module's own functions; local
/// variables and parameters never share a name with a sibling function in
/// the bundled sources, so the rename is unambiguous.
fn rewrite_fn_body_calls(body: &mut FunctionBody, rename: &HashMap<String, String>) {
    match body {
        FunctionBody::Block(block) => rewrite_block(block, rename),
        FunctionBody::Expr(expr) => rewrite_expr(expr, rename),
        FunctionBody::None => {}
    }
}

fn rewrite_block(block: &mut Block, rename: &HashMap<String, String>) {
    for stmt in &mut block.stmts {
        rewrite_stmt(stmt, rename);
    }
    if let Some(trailing) = &mut block.trailing {
        rewrite_expr(trailing, rename);
    }
}

fn rewrite_stmt(stmt: &mut Stmt, rename: &HashMap<String, String>) {
    match &mut stmt.kind {
        StmtKind::Let { init, .. } => {
            if let Some(e) = init {
                rewrite_expr(e, rename);
            }
        }
        StmtKind::Return(e) | StmtKind::Break(e) => {
            if let Some(e) = e {
                rewrite_expr(e, rename);
            }
        }
        StmtKind::Defer(e) | StmtKind::Expr(e) => rewrite_expr(e, rename),
        StmtKind::Assign { target, value, .. } => {
            rewrite_expr(target, rename);
            rewrite_expr(value, rename);
        }
        StmtKind::Continue => {}
    }
}

fn rewrite_expr(expr: &mut Expr, rename: &HashMap<String, String>) {
    match &mut expr.kind {
        ExprKind::Ident { name, .. } => {
            if let Some(replacement) = rename.get(name) {
                *name = replacement.clone();
            }
        }
        ExprKind::InterpolatedString(fragments) => {
            for frag in fragments {
                if let StrFragment::Expr(e) = frag {
                    rewrite_expr(e, rename);
                }
            }
        }
        ExprKind::Array(items) | ExprKind::Tuple(items) => {
            for e in items {
                rewrite_expr(e, rename);
            }
        }
        ExprKind::StructLit { fields, .. } => {
            for f in fields {
                rewrite_expr(&mut f.value, rename);
            }
        }
        ExprKind::Paren(inner) | ExprKind::Try(inner) => rewrite_expr(inner, rename),
        ExprKind::Block(block) => rewrite_block(block, rename),
        ExprKind::Unary { operand, .. } => rewrite_expr(operand, rename),
        ExprKind::Binary { lhs, rhs, .. } => {
            rewrite_expr(lhs, rename);
            rewrite_expr(rhs, rename);
        }
        ExprKind::Range { start, end, .. } => {
            rewrite_expr(start, rename);
            rewrite_expr(end, rename);
        }
        ExprKind::Call { callee, args } => {
            rewrite_expr(callee, rename);
            for a in args {
                rewrite_expr(a, rename);
            }
        }
        ExprKind::MethodCall { receiver, args, .. } => {
            rewrite_expr(receiver, rename);
            for a in args {
                rewrite_expr(a, rename);
            }
        }
        ExprKind::Field { receiver, .. } => rewrite_expr(receiver, rename),
        ExprKind::Index { receiver, index } => {
            rewrite_expr(receiver, rename);
            rewrite_expr(index, rename);
        }
        ExprKind::If {
            cond,
            then_branch,
            else_branch,
        } => {
            rewrite_expr(cond, rename);
            rewrite_block(then_branch, rename);
            if let Some(else_branch) = else_branch {
                match else_branch.as_mut() {
                    ElseBranch::If(e) => rewrite_expr(e, rename),
                    ElseBranch::Block(b) => rewrite_block(b, rename),
                }
            }
        }
        ExprKind::Match { scrutinee, arms } => {
            rewrite_expr(scrutinee, rename);
            for arm in arms.iter_mut() {
                rewrite_match_arm(arm, rename);
            }
        }
        ExprKind::Loop(block) => rewrite_block(block, rename),
        ExprKind::While { cond, body } => {
            rewrite_expr(cond, rename);
            rewrite_block(body, rename);
        }
        ExprKind::For { iter, body, .. } => {
            rewrite_expr(iter, rename);
            rewrite_block(body, rename);
        }
        ExprKind::Lambda { body, .. } => match body {
            LambdaBody::Block(b) => rewrite_block(b, rename),
            LambdaBody::Expr(e) => rewrite_expr(e, rename),
        },
        // Leaf literals and the `self`/`Self` keywords carry no nested
        // expressions to rewrite.
        ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::Bool(_)
        | ExprKind::Str(_)
        | ExprKind::BlockStr(_)
        | ExprKind::Char(_)
        | ExprKind::CStr(_)
        | ExprKind::SelfLower
        | ExprKind::SelfUpper => {}
    }
}

fn rewrite_match_arm(arm: &mut MatchArm, rename: &HashMap<String, String>) {
    if let Some(guard) = &mut arm.guard {
        rewrite_expr(guard, rename);
    }
    rewrite_expr(&mut arm.body, rename);
}

/// Build a resolve error for a bundled module that failed to load. A
/// failure here is a compiler bug (the bundled source is shipped with
/// the compiler), not a user error, but it is surfaced through the
/// normal error channel anchored at a synthetic span.
fn bundled_error(module: &str, detail: String) -> RavenError {
    let span = crate::span::Span::point(
        Arc::new(PathBuf::from(format!("<bundled>/std/{module}.rv"))),
        0,
        1,
        1,
    );
    RavenError::resolve(
        ResolveError::UnresolvedImport(format!("std/{module}")),
        span,
    )
    .with_hint(format!("bundled stdlib module failed to load: {detail}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;
    use crate::parser::parse;

    fn parse_src(src: &str) -> File {
        let tokens = Lexer::new(src.to_string(), PathBuf::from("main.rv"))
            .tokenize()
            .expect("lex");
        parse(&tokens).expect("parse")
    }

    #[test]
    fn io_module_is_bundled() {
        assert!(bundled_source("io").is_some());
        assert!(bundled_source("nope").is_none());
    }

    #[test]
    fn string_module_is_bundled() {
        assert!(bundled_source("string").is_some());
    }

    #[test]
    fn math_module_is_bundled() {
        assert!(bundled_source("math").is_some());
    }

    #[test]
    fn path_module_is_bundled() {
        assert!(bundled_source("path").is_some());
    }

    #[test]
    fn error_module_is_bundled() {
        assert!(bundled_source("error").is_some());
    }

    #[test]
    fn hash_module_is_bundled() {
        assert!(bundled_source("hash").is_some());
    }

    #[test]
    fn test_module_is_bundled() {
        assert!(bundled_source("test").is_some());
    }

    #[test]
    fn encoding_module_is_bundled() {
        assert!(bundled_source("encoding").is_some());
    }

    #[test]
    fn random_module_is_bundled() {
        assert!(bundled_source("random").is_some());
    }

    #[test]
    fn env_module_is_bundled() {
        assert!(bundled_source("env").is_some());
    }

    #[test]
    fn fs_module_is_bundled() {
        assert!(bundled_source("fs").is_some());
    }

    #[test]
    fn fmt_module_is_bundled() {
        assert!(bundled_source("fmt").is_some());
    }

    #[test]
    fn time_module_is_bundled() {
        assert!(bundled_source("time").is_some());
    }

    #[test]
    fn net_module_is_bundled() {
        assert!(bundled_source("net").is_some());
    }

    #[test]
    fn http_module_is_bundled() {
        assert!(bundled_source("http").is_some());
    }

    #[test]
    fn json_module_is_bundled() {
        assert!(bundled_source("json").is_some());
    }

    #[test]
    fn regex_module_is_bundled() {
        assert!(bundled_source("regex").is_some());
    }

    #[test]
    fn process_module_is_bundled() {
        assert!(bundled_source("process").is_some());
    }

    #[test]
    fn ffi_module_is_bundled() {
        assert!(bundled_source("ffi").is_some());
    }

    #[test]
    fn intra_module_sibling_calls_are_namespaced() {
        // `std/string`'s `trim` is a method on `impl String` that calls the
        // module's free helper `is_space_byte`. After expansion the call
        // site inside the method body must reference the namespaced name so
        // it resolves to the renamed free declaration.
        let user = parse_src("import std/string\nfun main() {}\n");
        let combined = expand_with_stdlib(&user).expect("expand");
        let trim_fn = combined
            .items
            .iter()
            .filter_map(|d| match &d.kind {
                DeclKind::Impl(imp) => Some(imp),
                _ => None,
            })
            .flat_map(|imp| imp.items.iter())
            .find(|f| f.name == "trim")
            .expect("trim method present");
        let mut idents = Vec::new();
        if let FunctionBody::Block(b) = &trim_fn.body {
            collect_block_idents(b, &mut idents);
        } else {
            panic!("trim has a block body");
        }
        assert!(
            idents.iter().any(|n| n == "std.string.is_space_byte"),
            "trim body should call the namespaced free sibling, got: {idents:?}"
        );
        assert!(
            !idents.iter().any(|n| n == "is_space_byte"),
            "no bare sibling call should remain, got: {idents:?}"
        );
    }

    fn collect_block_idents(block: &Block, out: &mut Vec<String>) {
        for stmt in &block.stmts {
            match &stmt.kind {
                StmtKind::Let { init: Some(e), .. } => collect_expr_idents(e, out),
                StmtKind::Return(Some(e)) | StmtKind::Expr(e) => collect_expr_idents(e, out),
                StmtKind::Assign { target, value, .. } => {
                    collect_expr_idents(target, out);
                    collect_expr_idents(value, out);
                }
                _ => {}
            }
        }
        if let Some(t) = &block.trailing {
            collect_expr_idents(t, out);
        }
    }

    fn collect_expr_idents(expr: &Expr, out: &mut Vec<String>) {
        match &expr.kind {
            ExprKind::Ident { name, .. } => out.push(name.clone()),
            ExprKind::Call { callee, args } => {
                collect_expr_idents(callee, out);
                for a in args {
                    collect_expr_idents(a, out);
                }
            }
            ExprKind::Binary { lhs, rhs, .. } => {
                collect_expr_idents(lhs, out);
                collect_expr_idents(rhs, out);
            }
            ExprKind::Unary { operand, .. } => collect_expr_idents(operand, out),
            ExprKind::Paren(e) => collect_expr_idents(e, out),
            ExprKind::If {
                cond,
                then_branch,
                else_branch,
            } => {
                collect_expr_idents(cond, out);
                collect_block_idents(then_branch, out);
                if let Some(eb) = else_branch {
                    match eb.as_ref() {
                        ElseBranch::If(e) => collect_expr_idents(e, out),
                        ElseBranch::Block(b) => collect_block_idents(b, out),
                    }
                }
            }
            ExprKind::While { cond, body } => {
                collect_expr_idents(cond, out);
                collect_block_idents(body, out);
            }
            ExprKind::Block(b) => collect_block_idents(b, out),
            _ => {}
        }
    }

    #[test]
    fn mangling_uses_dotted_namespace() {
        assert_eq!(mangle_stdlib_fn("io", "println"), "std.io.println");
    }

    #[test]
    fn expand_prepends_namespaced_io_functions() {
        let user = parse_src("import std/io { println }\nfun main() {}\n");
        let combined = expand_with_stdlib(&user).expect("expand");
        let names: Vec<String> = combined
            .items
            .iter()
            .filter_map(|d| match &d.kind {
                DeclKind::Function(f) => Some(f.name.clone()),
                _ => None,
            })
            .collect();
        assert!(names.contains(&"std.io.println".to_string()));
        assert!(names.contains(&"main".to_string()));
        // The user `main` keeps its bare name; only stdlib names mangle.
        assert!(!names.contains(&"std.io.main".to_string()));
    }

    #[test]
    fn no_std_import_still_merges_the_prelude() {
        // Even with no explicit `import std/...`, the prelude (`std/core`)
        // is implicitly merged so its traits and built-in impls are always
        // in scope. The combined file therefore holds the prelude items
        // plus the user's, and the user's own items still trail.
        let user = parse_src("fun main() {}\n");
        let combined = expand_with_stdlib(&user).expect("expand");
        assert!(
            combined.items.len() > user.items.len(),
            "the prelude should add items, got {} (user had {})",
            combined.items.len(),
            user.items.len()
        );
        // The user's `main` is preserved and trails the prelude.
        assert!(matches!(
            combined.items.last().map(|d| &d.kind),
            Some(DeclKind::Function(f)) if f.name == "main"
        ));
        // The prelude declares the `ToString` trait.
        assert!(combined
            .items
            .iter()
            .any(|d| matches!(&d.kind, DeclKind::Trait(t) if t.name == "ToString")));
    }

    #[test]
    fn transitive_std_import_merges_dependency_once() {
        // `std/path` imports `std/string`. A user importing only `std/path`
        // must still get `std/string`'s items merged (so path's `String`
        // method calls resolve), and exactly once.
        let user = parse_src("import std/path { basename }\nfun main() {}\n");
        let combined = expand_with_stdlib(&user).expect("expand");

        // `std/string` declares the free helper `is_space_byte`; it must be
        // present under its namespaced name exactly once.
        let string_helper = mangle_stdlib_fn("string", "is_space_byte");
        let count = combined
            .items
            .iter()
            .filter(|d| matches!(&d.kind, DeclKind::Function(f) if f.name == string_helper))
            .count();
        assert_eq!(count, 1, "std/string must merge exactly once");

        // `std/string`'s `impl String` methods (resolved by type) must be
        // present so path's `p.length()` etc. resolve.
        let has_length = combined.items.iter().any(|d| {
            matches!(
                &d.kind,
                DeclKind::Impl(imp) if imp.items.iter().any(|m| m.name == "length")
            )
        });
        assert!(has_length, "std/string impl methods must be merged");

        // The bundled module's own `import std/string` line must not leak
        // into the combined file as an import item. The only std import
        // present should be the user's own `import std/path`.
        let leaked_string_import = combined.items.iter().any(|d| match &d.kind {
            DeclKind::Import(i) => matches!(
                &i.source,
                ImportSource::Std(s) if s.first().map(|x| x.as_str()) == Some("string")
            ),
            _ => false,
        });
        assert!(
            !leaked_string_import,
            "a bundled module's std import must be stripped from the merged file"
        );
    }

    #[test]
    fn duplicate_imports_load_module_once() {
        let user = parse_src("import std/io { println }\nimport std/io { print }\nfun main() {}\n");
        let combined = expand_with_stdlib(&user).expect("expand");
        let println_count = combined
            .items
            .iter()
            .filter(|d| matches!(&d.kind, DeclKind::Function(f) if f.name == "std.io.println"))
            .count();
        assert_eq!(println_count, 1);
    }

    #[test]
    fn local_key_is_stable_per_path() {
        let a = PathBuf::from("/tmp/helper.rv");
        let b = PathBuf::from("/tmp/other.rv");
        assert_eq!(local_module_key(&a), local_module_key(&a));
        assert_ne!(local_module_key(&a), local_module_key(&b));
        let key = local_module_key(&a);
        assert!(key.starts_with("loc."));
        assert_eq!(mangle_local_fn(&key, "greet"), format!("{key}.greet"));
    }

    /// Write `files` (relative name to source) into a fresh temp dir and
    /// return the dir and the absolute path of `entry`.
    fn write_temp_project(files: &[(&str, &str)], entry: &str) -> (PathBuf, PathBuf) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "raven_stdlib_test_{}_{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("mkdir");
        for (name, src) in files {
            std::fs::write(dir.join(name), src).expect("write");
        }
        let entry_path = dir.join(entry);
        (dir, entry_path)
    }

    fn parse_at(src: &str, path: &Path) -> File {
        let tokens = Lexer::new(src.to_string(), path.to_path_buf())
            .tokenize()
            .expect("lex");
        parse(&tokens).expect("parse")
    }

    #[test]
    fn local_module_functions_are_merged_and_namespaced() {
        let (dir, entry) = write_temp_project(
            &[
                (
                    "helper.rv",
                    "fun greet(name: String) -> String { return name }\n",
                ),
                ("main.rv", "import \"./helper\" { greet }\nfun main() {}\n"),
            ],
            "main.rv",
        );
        let canon = dir.join("helper.rv").canonicalize().expect("canon");
        let user = parse_at("import \"./helper\" { greet }\nfun main() {}\n", &entry);
        let combined = expand_with_stdlib(&user).expect("expand");
        let key = local_module_key(&canon);
        let mangled = mangle_local_fn(&key, "greet");
        let present = combined
            .items
            .iter()
            .any(|d| matches!(&d.kind, DeclKind::Function(f) if f.name == mangled));
        assert!(present, "local function should merge under {mangled}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn local_module_struct_keeps_its_own_name() {
        let (dir, entry) = write_temp_project(
            &[
                ("shapes.rv", "struct Point { x: Int }\n"),
                ("main.rv", "import \"./shapes\" { Point }\nfun main() {}\n"),
            ],
            "main.rv",
        );
        let user = parse_at("import \"./shapes\" { Point }\nfun main() {}\n", &entry);
        let combined = expand_with_stdlib(&user).expect("expand");
        let has_point = combined
            .items
            .iter()
            .any(|d| matches!(&d.kind, DeclKind::Struct(s) if s.name == "Point"));
        assert!(has_point, "a local struct merges under its own name");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn transitive_local_imports_merge_and_rewrite_calls() {
        let (dir, entry) = write_temp_project(
            &[
                ("b.rv", "fun base() -> Int { return 1 }\n"),
                (
                    "a.rv",
                    "import \"./b\" { base }\nfun via() -> Int { return base() }\n",
                ),
                ("main.rv", "import \"./a\" { via }\nfun main() {}\n"),
            ],
            "main.rv",
        );
        let canon_b = dir.join("b.rv").canonicalize().expect("canon b");
        let user = parse_at("import \"./a\" { via }\nfun main() {}\n", &entry);
        let combined = expand_with_stdlib(&user).expect("expand");

        // `a::via` calls `base`, imported from `./b`. The merged `via` body
        // must reference `b`'s namespaced symbol, not the bare name.
        let key_b = local_module_key(&canon_b);
        let base_mangled = mangle_local_fn(&key_b, "base");
        let via = combined
            .items
            .iter()
            .filter_map(|d| match &d.kind {
                DeclKind::Function(f) if f.name.ends_with(".via") => Some(f),
                _ => None,
            })
            .next()
            .expect("via present");
        let mut idents = Vec::new();
        collect_fn_body_idents(&via.body, &mut idents);
        assert!(
            idents.iter().any(|n| *n == base_mangled),
            "via should call {base_mangled}, got {idents:?}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    fn collect_fn_body_idents(body: &FunctionBody, out: &mut Vec<String>) {
        if let FunctionBody::Block(b) = body {
            collect_block_idents(b, out);
        }
    }
}
