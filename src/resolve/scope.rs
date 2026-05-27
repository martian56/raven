//! Lexical scope stack used by the resolver.
//!
//! Scopes are a linked list of frames, each holding a `name -> Binding`
//! map and a [`ScopeKind`] tag. Lookups walk innermost to outermost and
//! return the first match (shadowing). Inserting a name that already
//! exists in the same frame is reported as a duplicate declaration.

use std::collections::HashMap;

use crate::error::{RavenError, ResolveError};
use crate::span::Span;

use super::bindings::Binding;

/// What kind of scope a frame represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeKind {
    /// File level scope: top level items and import aliases.
    Module,
    /// An `impl` block. Provides `Self` and (for methods) `self`.
    Impl,
    /// A function body. Holds parameters and generic parameters.
    Function,
    /// A `{ ... }` block expression.
    Block,
    /// A pattern binding scope (match arm, for head, let pattern body).
    Pattern,
}

/// One frame on the scope stack.
#[derive(Debug, Clone)]
pub struct Scope {
    pub kind: ScopeKind,
    pub names: HashMap<String, ScopeEntry>,
}

/// One entry in a [`Scope`].
#[derive(Debug, Clone)]
pub struct ScopeEntry {
    pub binding: Binding,
    /// Where the name was first declared. Used for the
    /// `DuplicateDeclaration` error and for the resolved use map.
    pub declared_at: Span,
}

impl Scope {
    /// Construct an empty scope of the given kind.
    pub fn new(kind: ScopeKind) -> Self {
        Scope {
            kind,
            names: HashMap::new(),
        }
    }
}

/// A stack of scopes. The innermost frame is the last element.
#[derive(Debug, Clone, Default)]
pub struct ScopeStack {
    frames: Vec<Scope>,
}

impl ScopeStack {
    /// Construct a stack with a single empty `Module` frame.
    pub fn new() -> Self {
        ScopeStack {
            frames: vec![Scope::new(ScopeKind::Module)],
        }
    }

    /// Push a fresh frame of the given kind.
    pub fn push(&mut self, kind: ScopeKind) {
        self.frames.push(Scope::new(kind));
    }

    /// Pop the innermost frame. Panics if only the root remains; this is
    /// a resolver bug rather than a user error.
    pub fn pop(&mut self) {
        assert!(self.frames.len() > 1, "cannot pop the module scope");
        self.frames.pop();
    }

    /// The current innermost scope kind.
    pub fn current_kind(&self) -> ScopeKind {
        self.frames.last().expect("scope stack is never empty").kind
    }

    /// Insert a new binding into the innermost frame. Returns
    /// `DuplicateDeclaration` if the name is already present there.
    pub fn insert(
        &mut self,
        name: &str,
        binding: Binding,
        declared_at: Span,
    ) -> Result<(), RavenError> {
        let top = self.frames.last_mut().expect("scope stack is never empty");
        if let Some(prev) = top.names.get(name) {
            return Err(RavenError::resolve(
                ResolveError::DuplicateDeclaration {
                    name: name.to_string(),
                    first_span: prev.declared_at.clone(),
                },
                declared_at,
            ));
        }
        top.names.insert(
            name.to_string(),
            ScopeEntry {
                binding,
                declared_at,
            },
        );
        Ok(())
    }

    /// Insert a binding, but allow shadowing (used by `let` inside
    /// blocks and pattern bindings, which shadow outer names but never
    /// conflict at module level).
    pub fn insert_shadowing(&mut self, name: &str, binding: Binding, declared_at: Span) {
        let top = self.frames.last_mut().expect("scope stack is never empty");
        top.names.insert(
            name.to_string(),
            ScopeEntry {
                binding,
                declared_at,
            },
        );
    }

    /// Look up `name` starting from the innermost frame.
    pub fn lookup(&self, name: &str) -> Option<&ScopeEntry> {
        for frame in self.frames.iter().rev() {
            if let Some(entry) = frame.names.get(name) {
                return Some(entry);
            }
        }
        None
    }

    /// True if any enclosing frame is an `Impl` scope.
    pub fn in_impl(&self) -> bool {
        self.frames.iter().any(|f| f.kind == ScopeKind::Impl)
    }

    /// Borrow the module (root) frame's name map.
    pub fn module_names(&self) -> &HashMap<String, ScopeEntry> {
        &self.frames[0].names
    }
}

#[cfg(test)]
mod scope_tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use super::super::bindings::{Binding, DeclId};
    use super::*;

    fn span(start: usize, end: usize) -> Span {
        Span::new(
            Arc::new(PathBuf::from("test.rv")),
            start,
            end,
            1,
            (start as u32) + 1,
        )
    }

    #[test]
    fn root_frame_is_module() {
        let s = ScopeStack::new();
        assert_eq!(s.current_kind(), ScopeKind::Module);
    }

    #[test]
    fn push_and_pop_changes_kind() {
        let mut s = ScopeStack::new();
        s.push(ScopeKind::Function);
        assert_eq!(s.current_kind(), ScopeKind::Function);
        s.pop();
        assert_eq!(s.current_kind(), ScopeKind::Module);
    }

    #[test]
    fn insert_then_lookup_returns_binding() {
        let mut s = ScopeStack::new();
        s.insert("foo", Binding::Function(DeclId(0)), span(0, 3))
            .unwrap();
        let entry = s.lookup("foo").expect("should resolve");
        assert!(matches!(entry.binding, Binding::Function(DeclId(0))));
    }

    #[test]
    fn duplicate_in_same_frame_is_error() {
        let mut s = ScopeStack::new();
        s.insert("foo", Binding::Function(DeclId(0)), span(0, 3))
            .unwrap();
        let err = s
            .insert("foo", Binding::Function(DeclId(1)), span(10, 13))
            .unwrap_err();
        match err {
            RavenError::Resolve(ResolveError::DuplicateDeclaration { name, .. }, _, _) => {
                assert_eq!(name, "foo");
            }
            other => panic!("expected DuplicateDeclaration, got {:?}", other),
        }
    }

    #[test]
    fn inner_scope_shadows_outer() {
        let mut s = ScopeStack::new();
        s.insert("x", Binding::Function(DeclId(0)), span(0, 1))
            .unwrap();
        s.push(ScopeKind::Block);
        s.insert_shadowing("x", Binding::Local(span(10, 11)), span(10, 11));
        let entry = s.lookup("x").unwrap();
        assert!(matches!(entry.binding, Binding::Local(_)));
        s.pop();
        let entry = s.lookup("x").unwrap();
        assert!(matches!(entry.binding, Binding::Function(_)));
    }

    #[test]
    fn in_impl_detects_enclosing_impl_frame() {
        let mut s = ScopeStack::new();
        assert!(!s.in_impl());
        s.push(ScopeKind::Impl);
        s.push(ScopeKind::Function);
        s.push(ScopeKind::Block);
        assert!(s.in_impl());
    }
}
