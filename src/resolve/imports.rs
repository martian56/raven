//! Import resolution.
//!
//! Three import shapes are recognized:
//!
//! 1. `import std/<path>`: looked up in a static registry of stdlib
//!    module names. Contents are not loaded; member resolution is the
//!    type checker's responsibility.
//! 2. `import "github.com/<user>/<repo>[/<sub>]"`: defers to rvpm. The
//!    resolver records the target and continues.
//! 3. `import "./<path>"` or `"../<path>"`: read through the
//!    [`SourceLoader`] and recursively parsed and resolved.
//!
//! After this pass, every import's alias (or each selector when a
//! selector list is present) is inserted into the current module
//! scope.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::ast::{DeclKind, File, Import, ImportSource};
use crate::error::{RavenError, ResolveError};
use crate::lexer::Lexer;
use crate::parser::parse;
use crate::span::Span;

use super::bindings::{Binding, ImportId, ImportTarget, ResolvedImport};
use super::scope::ScopeStack;

/// Pluggable filesystem hook used for local imports. Tests inject an
/// in memory loader keyed by relative path; the real CLI uses
/// [`FsLoader`].
pub trait SourceLoader {
    /// Resolve `target` (a relative path like `./helpers` or
    /// `../util`) starting from `importing` (the path of the file
    /// containing the import). Return `Some((canonical_path, source))`
    /// when found, `None` otherwise.
    fn load(&mut self, importing: &Path, target: &str) -> Option<LoadedSource>;
}

/// One loaded source file's text plus a canonical path used for cycle
/// detection.
#[derive(Debug, Clone)]
pub struct LoadedSource {
    pub canonical_path: PathBuf,
    pub source: String,
}

/// Filesystem backed loader. Resolves `target` relative to
/// `importing.parent()` and reads the file from disk. A `.rv`
/// extension is appended if `target` does not already have an
/// extension. Used by the CLI; tests rarely touch this.
#[derive(Debug, Default)]
pub struct FsLoader;

impl SourceLoader for FsLoader {
    fn load(&mut self, importing: &Path, target: &str) -> Option<LoadedSource> {
        let parent = importing.parent().unwrap_or_else(|| Path::new("."));
        let mut path = parent.join(target);
        if path.extension().is_none() {
            path.set_extension("rv");
        }
        let canonical_path = path.canonicalize().unwrap_or_else(|_| path.clone());
        let source = std::fs::read_to_string(&path).ok()?;
        Some(LoadedSource {
            canonical_path,
            source,
        })
    }
}

/// The recognized stdlib module names. Anything else under `std/` is
/// `UnresolvedImport`. The list intentionally mirrors the v1 stdlib so
/// future code can be ported without re writing imports.
pub const STDLIB_MODULES: &[&str] = &[
    "io",
    "iter",
    "collections",
    "string",
    "math",
    "fs",
    "net",
    "http",
    "time",
    "json",
    "ffi",
];

/// Resolve every import declaration in `file`, inserting alias /
/// selector bindings into `scope` and appending [`ResolvedImport`]s to
/// `out_imports`.
///
/// `in_progress` carries the set of canonical paths currently being
/// resolved so we can detect a cycle. The set is mutated as files are
/// pushed and popped during recursion.
pub fn resolve_imports(
    file: &File,
    scope: &mut ScopeStack,
    loader: &mut dyn SourceLoader,
    out_imports: &mut Vec<ResolvedImport>,
    in_progress: &mut HashSet<PathBuf>,
) -> Result<(), RavenError> {
    for decl in &file.items {
        let DeclKind::Import(import) = &decl.kind else {
            continue;
        };
        let id = ImportId(out_imports.len());
        let resolved = resolve_one_import(import, &decl.span, loader, in_progress, id)?;
        bind_import(import, &resolved, id, scope)?;
        out_imports.push(resolved);
    }
    Ok(())
}

fn resolve_one_import(
    import: &Import,
    decl_span: &Span,
    loader: &mut dyn SourceLoader,
    in_progress: &mut HashSet<PathBuf>,
    id: ImportId,
) -> Result<ResolvedImport, RavenError> {
    match &import.source {
        ImportSource::Std(segments) => {
            // The first segment after `std` is the module name; nested
            // segments select a submodule, which is also looked up in
            // the registry by its leading name in v2.0.
            let head = segments
                .first()
                .ok_or_else(|| invalid_import(decl_span, "std import has no module name"))?;
            if !STDLIB_MODULES.contains(&head.as_str()) {
                return Err(RavenError::resolve(
                    ResolveError::UnresolvedImport(format!("std/{}", segments.join("/"))),
                    import.span.clone(),
                )
                .with_hint("unknown stdlib module; see `docs/v2/specs/resolver.md`"));
            }
            Ok(ResolvedImport {
                id,
                path: format!("std/{}", segments.join("/")),
                target: ImportTarget::StdlibModule {
                    segments: segments.clone(),
                },
                span: import.span.clone(),
            })
        }
        ImportSource::Quoted(path) => {
            if let Some(pkg) = parse_github_path(path) {
                Ok(ResolvedImport {
                    id,
                    path: path.clone(),
                    target: pkg,
                    span: import.span.clone(),
                })
            } else if path.starts_with("./") || path.starts_with("../") {
                resolve_local_import(import, decl_span, path, loader, in_progress, id)
            } else {
                Err(RavenError::resolve(
                    ResolveError::UnresolvedImport(path.clone()),
                    import.span.clone(),
                )
                .with_hint("expected `std/...`, `github.com/<user>/<repo>`, or `./relative` path"))
            }
        }
    }
}

fn parse_github_path(s: &str) -> Option<ImportTarget> {
    let rest = s.strip_prefix("github.com/")?;
    let mut parts = rest.split('/');
    let user = parts.next()?.to_string();
    let repo = parts.next()?.to_string();
    let subpath: Vec<String> = parts.map(|s| s.to_string()).collect();
    if user.is_empty() || repo.is_empty() {
        return None;
    }
    Some(ImportTarget::ExternalPackage {
        host: "github.com".to_string(),
        user,
        repo,
        subpath,
    })
}

fn resolve_local_import(
    import: &Import,
    _decl_span: &Span,
    path: &str,
    loader: &mut dyn SourceLoader,
    in_progress: &mut HashSet<PathBuf>,
    id: ImportId,
) -> Result<ResolvedImport, RavenError> {
    let importing_file = (*import.span.file).clone();
    let loaded = loader.load(&importing_file, path).ok_or_else(|| {
        RavenError::resolve(
            ResolveError::UnresolvedImport(path.to_string()),
            import.span.clone(),
        )
        .with_hint("file not found by the source loader")
    })?;

    if in_progress.contains(&loaded.canonical_path) {
        return Err(RavenError::resolve(
            ResolveError::CyclicImport(path.to_string()),
            import.span.clone(),
        ));
    }

    // Lex, parse, and walk the inner file to discover what names it
    // exports. We don't resolve the inner file's bodies here (use sites
    // inside the inner file are not our concern at this pass); we only
    // need its top level names to populate `module_names`.
    let tokens = Lexer::new(loaded.source.clone(), loaded.canonical_path.clone())
        .tokenize()
        .map_err(|e| {
            // Surface the underlying lex error with the import span so
            // the user sees where the import points at.
            RavenError::resolve(
                ResolveError::UnresolvedImport(path.to_string()),
                import.span.clone(),
            )
            .with_hint(format!("inner file failed to lex: {}", e))
        })?;
    let inner_file = parse(&tokens).map_err(|e| {
        RavenError::resolve(
            ResolveError::UnresolvedImport(path.to_string()),
            import.span.clone(),
        )
        .with_hint(format!("inner file failed to parse: {}", e))
    })?;

    in_progress.insert(loaded.canonical_path.clone());

    // Recursively resolve the inner file's imports too. We don't
    // care about its bindings (it has its own scope), but a transitive
    // cyclic import should surface during this recursion.
    let mut inner_scope = ScopeStack::new();
    let mut inner_imports = Vec::new();
    let inner_result = resolve_imports(
        &inner_file,
        &mut inner_scope,
        loader,
        &mut inner_imports,
        in_progress,
    );
    in_progress.remove(&loaded.canonical_path);
    inner_result?;

    // Collect inner top level names so callers can ask the module what
    // it exports. We avoid running the full item collection so a single
    // duplicate inside the imported file doesn't fail the importing
    // file's resolution; that's the inner file's problem when it gets
    // resolved on its own.
    let mut module_names: Vec<String> = Vec::new();
    for d in &inner_file.items {
        match &d.kind {
            DeclKind::Function(f) => module_names.push(f.name.clone()),
            DeclKind::Struct(s) => module_names.push(s.name.clone()),
            DeclKind::Trait(t) => module_names.push(t.name.clone()),
            DeclKind::Enum(e) => module_names.push(e.name.clone()),
            DeclKind::Const(c) => module_names.push(c.name.clone()),
            DeclKind::Let(l) => module_names.push(l.name.clone()),
            DeclKind::Extern(ext) => {
                for it in &ext.items {
                    module_names.push(it.name.clone());
                }
            }
            DeclKind::Impl(_) | DeclKind::Import(_) => {}
        }
    }

    Ok(ResolvedImport {
        id,
        path: path.to_string(),
        target: ImportTarget::LocalModule {
            canonical_path: loaded.canonical_path,
            module_names,
        },
        span: import.span.clone(),
    })
}

/// Insert the alias and any selector names into `scope`.
fn bind_import(
    import: &Import,
    resolved: &ResolvedImport,
    id: ImportId,
    scope: &mut ScopeStack,
) -> Result<(), RavenError> {
    // The module name a `std/<module>` import resolves to, used to find
    // the bundled functions the resolver merged ahead of this pass. When
    // present, a selector binds directly to the namespaced function so
    // the rest of the pipeline sees `println` as an ordinary call to a
    // known function rather than a deferred import member.
    let std_module: Option<&str> = match &resolved.target {
        ImportTarget::StdlibModule { segments } => segments.first().map(|s| s.as_str()),
        _ => None,
    };

    if !import.selectors.is_empty() {
        for name in &import.selectors {
            // For a bundled stdlib module, bind the selector to the
            // namespaced function the resolver merged into the module
            // scope. Fall back to the deferred `ImportedItem` binding when
            // the function is not present (an unknown selector, or a
            // resolver run that did not merge the bundle, as in unit
            // tests that call `resolve_imports` directly).
            if let Some(module) = std_module {
                let mangled = super::stdlib::mangle_stdlib_fn(module, name);
                if let Some(entry) = scope.lookup(&mangled) {
                    let binding = entry.binding.clone();
                    scope.insert(name, binding, import.span.clone())?;
                    continue;
                }
            }
            scope.insert(
                name,
                Binding::ImportedItem {
                    import_id: id,
                    name: name.clone(),
                },
                import.span.clone(),
            )?;
        }
    } else {
        let alias = match (&import.alias, &resolved.target) {
            (Some(a), _) => a.clone(),
            (None, ImportTarget::StdlibModule { segments }) => segments
                .last()
                .cloned()
                .unwrap_or_else(|| "std".to_string()),
            (None, ImportTarget::ExternalPackage { repo, subpath, .. }) => {
                subpath.last().cloned().unwrap_or_else(|| repo.clone())
            }
            (None, ImportTarget::LocalModule { canonical_path, .. }) => canonical_path
                .file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| "module".to_string()),
        };
        scope.insert(&alias, Binding::ImportAlias(id), import.span.clone())?;
    }
    Ok(())
}

fn invalid_import(span: &Span, msg: &str) -> RavenError {
    RavenError::resolve(
        ResolveError::UnresolvedImport(msg.to_string()),
        span.clone(),
    )
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use crate::lexer::Lexer;
    use crate::parser::parse;

    use super::super::scope::ScopeStack;
    use super::*;

    /// In memory loader keyed by relative target path (already
    /// normalized by the caller's parent directory in the test).
    #[derive(Default)]
    struct MapLoader {
        files: HashMap<String, (PathBuf, String)>,
    }

    impl MapLoader {
        fn add(&mut self, key: &str, canon: &str, src: &str) {
            self.files
                .insert(key.to_string(), (PathBuf::from(canon), src.to_string()));
        }
    }

    impl SourceLoader for MapLoader {
        fn load(&mut self, _importing: &Path, target: &str) -> Option<LoadedSource> {
            let (canon, src) = self.files.get(target)?;
            Some(LoadedSource {
                canonical_path: canon.clone(),
                source: src.clone(),
            })
        }
    }

    fn parse_src(src: &str, path: &str) -> File {
        let tokens = Lexer::new(src.to_string(), PathBuf::from(path))
            .tokenize()
            .expect("lex");
        parse(&tokens).expect("parse")
    }

    #[test]
    fn std_io_import_binds_alias() {
        let file = parse_src("import std/io\nfun main() {}\n", "main.rv");
        let mut scope = ScopeStack::new();
        let mut loader = MapLoader::default();
        let mut imports = Vec::new();
        let mut in_progress = HashSet::new();
        resolve_imports(
            &file,
            &mut scope,
            &mut loader,
            &mut imports,
            &mut in_progress,
        )
        .expect("ok");
        assert_eq!(imports.len(), 1);
        assert!(matches!(
            imports[0].target,
            ImportTarget::StdlibModule { .. }
        ));
        let entry = scope.lookup("io").expect("alias bound");
        assert!(matches!(entry.binding, Binding::ImportAlias(_)));
    }

    #[test]
    fn unknown_std_module_is_unresolved() {
        let file = parse_src("import std/nope\n", "main.rv");
        let mut scope = ScopeStack::new();
        let mut loader = MapLoader::default();
        let mut imports = Vec::new();
        let mut in_progress = HashSet::new();
        let err = resolve_imports(
            &file,
            &mut scope,
            &mut loader,
            &mut imports,
            &mut in_progress,
        )
        .unwrap_err();
        match err {
            RavenError::Resolve(ResolveError::UnresolvedImport(p), _, _) => {
                assert!(p.contains("nope"));
            }
            other => panic!("expected UnresolvedImport, got {:?}", other),
        }
    }

    #[test]
    fn selector_list_binds_each_selector() {
        let file = parse_src(
            "import std/io { println, eprintln }\nfun main() {}\n",
            "main.rv",
        );
        let mut scope = ScopeStack::new();
        let mut loader = MapLoader::default();
        let mut imports = Vec::new();
        let mut in_progress = HashSet::new();
        resolve_imports(
            &file,
            &mut scope,
            &mut loader,
            &mut imports,
            &mut in_progress,
        )
        .expect("ok");
        let pr = scope.lookup("println").expect("println bound");
        assert!(matches!(pr.binding, Binding::ImportedItem { .. }));
        let ep = scope.lookup("eprintln").expect("eprintln bound");
        assert!(matches!(ep.binding, Binding::ImportedItem { .. }));
    }

    #[test]
    fn github_path_records_external_package() {
        let file = parse_src(
            "import \"github.com/martian56/raven-http\" as http\n",
            "main.rv",
        );
        let mut scope = ScopeStack::new();
        let mut loader = MapLoader::default();
        let mut imports = Vec::new();
        let mut in_progress = HashSet::new();
        resolve_imports(
            &file,
            &mut scope,
            &mut loader,
            &mut imports,
            &mut in_progress,
        )
        .expect("ok");
        assert!(matches!(
            imports[0].target,
            ImportTarget::ExternalPackage { .. }
        ));
        assert!(scope.lookup("http").is_some());
    }

    #[test]
    fn local_import_loads_inner_file_names() {
        let file = parse_src("import \"./helpers\"\nfun main() {}\n", "main.rv");
        let mut loader = MapLoader::default();
        loader.add(
            "./helpers",
            "helpers.rv",
            "fun helper() {}\nstruct H { x: Int }\n",
        );
        let mut scope = ScopeStack::new();
        let mut imports = Vec::new();
        let mut in_progress = HashSet::new();
        resolve_imports(
            &file,
            &mut scope,
            &mut loader,
            &mut imports,
            &mut in_progress,
        )
        .expect("ok");
        match &imports[0].target {
            ImportTarget::LocalModule { module_names, .. } => {
                assert!(module_names.contains(&"helper".to_string()));
                assert!(module_names.contains(&"H".to_string()));
            }
            other => panic!("expected LocalModule, got {:?}", other),
        }
    }

    #[test]
    fn cyclic_local_import_is_detected() {
        // a.rv imports ./b; b.rv imports ./a.
        let file = parse_src("import \"./b\"\n", "a.rv");
        let mut loader = MapLoader::default();
        loader.add("./a", "a.rv", "import \"./b\"\n");
        loader.add("./b", "b.rv", "import \"./a\"\n");
        let mut scope = ScopeStack::new();
        let mut imports = Vec::new();
        let mut in_progress = HashSet::new();
        in_progress.insert(PathBuf::from("a.rv"));
        let err = resolve_imports(
            &file,
            &mut scope,
            &mut loader,
            &mut imports,
            &mut in_progress,
        )
        .unwrap_err();
        match err {
            RavenError::Resolve(ResolveError::CyclicImport(_), _, _) => {}
            other => panic!("expected CyclicImport, got {:?}", other),
        }
    }

    #[test]
    fn missing_local_file_is_unresolved() {
        let file = parse_src("import \"./gone\"\n", "main.rv");
        let mut loader = MapLoader::default();
        let mut scope = ScopeStack::new();
        let mut imports = Vec::new();
        let mut in_progress = HashSet::new();
        let err = resolve_imports(
            &file,
            &mut scope,
            &mut loader,
            &mut imports,
            &mut in_progress,
        )
        .unwrap_err();
        match err {
            RavenError::Resolve(ResolveError::UnresolvedImport(p), _, _) => {
                assert_eq!(p, "./gone");
            }
            other => panic!("expected UnresolvedImport, got {:?}", other),
        }
    }
}
