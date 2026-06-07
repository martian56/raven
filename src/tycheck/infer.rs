//! Type inference variables and union-find.
//!
//! Hindley-Milner style inference with let-polymorphism. The checker
//! allocates a fresh inference variable whenever it needs to defer the
//! choice of a concrete type (a generic parameter at a use site, a
//! `let` binding without an annotation, etc.) and unifies the variable
//! with whatever concrete type the surrounding context implies.
//!
//! Unification is structural with an occurs check. Variables are
//! resolved through a union-find table with path compression. Trait
//! bounds attached to a variable are verified eagerly when the variable
//! resolves to a concrete type.
//!
//! See `docs/v2/specs/generics.md` for the design.

use std::collections::HashMap;

use crate::error::{RavenError, TypeError};
use crate::span::Span;

use super::ty::{InferVarId, ParamId, Ty};

/// State for one inference run. Reset per top-level body so inference
/// scopes do not leak across declarations.
#[derive(Debug, Default)]
pub struct InferCtx {
    /// One slot per variable. `slots[i].parent == i` means the variable
    /// is a root (unsolved or solved by an entry in `slots[i].solved`).
    slots: Vec<Slot>,
    /// Pending bounds keyed by variable root. Bounds attach to the
    /// variable's representative; merging variables merges the bound
    /// lists.
    bounds: HashMap<InferVarId, Vec<PendingBound>>,
    /// Deferred "element of an iterator" links. Each entry says the
    /// element variable is the `T` of `source: Iterator<T>`. When the
    /// source resolves to a concrete iterator type, the element variable
    /// is unified with that type's element so a call like
    /// `collect(pipeline)` can infer the element type from the argument
    /// even though `T` appears only in the function's `S: Iterator<T>`
    /// bound and never in a parameter position.
    iterator_links: Vec<IteratorLink>,
    /// Trait impls visible to this body as `(trait name, implementing type)`
    /// pairs, so a pending bound can be verified the moment its variable
    /// resolves to a simple concrete type (for example a `Map` key inferred to
    /// a struct that has no `Hash` impl).
    trait_impls: Vec<(String, Ty)>,
}

/// A deferred link from an iterator-typed variable to its element
/// variable, resolved once the source variable is known.
#[derive(Debug, Clone)]
struct IteratorLink {
    source: InferVarId,
    element: InferVarId,
    span: Span,
}

#[derive(Debug, Default, Clone)]
struct Slot {
    parent: u32,
    rank: u32,
    solved: Option<Ty>,
    span: Option<Span>,
}

/// A pending trait bound recorded against an inference variable.
#[derive(Debug, Clone)]
pub struct PendingBound {
    pub trait_name: String,
    pub span: Span,
}

impl InferCtx {
    /// Construct an empty inference context.
    pub fn new() -> Self {
        Self::default()
    }

    /// Allocate a fresh inference variable. `span` is the source span
    /// associated with the variable's introduction; it is used by
    /// `CannotInferType` errors.
    pub fn fresh(&mut self, span: Span) -> InferVarId {
        let id = self.slots.len() as u32;
        self.slots.push(Slot {
            parent: id,
            rank: 0,
            solved: None,
            span: Some(span),
        });
        InferVarId(id)
    }

    /// Record the trait impls in scope so a pending bound can be checked
    /// against a concrete type as soon as its variable resolves.
    pub fn set_trait_impls(&mut self, impls: Vec<(String, Ty)>) {
        self.trait_impls = impls;
    }

    /// Attach a trait bound to a variable.
    pub fn add_bound(&mut self, v: InferVarId, trait_name: impl Into<String>, span: Span) {
        let root = self.find(v);
        self.bounds.entry(root).or_default().push(PendingBound {
            trait_name: trait_name.into(),
            span,
        });
    }

    /// Bounds pending on `v` (looked up through its root).
    pub fn bounds_of(&self, v: InferVarId) -> Vec<PendingBound> {
        let root = self.find(v);
        self.bounds.get(&root).cloned().unwrap_or_default()
    }

    /// Record that `element` is the `T` of `source: Iterator<T>`. Resolved
    /// later by `solve_iterator_links` once `source` is concrete.
    pub fn add_iterator_link(&mut self, source: InferVarId, element: InferVarId, span: Span) {
        self.iterator_links.push(IteratorLink {
            source,
            element,
            span,
        });
    }

    /// For each deferred iterator link whose source has resolved to a
    /// concrete iterator type, unify the element variable with that type's
    /// element. `elem_of` maps a concrete source type to its `Iterator`
    /// element type (the checker supplies it because the impl table lives
    /// outside the inference context). Returns once a fixed point is
    /// reached. Unsolved links are left for `CannotInferType` to report.
    pub fn solve_iterator_links(
        &mut self,
        elem_of: &impl Fn(&Ty) -> Option<Ty>,
    ) -> Result<(), RavenError> {
        let links = self.iterator_links.clone();
        for link in links {
            let source = self.resolve(&Ty::Var(link.source));
            if source.has_var() {
                continue;
            }
            if let Some(elem) = elem_of(&source) {
                self.unify(&Ty::Var(link.element), &elem, &link.span)?;
            }
        }
        Ok(())
    }

    /// Return the union-find root for `v`. Performs path compression.
    pub fn find(&self, v: InferVarId) -> InferVarId {
        // Iterative path traversal.
        let mut cur = v.0 as usize;
        while self.slots[cur].parent != cur as u32 {
            cur = self.slots[cur].parent as usize;
        }
        InferVarId(cur as u32)
    }

    fn find_mut(&mut self, v: InferVarId) -> InferVarId {
        let mut cur = v.0 as usize;
        while self.slots[cur].parent != cur as u32 {
            let next = self.slots[cur].parent as usize;
            self.slots[cur].parent = self.slots[next].parent;
            cur = next;
        }
        InferVarId(cur as u32)
    }

    /// Resolve `ty` against the current inference state. Variables are
    /// replaced with their solved form when one exists; otherwise the
    /// variable is left in place. Recursive types are walked.
    pub fn resolve(&self, ty: &Ty) -> Ty {
        match ty {
            Ty::Var(v) => {
                let root = self.find(*v);
                match &self.slots[root.0 as usize].solved {
                    Some(t) => self.resolve(t),
                    None => Ty::Var(root),
                }
            }
            Ty::Option(t) => Ty::Option(Box::new(self.resolve(t))),
            Ty::Result(a, b) => Ty::Result(Box::new(self.resolve(a)), Box::new(self.resolve(b))),
            Ty::List(t) => Ty::List(Box::new(self.resolve(t))),
            Ty::SelfTy(t) => Ty::SelfTy(Box::new(self.resolve(t))),
            Ty::Function { params, ret } => Ty::Function {
                params: params.iter().map(|t| self.resolve(t)).collect(),
                ret: Box::new(self.resolve(ret)),
            },
            Ty::Struct { id, name, args } => Ty::Struct {
                id: *id,
                name: name.clone(),
                args: args.iter().map(|t| self.resolve(t)).collect(),
            },
            Ty::Enum { id, name, args } => Ty::Enum {
                id: *id,
                name: name.clone(),
                args: args.iter().map(|t| self.resolve(t)).collect(),
            },
            Ty::Ffi(super::ty::FfiTy::CPtr(inner)) => {
                Ty::Ffi(super::ty::FfiTy::CPtr(Box::new(self.resolve(inner))))
            }
            _ => ty.clone(),
        }
    }

    /// Finalize a type: resolve it and check that no unresolved
    /// variable remains. If one does, return a `CannotInferType` error
    /// anchored at the variable's introduction span (or `fallback`).
    pub fn finalize(&self, ty: &Ty, fallback: &Span) -> Result<Ty, RavenError> {
        let resolved = self.resolve(ty);
        if let Some((v_span, var_id)) = first_unresolved_var(&resolved) {
            // Prefer the span recorded with the variable at creation.
            let span = self
                .slots
                .get(var_id.0 as usize)
                .and_then(|s| s.span.clone())
                .or(Some(v_span))
                .unwrap_or_else(|| fallback.clone());
            return Err(RavenError::ty(TypeError::CannotInferType, span));
        }
        Ok(resolved)
    }

    /// Unify two types under the current inference state. On failure,
    /// returns a `TypeMismatch` at `span`. Records new solutions in the
    /// union-find table.
    pub fn unify(&mut self, a: &Ty, b: &Ty, span: &Span) -> Result<(), RavenError> {
        let a = self.resolve(a);
        let b = self.resolve(b);
        unify_inner(self, &a, &b, span)
    }
}

fn unify_inner(cx: &mut InferCtx, a: &Ty, b: &Ty, span: &Span) -> Result<(), RavenError> {
    if a == b {
        return Ok(());
    }
    if a.is_error() || b.is_error() {
        return Ok(());
    }
    match (a, b) {
        // Strip Self wrappers symmetrically.
        (Ty::SelfTy(x), other) | (other, Ty::SelfTy(x)) => unify_inner(cx, x, other, span),
        (Ty::Var(va), Ty::Var(vb)) => {
            let ra = cx.find_mut(*va);
            let rb = cx.find_mut(*vb);
            if ra == rb {
                return Ok(());
            }
            // Union by rank, prefer one already solved.
            let ra_solved = cx.slots[ra.0 as usize].solved.clone();
            let rb_solved = cx.slots[rb.0 as usize].solved.clone();
            let (root, other, root_solved, other_solved) =
                if cx.slots[ra.0 as usize].rank >= cx.slots[rb.0 as usize].rank {
                    (ra, rb, ra_solved, rb_solved)
                } else {
                    (rb, ra, rb_solved, ra_solved)
                };
            // Merge.
            cx.slots[other.0 as usize].parent = root.0;
            if cx.slots[root.0 as usize].rank == cx.slots[other.0 as usize].rank {
                cx.slots[root.0 as usize].rank += 1;
            }
            // Merge bounds.
            if let Some(bs) = cx.bounds.remove(&other) {
                cx.bounds.entry(root).or_default().extend(bs);
            }
            // Reconcile solutions.
            match (root_solved, other_solved) {
                (Some(rs), Some(os)) => {
                    unify_inner(cx, &rs, &os, span)?;
                }
                (None, Some(os)) => {
                    cx.slots[root.0 as usize].solved = Some(os);
                }
                _ => {}
            }
            Ok(())
        }
        (Ty::Var(v), other) | (other, Ty::Var(v)) => {
            let root = cx.find_mut(*v);
            if occurs(root, other) {
                return Err(RavenError::ty(
                    TypeError::OccursCheck {
                        var: format!("?{}", root.0),
                        ty: format!("{}", other),
                    },
                    span.clone(),
                ));
            }
            // If the variable is already solved, unify both solutions.
            if let Some(prev) = cx.slots[root.0 as usize].solved.clone() {
                return unify_inner(cx, &prev, other, span);
            }
            cx.slots[root.0 as usize].solved = Some(other.clone());
            // Eagerly verify pending bounds now that we know the type.
            if let Some(bs) = cx.bounds.get(&root).cloned() {
                for b in bs {
                    check_bound_eager(&cx.trait_impls, other, &b)?;
                }
            }
            Ok(())
        }
        (Ty::Option(x), Ty::Option(y)) => unify_inner(cx, x, y, span),
        (Ty::List(x), Ty::List(y)) => unify_inner(cx, x, y, span),
        (Ty::Result(t1, e1), Ty::Result(t2, e2)) => {
            unify_inner(cx, t1, t2, span)?;
            unify_inner(cx, e1, e2, span)
        }
        (
            Ty::Function {
                params: pa,
                ret: ra,
            },
            Ty::Function {
                params: pb,
                ret: rb,
            },
        ) => {
            if pa.len() != pb.len() {
                return Err(mismatch(a, b, span));
            }
            for (x, y) in pa.iter().zip(pb.iter()) {
                unify_inner(cx, x, y, span)?;
            }
            unify_inner(cx, ra, rb, span)
        }
        (
            Ty::Struct {
                id: ia,
                args: aa,
                name: na,
                ..
            },
            Ty::Struct {
                id: ib,
                args: ab,
                name: nb,
                ..
            },
        ) => {
            if ia != ib {
                return Err(mismatch_named(na, nb, span));
            }
            if aa.len() != ab.len() {
                return Err(RavenError::ty(
                    TypeError::GenericArityMismatch {
                        decl: na.clone(),
                        expected: aa.len(),
                        actual: ab.len(),
                    },
                    span.clone(),
                ));
            }
            for (x, y) in aa.iter().zip(ab.iter()) {
                unify_inner(cx, x, y, span)?;
            }
            Ok(())
        }
        (
            Ty::Enum {
                id: ia,
                args: aa,
                name: na,
                ..
            },
            Ty::Enum {
                id: ib,
                args: ab,
                name: nb,
                ..
            },
        ) => {
            if ia != ib {
                return Err(mismatch_named(na, nb, span));
            }
            if aa.len() != ab.len() {
                return Err(RavenError::ty(
                    TypeError::GenericArityMismatch {
                        decl: na.clone(),
                        expected: aa.len(),
                        actual: ab.len(),
                    },
                    span.clone(),
                ));
            }
            for (x, y) in aa.iter().zip(ab.iter()) {
                unify_inner(cx, x, y, span)?;
            }
            Ok(())
        }
        (Ty::Param(pa), Ty::Param(pb)) => {
            if pa == pb {
                Ok(())
            } else {
                Err(mismatch(a, b, span))
            }
        }
        // Two opaque typed pointers unify when their pointee types do.
        // The other FFI primitives are handled by the `a == b`
        // short-circuit at the top; a CStr against a CInt, or any FFI
        // type against a native type, falls through to the mismatch.
        (Ty::Ffi(super::ty::FfiTy::CPtr(x)), Ty::Ffi(super::ty::FfiTy::CPtr(y))) => {
            unify_inner(cx, x, y, span)
        }
        _ => Err(mismatch(a, b, span)),
    }
}

fn first_unresolved_var(ty: &Ty) -> Option<(Span, InferVarId)> {
    match ty {
        Ty::Var(v) => {
            // The span argument is filled by the caller's record; use a
            // sentinel here. The caller maps this to the slot's span.
            Some((
                Span::new(
                    std::sync::Arc::new(std::path::PathBuf::from("<inference>")),
                    0,
                    0,
                    0,
                    0,
                ),
                *v,
            ))
        }
        Ty::Option(t) | Ty::List(t) | Ty::SelfTy(t) => first_unresolved_var(t),
        Ty::Result(a, b) => first_unresolved_var(a).or_else(|| first_unresolved_var(b)),
        Ty::Struct { args, .. } | Ty::Enum { args, .. } => {
            args.iter().find_map(first_unresolved_var)
        }
        Ty::Function { params, ret } => params
            .iter()
            .find_map(first_unresolved_var)
            .or_else(|| first_unresolved_var(ret)),
        Ty::Ffi(super::ty::FfiTy::CPtr(inner)) => first_unresolved_var(inner),
        _ => None,
    }
}

fn occurs(v: InferVarId, ty: &Ty) -> bool {
    match ty {
        Ty::Var(other) => *other == v,
        Ty::Option(t) | Ty::List(t) | Ty::SelfTy(t) => occurs(v, t),
        Ty::Result(a, b) => occurs(v, a) || occurs(v, b),
        Ty::Struct { args, .. } | Ty::Enum { args, .. } => args.iter().any(|t| occurs(v, t)),
        Ty::Function { params, ret } => params.iter().any(|t| occurs(v, t)) || occurs(v, ret),
        Ty::Ffi(super::ty::FfiTy::CPtr(inner)) => occurs(v, inner),
        _ => false,
    }
}

fn mismatch(a: &Ty, b: &Ty, span: &Span) -> RavenError {
    RavenError::ty(
        TypeError::TypeMismatch {
            expected: format!("{}", a),
            actual: format!("{}", b),
        },
        span.clone(),
    )
}

fn mismatch_named(a: &str, b: &str, span: &Span) -> RavenError {
    RavenError::ty(
        TypeError::TypeMismatch {
            expected: a.to_string(),
            actual: b.to_string(),
        },
        span.clone(),
    )
}

/// Verify that `ty` satisfies `bound`. The check is intentionally
/// shallow: it succeeds for the `Error` placeholder and for any type
/// that has at least one matching trait impl in the environment. The
/// caller threads the environment when richer bound checking is needed;
/// this eager path is conservative and prefers false positives only for
/// concrete primitive shapes that obviously cannot satisfy a bound.
fn check_bound_eager(
    impls: &[(String, Ty)],
    ty: &Ty,
    bound: &PendingBound,
) -> Result<(), RavenError> {
    // Only judge a *simple* concrete type whose impl can be matched exactly: a
    // primitive or a monomorphic struct/enum. Inference variables and type
    // parameters stay pending (a later resolution or the call site checks
    // them); generic instantiations are deferred because matching a generic
    // impl by structural equality would mis-fire. The matched cases are exactly
    // those the declared-type well-formedness pass also judges, so the two
    // checks stay consistent.
    let simple = match ty {
        Ty::Var(_) | Ty::Param(_) | Ty::Error => false,
        Ty::Struct { args, .. } | Ty::Enum { args, .. } => args.is_empty(),
        Ty::List(_) | Ty::Option(_) | Ty::Result(_, _) | Ty::Function { .. } | Ty::SelfTy(_) => {
            false
        }
        _ => true,
    };
    if !simple {
        return Ok(());
    }
    let satisfied = impls
        .iter()
        .any(|(t, self_ty)| t == &bound.trait_name && super::env::tys_equal(self_ty, ty));
    if satisfied {
        return Ok(());
    }
    let ty_name = format!("{}", ty);
    Err(RavenError::ty(
        TypeError::BoundNotSatisfied {
            ty: ty_name.clone(),
            trait_name: bound.trait_name.clone(),
        },
        bound.span.clone(),
    )
    .with_hint(format!(
        "`{ty}` must implement `{tr}`; add `@derive({tr})` to its definition, or write an `impl {tr} for {ty}`",
        ty = ty_name,
        tr = bound.trait_name
    )))
}

/// Substitute `subst` into `ty`. Walks recursively. `subst` maps each
/// declared generic parameter to its instantiation; entries missing
/// from the map pass through unchanged.
pub fn substitute(ty: &Ty, subst: &HashMap<ParamId, Ty>) -> Ty {
    match ty {
        Ty::Param(p) => subst.get(p).cloned().unwrap_or_else(|| ty.clone()),
        Ty::Option(t) => Ty::Option(Box::new(substitute(t, subst))),
        Ty::List(t) => Ty::List(Box::new(substitute(t, subst))),
        Ty::Result(a, b) => Ty::Result(
            Box::new(substitute(a, subst)),
            Box::new(substitute(b, subst)),
        ),
        Ty::SelfTy(t) => Ty::SelfTy(Box::new(substitute(t, subst))),
        Ty::Function { params, ret } => Ty::Function {
            params: params.iter().map(|t| substitute(t, subst)).collect(),
            ret: Box::new(substitute(ret, subst)),
        },
        Ty::Struct { id, name, args } => Ty::Struct {
            id: *id,
            name: name.clone(),
            args: args.iter().map(|t| substitute(t, subst)).collect(),
        },
        Ty::Enum { id, name, args } => Ty::Enum {
            id: *id,
            name: name.clone(),
            args: args.iter().map(|t| substitute(t, subst)).collect(),
        },
        Ty::Ffi(super::ty::FfiTy::CPtr(inner)) => {
            Ty::Ffi(super::ty::FfiTy::CPtr(Box::new(substitute(inner, subst))))
        }
        _ => ty.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolve::DeclId;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn span() -> Span {
        Span::new(Arc::new(PathBuf::from("t.rv")), 0, 0, 1, 1)
    }

    #[test]
    fn fresh_returns_unique_ids() {
        let mut cx = InferCtx::new();
        let a = cx.fresh(span());
        let b = cx.fresh(span());
        assert_ne!(a, b);
    }

    #[test]
    fn unify_var_with_concrete_solves() {
        let mut cx = InferCtx::new();
        let v = cx.fresh(span());
        cx.unify(&Ty::Var(v), &Ty::Int, &span()).unwrap();
        assert_eq!(cx.resolve(&Ty::Var(v)), Ty::Int);
    }

    #[test]
    fn unify_two_vars_then_concrete() {
        let mut cx = InferCtx::new();
        let a = cx.fresh(span());
        let b = cx.fresh(span());
        cx.unify(&Ty::Var(a), &Ty::Var(b), &span()).unwrap();
        cx.unify(&Ty::Var(a), &Ty::Bool, &span()).unwrap();
        assert_eq!(cx.resolve(&Ty::Var(b)), Ty::Bool);
    }

    #[test]
    fn unify_int_with_bool_errors() {
        let mut cx = InferCtx::new();
        let err = cx.unify(&Ty::Int, &Ty::Bool, &span()).unwrap_err();
        assert!(matches!(err, RavenError::Type(_, _, _)));
    }

    #[test]
    fn occurs_check_blocks_self_reference() {
        let mut cx = InferCtx::new();
        let v = cx.fresh(span());
        let inside = Ty::List(Box::new(Ty::Var(v)));
        let err = cx.unify(&Ty::Var(v), &inside, &span()).unwrap_err();
        match err {
            RavenError::Type(b, _, _) => assert!(matches!(*b, TypeError::OccursCheck { .. })),
            _ => panic!("expected OccursCheck"),
        }
    }

    #[test]
    fn unify_walks_into_options_and_lists() {
        let mut cx = InferCtx::new();
        let v = cx.fresh(span());
        let lhs = Ty::List(Box::new(Ty::Var(v)));
        let rhs = Ty::List(Box::new(Ty::Int));
        cx.unify(&lhs, &rhs, &span()).unwrap();
        assert_eq!(cx.resolve(&Ty::Var(v)), Ty::Int);
    }

    #[test]
    fn finalize_reports_unsolved() {
        let mut cx = InferCtx::new();
        let v = cx.fresh(span());
        let err = cx.finalize(&Ty::Var(v), &span()).unwrap_err();
        match err {
            RavenError::Type(b, _, _) => assert!(matches!(*b, TypeError::CannotInferType)),
            _ => panic!("expected CannotInferType"),
        }
    }

    #[test]
    fn finalize_resolves_when_solved() {
        let mut cx = InferCtx::new();
        let v = cx.fresh(span());
        cx.unify(&Ty::Var(v), &Ty::Int, &span()).unwrap();
        let t = cx.finalize(&Ty::Var(v), &span()).unwrap();
        assert_eq!(t, Ty::Int);
    }

    #[test]
    fn substitute_replaces_param() {
        let owner = span();
        let p = ParamId::new(&owner, 0, "T");
        let mut map = HashMap::new();
        map.insert(p.clone(), Ty::Int);
        let ty = Ty::List(Box::new(Ty::Param(p)));
        let s = substitute(&ty, &map);
        assert_eq!(s, Ty::List(Box::new(Ty::Int)));
    }

    #[test]
    fn substitute_walks_struct_args() {
        let owner = span();
        let p = ParamId::new(&owner, 0, "T");
        let mut map = HashMap::new();
        map.insert(p.clone(), Ty::Bool);
        let ty = Ty::Struct {
            id: DeclId(0),
            name: "Box".into(),
            args: vec![Ty::Param(p)],
        };
        let s = substitute(&ty, &map);
        match s {
            Ty::Struct { args, .. } => assert_eq!(args, vec![Ty::Bool]),
            _ => panic!("expected struct"),
        }
    }
}
