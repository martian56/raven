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

use crate::ast::{
    Block, DeclKind, ElseBranch, Expr, ExprKind, File, FunctionBody, ImportSource, LambdaBody,
    MatchArm, Stmt, StmtKind, StrFragment,
};
use crate::error::{RavenError, ResolveError};
use crate::lexer::Lexer;
use crate::parser::parse;

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
    for module in &wanted {
        let module_file = parse_bundled_module(module)?;

        // Collect the module's own top level function names so a call to
        // a sibling function (for example `trim` calling `is_space_byte`)
        // can be rewritten to the namespaced name. The declarations are
        // renamed below; without rewriting the call sites a sibling call
        // would resolve to a bare name that no longer exists.
        let siblings: BTreeSet<String> = module_file
            .items
            .iter()
            .filter_map(|d| match &d.kind {
                DeclKind::Function(f) => Some(f.name.clone()),
                _ => None,
            })
            .collect();

        for mut decl in module_file.items {
            // A bundled module's own `import std/...` declarations are
            // consumed by this expander (the imported module is merged
            // separately); they must not leak into the combined file as
            // import items, which is why they are dropped here.
            if matches!(&decl.kind, DeclKind::Import(_)) {
                continue;
            }
            match &mut decl.kind {
                DeclKind::Function(f) => {
                    rewrite_fn_body_calls(&mut f.body, module, &siblings);
                    f.name = mangle_stdlib_fn(module, &f.name);
                }
                DeclKind::Impl(i) => {
                    // An `impl` on a built in type (for example
                    // `impl String { ... }`) keeps its method names: a
                    // method is dispatched by the receiver's type through
                    // the per type symbol `<RecvType>$<method>`, not by a
                    // free function name, so it never collides with user
                    // code and needs no namespacing. Its body, however,
                    // may call sibling free functions of the same module,
                    // which were renamed above; rewrite those call sites
                    // the same way a free function body is rewritten.
                    for m in &mut i.items {
                        rewrite_fn_body_calls(&mut m.body, module, &siblings);
                    }
                }
                _ => {}
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
fn rewrite_fn_body_calls(body: &mut FunctionBody, module: &str, siblings: &BTreeSet<String>) {
    match body {
        FunctionBody::Block(block) => rewrite_block(block, module, siblings),
        FunctionBody::Expr(expr) => rewrite_expr(expr, module, siblings),
        FunctionBody::None => {}
    }
}

fn rewrite_block(block: &mut Block, module: &str, siblings: &BTreeSet<String>) {
    for stmt in &mut block.stmts {
        rewrite_stmt(stmt, module, siblings);
    }
    if let Some(trailing) = &mut block.trailing {
        rewrite_expr(trailing, module, siblings);
    }
}

fn rewrite_stmt(stmt: &mut Stmt, module: &str, siblings: &BTreeSet<String>) {
    match &mut stmt.kind {
        StmtKind::Let { init, .. } => {
            if let Some(e) = init {
                rewrite_expr(e, module, siblings);
            }
        }
        StmtKind::Return(e) | StmtKind::Break(e) => {
            if let Some(e) = e {
                rewrite_expr(e, module, siblings);
            }
        }
        StmtKind::Defer(e) | StmtKind::Expr(e) => rewrite_expr(e, module, siblings),
        StmtKind::Assign { target, value, .. } => {
            rewrite_expr(target, module, siblings);
            rewrite_expr(value, module, siblings);
        }
        StmtKind::Continue => {}
    }
}

fn rewrite_expr(expr: &mut Expr, module: &str, siblings: &BTreeSet<String>) {
    match &mut expr.kind {
        ExprKind::Ident { name, .. } => {
            if siblings.contains(name) {
                *name = mangle_stdlib_fn(module, name);
            }
        }
        ExprKind::InterpolatedString(fragments) => {
            for frag in fragments {
                if let StrFragment::Expr(e) = frag {
                    rewrite_expr(e, module, siblings);
                }
            }
        }
        ExprKind::Array(items) | ExprKind::Tuple(items) => {
            for e in items {
                rewrite_expr(e, module, siblings);
            }
        }
        ExprKind::StructLit { fields, .. } => {
            for f in fields {
                rewrite_expr(&mut f.value, module, siblings);
            }
        }
        ExprKind::Paren(inner) | ExprKind::Try(inner) => rewrite_expr(inner, module, siblings),
        ExprKind::Block(block) => rewrite_block(block, module, siblings),
        ExprKind::Unary { operand, .. } => rewrite_expr(operand, module, siblings),
        ExprKind::Binary { lhs, rhs, .. } => {
            rewrite_expr(lhs, module, siblings);
            rewrite_expr(rhs, module, siblings);
        }
        ExprKind::Range { start, end, .. } => {
            rewrite_expr(start, module, siblings);
            rewrite_expr(end, module, siblings);
        }
        ExprKind::Call { callee, args } => {
            rewrite_expr(callee, module, siblings);
            for a in args {
                rewrite_expr(a, module, siblings);
            }
        }
        ExprKind::MethodCall { receiver, args, .. } => {
            rewrite_expr(receiver, module, siblings);
            for a in args {
                rewrite_expr(a, module, siblings);
            }
        }
        ExprKind::Field { receiver, .. } => rewrite_expr(receiver, module, siblings),
        ExprKind::Index { receiver, index } => {
            rewrite_expr(receiver, module, siblings);
            rewrite_expr(index, module, siblings);
        }
        ExprKind::If {
            cond,
            then_branch,
            else_branch,
        } => {
            rewrite_expr(cond, module, siblings);
            rewrite_block(then_branch, module, siblings);
            if let Some(else_branch) = else_branch {
                match else_branch.as_mut() {
                    ElseBranch::If(e) => rewrite_expr(e, module, siblings),
                    ElseBranch::Block(b) => rewrite_block(b, module, siblings),
                }
            }
        }
        ExprKind::Match { scrutinee, arms } => {
            rewrite_expr(scrutinee, module, siblings);
            for arm in arms.iter_mut() {
                rewrite_match_arm(arm, module, siblings);
            }
        }
        ExprKind::Loop(block) => rewrite_block(block, module, siblings),
        ExprKind::While { cond, body } => {
            rewrite_expr(cond, module, siblings);
            rewrite_block(body, module, siblings);
        }
        ExprKind::For { iter, body, .. } => {
            rewrite_expr(iter, module, siblings);
            rewrite_block(body, module, siblings);
        }
        ExprKind::Lambda { body, .. } => match body {
            LambdaBody::Block(b) => rewrite_block(b, module, siblings),
            LambdaBody::Expr(e) => rewrite_expr(e, module, siblings),
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

fn rewrite_match_arm(arm: &mut MatchArm, module: &str, siblings: &BTreeSet<String>) {
    if let Some(guard) = &mut arm.guard {
        rewrite_expr(guard, module, siblings);
    }
    rewrite_expr(&mut arm.body, module, siblings);
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
}
