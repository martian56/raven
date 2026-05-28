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

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use crate::ast::{DeclKind, File, ImportSource};
use crate::error::{RavenError, ResolveError};
use crate::lexer::Lexer;
use crate::parser::parse;

/// The embedded source of one bundled stdlib module, keyed by its module
/// path under `std/`. A `std/io` import maps to the `"io"` entry. The
/// list grows as later modules (issues #72 to #80) land; each adds one
/// `include_str!` row here.
pub const BUNDLED_MODULES: &[(&str, &str)] = &[("io", include_str!("../../stdlib/std/io.rv"))];

/// The separator used when namespacing a bundled function name. The
/// resulting name (for example `std.io.println`) is unwritable by a user
/// because Raven identifiers cannot contain `.`.
pub const NAMESPACE_SEP: char = '.';

/// Build the mangled name of a stdlib function: `std.<module>.<name>`.
pub fn mangle_stdlib_fn(module: &str, name: &str) -> String {
    format!("std{sep}{module}{sep}{name}", sep = NAMESPACE_SEP)
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
    let mut wanted: BTreeSet<String> = BTreeSet::new();
    for decl in &user.items {
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

    let mut combined_items = Vec::new();
    for module in &wanted {
        let source = bundled_source(module).expect("module presence checked above");
        let virtual_path = PathBuf::from(format!("<bundled>/std/{module}.rv"));
        let tokens = Lexer::new(source.to_string(), virtual_path.clone())
            .tokenize()
            .map_err(|e| bundled_error(module, format!("lex: {e}")))?;
        let module_file =
            parse(&tokens).map_err(|e| bundled_error(module, format!("parse: {e}")))?;
        for mut decl in module_file.items {
            if let DeclKind::Function(f) = &mut decl.kind {
                f.name = mangle_stdlib_fn(module, &f.name);
            }
            combined_items.push(decl);
        }
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
    fn no_std_import_leaves_file_unchanged() {
        let user = parse_src("fun main() {}\n");
        let combined = expand_with_stdlib(&user).expect("expand");
        assert_eq!(combined.items.len(), user.items.len());
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
}
