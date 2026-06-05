//! Item collection: the first resolver pass.
//!
//! Walks the top level of an [`ast::File`] and inserts every declared
//! name into the module scope. After this pass runs, the second pass
//! (body walk) can resolve forward references because every item is
//! already visible in the module scope.
//!
//! This module does NOT walk function bodies, expression initializers,
//! or anything below the top level. It also does not resolve imports;
//! that lives in [`super::imports`] and runs interleaved with this pass
//! at the same module scope.

use crate::ast::{Decl, DeclKind, File};
use crate::error::RavenError;

use super::bindings::{Binding, DeclId};
use super::scope::ScopeStack;

/// Collect every top level declaration into `scope` as the module
/// scope. Returns the first duplicate declaration error encountered, if
/// any.
///
/// Import declarations are skipped here: [`super::imports::resolve_imports`]
/// processes them so that import aliases can be inserted with full target
/// information.
pub fn collect_items(file: &File, scope: &mut ScopeStack) -> Result<(), RavenError> {
    for (idx, decl) in file.items.iter().enumerate() {
        let id = DeclId(idx);
        collect_decl(decl, id, scope)?;
    }
    Ok(())
}

fn collect_decl(decl: &Decl, id: DeclId, scope: &mut ScopeStack) -> Result<(), RavenError> {
    match &decl.kind {
        DeclKind::Function(f) => {
            scope.insert(&f.name, Binding::Function(id), decl.span.clone())?;
        }
        DeclKind::Struct(s) => {
            scope.insert(&s.name, Binding::Struct(id), decl.span.clone())?;
        }
        DeclKind::Trait(t) => {
            scope.insert(&t.name, Binding::Trait(id), decl.span.clone())?;
        }
        DeclKind::Enum(e) => {
            scope.insert(&e.name, Binding::Enum(id), decl.span.clone())?;
            // Variants are not inserted at module level. They are
            // accessed through the enum name (`Color::Red`) at use
            // sites, and that lookup is the type checker's job.
        }
        DeclKind::Extern(ext) => {
            for (item_index, item) in ext.items.iter().enumerate() {
                // Two bundled modules may declare the same C symbol (for
                // example several stdlib modules each declare
                // `raven_int_to_float`). Redeclaring an extern name is benign:
                // every declaration of a given symbol resolves to the same
                // linker function, and codegen keys externs by name. Skip the
                // duplicate rather than reporting a conflict, while still
                // rejecting a clash with a non-extern declaration.
                if let Some(existing) = scope.lookup(&item.name) {
                    if matches!(existing.binding, Binding::Extern { .. }) {
                        continue;
                    }
                }
                scope.insert(
                    &item.name,
                    Binding::Extern {
                        decl_id: id,
                        item_index,
                    },
                    item.span.clone(),
                )?;
            }
        }
        DeclKind::Const(c) => {
            scope.insert(&c.name, Binding::Const(id), decl.span.clone())?;
        }
        DeclKind::Let(l) => {
            scope.insert(&l.name, Binding::Static(id), decl.span.clone())?;
        }
        DeclKind::Impl(_) => {
            // Impl blocks declare no module level name. Their items are
            // discovered through the implementing type at use sites,
            // which the type checker handles.
        }
        DeclKind::Import(_) => {
            // Handled by `imports::resolve_imports`.
        }
        // Macros are expanded before the compiler parses; this node only
        // appears in the formatter, so it declares no resolvable name.
        DeclKind::Macro(_) => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::lexer::Lexer;
    use crate::parser::parse;
    use std::path::PathBuf;

    use super::super::scope::{ScopeKind, ScopeStack};
    use super::*;

    fn parse_src(src: &str) -> File {
        let tokens = Lexer::new(src.to_string(), PathBuf::from("test.rv"))
            .tokenize()
            .expect("lex");
        parse(&tokens).expect("parse")
    }

    #[test]
    fn collects_functions_and_structs() {
        let file = parse_src("fun foo() {}\nstruct Point { x: Int, y: Int }\nfun bar() {}\n");
        let mut s = ScopeStack::new();
        collect_items(&file, &mut s).unwrap();
        let foo = s.lookup("foo").expect("foo bound");
        assert!(matches!(foo.binding, Binding::Function(_)));
        let pt = s.lookup("Point").expect("Point bound");
        assert!(matches!(pt.binding, Binding::Struct(_)));
        let bar = s.lookup("bar").expect("bar bound");
        assert!(matches!(bar.binding, Binding::Function(_)));
    }

    #[test]
    fn duplicate_top_level_name_is_error() {
        let file = parse_src("fun dup() {}\nfun dup() {}\n");
        let mut s = ScopeStack::new();
        let err = collect_items(&file, &mut s).unwrap_err();
        assert!(matches!(err, RavenError::Resolve(_, _, _)));
    }

    #[test]
    fn extern_items_become_module_bindings() {
        let file = parse_src("extern \"C\" {\nfun puts(s: String) -> Int\n}\n");
        let mut s = ScopeStack::new();
        collect_items(&file, &mut s).unwrap();
        let p = s.lookup("puts").expect("extern fn bound");
        assert!(matches!(p.binding, Binding::Extern { .. }));
    }

    #[test]
    fn impl_blocks_add_no_module_names() {
        let file = parse_src("struct Point { x: Int }\nimpl Point { fun get(self) -> Int = 1 }\n");
        let mut s = ScopeStack::new();
        collect_items(&file, &mut s).unwrap();
        // Point is in scope, `get` is not (it's reachable only through
        // method dispatch on Point, handled later).
        assert!(s.lookup("Point").is_some());
        assert!(s.lookup("get").is_none());
        assert_eq!(s.current_kind(), ScopeKind::Module);
    }

    #[test]
    fn const_and_let_become_module_bindings() {
        let file = parse_src("const PI: Int = 3\nlet counter: Int = 0\n");
        let mut s = ScopeStack::new();
        collect_items(&file, &mut s).unwrap();
        assert!(matches!(s.lookup("PI").unwrap().binding, Binding::Const(_)));
        assert!(matches!(
            s.lookup("counter").unwrap().binding,
            Binding::Static(_)
        ));
    }
}
