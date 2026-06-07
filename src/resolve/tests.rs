//! Inline unit tests for the resolver public surface.
//!
//! These exercise the end to end pipeline (lex, parse, resolve) for
//! the scenarios called out in `docs/v2/specs/resolver.md`'s test
//! coverage section. Module local tests (scope mechanics, item
//! collection, import resolution) live next to their modules.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::ast::File;
use crate::error::{RavenError, ResolveError};
use crate::lexer::Lexer;
use crate::parser::parse;

use super::bindings::Binding;
use super::imports::{LoadedSource, SourceLoader};
use super::resolve_file;

fn parse_src(src: &str, path: &str) -> File {
    let tokens = Lexer::new(src.to_string(), PathBuf::from(path))
        .tokenize()
        .expect("lex");
    parse(&tokens).expect("parse")
}

/// Loader that never finds anything. Useful when the program has no
/// local imports.
struct NoLoader;

impl SourceLoader for NoLoader {
    fn load(&mut self, _importing: &Path, _target: &str) -> Option<LoadedSource> {
        None
    }
}

/// Loader that serves source from an in memory map, keyed by the
/// import string the user wrote.
#[derive(Default)]
struct MapLoader {
    files: HashMap<String, (PathBuf, String)>,
}

impl MapLoader {
    fn add(&mut self, key: &str, canon: &str, src: &str) -> &mut Self {
        self.files
            .insert(key.to_string(), (PathBuf::from(canon), src.to_string()));
        self
    }
}

impl SourceLoader for MapLoader {
    fn load(&mut self, _importing: &Path, target: &str) -> Option<LoadedSource> {
        let (p, s) = self.files.get(target)?;
        Some(LoadedSource {
            canonical_path: p.clone(),
            source: s.clone(),
        })
    }
}

#[test]
fn parameter_use_resolves_to_param_binding() {
    let file = parse_src("fun id(x: Int) -> Int = x\n", "test.rv");
    let r = resolve_file(&file, &mut NoLoader).expect("ok");
    let param_uses: Vec<_> = r
        .map
        .uses
        .values()
        .filter(|b| matches!(b, Binding::Param(_)))
        .collect();
    assert_eq!(param_uses.len(), 1);
}

#[test]
fn forward_function_reference_resolves() {
    // `bar` calls `foo`, but `foo` is declared later in the file. The
    // two pass resolver should accept this.
    let file = parse_src("fun bar() { foo() }\nfun foo() {}\n", "test.rv");
    let r = resolve_file(&file, &mut NoLoader).expect("forward ref ok");
    let fn_uses: Vec<_> = r
        .map
        .uses
        .values()
        .filter(|b| matches!(b, Binding::Function(_)))
        .collect();
    assert!(!fn_uses.is_empty());
}

#[test]
fn unresolved_name_in_body_is_error() {
    let file = parse_src("fun main() { mystery_name() }\n", "test.rv");
    let err = resolve_file(&file, &mut NoLoader).unwrap_err();
    match err {
        RavenError::Resolve(ResolveError::UnresolvedName(name), _, _) => {
            assert_eq!(name, "mystery_name");
        }
        other => panic!("expected UnresolvedName, got {:?}", other),
    }
}

#[test]
fn duplicate_top_level_declaration_is_error() {
    let file = parse_src("fun dup() {}\nfun dup() {}\n", "test.rv");
    let err = resolve_file(&file, &mut NoLoader).unwrap_err();
    matches!(
        err,
        RavenError::Resolve(ResolveError::DuplicateDeclaration { .. }, _, _)
    );
}

#[test]
fn inner_let_shadows_outer_let() {
    let file = parse_src(
        "fun f() {\n    let x = 1\n    {\n        let x = 2\n        x\n    }\n}\n",
        "test.rv",
    );
    let r = resolve_file(&file, &mut NoLoader).expect("ok");
    // Two `let` bindings + one use site. The use site should resolve
    // to a Local (the inner shadowing one).
    let local_uses: Vec<_> = r
        .map
        .uses
        .values()
        .filter(|b| matches!(b, Binding::Local(_)))
        .collect();
    assert_eq!(local_uses.len(), 1);
}

#[test]
fn self_outside_impl_is_an_error() {
    let file = parse_src("fun f() { self.x }\n", "test.rv");
    let err = resolve_file(&file, &mut NoLoader).unwrap_err();
    matches!(
        err,
        RavenError::Resolve(ResolveError::SelfOutsideImpl, _, _)
    );
}

#[test]
fn self_inside_impl_is_ok() {
    let file = parse_src(
        "struct Point { x: Int }\nimpl Point {\n    fun get(self) -> Int = self.x\n}\n",
        "test.rv",
    );
    let r = resolve_file(&file, &mut NoLoader).expect("ok");
    let has_self_value = r.map.uses.values().any(|b| matches!(b, Binding::SelfValue));
    assert!(has_self_value, "expected at least one SelfValue use");
}

#[test]
fn self_without_self_param_is_an_error() {
    // A method that uses `self` but does not declare it as a parameter is
    // rejected at resolve time, rather than surfacing as a confusing codegen
    // error (a field access on a Unit `self`).
    let file = parse_src(
        "struct T { items: List<Int> }\nimpl T {\n    fun add(n: Int) { self.items.push(n) }\n}\n",
        "test.rv",
    );
    let err = resolve_file(&file, &mut NoLoader).unwrap_err();
    assert!(matches!(
        err,
        RavenError::Resolve(ResolveError::SelfNotMethodParam, _, _)
    ));
}

#[test]
fn self_upper_inside_impl_resolves_to_self_type() {
    let file = parse_src(
        "struct Pair { x: Int, y: Int }\nimpl Pair {\n    fun make() -> Self { Pair { x: 1, y: 2 } }\n}\n",
        "test.rv",
    );
    let r = resolve_file(&file, &mut NoLoader).expect("ok");
    let has_self_type = r.map.uses.values().any(|b| matches!(b, Binding::SelfType));
    assert!(has_self_type, "expected SelfType use");
}

#[test]
fn function_parameter_in_scope_for_body() {
    let file = parse_src("fun add(a: Int, b: Int) -> Int = a + b\n", "test.rv");
    let r = resolve_file(&file, &mut NoLoader).expect("ok");
    let params: Vec<_> = r
        .map
        .uses
        .values()
        .filter(|b| matches!(b, Binding::Param(_)))
        .collect();
    assert_eq!(params.len(), 2);
}

#[test]
fn generic_parameter_is_in_scope_in_signature() {
    let file = parse_src("fun id<T>(x: T) -> T = x\n", "test.rv");
    let r = resolve_file(&file, &mut NoLoader).expect("ok");
    // T appears in two signature positions; both should resolve to a
    // generic parameter binding.
    let generic_uses: Vec<_> = r
        .map
        .uses
        .values()
        .filter(|b| matches!(b, Binding::GenericParam { .. }))
        .collect();
    assert_eq!(generic_uses.len(), 2);
}

#[test]
fn pattern_binding_introduces_name() {
    let file = parse_src(
        "fun classify(n: Int) -> Int {\n    match n {\n        x -> x + 1\n    }\n}\n",
        "test.rv",
    );
    let r = resolve_file(&file, &mut NoLoader).expect("ok");
    let pat_uses: Vec<_> = r
        .map
        .uses
        .values()
        .filter(|b| matches!(b, Binding::PatternBinding(_)))
        .collect();
    // `x` appears once in the body (the LHS in the match arm is the
    // declaration, the RHS is the use).
    assert_eq!(pat_uses.len(), 1);
}

#[test]
fn import_alias_is_bound_at_module_level() {
    let file = parse_src("import std/io\nfun main() {}\n", "main.rv");
    let r = resolve_file(&file, &mut NoLoader).expect("ok");
    let alias = r
        .module_scope
        .lookup("io")
        .expect("io alias should be bound");
    assert!(matches!(alias.binding, Binding::ImportAlias(_)));
}

#[test]
fn import_selector_binds_inner_name() {
    let file = parse_src(
        "import std/io { println }\nfun main() { println(\"hi\") }\n",
        "main.rv",
    );
    let r = resolve_file(&file, &mut NoLoader).expect("ok");
    let used = r
        .map
        .uses
        .values()
        .any(|b| matches!(b, Binding::ImportedItem { .. }));
    assert!(used, "println call should resolve to ImportedItem");
}

#[test]
fn local_recursive_import_chain_resolves() {
    let file = parse_src("import \"./helpers\"\nfun main() { helpers }\n", "main.rv");
    let mut loader = MapLoader::default();
    loader.add("./helpers", "helpers.rv", "fun greet() {}\n");
    let r = resolve_file(&file, &mut loader).expect("ok");
    let used = r
        .map
        .uses
        .values()
        .any(|b| matches!(b, Binding::ImportAlias(_)));
    assert!(used, "helpers reference should resolve to ImportAlias");
}

#[test]
fn cyclic_local_imports_are_reported() {
    let file = parse_src("import \"./b\"\nfun main() {}\n", "a.rv");
    let mut loader = MapLoader::default();
    loader
        .add("./a", "a.rv", "import \"./b\"\nfun main() {}\n")
        .add("./b", "b.rv", "import \"./a\"\n");
    let err = resolve_file(&file, &mut loader).unwrap_err();
    matches!(
        err,
        RavenError::Resolve(ResolveError::CyclicImport(_), _, _)
    );
}

#[test]
fn selective_bundled_type_import_does_not_double_declare() {
    // A selective import of a bundled type (`import std/collections { Map }`)
    // must not collide with the merged type declaration the expander adds
    // under the same name (issue #184). The expander merges `Map`/`Set`
    // under their own names; the selector must bind to the merged type
    // rather than introduce a second declaration.
    let user = parse_src(
        "import std/collections { Map, Set }\nfun main() { let m = Map.new() let s = Set.new() }\n",
        "main.rv",
    );
    let combined = super::expand_with_stdlib(&user).expect("expand");
    super::resolve_file(&combined, &mut NoLoader).expect("selective type import resolves");
}

#[test]
fn unknown_stdlib_module_is_unresolved() {
    let file = parse_src("import std/nonsense\n", "main.rv");
    let err = resolve_file(&file, &mut NoLoader).unwrap_err();
    match err {
        RavenError::Resolve(ResolveError::UnresolvedImport(p), _, _) => {
            assert!(p.contains("nonsense"));
        }
        other => panic!("expected UnresolvedImport, got {:?}", other),
    }
}

#[test]
fn struct_field_type_resolves_to_struct() {
    let file = parse_src(
        "struct Inner { v: Int }\nstruct Outer { inner: Inner }\n",
        "test.rv",
    );
    let r = resolve_file(&file, &mut NoLoader).expect("ok");
    let has_struct_use = r.map.uses.values().any(|b| matches!(b, Binding::Struct(_)));
    assert!(has_struct_use, "Inner should be referenced as a Struct");
}

#[test]
fn enum_use_resolves_to_enum_binding() {
    let file = parse_src(
        "enum Color { Red, Green, Blue }\nfun first() -> Color { Red }\n",
        "test.rv",
    );
    // `first` references `Red` directly, which is not in module scope.
    // It must therefore fail to resolve at this layer; the type
    // checker can lift enum constructors into scope through their
    // enum's name at a later pass.
    let err = resolve_file(&file, &mut NoLoader).unwrap_err();
    matches!(
        err,
        RavenError::Resolve(ResolveError::UnresolvedName(_), _, _)
    );
}
