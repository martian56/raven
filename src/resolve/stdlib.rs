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
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::ast::{
    Block, Decl, DeclKind, ElseBranch, Expr, ExprKind, File, Function, FunctionBody, GenericParam,
    ImportSource, LambdaBody, MatchArm, Pattern, PatternKind, Stmt, StmtKind, StrFragment, Type,
    TypeKind, TypePath, VariantPayload,
};
use crate::error::{RavenError, ResolveError};
use crate::lexer::Lexer;
use crate::macros::{collect_macro_table, expand_tokens_hygienic, DefSites};
use crate::parser::{parse, parse_with_macros};

use super::imports::{FsLoader, GithubPath, SourceLoader};

/// The embedded source of one bundled stdlib module, keyed by its module
/// path under `std/`. A `std/io` import maps to the `"io"` entry. The
/// list grows as later modules (issues #72 to #80) land; each adds one
/// `include_str!` row here.
pub const BUNDLED_MODULES: &[(&str, &str)] = &[
    ("core", include_str!("../../stdlib/std/core.rv")),
    ("io", include_str!("../../stdlib/std/io.rv")),
    ("string", include_str!("../../stdlib/std/string.rv")),
    ("iter", include_str!("../../stdlib/std/iter.rv")),
    ("list", include_str!("../../stdlib/std/list.rv")),
    ("option", include_str!("../../stdlib/std/option.rv")),
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
    ("sync", include_str!("../../stdlib/std/sync.rv")),
    ("tls", include_str!("../../stdlib/std/tls.rv")),
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

/// The rename-map key under which a whole-module alias (`import "./b" as dep`)
/// records its target module key, so the merge rewrite can turn a qualified
/// `dep.member` access into the namespaced symbol. The `@` prefix cannot be a
/// real identifier, so it never collides with an ordinary rename entry and a
/// bare-identifier rewrite never matches it.
fn alias_rename_key(alias: &str) -> String {
    format!("@module-alias@{alias}")
}

/// Build a namespacing key for one external package source file:
/// `ext.<host>.<user>.<repo>.<hash>`. The host/user/repo segments are
/// sanitized (any `.` in `host` becomes `_`) so the key has a fixed dot
/// arity, and a hash of the resolved source file path disambiguates two
/// files within the same package. As with the local key, the `.` makes
/// the result unwritable by a user, so an external function never
/// collides with a user declaration.
pub fn external_module_key(host: &str, user: &str, repo: &str, source_path: &Path) -> String {
    let mut hasher = DefaultHasher::new();
    source_path.to_string_lossy().hash(&mut hasher);
    let h = host.replace('.', "_");
    format!(
        "ext{sep}{h}{sep}{user}{sep}{repo}{sep}{:016x}",
        hasher.finish(),
        sep = NAMESPACE_SEP
    )
}

/// Build the mangled name of an external package function:
/// `ext.<host>.<user>.<repo>.<hash>.<name>`.
pub fn mangle_external_fn(key: &str, name: &str) -> String {
    format!("{key}{sep}{name}", sep = NAMESPACE_SEP)
}

/// The package context an external (`github.com/...`) import resolves
/// against. It pairs the rvpm cache root with the loaded `rv.lock` map
/// from `github.com/<user>/<repo>` source paths to their pinned version,
/// so an `import "github.com/user/repo[/sub]"` can be located in the
/// cache. Bundled and local imports never consult it.
#[derive(Debug, Clone)]
pub struct PackageContext {
    cache_root: PathBuf,
    /// Map from the lowercased `github.com/<user>/<repo>` source to its locked
    /// `(canonical source, pinned version)`. GitHub owner and repo paths are
    /// case-insensitive, so an import that differs only in case from the locked
    /// source still resolves; the canonical (locked) casing is used for the
    /// cache path, matching the directory the fetch created.
    locked_versions: BTreeMap<String, (String, String)>,
}

impl PackageContext {
    /// Build a context from an explicit cache root and a lock file.
    pub fn new(cache_root: PathBuf, lock: &crate::lock::LockFile) -> PackageContext {
        let mut locked_versions = BTreeMap::new();
        for p in &lock.packages {
            locked_versions.insert(
                p.source.to_ascii_lowercase(),
                (p.source.clone(), p.version.clone()),
            );
        }
        PackageContext {
            cache_root,
            locked_versions,
        }
    }

    /// Look up an import source against the lock, case-insensitively, returning
    /// the canonical (locked) source and its pinned version.
    fn locked(&self, source: &str) -> Option<&(String, String)> {
        self.locked_versions.get(&source.to_ascii_lowercase())
    }

    fn package_dir(&self, source: &str) -> Option<PathBuf> {
        let (canonical, version) = self.locked(source)?;
        let gh = GithubPath::parse(canonical)?;
        Some(crate::pkg::cache_dir_in(
            &self.cache_root,
            &gh.host,
            &gh.user,
            &gh.repo,
            version,
        ))
    }

    fn checked_package_path(&self, source: &str, candidate: PathBuf) -> Option<PathBuf> {
        if !candidate.exists() {
            return Some(candidate);
        }

        let package_dir = self.package_dir(source)?;
        let cache_root = self.cache_root.canonicalize().ok()?;
        let package_root = package_dir.canonicalize().ok()?;
        if !package_root.starts_with(&cache_root) {
            return None;
        }

        let resolved = candidate.canonicalize().ok()?;
        resolved.starts_with(&package_root).then_some(resolved)
    }

    /// Resolve the cached `.rv` source file for a `github.com/<user>/<repo>`
    /// path (the `source` key in the lock) and an import `subpath`.
    ///
    /// The bare `github.com/user/repo` import (no subpath) resolves to the
    /// package's `lib.rv` at the cached root. A `subpath` selects a `.rv`
    /// file by joining its components and appending `.rv`, so
    /// `github.com/acme/greet/lib` resolves to `<cachedir>/lib.rv` and
    /// `github.com/acme/greet/util/text` resolves to
    /// `<cachedir>/util/text.rv`. Returns the resolved file path, or `None`
    /// when the package is not pinned in the lock.
    pub fn external_source_path(&self, source: &str, subpath: &[String]) -> Option<PathBuf> {
        let mut file = self.package_dir(source)?;
        if subpath.is_empty() {
            file.push("lib.rv");
        } else {
            for seg in &subpath[..subpath.len() - 1] {
                file.push(seg);
            }
            file.push(format!("{}.rv", subpath[subpath.len() - 1]));
        }
        self.checked_package_path(source, file)
    }

    fn external_local_source_path(
        &self,
        source: &str,
        importing: &Path,
        target: &str,
    ) -> Option<PathBuf> {
        let parent = importing.parent().unwrap_or_else(|| Path::new("."));
        let mut path = parent.join(target);
        if path.extension().is_none() {
            path.set_extension("rv");
        }
        self.checked_package_path(source, path)
    }

    /// The path to a cached package's own `rv.toml`, used to read its
    /// transitive dependencies. Returns `None` when the package is not
    /// pinned in the lock.
    fn package_manifest_path(&self, source: &str) -> Option<PathBuf> {
        let path = self.package_dir(source)?.join("rv.toml");
        self.checked_package_path(source, path)
    }
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
    expand_with_stdlib_ctx(user, None).map(|(file, _def_sites)| file)
}

/// Expand `user` like [`expand_with_stdlib`], additionally resolving any
/// external (`github.com/...`) imports through `ctx` (the rvpm cache plus
/// the project's `rv.lock`).
///
/// When `ctx` is `None`, the behavior is identical to bundled+local
/// expansion and an external import is left for the import pass to handle
/// (it stays a deferred `ExternalPackage` target). When `ctx` is
/// `Some`, each external import's source is read from the cache, parsed,
/// namespaced under `ext.<host>.<user>.<repo>.<hash>`, and merged through
/// the same [`merge_module_items`] core the bundled and local paths use.
/// The external package's own dependencies (from its cached `rv.toml`)
/// are merged transitively and deduplicated by resolved source path.
pub fn expand_with_stdlib_ctx(
    user: &File,
    ctx: Option<&PackageContext>,
) -> Result<(File, DefSites), RavenError> {
    // Definition-site identifiers introduced by macros in imported local
    // modules, accumulated as the modules are loaded and handed back to the
    // resolver alongside the entry file's own.
    let mut def_sites = DefSites::new();
    // The `@derive` expansion emits global helper functions under a reserved
    // prefix; reject a user declaration that would collide with one before any
    // merging, so the error names the user's declaration rather than the
    // synthetic helper source.
    super::derive::reject_reserved_helper_names(user)?;
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

    // `@derive(Debug)` synthesizes an `impl Debug`, whose trait lives in
    // `std/fmt`. Force-merge that module so the generated impl resolves even
    // when the user wrote no `import std/fmt` line, mirroring how the prelude
    // is always present.
    if super::derive::needs_fmt_module(user) {
        wanted.insert("fmt".to_string());
    }

    // `@derive(ToJson)`/`@derive(FromJson)` synthesize impls that reference
    // the `JsonValue` tree and the JSON traits in `std/json`. Force-merge
    // that module (it transitively pulls in `std/error` and
    // `std/collections`) so the generated impls resolve even when the user
    // wrote no `import std/json` line.
    if super::derive::needs_json_module(user) {
        wanted.insert("json".to_string());
    }

    // Load every local `./`/`../` module reachable from the user file,
    // transitively, before computing the bundled set: a local module may
    // itself `import std/...`, and those bundled modules must merge too so
    // the local module's own calls resolve.
    let mut loader = FsLoader;
    let local_modules = load_local_modules(user, &mut loader, &mut def_sites)?;
    for module_file in &local_modules {
        collect_std_module_imports(module_file, &mut wanted);
    }

    // Load every external (`github.com/...`) package source reachable from
    // the user file and its local modules, transitively. Each loaded file
    // may itself `import std/...`, so its bundled modules must merge too.
    let external_modules = match ctx {
        Some(ctx) => load_external_modules(user, &local_modules, ctx)?,
        None => Vec::new(),
    };
    for ext in &external_modules {
        collect_std_module_imports(&ext.file, &mut wanted);
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

    // The shared JSON derive helper free functions are global and fixed-named,
    // so they are emitted exactly once for the whole program below, after every
    // module and the user's own items have had their derives expanded. Track
    // whether any of those expansions needs them.
    let mut needs_json_helpers = false;

    // Merge the local modules loaded above through the same merge core,
    // with a path derived namespace instead of the `std.<module>.` one.
    for module_file in local_modules {
        let importing = (*module_file.span.file).clone();
        let key = local_module_key(&importing);
        let mut rename = import_rename_map(&module_file, &importing, &mut loader);
        // A local module may itself import an external (github) package. Map
        // those selectors to their `ext.<...>` symbols too, the same as an
        // external module does, so a free function the local module imports
        // (`import "github.com/..." { f }`, then `f(...)`) is rewritten rather
        // than left unresolved. `import_rename_map` cannot do this on its own
        // because it has no package context.
        if let Some(ctx) = ctx {
            for (selector, symbol) in external_import_rename_map(&module_file, ctx, None) {
                rename.insert(selector, symbol);
            }
            for (alias_key, target_key) in whole_module_external_alias_renames(&module_file, ctx) {
                rename.insert(alias_key, target_key);
            }
        }
        // A whole-module alias (`import "./b" as dep`) is recorded so the rewrite
        // can turn a qualified `dep.fn()` call into b's namespaced symbol; the
        // import declaration itself is stripped by the merge.
        for (alias_key, target_key) in
            whole_module_alias_renames(&module_file, &importing, &mut loader)
        {
            rename.insert(alias_key, target_key);
        }
        // A bare stdlib import (`import std/net`, no selectors) makes `net.fn()`
        // usable. Record the implicit alias so the merge rewrites the qualified
        // call to the stdlib's namespaced symbol. Without this the qualifier
        // resolves in a main file (whose import declarations are kept) but not in
        // a merged library file, where the import is stripped (issue #831).
        for (alias_key, target_key) in whole_module_stdlib_alias_renames(&module_file) {
            rename.insert(alias_key, target_key);
        }
        for name in top_level_fn_names(&module_file) {
            rename.insert(name.clone(), mangle_local_fn(&key, &name));
        }
        // Types are namespaced the same way functions are, so two local
        // modules can both declare a type of the same name.
        for name in top_level_type_names(&module_file) {
            rename.insert(name.clone(), mangle_local_fn(&key, &name));
        }
        // Module globals (`let`/`const`) are namespaced too, so two local
        // modules can each declare a global of the same name.
        for name in top_level_global_names(&module_file) {
            rename.insert(name.clone(), mangle_local_fn(&key, &name));
        }
        // A per-module label keeps the generated source's use-site spans from
        // colliding with another module's `<derive>` source.
        let (items, needs) = expand_module_derives(module_file.items, &format!("<derive:{key}>"))?;
        needs_json_helpers |= needs;
        merge_module_items(items, &rename, &mut combined_items);
    }

    // Merge the external package sources through the same merge core, with
    // an `ext.<host>.<user>.<repo>.<hash>` namespace. Each module's rename
    // map carries its own functions plus the names it selectively imports
    // from sibling external sources (resolved through the same context).
    if let Some(ctx) = ctx {
        for ext in external_modules {
            let key = external_module_key(&ext.host, &ext.user, &ext.repo, &ext.source_path);
            let mut rename = external_import_rename_map(&ext.file, ctx, Some(&ext.source));
            // A bare stdlib import (`import std/net`, no selectors) used through a
            // qualifier inside an external package, the same #831 case as the
            // local path above.
            for (alias_key, target_key) in whole_module_stdlib_alias_renames(&ext.file) {
                rename.insert(alias_key, target_key);
            }
            // A whole-module alias (`import "github.com/x/b" as dep`) inside an
            // external module is recorded too, so a qualified `dep.fn()` resolves
            // to b's namespaced symbol after the import declaration is stripped.
            for (alias_key, target_key) in whole_module_external_alias_renames(&ext.file, ctx) {
                rename.insert(alias_key, target_key);
            }
            // External package files may import sibling files with a local path.
            // Their import declarations are stripped during merge too, so keep
            // whole-module aliases for those sibling modules.
            for (alias_key, target_key) in
                whole_module_external_local_alias_renames(&ext.file, ctx, &ext.source)
            {
                rename.insert(alias_key, target_key);
            }
            for name in top_level_fn_names(&ext.file) {
                rename.insert(name.clone(), mangle_external_fn(&key, &name));
            }
            // Types are namespaced like functions, so two packages can both
            // export a type of the same name without colliding at merge.
            for name in top_level_type_names(&ext.file) {
                rename.insert(name.clone(), mangle_external_fn(&key, &name));
            }
            // Module globals are namespaced too, the same as functions.
            for name in top_level_global_names(&ext.file) {
                rename.insert(name.clone(), mangle_external_fn(&key, &name));
            }
            let (items, needs) = expand_module_derives(ext.file.items, &format!("<derive:{key}>"))?;
            needs_json_helpers |= needs;
            merge_module_items(items, &rename, &mut combined_items);
        }
    }

    // The user's items follow the stdlib items so user DeclIds shift by a
    // fixed, known amount but otherwise keep their relative order. The
    // combined file's span borrows the user's file path for diagnostics
    // that key off the top level file.
    combined_items.extend(user.items.iter().cloned());

    // Synthesize trait impls for every `@derive(...)` attribute. The user's
    // own types carry the attributes, so this runs over the full combined
    // item list (a stdlib type never carries a derive, so scanning all of it
    // is harmless). The generated impls append after the user items and flow
    // through resolve, type checking, and codegen like hand written ones.
    let mut derived_impls = Vec::new();
    needs_json_helpers |=
        super::derive::expand_derives(&combined_items, &mut derived_impls, "<derive>")?;
    combined_items.append(&mut derived_impls);

    // Emit the shared JSON derive helpers exactly once for the whole program,
    // as bare global free functions the derived `from_json` bodies (in the user
    // and in every merged module) all call by name. Emitting them per module
    // declared them several times in a multi-file project.
    if needs_json_helpers {
        combined_items.extend(super::derive::json_helper_decls()?);
    }

    Ok((
        File {
            items: combined_items,
            span: user.span.clone(),
        },
        def_sites,
    ))
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
/// Apply a declaration-name rename from the map, if present.
fn rename_decl(name: &mut String, rename: &HashMap<String, String>) {
    if let Some(replacement) = rename.get(name) {
        *name = replacement.clone();
    }
}

/// Clear `@derive(...)` requests from a module's types.
fn strip_derives(items: &mut [Decl]) {
    for d in items {
        match &mut d.kind {
            DeclKind::Struct(s) => s.derives.clear(),
            DeclKind::Enum(e) => e.derives.clear(),
            _ => {}
        }
    }
}

/// Expand a merged module's `@derive(...)` requests on its bare type names,
/// before the module is namespaced. The synthesized impls are appended to the
/// module's items and the derive requests are stripped, so the impls are
/// namespaced together with the type they target (the global derive pass runs
/// on the bare-named user and stdlib items only). Doing this after namespacing
/// would fail, since the generated source is re-lexed and a namespaced name
/// carries dots and a hash that do not lex as one identifier.
fn expand_module_derives(
    mut items: Vec<Decl>,
    source_label: &str,
) -> Result<(Vec<Decl>, bool), RavenError> {
    let mut derived = Vec::new();
    let needs_json = super::derive::expand_derives(&items, &mut derived, source_label)?;
    strip_derives(&mut items);
    items.extend(derived);
    Ok((items, needs_json))
}

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
                rewrite_fn(f, rename);
                rename_decl(&mut f.name, rename);
            }
            DeclKind::Struct(s) => {
                rename_decl(&mut s.name, rename);
                rewrite_generics(&mut s.generics, rename);
                for field in &mut s.fields {
                    rewrite_type(&mut field.ty, rename);
                }
            }
            DeclKind::Enum(e) => {
                rename_decl(&mut e.name, rename);
                rewrite_generics(&mut e.generics, rename);
                for v in &mut e.variants {
                    match &mut v.payload {
                        VariantPayload::Tuple(tys) => {
                            for t in tys {
                                rewrite_type(t, rename);
                            }
                        }
                        VariantPayload::Struct(fields) => {
                            for f in fields {
                                rewrite_type(&mut f.ty, rename);
                            }
                        }
                        VariantPayload::Unit => {}
                    }
                }
            }
            DeclKind::Trait(t) => {
                rename_decl(&mut t.name, rename);
                rewrite_generics(&mut t.generics, rename);
                for m in &mut t.members {
                    rewrite_fn(m, rename);
                }
            }
            DeclKind::Impl(i) => {
                // The impl target and (for a trait impl) the trait it
                // implements are type references that follow the same rename
                // as the declarations. Method names are dispatched by receiver
                // type so they keep their names, but their signatures and
                // bodies reference types that may have been namespaced.
                rewrite_generics(&mut i.generics, rename);
                rewrite_type_path(&mut i.trait_or_type, rename);
                if let Some(for_type) = &mut i.for_type {
                    rewrite_type_path(for_type, rename);
                }
                for m in &mut i.items {
                    rewrite_fn(m, rename);
                }
            }
            DeclKind::Const(c) => {
                rename_decl(&mut c.name, rename);
                if let Some(t) = &mut c.ty {
                    rewrite_type(t, rename);
                }
                rewrite_expr(&mut c.value, rename);
            }
            DeclKind::Let(l) => {
                rename_decl(&mut l.name, rename);
                if let Some(t) = &mut l.ty {
                    rewrite_type(t, rename);
                }
                if let Some(e) = &mut l.init {
                    rewrite_expr(e, rename);
                }
            }
            DeclKind::Extern(_) | DeclKind::Import(_) | DeclKind::Macro(_) => {}
        }
        combined.push(decl);
    }
}

/// Load every local `./` or `../` module reachable from `user`,
/// transitively, returning each parsed file exactly once. Each returned
/// file's `span.file` is its canonical path, so a later caller can derive
/// the namespacing key from the same path the import binder uses.
///
/// Modules come back in dependency order: a module appears after every module
/// it imports (a post-order walk). The combined program's global-initializer
/// runs the modules' top-level `let` initializers in this order, so an imported
/// module's globals are initialized before an importer that reads them at load
/// time.
///
/// Modules are deduplicated by canonical path, so a module imported from
/// several places loads once. A cycle (a module that imports itself
/// directly or transitively) is broken gracefully: each module is loaded
/// once and the back edge is ignored, mirroring the bundled set's fixed
/// point behavior. A missing file is left for the import resolution pass
/// to report with a precise span; this function only loads what it can.
fn load_local_modules(
    user: &File,
    loader: &mut dyn SourceLoader,
    def_sites: &mut DefSites,
) -> Result<Vec<File>, RavenError> {
    let mut loaded_paths: BTreeSet<PathBuf> = BTreeSet::new();
    let mut out: Vec<File> = Vec::new();
    for (importing, target) in local_import_targets(user) {
        load_local_module(
            &importing,
            &target,
            loader,
            &mut loaded_paths,
            &mut out,
            def_sites,
        )?;
    }
    Ok(out)
}

/// Load the module `target` (resolved relative to `importing`) and, before it,
/// every module it imports, depth first. A module is pushed to `out` only after
/// its dependencies, so the result is in dependency order; a module already in
/// `loaded` (a diamond or a cycle back edge) is skipped.
fn load_local_module(
    importing: &Path,
    target: &str,
    loader: &mut dyn SourceLoader,
    loaded: &mut BTreeSet<PathBuf>,
    out: &mut Vec<File>,
    def_sites: &mut DefSites,
) -> Result<(), RavenError> {
    let Some(loaded_mod) = loader.load(importing, target) else {
        return Ok(());
    };
    if !loaded.insert(loaded_mod.canonical_path.clone()) {
        return Ok(());
    }

    let tokens = Lexer::new(loaded_mod.source.clone(), loaded_mod.canonical_path.clone())
        .tokenize()
        .map_err(|e| local_error(&loaded_mod.canonical_path, format!("lex: {e}")))?;
    // Expand the module's own declarative macros, the same pre-pass the entry
    // file gets, so a macro call inside an imported module is rewritten rather
    // than reaching later stages unexpanded. The macro table (collected before
    // the definitions are stripped) handles a call inside a `"${...}"`
    // interpolation fragment, and the def-sites flow to the resolver so a free
    // identifier the module's macros introduce resolves at the module scope.
    let table = collect_macro_table(&tokens)
        .map_err(|e| local_error(&loaded_mod.canonical_path, format!("macro: {e}")))?;
    let (tokens, module_def_sites) = expand_tokens_hygienic(&tokens)
        .map_err(|e| local_error(&loaded_mod.canonical_path, format!("macro: {e}")))?;
    def_sites.extend(module_def_sites);
    let (module_file, interp_def_sites) = parse_with_macros(&tokens, table)
        .map_err(|e| local_error(&loaded_mod.canonical_path, format!("parse: {e}")))?;
    // Fold in the def-sites a macro expanded inside one of the module's
    // `"${...}"` interpolation fragments introduced.
    def_sites.extend(interp_def_sites);

    // Load the imported modules first, so they sit ahead of this one in `out`
    // and their globals initialize before this module's do.
    for (_, dep) in local_import_targets(&module_file) {
        load_local_module(
            &loaded_mod.canonical_path,
            &dep,
            loader,
            loaded,
            out,
            def_sites,
        )?;
    }
    out.push(module_file);
    Ok(())
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
                        for sel in &import.selectors {
                            // Only functions are namespaced; a type keeps its
                            // own name (see `merge_module_items`), so a type
                            // selector needs no rename. The call site uses the
                            // local name, mapped to the exported name's symbol.
                            if fns.contains(&sel.name) {
                                map.insert(
                                    sel.local().to_string(),
                                    mangle_stdlib_fn(module, &sel.name),
                                );
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
                        let types = top_level_type_names(&target);
                        for sel in &import.selectors {
                            if fns.contains(&sel.name) || types.contains(&sel.name) {
                                map.insert(
                                    sel.local().to_string(),
                                    mangle_local_fn(&key, &sel.name),
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    map
}

/// Collect the whole-module imports of `file` (`import "./b"` or
/// `import "./b" as dep`, with no selector list) as `(alias rename key, target
/// module key)` pairs. A merged module strips its own import declarations, so a
/// qualified `dep.member` or `b.member` call would otherwise lose the binding
/// the import stood for; the merge rewrite uses these to namespace it instead.
/// Only local (`./`, `../`) imports are handled here.
fn whole_module_alias_renames(
    file: &File,
    importing: &Path,
    loader: &mut dyn SourceLoader,
) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for decl in &file.items {
        let DeclKind::Import(import) = &decl.kind else {
            continue;
        };
        if !import.selectors.is_empty() {
            continue;
        }
        let ImportSource::Quoted(path) = &import.source else {
            continue;
        };
        if !(path.starts_with("./") || path.starts_with("../")) {
            continue;
        }
        if let Some(loaded) = loader.load(importing, path) {
            let alias = import.alias.clone().unwrap_or_else(|| {
                loaded
                    .canonical_path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or(path)
                    .to_string()
            });
            let key = local_module_key(&loaded.canonical_path);
            out.push((alias_rename_key(&alias), key));
        }
    }
    out
}

/// Collect bare stdlib imports (`import std/net`, no selector list) as
/// `(alias rename key, "std.<module>" target key)` pairs, so a qualified
/// `net.fn()` call is rewritten to the stdlib's namespaced symbol
/// (`mangle_local_fn("std.net", "fn")` equals `mangle_stdlib_fn("net", "fn")`).
/// The implicit alias is the explicit `as` name or the last path segment,
/// matching `bind_import`. Mirrors [`whole_module_alias_renames`] for the
/// stdlib case so qualified access works in a merged library file too.
fn whole_module_stdlib_alias_renames(file: &File) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for decl in &file.items {
        let DeclKind::Import(import) = &decl.kind else {
            continue;
        };
        if !import.selectors.is_empty() {
            continue;
        }
        let ImportSource::Std(segments) = &import.source else {
            continue;
        };
        let Some(module) = segments.first() else {
            continue;
        };
        if parse_bundled_module(module).is_err() {
            continue;
        }
        let alias = import.alias.clone().unwrap_or_else(|| {
            segments
                .last()
                .cloned()
                .unwrap_or_else(|| module.to_string())
        });
        let target_key = format!("std{sep}{module}", sep = NAMESPACE_SEP);
        out.push((alias_rename_key(&alias), target_key));
    }
    out
}

/// Like [`whole_module_alias_renames`] but for a `github.com/...` whole-module
/// import, mapping the alias (explicit or default) to the target package's
/// `ext.` namespace key. The two share the `key.member` mangling, so the same
/// rewrite handles a qualified call through either kind of alias.
fn whole_module_external_alias_renames(file: &File, ctx: &PackageContext) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for decl in &file.items {
        let DeclKind::Import(import) = &decl.kind else {
            continue;
        };
        if !import.selectors.is_empty() {
            continue;
        }
        let ImportSource::Quoted(path) = &import.source else {
            continue;
        };
        let Some(gh) = GithubPath::parse(path) else {
            continue;
        };
        let source = format!("github.com/{}/{}", gh.user, gh.repo);
        if let Some(src_path) = ctx.external_source_path(&source, &gh.subpath) {
            let alias = import.alias.clone().unwrap_or_else(|| {
                gh.subpath
                    .last()
                    .cloned()
                    .unwrap_or_else(|| gh.repo.clone())
            });
            let key = external_module_key(&gh.host, &gh.user, &gh.repo, &src_path);
            out.push((alias_rename_key(&alias), key));
        }
    }
    out
}

fn whole_module_external_local_alias_renames(
    file: &File,
    ctx: &PackageContext,
    source: &str,
) -> Vec<(String, String)> {
    let Some(gh) = GithubPath::parse(source) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for decl in &file.items {
        let DeclKind::Import(import) = &decl.kind else {
            continue;
        };
        if !import.selectors.is_empty() {
            continue;
        }
        let ImportSource::Quoted(path) = &import.source else {
            continue;
        };
        if !(path.starts_with("./") || path.starts_with("../")) {
            continue;
        }
        if let Some(src_path) =
            ctx.external_local_source_path(source, file.span.file.as_ref().as_path(), path)
        {
            let alias = import.alias.clone().unwrap_or_else(|| {
                src_path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or(path)
                    .to_string()
            });
            let key = external_module_key(&gh.host, &gh.user, &gh.repo, &src_path);
            out.push((alias_rename_key(&alias), key));
        }
    }
    out
}

/// One loaded external package source file plus the package identity it
/// belongs to, so the merge can build its `ext.` namespace key.
struct ExternalModule {
    source: String,
    host: String,
    user: String,
    repo: String,
    /// The resolved cache path of the `.rv` source file.
    source_path: PathBuf,
    file: File,
}

/// The components of one external import: the `github.com/<user>/<repo>`
/// lock source key and the import `subpath`.
fn external_import_targets(file: &File) -> Vec<(String, Vec<String>)> {
    let mut out = Vec::new();
    for decl in &file.items {
        if let DeclKind::Import(import) = &decl.kind {
            if let ImportSource::Quoted(path) = &import.source {
                if let Some(gh) = GithubPath::parse(path) {
                    let source = format!("github.com/{}/{}", gh.user, gh.repo);
                    out.push((source, gh.subpath));
                }
            }
        }
    }
    out
}

/// Load every external package source reachable from `user` and its
/// `local_modules`, transitively, returning each parsed file exactly once
/// (deduplicated by its resolved cache path).
///
/// Each import resolves to a `.rv` file in the rvpm cache through `ctx`.
/// The loaded file's own external imports are followed, and the package's
/// cached `rv.toml` is read so a bare-package dependency (with no explicit
/// import of one of its files) still has its declarations available. An
/// import whose package is not pinned in the lock, or whose source file is
/// missing, is skipped here; the import resolution pass reports it with a
/// precise span.
fn load_external_modules(
    user: &File,
    local_modules: &[File],
    ctx: &PackageContext,
) -> Result<Vec<ExternalModule>, RavenError> {
    let mut queue: Vec<(String, PathBuf)> = Vec::new();
    for (source, subpath) in external_import_targets(user) {
        if let Some(path) = ctx.external_source_path(&source, &subpath) {
            queue.push((source, path));
        }
    }
    for m in local_modules {
        for (source, subpath) in external_import_targets(m) {
            if let Some(path) = ctx.external_source_path(&source, &subpath) {
                queue.push((source, path));
            }
        }
    }
    let mut loaded_paths: BTreeSet<PathBuf> = BTreeSet::new();
    let mut out: Vec<ExternalModule> = Vec::new();

    while let Some((source, path)) = queue.pop() {
        if !loaded_paths.insert(path.clone()) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let gh = match GithubPath::parse(&source) {
            Some(gh) => gh,
            None => continue,
        };

        let tokens = Lexer::new(text, path.clone())
            .tokenize()
            .map_err(|e| external_error(&path, format!("lex: {e}")))?;
        let module_file =
            parse(&tokens).map_err(|e| external_error(&path, format!("parse: {e}")))?;

        // Follow this file's own external imports (to sibling files in the
        // same package or to other packages).
        for (dep_source, dep_subpath) in external_import_targets(&module_file) {
            if let Some(path) = ctx.external_source_path(&dep_source, &dep_subpath) {
                queue.push((dep_source, path));
            }
        }
        // Follow local imports declared by a cached package file. They resolve
        // relative to the importing package source and are checked to stay
        // inside the package root before any read happens.
        for (_, dep) in local_import_targets(&module_file) {
            if let Some(path) = ctx.external_local_source_path(&source, &path, &dep) {
                queue.push((source.clone(), path));
            }
        }
        // Read the package's cached manifest and queue each dependency's
        // entry file so a transitively required package merges even when
        // only its package root is imported.
        if let Some(manifest_path) = ctx.package_manifest_path(&source) {
            if let Ok(manifest) = crate::manifest::Manifest::load(&manifest_path) {
                for dep in &manifest.dependencies {
                    if let Some(path) = ctx.external_source_path(&dep.path, &[]) {
                        queue.push((dep.path.clone(), path));
                    }
                }
            }
        }

        out.push(ExternalModule {
            source,
            host: gh.host,
            user: gh.user,
            repo: gh.repo,
            source_path: path,
            file: module_file,
        });
    }

    Ok(out)
}

/// Build the rename entries an external module needs for the names it
/// selectively imports from OTHER external sources, mapping each selector
/// to the sibling's `ext.` namespaced symbol. Mirrors [`import_rename_map`]
/// for the external case. Selectors that name a type keep their own name
/// and need no rename.
fn external_import_rename_map(
    file: &File,
    ctx: &PackageContext,
    current_source: Option<&str>,
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
            // A stdlib selector (`import std/time { now_millis }`) inside an
            // external package binds the same way it does for bundled and
            // local modules. The package's stdlib imports are force-merged
            // into the bundled set (see `wanted`), so the functions exist
            // under `std.<module>.<name>` and the call sites must rename to
            // match. Without this, free functions a dependency imports from
            // std stay unresolved while its types and methods resolve.
            ImportSource::Std(segments) => {
                if let Some(module) = segments.first() {
                    if let Ok(target) = parse_bundled_module(module) {
                        let fns = top_level_fn_names(&target);
                        for sel in &import.selectors {
                            if fns.contains(&sel.name) {
                                map.insert(
                                    sel.local().to_string(),
                                    mangle_stdlib_fn(module, &sel.name),
                                );
                            }
                        }
                    }
                }
            }
            ImportSource::Quoted(path) => {
                let (gh, src_path) = if let Some(gh) = GithubPath::parse(path) {
                    let source = format!("github.com/{}/{}", gh.user, gh.repo);
                    let Some(src_path) = ctx.external_source_path(&source, &gh.subpath) else {
                        continue;
                    };
                    (gh, src_path)
                } else if path.starts_with("./") || path.starts_with("../") {
                    let Some(source) = current_source else {
                        continue;
                    };
                    let Some(gh) = GithubPath::parse(source) else {
                        continue;
                    };
                    let Some(src_path) = ctx.external_local_source_path(
                        source,
                        file.span.file.as_ref().as_path(),
                        path,
                    ) else {
                        continue;
                    };
                    (gh, src_path)
                } else {
                    continue;
                };
                let Ok(text) = std::fs::read_to_string(&src_path) else {
                    continue;
                };
                let Some(target) = parse_loaded(&text, &src_path) else {
                    continue;
                };
                let key = external_module_key(&gh.host, &gh.user, &gh.repo, &src_path);
                let fns = top_level_fn_names(&target);
                let types = top_level_type_names(&target);
                let globals = top_level_global_names(&target);
                for sel in &import.selectors {
                    if fns.contains(&sel.name)
                        || types.contains(&sel.name)
                        || globals.contains(&sel.name)
                    {
                        map.insert(sel.local().to_string(), mangle_external_fn(&key, &sel.name));
                    }
                }
            }
        }
    }
    map
}

/// Build a resolve error for an external package source that failed to
/// load. The lex or parse error is anchored at the start of the file.
fn external_error(path: &Path, detail: String) -> RavenError {
    let span = crate::span::Span::point(Arc::new(path.to_path_buf()), 0, 1, 1);
    RavenError::resolve(
        ResolveError::UnresolvedImport(path.display().to_string()),
        span,
    )
    .with_hint(format!("external package source failed to load: {detail}"))
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

/// Top level type names (struct, enum, trait) a module declares. Like
/// functions, an external or local module's types are namespaced at merge so
/// two packages can both export a type of the same name; the caller adds these
/// to the rename map.
fn top_level_type_names(file: &File) -> BTreeSet<String> {
    file.items
        .iter()
        .filter_map(|d| match &d.kind {
            DeclKind::Struct(s) => Some(s.name.clone()),
            DeclKind::Enum(e) => Some(e.name.clone()),
            DeclKind::Trait(t) => Some(t.name.clone()),
            _ => None,
        })
        .collect()
}

/// The module-level global (`let`/`const`) names a module declares. These are
/// namespaced like functions and types so two modules can each declare a global
/// of the same name without colliding under their bare names at merge.
fn top_level_global_names(file: &File) -> BTreeSet<String> {
    file.items
        .iter()
        .filter_map(|d| match &d.kind {
            DeclKind::Let(l) => Some(l.name.clone()),
            DeclKind::Const(c) => Some(c.name.clone()),
            _ => None,
        })
        .collect()
}

/// Rewrite the type names a type expression mentions, following the rename map.
/// Only the head segment of a path names a type; nested generic arguments are
/// rewritten recursively. Built-in and unrenamed names are left untouched.
fn rewrite_type(ty: &mut Type, rename: &HashMap<String, String>) {
    match &mut ty.kind {
        TypeKind::Path(p) | TypeKind::Dyn(p) => rewrite_type_path(p, rename),
        TypeKind::Optional(inner) => rewrite_type(inner, rename),
        TypeKind::Function { params, ret } => {
            for p in params {
                rewrite_type(p, rename);
            }
            rewrite_type(ret, rename);
        }
        TypeKind::Unit => {}
    }
}

fn rewrite_type_path(p: &mut TypePath, rename: &HashMap<String, String>) {
    if let Some(head) = p.segments.first_mut() {
        if let Some(replacement) = rename.get(&head.name) {
            head.name = replacement.clone();
        }
    }
    for seg in &mut p.segments {
        for g in &mut seg.generics {
            rewrite_type(g, rename);
        }
    }
}

/// Rewrite the trait names a generic parameter list bounds against, for
/// example the `Score` in `fun f<T: Score>(...)`.
fn rewrite_generics(generics: &mut [GenericParam], rename: &HashMap<String, String>) {
    for g in generics {
        for bound in &mut g.bounds {
            rewrite_type_path(bound, rename);
        }
    }
}

/// Rewrite a function's signature (generic bounds, parameter and return types)
/// and body. Used for free functions, trait members, and impl methods.
/// Whether `s` begins with a lowercase letter. A pattern's bare identifier is a
/// binding when it is lowercase and an enum or struct constructor when it is
/// PascalCase, so this tells a shadowing local apart from a constructor name.
fn starts_lowercase(s: &str) -> bool {
    s.chars().next().is_some_and(|c| c.is_lowercase())
}

/// Collect the value names a pattern binds, so a binding that shadows a module
/// global is dropped from the rename map within its scope. A bare lowercase
/// identifier is a binding; a PascalCase one names a constructor and binds
/// nothing. A tuple's leading name is a constructor; a struct field with no
/// nested pattern is a shorthand binding of the field name.
fn collect_pattern_bindings(pat: &Pattern, out: &mut Vec<String>) {
    match &pat.kind {
        PatternKind::Ident(name) => {
            if starts_lowercase(name) {
                out.push(name.clone());
            }
        }
        PatternKind::Tuple { elements, .. } => {
            for e in elements {
                collect_pattern_bindings(e, out);
            }
        }
        PatternKind::Struct { fields, .. } => {
            for f in fields {
                match &f.pattern {
                    Some(p) => collect_pattern_bindings(p, out),
                    None => out.push(f.name.clone()),
                }
            }
        }
        PatternKind::Wildcard | PatternKind::Literal(_) | PatternKind::Range { .. } => {}
    }
}

/// A copy of `rename` with every `bound` name it holds removed, or `None` when
/// `bound` shadows nothing (so the caller reuses the original map without a
/// clone). A local that shadows a module global must keep its own value rather
/// than be rewritten to the global's namespaced name within its scope.
fn rename_without_shadowed(
    rename: &HashMap<String, String>,
    bound: &[String],
) -> Option<HashMap<String, String>> {
    if bound.iter().any(|n| rename.contains_key(n)) {
        let mut m = rename.clone();
        for n in bound {
            m.remove(n);
        }
        Some(m)
    } else {
        None
    }
}

fn rewrite_fn(f: &mut Function, rename: &HashMap<String, String>) {
    rewrite_generics(&mut f.generics, rename);
    for p in &mut f.params {
        rewrite_type(&mut p.ty, rename);
    }
    if let Some(ret) = &mut f.ret {
        rewrite_type(ret, rename);
    }
    // A parameter shadows a module global of the same name throughout the body,
    // so it is dropped from the rename map the body is rewritten with.
    let bound: Vec<String> = f.params.iter().map(|p| p.name.clone()).collect();
    match rename_without_shadowed(rename, &bound) {
        Some(inner) => rewrite_fn_body_calls(&mut f.body, &inner),
        None => rewrite_fn_body_calls(&mut f.body, rename),
    }
}

/// Rewrite the type names a pattern mentions: a struct pattern's name is a
/// type, while a tuple pattern's name is an enum variant (resolved by the
/// scrutinee) that is never in the type rename map, so the lookup is a no-op
/// there. Nested patterns recurse.
fn rewrite_pattern(pat: &mut Pattern, rename: &HashMap<String, String>) {
    match &mut pat.kind {
        PatternKind::Struct { name, fields } => {
            if let Some(r) = rename.get(name) {
                *name = r.clone();
            }
            for f in fields {
                if let Some(p) = &mut f.pattern {
                    rewrite_pattern(p, rename);
                }
            }
        }
        PatternKind::Tuple { name, elements } => {
            if let Some(n) = name {
                if let Some(r) = rename.get(n) {
                    *n = r.clone();
                }
            }
            for e in elements {
                rewrite_pattern(e, rename);
            }
        }
        PatternKind::Wildcard
        | PatternKind::Literal(_)
        | PatternKind::Ident(_)
        | PatternKind::Range { .. } => {}
    }
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
    // A `let` shadows a same-named module global for the rest of the block, so
    // the name is dropped from the rename map once it is bound. The map is only
    // cloned when a binding actually shadows a global.
    let mut shadowed: Option<HashMap<String, String>> = None;
    for stmt in &mut block.stmts {
        rewrite_stmt(stmt, shadowed.as_ref().unwrap_or(rename));
        if let StmtKind::Let { name, .. } = &stmt.kind {
            if shadowed.as_ref().unwrap_or(rename).contains_key(name) {
                let mut m = shadowed.as_ref().unwrap_or(rename).clone();
                m.remove(name);
                shadowed = Some(m);
            }
        }
    }
    if let Some(trailing) = &mut block.trailing {
        rewrite_expr(trailing, shadowed.as_ref().unwrap_or(rename));
    }
}

fn rewrite_stmt(stmt: &mut Stmt, rename: &HashMap<String, String>) {
    match &mut stmt.kind {
        StmtKind::Let { ty, init, .. } => {
            if let Some(t) = ty {
                rewrite_type(t, rename);
            }
            if let Some(e) = init {
                rewrite_expr(e, rename);
            }
        }
        StmtKind::Return(e) | StmtKind::Break(e) => {
            if let Some(e) = e {
                rewrite_expr(e, rename);
            }
        }
        StmtKind::Defer(e) | StmtKind::Spawn(e) | StmtKind::Expr(e) => rewrite_expr(e, rename),
        StmtKind::Assign { target, value, .. } => {
            rewrite_expr(target, rename);
            rewrite_expr(value, rename);
        }
        StmtKind::Continue => {}
    }
}

/// The identifier name of `e` when it is a bare `Ident` with no generic
/// arguments (a whole-module alias receiver looks like this), else `None`.
fn bare_ident_name(e: &Expr) -> Option<&str> {
    match &e.kind {
        ExprKind::Ident { name, generics } if generics.is_empty() => Some(name),
        _ => None,
    }
}

/// Rewrite a `dep.member` access where `dep` is a whole-module alias into the
/// target module's namespaced symbol: `dep.fn(args)` becomes a call to
/// `loc.<hash>.fn`, and a `dep.GLOBAL` field access becomes the bare namespaced
/// global. Returns true when it rewrote `expr` (the caller then skips the
/// ordinary walk for this node). Other expressions are left untouched.
fn rewrite_module_alias_access(expr: &mut Expr, rename: &HashMap<String, String>) -> bool {
    // Resolve the alias target without holding a borrow across the mutation.
    let (target_key, member, is_call) = match &expr.kind {
        ExprKind::MethodCall { receiver, name, .. } => {
            let Some(alias) = bare_ident_name(receiver) else {
                return false;
            };
            match rename.get(&alias_rename_key(alias)) {
                Some(key) => (key.clone(), name.clone(), true),
                None => return false,
            }
        }
        ExprKind::Field { receiver, name } => {
            let Some(alias) = bare_ident_name(receiver) else {
                return false;
            };
            match rename.get(&alias_rename_key(alias)) {
                Some(key) => (key.clone(), name.clone(), false),
                None => return false,
            }
        }
        _ => return false,
    };
    let symbol = mangle_local_fn(&target_key, &member);
    let span = expr.span.clone();
    if is_call {
        let ExprKind::MethodCall { generics, args, .. } = &mut expr.kind else {
            return false;
        };
        let mut gens = std::mem::take(generics);
        let mut call_args = std::mem::take(args);
        for g in &mut gens {
            rewrite_type(g, rename);
        }
        for a in &mut call_args {
            rewrite_expr(a, rename);
        }
        let callee = Expr {
            kind: ExprKind::Ident {
                name: symbol,
                generics: gens,
            },
            span,
        };
        expr.kind = ExprKind::Call {
            callee: Box::new(callee),
            args: call_args,
        };
    } else {
        expr.kind = ExprKind::Ident {
            name: symbol,
            generics: Vec::new(),
        };
    }
    true
}

fn rewrite_expr(expr: &mut Expr, rename: &HashMap<String, String>) {
    // A qualified access through a whole-module alias (`dep.fn()` / `dep.GLOBAL`
    // from `import "./b" as dep`) is rewritten to b's namespaced symbol before
    // the ordinary walk, since the alias binding itself is gone after the merge.
    if rewrite_module_alias_access(expr, rename) {
        return;
    }
    match &mut expr.kind {
        ExprKind::Ident { name, generics } => {
            if let Some(replacement) = rename.get(name) {
                *name = replacement.clone();
            }
            for g in generics {
                rewrite_type(g, rename);
            }
        }
        ExprKind::InterpolatedString(fragments) => {
            for frag in fragments {
                if let StrFragment::Expr(e) = frag {
                    rewrite_expr(e, rename);
                }
            }
        }
        ExprKind::Array(items) | ExprKind::Tuple(items) | ExprKind::SetLit(items) => {
            for e in items {
                rewrite_expr(e, rename);
            }
        }
        ExprKind::MapLit(pairs) => {
            for (k, v) in pairs {
                rewrite_expr(k, rename);
                rewrite_expr(v, rename);
            }
        }
        ExprKind::StructLit {
            name,
            generics,
            fields,
        } => {
            if let Some(r) = rename.get(name) {
                *name = r.clone();
            }
            for g in generics {
                rewrite_type(g, rename);
            }
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
        ExprKind::MethodCall {
            receiver,
            generics,
            args,
            ..
        } => {
            rewrite_expr(receiver, rename);
            // A method-call type argument (`req.json<NewTask>()`) names a type
            // the same way an `Ident` or `StructLit` generic does, so it must be
            // namespaced too. Without this a local or external type used only as
            // a method type argument stayed bare and failed to resolve.
            for g in generics {
                rewrite_type(g, rename);
            }
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
        ExprKind::For {
            pattern,
            iter,
            body,
        } => {
            rewrite_expr(iter, rename);
            // The loop pattern binds names that shadow a global within the body.
            let mut bound = Vec::new();
            collect_pattern_bindings(pattern, &mut bound);
            match rename_without_shadowed(rename, &bound) {
                Some(inner) => rewrite_block(body, &inner),
                None => rewrite_block(body, rename),
            }
        }
        ExprKind::Lambda { params, body, .. } => {
            // Lambda parameters shadow a global of the same name in the body.
            let bound: Vec<String> = params.iter().map(|p| p.name.clone()).collect();
            let inner = rename_without_shadowed(rename, &bound);
            let active = inner.as_ref().unwrap_or(rename);
            match body {
                LambdaBody::Block(b) => rewrite_block(b, active),
                LambdaBody::Expr(e) => rewrite_expr(e, active),
            }
        }
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
        // A macro call only appears in formatter-parsed source; the stdlib
        // rename pass runs after expansion, so it never sees one.
        | ExprKind::MacroCall(_)
        | ExprKind::SelfUpper => {}
    }
}

fn rewrite_match_arm(arm: &mut MatchArm, rename: &HashMap<String, String>) {
    rewrite_pattern(&mut arm.pattern, rename);
    // The arm pattern binds names that shadow a global in the guard and body.
    let mut bound = Vec::new();
    collect_pattern_bindings(&arm.pattern, &mut bound);
    let inner = rename_without_shadowed(rename, &bound);
    let active = inner.as_ref().unwrap_or(rename);
    if let Some(guard) = &mut arm.guard {
        rewrite_expr(guard, active);
    }
    rewrite_expr(&mut arm.body, active);
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
    fn bundled_module_sibling_fn_selector_is_namespaced() {
        // std/fs does `import std/error { error_kind }` and calls
        // `error_kind(...)` inside `io_error`. After expansion that call
        // site must reference the dependency's namespaced symbol
        // (`std.error.error_kind`), not the bare name (issue #178).
        let user = parse_src("import std/fs { read }\nfun main() {}\n");
        let combined = expand_with_stdlib(&user).expect("expand");
        let io_error = combined
            .items
            .iter()
            .filter_map(|d| match &d.kind {
                DeclKind::Function(f) if f.name == mangle_stdlib_fn("fs", "io_error") => Some(f),
                _ => None,
            })
            .next()
            .expect("io_error present");
        let mut idents = Vec::new();
        collect_fn_body_idents(&io_error.body, &mut idents);
        assert!(
            idents.iter().any(|n| n == "std.error.error_kind"),
            "io_error should call the namespaced sibling, got: {idents:?}"
        );
        assert!(
            !idents.iter().any(|n| n == "error_kind"),
            "no bare sibling-module call should remain, got: {idents:?}"
        );
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
    fn external_key_is_stable_and_namespaced() {
        let p = PathBuf::from("/cache/github.com/acme/greet@v1.0.0/lib.rv");
        let k1 = external_module_key("github.com", "acme", "greet", &p);
        let k2 = external_module_key("github.com", "acme", "greet", &p);
        assert_eq!(k1, k2);
        assert!(k1.starts_with("ext.github_com.acme.greet."));
        assert_eq!(mangle_external_fn(&k1, "shout"), format!("{k1}.shout"));
        // A different source file in the same package yields a distinct key.
        let other = PathBuf::from("/cache/github.com/acme/greet@v1.0.0/util.rv");
        assert_ne!(
            k1,
            external_module_key("github.com", "acme", "greet", &other)
        );
    }

    #[test]
    fn external_source_path_maps_through_lock() {
        let lock = crate::lock::LockFile {
            version: crate::lock::LOCK_VERSION,
            packages: vec![crate::lock::LockedPackage {
                source: "github.com/acme/greet".to_string(),
                version: "v1.0.0".to_string(),
                hash: "sha256:abc".to_string(),
            }],
        };
        let ctx = PackageContext::new(PathBuf::from("/cache"), &lock);

        // Bare import resolves to lib.rv at the cached package root.
        let bare = ctx
            .external_source_path("github.com/acme/greet", &[])
            .expect("bare path");
        assert!(bare.ends_with("github.com/acme/greet@v1.0.0/lib.rv"));

        // A single-segment subpath selects <cachedir>/<seg>.rv.
        let sub = ctx
            .external_source_path("github.com/acme/greet", &["lib".to_string()])
            .expect("sub path");
        assert!(sub.ends_with("github.com/acme/greet@v1.0.0/lib.rv"));

        // A nested subpath joins directories then appends .rv.
        let nested = ctx
            .external_source_path(
                "github.com/acme/greet",
                &["util".to_string(), "text".to_string()],
            )
            .expect("nested path");
        assert!(nested.ends_with("github.com/acme/greet@v1.0.0/util/text.rv"));

        // An unlocked package resolves to nothing.
        assert!(ctx
            .external_source_path("github.com/acme/missing", &[])
            .is_none());
    }

    #[test]
    fn external_source_path_matches_lock_case_insensitively() {
        // GitHub owner/repo paths are case-insensitive, so an import that
        // differs only in case from the locked source still resolves, and the
        // cache path uses the canonical (locked) casing the fetch created.
        let lock = crate::lock::LockFile {
            version: crate::lock::LOCK_VERSION,
            packages: vec![crate::lock::LockedPackage {
                source: "github.com/Acme/Demo".to_string(),
                version: "v1.0.0".to_string(),
                hash: "sha256:abc".to_string(),
            }],
        };
        let ctx = PackageContext::new(PathBuf::from("/cache"), &lock);

        let path = ctx
            .external_source_path("github.com/acme/demo", &[])
            .expect("a case-different import still resolves");
        assert!(
            path.ends_with("github.com/Acme/Demo@v1.0.0/lib.rv"),
            "the cache path keeps the locked casing, got {}",
            path.display()
        );
        assert!(ctx.package_manifest_path("github.com/ACME/DEMO").is_some());
    }

    #[test]
    fn external_local_source_path_rejects_existing_escape() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let cache = std::env::temp_dir().join(format!(
            "raven_ext_escape_{}_{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let pkg_dir = cache.join("github.com").join("acme").join("pkg@v1.0.0");
        std::fs::create_dir_all(&pkg_dir).expect("mkdir package");
        std::fs::write(pkg_dir.join("lib.rv"), "fun inside() -> Int { return 1 }\n")
            .expect("write lib");
        let outside = pkg_dir.parent().expect("package parent").join("escape.rv");
        std::fs::write(&outside, "fun outside() -> Int { return 2 }\n").expect("write escape");

        let lock = crate::lock::LockFile {
            version: crate::lock::LOCK_VERSION,
            packages: vec![crate::lock::LockedPackage {
                source: "github.com/acme/pkg".to_string(),
                version: "v1.0.0".to_string(),
                hash: "sha256:abc".to_string(),
            }],
        };
        let ctx = PackageContext::new(cache.clone(), &lock);
        let lib = ctx
            .external_source_path("github.com/acme/pkg", &[])
            .expect("lib path");
        assert!(
            ctx.external_local_source_path("github.com/acme/pkg", &lib, "../escape")
                .is_none(),
            "an existing local import outside the cached package root must be rejected"
        );

        std::fs::remove_dir_all(&cache).ok();
    }

    #[cfg(unix)]
    #[test]
    fn external_source_path_rejects_symlink_escape() {
        use std::os::unix::fs::symlink;
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let cache = std::env::temp_dir().join(format!(
            "raven_ext_symlink_escape_{}_{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let pkg_dir = cache.join("github.com").join("acme").join("pkg@v1.0.0");
        let outside_dir = cache.join("outside");
        std::fs::create_dir_all(&pkg_dir).expect("mkdir package");
        std::fs::create_dir_all(&outside_dir).expect("mkdir outside");
        std::fs::write(
            outside_dir.join("util.rv"),
            "fun outside() -> Int { return 2 }\n",
        )
        .expect("write outside");
        symlink(outside_dir.join("util.rv"), pkg_dir.join("util.rv")).expect("create symlink");

        let lock = crate::lock::LockFile {
            version: crate::lock::LOCK_VERSION,
            packages: vec![crate::lock::LockedPackage {
                source: "github.com/acme/pkg".to_string(),
                version: "v1.0.0".to_string(),
                hash: "sha256:abc".to_string(),
            }],
        };
        let ctx = PackageContext::new(cache.clone(), &lock);
        assert!(
            ctx.external_source_path("github.com/acme/pkg", &["util".to_string()])
                .is_none(),
            "a symlinked source outside the cached package root must be rejected"
        );

        std::fs::remove_dir_all(&cache).ok();
    }

    #[test]
    fn external_function_merges_under_ext_namespace() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let cache = std::env::temp_dir().join(format!(
            "raven_ext_merge_{}_{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let pkg_dir = cache.join("github.com").join("acme").join("greet@v1.0.0");
        std::fs::create_dir_all(&pkg_dir).expect("mkdir");
        std::fs::write(
            pkg_dir.join("rv.toml"),
            "[package]\nname = \"greet\"\nversion = \"0.1.0\"\n",
        )
        .expect("write toml");
        let src_path = pkg_dir.join("lib.rv");
        std::fs::write(
            &src_path,
            "fun shout(s: String) -> String { return s.concat(\"!\") }\n",
        )
        .expect("write src");

        let lock = crate::lock::LockFile {
            version: crate::lock::LOCK_VERSION,
            packages: vec![crate::lock::LockedPackage {
                source: "github.com/acme/greet".to_string(),
                version: "v1.0.0".to_string(),
                hash: "sha256:abc".to_string(),
            }],
        };
        let ctx = PackageContext::new(cache.clone(), &lock);

        let user = parse_src(
            "import \"github.com/acme/greet/lib\" { shout }\nfun main() { print(shout(\"hi\")) }\n",
        );
        let (combined, _sites) = expand_with_stdlib_ctx(&user, Some(&ctx)).expect("expand");

        let resolved_src = ctx
            .external_source_path("github.com/acme/greet", &["lib".to_string()])
            .expect("resolved source path");
        let key = external_module_key("github.com", "acme", "greet", &resolved_src);
        let mangled = mangle_external_fn(&key, "shout");
        let present = combined
            .items
            .iter()
            .any(|d| matches!(&d.kind, DeclKind::Function(f) if f.name == mangled));
        assert!(present, "external shout should merge under {mangled}");

        std::fs::remove_dir_all(&cache).ok();
    }

    #[test]
    fn local_module_external_function_call_is_rewritten() {
        // Regression for #517: a local module that imports a free function from
        // a github dependency must have its call rewritten to the ext.<...>
        // symbol, the same as the entry file does. Before the fix the call
        // stayed a bare name and failed to resolve.
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let cache =
            std::env::temp_dir().join(format!("raven_517_cache_{}_{}", std::process::id(), n));
        let pkg_dir = cache.join("github.com").join("acme").join("greet@v1.0.0");
        std::fs::create_dir_all(&pkg_dir).expect("mkdir");
        std::fs::write(
            pkg_dir.join("rv.toml"),
            "[package]\nname = \"greet\"\nversion = \"0.1.0\"\n",
        )
        .expect("toml");
        let src_path = pkg_dir.join("lib.rv");
        std::fs::write(
            &src_path,
            "fun shout(s: String) -> String { return s.concat(\"!\") }\n",
        )
        .expect("src");

        let lock = crate::lock::LockFile {
            version: crate::lock::LOCK_VERSION,
            packages: vec![crate::lock::LockedPackage {
                source: "github.com/acme/greet".to_string(),
                version: "v1.0.0".to_string(),
                hash: "sha256:abc".to_string(),
            }],
        };
        let ctx = PackageContext::new(cache.clone(), &lock);

        let (proj, entry) = write_temp_project(
            &[
                (
                    "helper.rv",
                    "import \"github.com/acme/greet/lib\" { shout }\n\
                     fun loud(s: String) -> String { return shout(s) }\n",
                ),
                (
                    "main.rv",
                    "import \"./helper\" { loud }\nfun main() { print(loud(\"hi\")) }\n",
                ),
            ],
            "main.rv",
        );
        let canon = proj.join("helper.rv").canonicalize().expect("canon");
        let user = parse_at(
            "import \"./helper\" { loud }\nfun main() { print(loud(\"hi\")) }\n",
            &entry,
        );
        let (combined, _sites) = expand_with_stdlib_ctx(&user, Some(&ctx)).expect("expand");

        let resolved_src = ctx
            .external_source_path("github.com/acme/greet", &["lib".to_string()])
            .expect("resolved source path");
        let ext_shout = mangle_external_fn(
            &external_module_key("github.com", "acme", "greet", &resolved_src),
            "shout",
        );
        let loc_loud = mangle_local_fn(&local_module_key(&canon), "loud");
        let loud_fn = combined
            .items
            .iter()
            .find_map(|d| match &d.kind {
                DeclKind::Function(f) if f.name == loc_loud => Some(f),
                _ => None,
            })
            .expect("loud should merge under its local name");
        let body = format!("{:?}", loud_fn.body);
        assert!(
            body.contains(&ext_shout),
            "loud's body should call {ext_shout}; got {body}"
        );

        std::fs::remove_dir_all(&cache).ok();
        std::fs::remove_dir_all(&proj).ok();
    }

    #[test]
    fn external_local_selector_import_is_loaded_and_rewritten() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let cache = std::env::temp_dir().join(format!(
            "raven_ext_local_selector_{}_{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let pkg_dir = cache.join("github.com").join("acme").join("calc@v1.0.0");
        std::fs::create_dir_all(&pkg_dir).expect("mkdir");
        std::fs::write(
            pkg_dir.join("rv.toml"),
            "[package]\nname = \"calc\"\nversion = \"0.1.0\"\n",
        )
        .expect("toml");
        std::fs::write(
            pkg_dir.join("lib.rv"),
            "import \"./util\" { value }\nfun answer() -> Int { return value() }\n",
        )
        .expect("lib");
        std::fs::write(
            pkg_dir.join("util.rv"),
            "fun value() -> Int { return 42 }\n",
        )
        .expect("util");

        let lock = crate::lock::LockFile {
            version: crate::lock::LOCK_VERSION,
            packages: vec![crate::lock::LockedPackage {
                source: "github.com/acme/calc".to_string(),
                version: "v1.0.0".to_string(),
                hash: "sha256:abc".to_string(),
            }],
        };
        let ctx = PackageContext::new(cache.clone(), &lock);
        let user = parse_src(
            "import \"github.com/acme/calc\" { answer }\nfun main() { print(answer()) }\n",
        );
        let (combined, _sites) = expand_with_stdlib_ctx(&user, Some(&ctx)).expect("expand");

        let lib_path = ctx
            .external_source_path("github.com/acme/calc", &[])
            .expect("lib path");
        let util_path = ctx
            .external_local_source_path("github.com/acme/calc", &lib_path, "./util")
            .expect("util path");
        let answer = mangle_external_fn(
            &external_module_key("github.com", "acme", "calc", &lib_path),
            "answer",
        );
        let value = mangle_external_fn(
            &external_module_key("github.com", "acme", "calc", &util_path),
            "value",
        );
        let answer_fn = combined
            .items
            .iter()
            .find_map(|d| match &d.kind {
                DeclKind::Function(f) if f.name == answer => Some(f),
                _ => None,
            })
            .expect("answer merged");
        let mut idents = Vec::new();
        collect_fn_body_idents(&answer_fn.body, &mut idents);
        assert!(
            idents.iter().any(|n| *n == value),
            "answer should call {value}, got {idents:?}"
        );

        std::fs::remove_dir_all(&cache).ok();
    }

    #[test]
    fn external_whole_module_default_alias_is_rewritten() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let cache = std::env::temp_dir().join(format!(
            "raven_ext_whole_alias_{}_{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let base = cache.join("github.com").join("acme");
        let app_dir = base.join("app@v1.0.0");
        let dep_dir = base.join("dep@v2.0.0");
        std::fs::create_dir_all(&app_dir).expect("mkdir app");
        std::fs::create_dir_all(&dep_dir).expect("mkdir dep");
        std::fs::write(
            app_dir.join("rv.toml"),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\
             [dependencies]\n\"github.com/acme/dep\" = \"v2.0.0\"\n",
        )
        .expect("app toml");
        std::fs::write(
            dep_dir.join("rv.toml"),
            "[package]\nname = \"dep\"\nversion = \"0.1.0\"\n",
        )
        .expect("dep toml");
        std::fs::write(
            app_dir.join("lib.rv"),
            "import \"github.com/acme/dep\"\nfun answer() -> Int { return dep.value() }\n",
        )
        .expect("app lib");
        std::fs::write(dep_dir.join("lib.rv"), "fun value() -> Int { return 7 }\n")
            .expect("dep lib");

        let lock = crate::lock::LockFile {
            version: crate::lock::LOCK_VERSION,
            packages: vec![
                crate::lock::LockedPackage {
                    source: "github.com/acme/app".to_string(),
                    version: "v1.0.0".to_string(),
                    hash: "sha256:app".to_string(),
                },
                crate::lock::LockedPackage {
                    source: "github.com/acme/dep".to_string(),
                    version: "v2.0.0".to_string(),
                    hash: "sha256:dep".to_string(),
                },
            ],
        };
        let ctx = PackageContext::new(cache.clone(), &lock);
        let user = parse_src(
            "import \"github.com/acme/app\" { answer }\nfun main() { print(answer()) }\n",
        );
        let (combined, _sites) = expand_with_stdlib_ctx(&user, Some(&ctx)).expect("expand");

        let app_path = ctx
            .external_source_path("github.com/acme/app", &[])
            .expect("app path");
        let dep_path = ctx
            .external_source_path("github.com/acme/dep", &[])
            .expect("dep path");
        let answer = mangle_external_fn(
            &external_module_key("github.com", "acme", "app", &app_path),
            "answer",
        );
        let value = mangle_external_fn(
            &external_module_key("github.com", "acme", "dep", &dep_path),
            "value",
        );
        let answer_fn = combined
            .items
            .iter()
            .find_map(|d| match &d.kind {
                DeclKind::Function(f) if f.name == answer => Some(f),
                _ => None,
            })
            .expect("answer merged");
        let mut idents = Vec::new();
        collect_fn_body_idents(&answer_fn.body, &mut idents);
        assert!(
            idents.iter().any(|n| *n == value),
            "answer should call {value}, got {idents:?}"
        );

        std::fs::remove_dir_all(&cache).ok();
    }

    #[test]
    fn local_module_external_whole_module_alias_is_rewritten() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let cache = std::env::temp_dir().join(format!(
            "raven_local_ext_whole_alias_{}_{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let dep_dir = cache.join("github.com").join("acme").join("dep@v1.0.0");
        std::fs::create_dir_all(&dep_dir).expect("mkdir dep");
        std::fs::write(
            dep_dir.join("rv.toml"),
            "[package]\nname = \"dep\"\nversion = \"0.1.0\"\n",
        )
        .expect("dep toml");
        std::fs::write(dep_dir.join("lib.rv"), "fun value() -> Int { return 9 }\n")
            .expect("dep lib");

        let lock = crate::lock::LockFile {
            version: crate::lock::LOCK_VERSION,
            packages: vec![crate::lock::LockedPackage {
                source: "github.com/acme/dep".to_string(),
                version: "v1.0.0".to_string(),
                hash: "sha256:dep".to_string(),
            }],
        };
        let ctx = PackageContext::new(cache.clone(), &lock);

        let (proj, entry) = write_temp_project(
            &[
                (
                    "helper.rv",
                    "import \"github.com/acme/dep\"\nfun via() -> Int { return dep.value() }\n",
                ),
                (
                    "main.rv",
                    "import \"./helper\" { via }\nfun main() { print(via()) }\n",
                ),
            ],
            "main.rv",
        );
        let canon = proj.join("helper.rv").canonicalize().expect("canon");
        let user = parse_at(
            "import \"./helper\" { via }\nfun main() { print(via()) }\n",
            &entry,
        );
        let (combined, _sites) = expand_with_stdlib_ctx(&user, Some(&ctx)).expect("expand");

        let dep_path = ctx
            .external_source_path("github.com/acme/dep", &[])
            .expect("dep path");
        let via = mangle_local_fn(&local_module_key(&canon), "via");
        let value = mangle_external_fn(
            &external_module_key("github.com", "acme", "dep", &dep_path),
            "value",
        );
        let via_fn = combined
            .items
            .iter()
            .find_map(|d| match &d.kind {
                DeclKind::Function(f) if f.name == via => Some(f),
                _ => None,
            })
            .expect("via merged");
        let mut idents = Vec::new();
        collect_fn_body_idents(&via_fn.body, &mut idents);
        assert!(
            idents.iter().any(|n| *n == value),
            "via should call {value}, got {idents:?}"
        );

        std::fs::remove_dir_all(&cache).ok();
        std::fs::remove_dir_all(&proj).ok();
    }

    #[test]
    fn external_stdlib_free_function_import_is_renamed() {
        // A dependency that imports a std free function must have its call
        // sites renamed to the std namespace when merged, the same as its
        // types and methods already resolve. Regression for the case where
        // `import std/time { now_millis }` inside a package left `now_millis`
        // unresolved in the consumer.
        let lock = crate::lock::LockFile {
            version: crate::lock::LOCK_VERSION,
            packages: vec![],
        };
        let ctx = PackageContext::new(PathBuf::from("/cache"), &lock);
        let file = parse_src(
            "import std/time { now_millis }\nfun stamp() -> Int { return now_millis() }\n",
        );
        let map = external_import_rename_map(&file, &ctx, None);
        assert_eq!(
            map.get("now_millis"),
            Some(&mangle_stdlib_fn("time", "now_millis"))
        );
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
    fn local_module_struct_is_namespaced() {
        // A local module's type is namespaced like its functions, so two
        // modules can both declare a `Point` without colliding at merge.
        let (dir, entry) = write_temp_project(
            &[
                ("shapes.rv", "struct Point { x: Int }\n"),
                ("main.rv", "import \"./shapes\" { Point }\nfun main() {}\n"),
            ],
            "main.rv",
        );
        let canon = dir.join("shapes.rv").canonicalize().expect("canon");
        let user = parse_at("import \"./shapes\" { Point }\nfun main() {}\n", &entry);
        let combined = expand_with_stdlib(&user).expect("expand");
        let mangled = mangle_local_fn(&local_module_key(&canon), "Point");
        let present = combined
            .items
            .iter()
            .any(|d| matches!(&d.kind, DeclKind::Struct(s) if s.name == mangled));
        let bare = combined
            .items
            .iter()
            .any(|d| matches!(&d.kind, DeclKind::Struct(s) if s.name == "Point"));
        assert!(present, "a local struct should merge under {mangled}");
        assert!(!bare, "the bare `Point` name should not remain");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn json_derive_helpers_emitted_once_across_modules() {
        // Two local modules each derive a JSON trait. The shared helper free
        // functions are global and fixed-named, so they must be declared
        // exactly once in the combined program. Emitting them per module
        // declared `raven_derive_json_decode` several times, which resolve
        // rejected as "declared multiple times" in a multi-file project.
        let main_src = "import \"./a\" { A }\nimport \"./b\" { B }\nfun main() {}\n";
        let (dir, entry) = write_temp_project(
            &[
                ("a.rv", "@derive(FromJson)\nstruct A { x: Int }\n"),
                ("b.rv", "@derive(ToJson)\nstruct B { y: Int }\n"),
                ("main.rv", main_src),
            ],
            "main.rv",
        );
        let user = parse_at(main_src, &entry);
        let combined = expand_with_stdlib(&user).expect("expand");
        let decode_count = combined
            .items
            .iter()
            .filter(|d| matches!(&d.kind, DeclKind::Function(f) if f.name == "raven_derive_json_decode"))
            .count();
        assert_eq!(
            decode_count, 1,
            "the JSON decode helper must be declared exactly once across modules"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn module_globals_namespaced_across_modules() {
        // Two local modules each declare a module-level `let value`. They are
        // namespaced per module like functions, so merging them does not report
        // `value` as declared multiple times even though the importer selects
        // only the uniquely named functions.
        let main_src = "import \"./a\" { get_a }\nimport \"./b\" { get_b }\nfun main() {}\n";
        let (dir, entry) = write_temp_project(
            &[
                ("a.rv", "let value: Int = 1\nfun get_a() -> Int = value\n"),
                ("b.rv", "let value: Int = 10\nfun get_b() -> Int = value\n"),
                ("main.rv", main_src),
            ],
            "main.rv",
        );
        let user = parse_at(main_src, &entry);
        let combined = expand_with_stdlib(&user).expect("two same-named globals must merge");
        // Both globals survive the merge under distinct namespaced names.
        let global_count = combined
            .items
            .iter()
            .filter(|d| matches!(&d.kind, DeclKind::Let(l) if l.name.ends_with("value")))
            .count();
        assert_eq!(
            global_count, 2,
            "both module globals must be present under distinct names"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn reserved_derive_helper_name_is_rejected() {
        // A user declaration that uses the reserved `raven_derive_` prefix is
        // rejected with a clear message, since it would otherwise collide with a
        // generated JSON derive helper.
        let src = "fun raven_derive_json_decode() -> Int = 1\nfun main() {}\n";
        let user = parse_at(src, Path::new("main.rv"));
        let err = expand_with_stdlib(&user).expect_err("reserved name must be rejected");
        let msg = format!("{err}");
        assert!(
            msg.contains("raven_derive_json_decode") && msg.contains("reserved"),
            "expected a reserved-name error, got: {msg}"
        );
    }

    #[test]
    fn imported_module_globals_initialize_first() {
        // `a` imports `b` and initializes a global from `b`'s function at load
        // time. `b` must be merged ahead of `a`, so its global initializer runs
        // first and `a` reads the initialized value rather than zero.
        let main_src = "import \"./a\" { get_result }\nfun main() {}\n";
        let (dir, entry) = write_temp_project(
            &[
                ("b.rv", "let b_value: Int = 42\nfun get_b() -> Int = b_value\n"),
                (
                    "a.rv",
                    "import \"./b\" { get_b }\nlet a_result: Int = get_b()\nfun get_result() -> Int = a_result\n",
                ),
                ("main.rv", main_src),
            ],
            "main.rv",
        );
        let user = parse_at(main_src, &entry);
        let combined = expand_with_stdlib(&user).expect("expand");
        let pos = |needle: &str| {
            combined
                .items
                .iter()
                .position(|d| matches!(&d.kind, DeclKind::Let(l) if l.name.ends_with(needle)))
        };
        let b_pos = pos("b_value").expect("b_value global present");
        let a_pos = pos("a_result").expect("a_result global present");
        assert!(
            b_pos < a_pos,
            "the imported module's global must initialize first (b={b_pos}, a={a_pos})"
        );
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

    #[test]
    fn whole_module_alias_call_is_rewritten_in_a_merged_module() {
        // `a.rv` uses a whole-module alias (`import "./b" as dep`) and calls
        // `dep.base()`. When `a` is merged as an imported module its alias
        // binding is stripped, so the qualified call must be rewritten to `b`'s
        // namespaced symbol rather than left as an unresolved `dep`.
        let (dir, entry) = write_temp_project(
            &[
                ("b.rv", "fun base() -> Int { return 1 }\n"),
                (
                    "a.rv",
                    "import \"./b\" as dep\nfun via() -> Int { return dep.base() }\n",
                ),
                ("main.rv", "import \"./a\" { via }\nfun main() {}\n"),
            ],
            "main.rv",
        );
        let canon_b = dir.join("b.rv").canonicalize().expect("canon b");
        let user = parse_at("import \"./a\" { via }\nfun main() {}\n", &entry);
        let combined = expand_with_stdlib(&user).expect("expand");

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

    #[test]
    fn a_parameter_is_not_rewritten_to_a_shadowed_module_global() {
        // `module.rv` has a global `value` and an `echo(value)` whose parameter
        // shadows it. The merge namespaces the global, but the parameter use in
        // the body must stay bare so it reads the argument, not the global.
        let (dir, entry) = write_temp_project(
            &[
                (
                    "module.rv",
                    "let value: Int = 1\nfun echo(value: Int) -> Int { return value }\n",
                ),
                ("main.rv", "import \"./module\" { echo }\nfun main() {}\n"),
            ],
            "main.rv",
        );
        let canon = dir.join("module.rv").canonicalize().expect("canon");
        let user = parse_at("import \"./module\" { echo }\nfun main() {}\n", &entry);
        let combined = expand_with_stdlib(&user).expect("expand");

        let key = local_module_key(&canon);
        let global_mangled = mangle_local_fn(&key, "value");
        let echo = combined
            .items
            .iter()
            .filter_map(|d| match &d.kind {
                DeclKind::Function(f) if f.name.ends_with(".echo") => Some(f),
                _ => None,
            })
            .next()
            .expect("echo present");
        let mut idents = Vec::new();
        collect_fn_body_idents(&echo.body, &mut idents);
        assert!(
            idents.iter().any(|n| n == "value"),
            "the parameter use stays bare, got {idents:?}"
        );
        assert!(
            !idents.iter().any(|n| *n == global_mangled),
            "the parameter must not be rewritten to {global_mangled}, got {idents:?}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    fn collect_fn_body_idents(body: &FunctionBody, out: &mut Vec<String>) {
        if let FunctionBody::Block(b) = body {
            collect_block_idents(b, out);
        }
    }
}
