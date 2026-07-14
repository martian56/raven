//! Body check pass.
//!
//! For each declaration with a body (function, method inside an impl,
//! const initializer, top level let initializer), this pass walks the
//! body and assigns a [`Ty`] to every expression site. Inferred types
//! are recorded in the [`TypeMap`].
//!
//! The walker keeps a small stack of local variable bindings, the
//! current function's return type (used by `return`), and the current
//! `Self` type (used by methods). Errors short circuit the walk.

use std::collections::HashMap;

use crate::ast::{
    AssignOp, BinaryOp, Block, Decl, DeclKind, ElseBranch, Expr, ExprKind, FieldInit, Function,
    FunctionBody, LambdaBody, Stmt, StmtKind, StrFragment, UnaryOp,
};
use crate::error::{RavenError, TypeError};
use crate::resolve::{Binding, ResolvedFile, UseKey};
use crate::span::Span;

use super::builtin;
use super::collect::{resolve_ty, scope_from_params, GenericScope};
use super::env::{FnSig, GenericParamSig, TypeEnv, VariantPayloadSig};
use super::infer::{substitute, InferCtx};
use super::pattern;
use super::ty::InferVarId;
use super::ty::{FfiTy, ParamId, Ty};
use super::unify::assignable;
use super::TypeMap;

/// Walk every function body and module level expression in `resolved`,
/// recording each expression's inferred type in `types`.
pub fn check_bodies(
    resolved: &ResolvedFile<'_>,
    env: &TypeEnv,
    types: &mut TypeMap,
) -> Result<(), Vec<RavenError>> {
    // Each top-level item recovers at its own boundary, so an error in one
    // function or impl method does not hide errors in the next.
    let mut errors: Vec<RavenError> = Vec::new();
    for decl in &resolved.file.items {
        errors.extend(check_decl_body(decl, resolved, env, types));
    }
    dedup_errors(&mut errors);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Drop duplicate diagnostics, keeping the first occurrence. Two errors at
/// the same span with the same message are treated as duplicates, so a root
/// cause recorded once (and recovered to `Ty::Error`) does not spray the
/// same follow-on message from several sibling sites.
fn dedup_errors(errors: &mut Vec<RavenError>) {
    let mut seen: std::collections::HashSet<(std::path::PathBuf, usize, usize, String)> =
        std::collections::HashSet::new();
    errors.retain(|e| {
        let s = e.span();
        seen.insert((s.file.to_path_buf(), s.start, s.end, format!("{}", e)))
    });
}

fn check_decl_body(
    decl: &Decl,
    resolved: &ResolvedFile<'_>,
    env: &TypeEnv,
    types: &mut TypeMap,
) -> Vec<RavenError> {
    match &decl.kind {
        DeclKind::Function(f) => check_function(f, None, &[], resolved, env, types),
        DeclKind::Impl(i) => {
            let mut errors: Vec<RavenError> = Vec::new();
            let impl_sig = env.impls.iter().find(|s| s.span == i.span).cloned();
            let impl_generics = impl_sig
                .as_ref()
                .map(|s| s.generics.clone())
                .unwrap_or_default();
            let (impl_path, _) = match &i.for_type {
                Some(t) => (t, Some(())),
                None => (&i.trait_or_type, None),
            };
            let scope = scope_from_params(&impl_generics);
            let self_ty = match resolve_ty(
                &crate::ast::Type {
                    kind: crate::ast::TypeKind::Path(impl_path.clone()),
                    span: impl_path.span.clone(),
                },
                resolved,
                env,
                None,
                &scope,
            ) {
                Ok(t) => t,
                Err(e) => {
                    errors.push(e);
                    Ty::Error
                }
            };
            for f in &i.items {
                errors.extend(check_function(
                    f,
                    Some(&self_ty),
                    &impl_generics,
                    resolved,
                    env,
                    types,
                ));
            }
            errors
        }
        DeclKind::Trait(t) => {
            // Default bodies in trait declarations: walk them without
            // a concrete Self because we treat `Self` as an error
            // marker for now; trait default bodies that reference Self
            // remain limited in this release.
            let mut errors: Vec<RavenError> = Vec::new();
            for m in &t.members {
                if matches!(m.body, FunctionBody::None) {
                    continue;
                }
                errors.extend(check_function(m, None, &[], resolved, env, types));
            }
            errors
        }
        DeclKind::Const(c) => {
            let expected = env
                .consts
                .get(&const_id_of(decl, resolved))
                .cloned()
                .unwrap_or(Ty::Error);
            let mut cx = Checker::new(resolved, env, types, None, expected.clone());
            let actual = cx.check_expr_recover(&c.value);
            if !matches!(expected, Ty::Error) {
                cx.unify_recover(&expected, &actual, &c.value.span);
            }
            cx.finalize_into_errors();
            cx.take_errors()
        }
        DeclKind::Let(l) => {
            let scope = GenericScope::new();
            let mut errors: Vec<RavenError> = Vec::new();
            let expected = match &l.ty {
                Some(t) => match resolve_ty(t, resolved, env, None, &scope) {
                    Ok(t) => t,
                    Err(e) => {
                        errors.push(e);
                        Ty::Error
                    }
                },
                None => Ty::Error,
            };
            if let Some(init) = &l.init {
                let mut cx = Checker::new(resolved, env, types, None, expected.clone());
                cx.errors = std::mem::take(&mut errors);
                // Expose a declared `List<T>` element type so an empty `[]`
                // initializer can adopt it, the same as a local `let` does.
                if let Ty::List(elem) = &expected {
                    cx.array_hint = Some((**elem).clone());
                }
                let actual = cx.check_expr_recover(init);
                if !matches!(expected, Ty::Error) {
                    cx.unify_recover(&expected, &actual, &init.span);
                }
                cx.finalize_into_errors();
                return cx.take_errors();
            }
            errors
        }
        DeclKind::Struct(_)
        | DeclKind::Enum(_)
        | DeclKind::Extern(_)
        | DeclKind::Import(_)
        // Macros are expanded before the compiler parses; only the formatter
        // produces this node, so there is no body to check.
        | DeclKind::Macro(_) => Vec::new(),
    }
}

fn const_id_of(decl: &Decl, resolved: &ResolvedFile<'_>) -> crate::resolve::DeclId {
    use crate::resolve::DeclId;
    for (idx, d) in resolved.file.items.iter().enumerate() {
        if std::ptr::eq(d, decl) {
            return DeclId(idx);
        }
    }
    DeclId(usize::MAX)
}

fn check_function(
    f: &Function,
    self_ty: Option<&Ty>,
    extra_generics: &[GenericParamSig],
    resolved: &ResolvedFile<'_>,
    env: &TypeEnv,
    types: &mut TypeMap,
) -> Vec<RavenError> {
    // Build a generic scope: enclosing impl generics, then this
    // function's own generics.
    let fn_generics = super::collect::scope_from_params(extra_generics);
    // Layer the function's own generics on top.
    let f_params = super::collect::collect_generic_params_for_owner(&f.generics, &f.span);
    let mut full_scope = fn_generics;
    super::collect::push_into_scope(&mut full_scope, &f_params);

    // The return type is resolved before the body's checker exists; a
    // failure recovers to `Ty::Error` and is seeded into the sink below.
    let mut setup_errors: Vec<RavenError> = Vec::new();
    let ret_ty = match &f.ret {
        Some(t) => match resolve_ty(t, resolved, env, self_ty, &full_scope) {
            Ok(t) => t,
            Err(e) => {
                setup_errors.push(e);
                Ty::Error
            }
        },
        None => Ty::Unit,
    };

    let mut cx =
        Checker::new(resolved, env, types, self_ty.cloned(), ret_ty.clone()).with_scope(full_scope);
    cx.errors = setup_errors;
    // Record the trait bounds of every in-scope generic parameter (the
    // enclosing impl's plus this function's own) so a method call on a
    // `Ty::Param` value can resolve through its bound.
    cx.record_param_bounds(extra_generics);
    cx.record_param_bounds(&f_params);

    // Bind parameters into the local scope. The resolver records
    // `Binding::Param(span)` for parameter sites; we mirror that key.
    for p in &f.params {
        let ty = if p.name == "self" {
            self_ty
                .cloned()
                .map(|t| Ty::SelfTy(Box::new(t)))
                .unwrap_or(Ty::Error)
        } else {
            match cx.resolve_ast_ty(&p.ty) {
                Ok(t) => t,
                Err(e) => {
                    cx.push_error(e);
                    Ty::Error
                }
            }
        };
        cx.locals.insert(BindingKey::param(&p.span), ty);
    }

    match &f.body {
        FunctionBody::Block(b) => {
            let body_ty = cx.check_block(b).unwrap_or(Ty::Error);
            if b.trailing.is_some() && !matches!(ret_ty.strip_self(), Ty::Unit | Ty::Error) {
                cx.unify_recover(&ret_ty, &body_ty, &b.span);
            } else if !matches!(ret_ty, Ty::Unit | Ty::Error) {
                // No trailing expression and a non unit return type.
                // Acceptable as long as the body contains explicit
                // returns; we do not analyze control flow here.
            }
        }
        FunctionBody::Expr(e) => {
            let body_ty = cx.check_expr_recover(e);
            cx.unify_recover(&ret_ty, &body_ty, &e.span);
        }
        FunctionBody::None => {}
    }
    cx.finalize_into_errors();
    cx.take_errors()
}

/// A local typing scope.
struct Checker<'a, 'b> {
    resolved: &'a ResolvedFile<'a>,
    env: &'a TypeEnv,
    types: &'b mut TypeMap,
    /// The `Self` type of the enclosing impl, if any.
    self_ty: Option<Ty>,
    /// The current function's declared return type.
    return_ty: Ty,
    /// Local variable types keyed by the binding's declaration span.
    locals: HashMap<BindingKey, Ty>,
    /// Lexical scope of generic parameters from the enclosing
    /// declaration (impl + method).
    generic_scope: GenericScope,
    /// Trait bounds declared on each in-scope generic parameter, keyed by
    /// its [`ParamId`]. A method call on a value of type `Ty::Param(p)`
    /// looks up `p`'s bounds here to find the trait that declares the
    /// called method (bound-driven trait method dispatch).
    param_bounds: HashMap<ParamId, Vec<(String, Vec<Ty>)>>,
    /// Inference context for this body. Holds variables, their
    /// solutions, and any pending trait bounds.
    infer: InferCtx,
    /// Element type hint for an empty array literal, set from a `let`
    /// binding's declared `List<T>` type while its initializer is checked.
    /// An empty `[]` has no element to infer from, so it adopts this hint.
    array_hint: Option<Ty>,
    /// Accumulated diagnostics for this body. Statement and item checking
    /// recover at their boundaries (binding a failed value to `Ty::Error`)
    /// and push the error here instead of returning it, so one compile can
    /// report many independent errors. See `docs/v2/specs/tycheck.md`.
    errors: Vec<RavenError>,
    /// Binding keys of `const` locals in this body, used to reject a
    /// reassignment of an immutable local.
    const_locals: std::collections::HashSet<BindingKey>,
    /// Type-map keys this body recorded. Finalization resolves only these, not
    /// the whole shared map, so a later body never tries to resolve an earlier
    /// body's variable against its own (foreign) inference context.
    recorded: Vec<UseKey>,
    /// Stack of enclosing loops, one entry per `loop`/`while`/`for` currently
    /// being checked; `true` for a value-producing `loop`, `false` for
    /// `while`/`for`. Empty means `break`/`continue` here is outside any loop.
    /// Reset to empty when checking a lambda body, since a loop does not extend
    /// across a nested function.
    loop_kinds: Vec<bool>,
}

/// Keys used by the locals map. Mirrors the resolver's `Binding`
/// variants for `Param`, `Local`, and `PatternBinding`. We key by the
/// resolver's `UseKey` (file plus byte range) because `Span` is not
/// `Hash`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum BindingKey {
    Param(UseKey),
    Local(UseKey),
    Pattern(UseKey),
}

impl BindingKey {
    /// Construct a param key from a span.
    pub fn param(span: &Span) -> Self {
        BindingKey::Param(UseKey::from_span(span))
    }
    /// Construct a local key from a span.
    pub fn local(span: &Span) -> Self {
        BindingKey::Local(UseKey::from_span(span))
    }
    /// Construct a pattern key from a span.
    pub fn pattern(span: &Span) -> Self {
        BindingKey::Pattern(UseKey::from_span(span))
    }
}

impl<'a, 'b> Checker<'a, 'b> {
    fn new(
        resolved: &'a ResolvedFile<'a>,
        env: &'a TypeEnv,
        types: &'b mut TypeMap,
        self_ty: Option<Ty>,
        return_ty: Ty,
    ) -> Self {
        // Hand the inference context the trait impls in scope so a pending
        // bound is verified the moment its variable resolves to a concrete
        // type (catches, for example, an inferred `Map` key with no `Hash`).
        let mut infer = InferCtx::new();
        infer.set_trait_impls(
            env.impls
                .iter()
                .filter_map(|i| {
                    i.trait_name
                        .as_ref()
                        .map(|t| (t.clone(), i.self_ty.clone()))
                })
                .collect(),
        );
        Self {
            resolved,
            env,
            types,
            self_ty,
            return_ty,
            locals: HashMap::new(),
            generic_scope: GenericScope::new(),
            param_bounds: HashMap::new(),
            infer,
            array_hint: None,
            errors: Vec::new(),
            const_locals: std::collections::HashSet::new(),
            recorded: Vec::new(),
            loop_kinds: Vec::new(),
        }
    }

    fn with_scope(mut self, scope: GenericScope) -> Self {
        self.generic_scope = scope;
        self
    }

    /// Record a diagnostic in the sink and keep going. Used at recovery
    /// points so an error in one statement or item does not hide errors in
    /// the next.
    fn push_error(&mut self, e: RavenError) {
        self.errors.push(e);
    }

    /// Take the accumulated diagnostics, leaving the sink empty.
    fn take_errors(&mut self) -> Vec<RavenError> {
        std::mem::take(&mut self.errors)
    }

    /// Check an expression, recovering to `Ty::Error` (and recording the
    /// error) when it fails, so sibling statements still type-check.
    fn check_expr_recover(&mut self, expr: &Expr) -> Ty {
        match self.check_expr(expr) {
            Ok(t) => t,
            Err(e) => {
                self.push_error(e);
                Ty::Error
            }
        }
    }

    /// Unify, recording a mismatch in the sink instead of returning it.
    fn unify_recover(&mut self, expected: &Ty, actual: &Ty, span: &Span) {
        if let Err(e) = self.unify(expected, actual, span) {
            self.push_error(e);
        }
    }

    /// Record the trait bounds declared on a set of generic parameters so
    /// a method call on a `Ty::Param` value can find the trait that
    /// declares the called method.
    fn record_param_bounds(&mut self, params: &[GenericParamSig]) {
        for p in params {
            if !p.bounds.is_empty() {
                let entries: Vec<(String, Vec<Ty>)> = p
                    .bounds
                    .iter()
                    .enumerate()
                    .map(|(i, name)| {
                        let args = p.bound_args.get(i).cloned().unwrap_or_default();
                        (name.clone(), args)
                    })
                    .collect();
                self.param_bounds.insert(p.id.clone(), entries);
            }
        }
    }

    /// Convenience: resolve an AST type using the current generic scope.
    fn resolve_ast_ty(&self, t: &crate::ast::Type) -> Result<Ty, RavenError> {
        resolve_ty(
            t,
            self.resolved,
            self.env,
            self.self_ty.as_ref(),
            &self.generic_scope,
        )
    }

    /// Unify two types under the inference context. On failure, raise a
    /// TypeMismatch at `span` with a suggestion hint when possible.
    ///
    /// Before reporting a mismatch this checks for a `dyn Trait` unsizing
    /// coercion: a concrete struct or enum used where `dyn Trait` is
    /// expected, where that type implements the trait. When it applies,
    /// the coercion is recorded at `span` (the coerced expression's span)
    /// and unification succeeds.
    fn unify(&mut self, expected: &Ty, actual: &Ty, span: &Span) -> Result<(), RavenError> {
        let exp_resolved = self.infer.resolve(expected);
        if let Ty::Dyn { name, methods } = exp_resolved.strip_self() {
            let act_resolved = self.infer.resolve(actual);
            let concrete = act_resolved.strip_self().clone();
            if self.implements_trait(&concrete, name) {
                self.types.record_coercion(
                    span,
                    crate::tycheck::DynCoercion {
                        trait_name: name.clone(),
                        methods: methods.clone(),
                        concrete_ty: concrete,
                    },
                );
                return Ok(());
            }
            // A `dyn Trait` target with a concrete actual that does not
            // implement the trait: fall through to the mismatch report.
        }
        match self.infer.unify(expected, actual, span) {
            Ok(()) => Ok(()),
            Err(e) => {
                // Resolve both sides so the diagnostic uses the most
                // specific representation.
                let exp_resolved = self.infer.resolve(expected);
                let act_resolved = self.infer.resolve(actual);
                // Re-raise with the resolved display.
                let mut err = RavenError::ty(
                    TypeError::TypeMismatch {
                        expected: format!("{}", exp_resolved),
                        actual: format!("{}", act_resolved),
                    },
                    span.clone(),
                );
                if matches!(exp_resolved.strip_self(), Ty::Int)
                    && matches!(act_resolved.strip_self(), Ty::Float)
                {
                    err = err.with_hint("did you mean to call `.to_int()`?");
                } else if matches!(exp_resolved.strip_self(), Ty::Float)
                    && matches!(act_resolved.strip_self(), Ty::Int)
                {
                    err = err.with_hint("did you mean to call `.to_float()`?");
                }
                // For OccursCheck, propagate the original
                if matches!(e, RavenError::Type(ref b, _, _) if matches!(**b, TypeError::OccursCheck { .. }))
                {
                    return Err(e);
                }
                // For GenericArityMismatch and BoundNotSatisfied, also propagate.
                if let RavenError::Type(ref b, _, _) = e {
                    if matches!(
                        **b,
                        TypeError::GenericArityMismatch { .. }
                            | TypeError::BoundNotSatisfied { .. }
                            | TypeError::CannotInferType
                    ) {
                        return Err(e);
                    }
                }
                Err(err)
            }
        }
    }

    /// Whether `concrete` has a trait impl for the trait named `trait_name`.
    /// Used to validate a `dyn Trait` unsizing coercion. The match is by
    /// the implementing type's identity (ignoring `Self` wrappers) and the
    /// impl's recorded trait name.
    fn implements_trait(&self, concrete: &Ty, trait_name: &str) -> bool {
        if concrete.is_error() {
            return true;
        }
        self.env.impls.iter().any(|imp| {
            imp.trait_name.as_deref() == Some(trait_name)
                && super::env::tys_equal(&imp.self_ty, concrete)
        })
    }

    /// Require that a value of type `ty` can be rendered to a `String`
    /// through the `ToString` trait, for the built-in `print`. A
    /// `String` passes directly. A generic-parameter type passes when one
    /// of its bounds is `ToString`. Any other concrete type passes when
    /// it has a `ToString` impl (the auto-imported built-in impls cover
    /// the scalars; a user type provides its own). An inference variable
    /// records a pending `ToString` bound so the constraint is checked
    /// once the variable resolves. `Error` is accepted to avoid cascades.
    fn require_to_string(&mut self, ty: &Ty, span: &Span) -> Result<(), RavenError> {
        let resolved = self.infer.resolve(ty);
        let stripped = resolved.strip_self().clone();
        match &stripped {
            Ty::Str | Ty::Error => Ok(()),
            // The integer C FFI types have no `ToString` impl of their own;
            // they widen to `Int` at the use site (HIR lowering inserts the
            // cast) and render through the `Int` to-string path, so a C call
            // result such as `strlen(c"hi")` can be printed or interpolated.
            // The float C FFI types render through the `Float` path the same
            // way (`CFloat` widens f32 to f64 first).
            t if is_int_ffi(t) || is_float_ffi(t) => Ok(()),
            Ty::Var(v) => {
                self.infer
                    .add_bound(*v, "ToString".to_string(), span.clone());
                Ok(())
            }
            Ty::Param(p) => {
                let ok = self
                    .param_bounds
                    .get(p)
                    .map(|bs| bs.iter().any(|(name, _)| name == "ToString"))
                    .unwrap_or(false);
                if ok {
                    Ok(())
                } else {
                    Err(RavenError::ty(
                        TypeError::BoundNotSatisfied {
                            ty: p.name.clone(),
                            trait_name: "ToString".to_string(),
                        },
                        span.clone(),
                    )
                    .with_hint(format!(
                        "add a `ToString` bound to print a `{}` value: `{}: ToString`",
                        p.name, p.name
                    )))
                }
            }
            other if self.implements_trait(other, "ToString") => Ok(()),
            other => Err(RavenError::ty(
                TypeError::BoundNotSatisfied {
                    ty: format!("{}", other),
                    trait_name: "ToString".to_string(),
                },
                span.clone(),
            )
            .with_hint(format!(
                "values of type `{}` cannot be printed; implement `ToString` for it",
                other
            ))),
        }
    }

    /// After body checking, walk every recorded type and resolve any
    /// inference variables. Unsolved variables surface as
    /// `CannotInferType` errors. Also resolves locals so subsequent
    /// stages see concrete types.
    fn finalize_types(&mut self) -> Result<(), RavenError> {
        // First settle any deferred `Iterator<T>` element links: a call
        // such as `collect(pipeline)` leaves the element type `T` to be
        // inferred from the concrete argument bound to `S: Iterator<T>`.
        // Map each concrete source type to its iterator element by
        // structurally matching the `next` method's `Option<T>` return.
        let impls = self.env.impls.clone();
        let elem_of = move |ty: &Ty| -> Option<Ty> { iterator_elem_concrete(&impls, ty) };
        self.infer.solve_iterator_links(&elem_of)?;

        // Resolve only the entries this body recorded, replacing each value
        // with its resolved form and raising CannotInferType if a variable
        // remains. Resolving the whole shared map would try this body's
        // inference context against another body's variables.
        let keys: Vec<crate::resolve::UseKey> = self.recorded.clone();
        let infer = &self.infer;
        let mut first_err: Option<RavenError> = None;
        for k in keys {
            let cur = self.types.types.get(&k).cloned().unwrap_or(Ty::Error);
            let resolved = infer.resolve(&cur);
            if resolved.has_var() && first_err.is_none() {
                // Use the span recovered from the key for the diagnostic.
                let span = Span::new(k.file.clone(), k.start, k.end, 1, 1);
                match infer.finalize(&cur, &span) {
                    Ok(_) => {}
                    Err(e) => {
                        first_err = Some(e);
                    }
                }
            }
            self.types.types.insert(k, resolved);
        }
        // Judge deferred bounds on non-simple, fully concrete resolved types.
        // The eager check only judged simple types; a concrete instantiation
        // like `List<Int>` or `Pair<Int>` whose constructor has no impl of the
        // required trait at all would otherwise reach codegen as an unresolved
        // callee. A constructor that does have a (possibly bounded) impl is left
        // for the call site, so this never rejects otherwise valid code.
        for (v, bounds) in self.infer.all_bounds() {
            let resolved = self.infer.resolve(&Ty::Var(v));
            if !is_nonsimple_concrete(&resolved) {
                continue;
            }
            for b in &bounds {
                // `Iterator` is satisfied structurally (by having a `next`
                // method), not by an explicit impl, so the constructor-impl
                // check does not apply to it.
                if b.trait_name == "Iterator" {
                    continue;
                }
                if first_err.is_none()
                    && !super::wf::type_constructor_has_impl(self.env, &resolved, &b.trait_name)
                {
                    first_err = Some(
                        RavenError::ty(
                            TypeError::BoundNotSatisfied {
                                ty: format!("{}", resolved),
                                trait_name: b.trait_name.clone(),
                            },
                            b.span.clone(),
                        )
                        .with_hint(format!(
                            "`{ty}` does not implement `{tr}`; add `@derive({tr})` to its definition, or write an `impl {tr} for {ty}`",
                            ty = resolved,
                            tr = b.trait_name
                        )),
                    );
                }
            }
        }
        if let Some(e) = first_err {
            return Err(e);
        }
        Ok(())
    }

    /// Resolve inference variables and push any failure into the sink
    /// instead of returning it, so a body's type errors and its
    /// inference failures are reported together.
    fn finalize_into_errors(&mut self) {
        if let Err(e) = self.finalize_types() {
            self.push_error(e);
        }
    }

    /// Instantiate a function signature: substitute fresh inference
    /// variables for each declared generic parameter and record any
    /// bounds those variables carry.
    #[allow(dead_code)]
    fn instantiate_fn(
        &mut self,
        sig: &FnSig,
        span: &Span,
        explicit_args: &[Ty],
    ) -> Result<(Vec<Ty>, Ty), RavenError> {
        let subst = self.fresh_subst(&sig.generics, span, explicit_args, &sig.name)?;
        let params = sig
            .params
            .iter()
            .map(|t| substitute(t, &subst))
            .collect::<Vec<_>>();
        let ret = substitute(&sig.ret, &subst);
        Ok((params, ret))
    }

    /// Build a substitution that maps each declared generic param to a
    /// fresh inference variable, attaching bounds along the way. If
    /// `explicit_args` is non empty it must match the declared arity;
    /// each declared variable is unified with the corresponding
    /// explicit argument.
    fn fresh_subst(
        &mut self,
        generics: &[GenericParamSig],
        span: &Span,
        explicit_args: &[Ty],
        decl_name: &str,
    ) -> Result<HashMap<ParamId, Ty>, RavenError> {
        if !explicit_args.is_empty() && explicit_args.len() != generics.len() {
            return Err(RavenError::ty(
                TypeError::GenericArityMismatch {
                    decl: decl_name.to_string(),
                    expected: generics.len(),
                    actual: explicit_args.len(),
                },
                span.clone(),
            ));
        }
        // First create a fresh variable for every parameter so a bound
        // that mentions a sibling parameter (for example `S: Iterator<T>`)
        // can link to that sibling's variable regardless of order.
        let mut vars: Vec<InferVarId> = Vec::with_capacity(generics.len());
        let mut out: HashMap<ParamId, Ty> = HashMap::new();
        for p in generics.iter() {
            let v = self.infer.fresh(span.clone());
            vars.push(v);
            out.insert(p.id.clone(), Ty::Var(v));
        }
        for (i, p) in generics.iter().enumerate() {
            let v = vars[i];
            for (bi, b) in p.bounds.iter().enumerate() {
                self.infer.add_bound(v, b.clone(), span.clone());
                // For an `Iterator<T>` bound whose argument is a sibling
                // parameter, link this variable's element to that
                // sibling's variable so the element type can be inferred
                // from a concrete argument at the call site.
                if b == "Iterator" {
                    if let Some(Ty::Param(elem_id)) = p.bound_args.get(bi).and_then(|a| a.first()) {
                        if let Some(Ty::Var(elem_var)) = out.get(elem_id) {
                            self.infer.add_iterator_link(v, *elem_var, span.clone());
                        }
                    }
                }
            }
            if let Some(explicit) = explicit_args.get(i) {
                self.infer.unify(&Ty::Var(v), explicit, span)?;
            }
        }
        Ok(out)
    }

    fn check_block(&mut self, block: &Block) -> Result<Ty, RavenError> {
        // Each statement recovers at its own boundary: a failed statement
        // records its error and the block continues, so unrelated errors in
        // later statements are still reported in one compile.
        for stmt in &block.stmts {
            self.check_stmt(stmt);
        }
        let ty = match &block.trailing {
            Some(e) => self.check_expr_recover(e),
            None => Ty::Unit,
        };
        self.record(&block.span, ty.clone());
        Ok(ty)
    }

    /// Check one statement, recovering at the statement boundary. A failed
    /// sub-check records its error and the statement still introduces any
    /// binding it declares (bound to its annotated type, or `Ty::Error`), so
    /// later references do not cascade into spurious "undefined" errors.
    fn check_stmt(&mut self, stmt: &Stmt) {
        match &stmt.kind {
            StmtKind::Let {
                name: _,
                ty,
                init,
                mutable,
            } => {
                // A `const` local is immutable: record its binding key so a
                // later assignment to it is rejected.
                if !*mutable {
                    self.const_locals.insert(BindingKey::local(&stmt.span));
                }
                let declared = match ty {
                    Some(t) => match self.resolve_ast_ty(t) {
                        Ok(d) => {
                            // An explicit annotation such as `let m: Map<K, V>`
                            // must satisfy the generic bounds it names, the same
                            // check the declared-type pass applies to fields and
                            // signatures.
                            if let Err(e) = super::wf::check_type(self.env, &d, &stmt.span) {
                                self.push_error(e);
                            }
                            Some(d)
                        }
                        Err(e) => {
                            self.push_error(e);
                            Some(Ty::Error)
                        }
                    },
                    None => None,
                };
                // While checking the initializer, expose the declared
                // element type so an empty `[]` literal can adopt it.
                let prev_hint = self.array_hint.take();
                if let Some(Ty::List(elem)) = &declared {
                    self.array_hint = Some((**elem).clone());
                }
                let init_ty = init.as_ref().map(|e| self.check_expr_recover(e));
                self.array_hint = prev_hint;
                let final_ty = match (declared, init_ty) {
                    (Some(d), Some(i)) => {
                        self.unify_recover(&d, &i, &init.as_ref().unwrap().span);
                        d
                    }
                    (Some(d), None) => d,
                    (None, Some(i)) => i,
                    (None, None) => {
                        // Let with no type and no init: introduce a
                        // fresh inference variable to defer the type.
                        Ty::Var(self.infer.fresh(stmt.span.clone()))
                    }
                };
                self.locals
                    .insert(BindingKey::local(&stmt.span), final_ty.clone());
                self.record(&stmt.span, final_ty);
            }
            StmtKind::Return(e) => {
                let actual = match e {
                    Some(expr) => self.check_expr_recover(expr),
                    None => Ty::Unit,
                };
                let ret = self.return_ty.clone();
                self.unify_recover(&ret, &actual, &stmt.span);
            }
            StmtKind::Break(e) => {
                match self.loop_kinds.last() {
                    None => self.errors.push(RavenError::ty(
                        TypeError::Custom("`break` is only valid inside a loop".to_string()),
                        stmt.span.clone(),
                    )),
                    // A value carried by `break` is only meaningful in a `loop`,
                    // which yields it; a `while`/`for` produces no value, so the
                    // operand would be silently dropped by lowering.
                    Some(false) if e.is_some() => self.errors.push(RavenError::ty(
                        TypeError::Custom(
                            "`break` with a value is only valid inside a `loop`, not a `while` or `for`"
                                .to_string(),
                        ),
                        stmt.span.clone(),
                    )),
                    _ => {}
                }
                if let Some(expr) = e {
                    self.check_expr_recover(expr);
                }
            }
            StmtKind::Continue => {
                if self.loop_kinds.is_empty() {
                    self.errors.push(RavenError::ty(
                        TypeError::Custom("`continue` is only valid inside a loop".to_string()),
                        stmt.span.clone(),
                    ));
                }
            }
            StmtKind::Defer(e) => {
                self.check_expr_recover(e);
            }
            StmtKind::Spawn(e) => {
                // The goroutine body must be a `fun() -> Unit` closure.
                let ty = self.check_expr_recover(e);
                let expected = Ty::Function {
                    params: Vec::new(),
                    ret: Box::new(Ty::Unit),
                };
                self.unify_recover(&expected, &ty, &e.span);
            }
            StmtKind::Assign { target, op, value } => {
                // Reassigning a `const` local is rejected: the binding is
                // immutable. Only a direct `name = ...` is guarded here.
                if let ExprKind::Ident { name, .. } = &target.kind {
                    let is_const = matches!(
                        self.resolved.map.lookup(&target.span),
                        Some(crate::resolve::Binding::Local(decl))
                            if self.const_locals.contains(&BindingKey::local(decl))
                    );
                    if is_const {
                        self.push_error(RavenError::ty(
                            TypeError::Custom(format!(
                                "cannot assign to `{}` because it is a `const` binding",
                                name
                            )),
                            target.span.clone(),
                        ));
                    }
                }
                let target_ty = self.check_expr_recover(target);
                let value_ty = self.check_expr_recover(value);
                match op {
                    AssignOp::Assign => {
                        self.unify_recover(&target_ty, &value_ty, &value.span);
                    }
                    _ => {
                        // Compound: target op= value behaves like
                        // target = target op value. Reuse the binary
                        // op checker to pin the rule down.
                        let bin = compound_binary_op(*op);
                        if let Err(e) =
                            super::expr::check_binary(&target_ty, &value_ty, bin, &stmt.span)
                        {
                            self.push_error(e);
                        }
                    }
                }
            }
            StmtKind::Expr(e) => {
                self.check_expr_recover(e);
            }
        }
    }

    fn check_expr(&mut self, expr: &Expr) -> Result<Ty, RavenError> {
        let ty = self.check_expr_inner(expr)?;
        self.record(&expr.span, ty.clone());
        Ok(ty)
    }

    fn check_expr_inner(&mut self, expr: &Expr) -> Result<Ty, RavenError> {
        match &expr.kind {
            // A macro call only appears in formatter-parsed source; the
            // compile pipeline expands macros before type checking.
            ExprKind::MacroCall(_) => Ok(Ty::Error),
            ExprKind::Int(_) => Ok(Ty::Int),
            ExprKind::Float(_) => Ok(Ty::Float),
            ExprKind::Bool(_) => Ok(Ty::Bool),
            ExprKind::Str(_) | ExprKind::BlockStr(_) => Ok(Ty::Str),
            // A `c"..."` literal is a C string: a pointer to a static
            // null-terminated byte buffer. It types as the FFI `CStr`
            // type so it can be passed where a C function expects one.
            ExprKind::CStr(_) => Ok(Ty::Ffi(FfiTy::CStr)),
            ExprKind::InterpolatedString(fragments) => self.check_interpolated_string(fragments),
            ExprKind::Char(_) => Ok(Ty::Char),
            ExprKind::SelfLower => Ok(self
                .self_ty
                .clone()
                .map(|t| Ty::SelfTy(Box::new(t)))
                .unwrap_or(Ty::Error)),
            ExprKind::SelfUpper => Ok(self
                .self_ty
                .clone()
                .map(|t| Ty::SelfTy(Box::new(t)))
                .unwrap_or(Ty::Error)),
            ExprKind::Paren(inner) => self.check_expr(inner),
            ExprKind::Ident { name, generics } => self.check_ident(name, generics, &expr.span),
            ExprKind::Array(items) => self.check_array(items, &expr.span),
            ExprKind::SetLit(items) => self.check_set_lit(items, &expr.span),
            ExprKind::MapLit(pairs) => self.check_map_lit(pairs, &expr.span),
            ExprKind::Tuple(_) => Err(RavenError::ty(
                TypeError::Custom("tuple expressions are not yet supported".into()),
                expr.span.clone(),
            )),
            ExprKind::Block(b) => self.check_block(b),
            ExprKind::Unary { op, operand } => self.check_unary(*op, operand),
            ExprKind::Binary { op, lhs, rhs } => {
                let l = self.check_expr(lhs)?;
                let r = self.check_expr(rhs)?;
                // Resolve both operands through the inference table so
                // arithmetic on `?N` after a generic call sees the
                // solved type when one is available.
                let l = self.infer.resolve(&l);
                let r = self.infer.resolve(&r);
                check_binary(&l, &r, *op, &expr.span)
            }
            ExprKind::Range { start, end, .. } => {
                let s = self.check_expr(start)?;
                let e = self.check_expr(end)?;
                if !assignable(&Ty::Int, &s) || !assignable(&Ty::Int, &e) {
                    return Err(RavenError::ty(
                        TypeError::Custom("range bounds must be `Int`".into()),
                        expr.span.clone(),
                    ));
                }
                Ok(Ty::List(Box::new(Ty::Int)))
            }
            ExprKind::Call { callee, args } => self.check_call(callee, args, &expr.span),
            ExprKind::MethodCall {
                receiver,
                name,
                generics,
                args,
            } => self.check_method_call(receiver, name, generics, args, &expr.span),
            ExprKind::Field { receiver, name } => self.check_field(receiver, name, &expr.span),
            ExprKind::Index { receiver, index } => self.check_index(receiver, index, &expr.span),
            ExprKind::Try(inner) => {
                let inner_ty = self.check_expr(inner)?;
                let resolved = self.infer.resolve(&inner_ty);
                match resolved {
                    Ty::Result(t, e) => {
                        // `?` propagates `Err(e)` to the caller, so the function
                        // must return a Result whose error type accepts `e`.
                        match self.infer.resolve(&self.return_ty) {
                            Ty::Result(_, ret_e) => {
                                self.unify(&ret_e, &e, &expr.span)?;
                            }
                            Ty::Error => {}
                            other => {
                                return Err(RavenError::ty(
                                    TypeError::Custom(format!(
                                        "`?` on a Result propagates an error, but the \
                                         enclosing function returns `{}`, not a Result",
                                        other
                                    )),
                                    expr.span.clone(),
                                ));
                            }
                        }
                        Ok(*t)
                    }
                    Ty::Option(t) => {
                        // `?` on an Option propagates `None`, so the function
                        // must return an Option.
                        match self.infer.resolve(&self.return_ty) {
                            Ty::Option(_) | Ty::Error => {}
                            other => {
                                return Err(RavenError::ty(
                                    TypeError::Custom(format!(
                                        "`?` on an Option propagates `None`, but the \
                                         enclosing function returns `{}`, not an Option",
                                        other
                                    )),
                                    expr.span.clone(),
                                ));
                            }
                        }
                        Ok(*t)
                    }
                    Ty::Error => Ok(Ty::Error),
                    other => Err(RavenError::ty(
                        TypeError::Custom(format!(
                            "`?` operator requires Result or Option, got `{}`",
                            other
                        )),
                        expr.span.clone(),
                    )),
                }
            }
            ExprKind::If {
                cond,
                then_branch,
                else_branch,
            } => self.check_if(cond, then_branch, else_branch.as_deref(), &expr.span),
            ExprKind::Match { scrutinee, arms } => self.check_match(scrutinee, arms, &expr.span),
            ExprKind::Loop(b) => {
                self.loop_kinds.push(true);
                let r = self.check_block(b);
                self.loop_kinds.pop();
                r?;
                Ok(Ty::Unit)
            }
            ExprKind::While { cond, body } => {
                let c = self.check_expr(cond)?;
                self.unify(&Ty::Bool, &c, &cond.span)?;
                self.loop_kinds.push(false);
                let r = self.check_block(body);
                self.loop_kinds.pop();
                r?;
                Ok(Ty::Unit)
            }
            ExprKind::For {
                pattern: pat,
                iter,
                body,
            } => {
                let iter_ty = self.check_expr(iter)?;
                let resolved = self.infer.resolve(&iter_ty);
                let elem = match resolved.strip_self() {
                    Ty::List(t) => *t.clone(),
                    Ty::Error => Ty::Error,
                    // A generic parameter `S` bounded by `Iterator<T>`
                    // iterates at its bound's element type `T`. The loop is
                    // monomorphized once `S` is known at each call site.
                    Ty::Param(p) => match self.param_iterator_elem_ty(p) {
                        Some(t) => t,
                        None => {
                            return Err(RavenError::ty(
                                TypeError::Custom(format!(
                                    "cannot iterate over `{}`; add an `Iterator<T>` bound to it",
                                    p.name
                                )),
                                iter.span.clone(),
                            ));
                        }
                    },
                    other => {
                        // Any value whose type implements `Iterator<T>`
                        // (resolved by finding a `next` method returning
                        // `Option<T>`) can drive a `for` loop. The HIR
                        // desugars this to a `loop` that calls `next`.
                        match self.iterator_elem_ty(other, &iter.span) {
                            Some(t) => t,
                            None => {
                                return Err(RavenError::ty(
                                    TypeError::Custom(format!(
                                        "cannot iterate over `{}`; expected a `List<T>` or a type implementing `Iterator<T>`",
                                        other
                                    )),
                                    iter.span.clone(),
                                ));
                            }
                        }
                    }
                };
                let elem = self.infer.resolve(&elem);
                // Record the resolved element type at the pattern span so
                // HIR lowering can type the loop binding without redoing
                // method resolution (used by the iterator-driven path).
                self.record(&pat.span, elem.clone());
                pattern::bind(pat, &elem, self.env, &mut self.locals)?;
                self.loop_kinds.push(false);
                let r = self.check_block(body);
                self.loop_kinds.pop();
                r?;
                Ok(Ty::Unit)
            }
            ExprKind::Lambda {
                params,
                ret,
                body,
                params_inferred,
            } => self.check_lambda(params, ret.as_ref(), body, *params_inferred, &expr.span),
            ExprKind::StructLit {
                name,
                fields,
                generics: _,
            } => self.check_struct_lit(name, fields, &expr.span),
        }
    }

    fn check_ident(
        &mut self,
        name: &str,
        generics: &[crate::ast::Type],
        span: &Span,
    ) -> Result<Ty, RavenError> {
        // A bare `None` carries an unknown element type. Use a fresh
        // inference variable so it unifies with whatever concrete
        // `Option<T>` the surrounding context fixes (for example the other
        // arm of a `match`). Typing it as `Option<Error>` would swallow the
        // unification and leave the element type unresolved. `Some`, `Ok`,
        // and `Err` reach the checker as calls, so `None` is the only bare
        // constructor identifier handled here. This is checked before the
        // binding lookup because `None` is a built-in constructor with no
        // user declaration to bind to.
        if name == "None" {
            let v = self.infer.fresh(span.clone());
            return Ok(Ty::Option(Box::new(Ty::Var(v))));
        }
        if let Some(binding) = self.resolved.map.lookup(span).cloned() {
            // Resolve any explicit type arguments first. We pass them
            // through to the instantiation step below.
            let mut explicit_args = Vec::with_capacity(generics.len());
            for g in generics {
                explicit_args.push(self.resolve_ast_ty(g)?);
            }
            // Record the resolved explicit type arguments at the use site so
            // MIR can bind a callee's generic parameters that the value
            // arguments do not pin down (a `f<T>()` with no argument carrying
            // `T`). Resolved here under the enclosing generic scope, so a
            // `T` argument stays a `Ty::Param` and grounds per monomorphization.
            if !explicit_args.is_empty() {
                self.record_type_args(span, explicit_args.clone());
            }
            self.type_of_binding(&binding, span, &explicit_args)
        } else {
            Err(RavenError::ty(
                TypeError::Custom(format!("identifier `{}` has no type binding", name)),
                span.clone(),
            ))
        }
    }

    fn type_of_binding(
        &mut self,
        binding: &Binding,
        span: &Span,
        explicit_args: &[Ty],
    ) -> Result<Ty, RavenError> {
        match binding {
            Binding::Function(id) => {
                let sig = self
                    .env
                    .functions
                    .get(id)
                    .cloned()
                    .ok_or_else(|| ty_custom("function signature missing", span))?;
                let (params, ret) = if sig.generics.is_empty() {
                    if !explicit_args.is_empty() {
                        return Err(RavenError::ty(
                            TypeError::GenericArityMismatch {
                                decl: sig.name.clone(),
                                expected: 0,
                                actual: explicit_args.len(),
                            },
                            span.clone(),
                        ));
                    }
                    (sig.params.clone(), sig.ret.clone())
                } else {
                    self.instantiate_fn(&sig, span, explicit_args)?
                };
                Ok(Ty::Function {
                    params,
                    ret: Box::new(ret),
                })
            }
            Binding::Extern {
                decl_id,
                item_index,
            } => {
                let sig = self
                    .env
                    .externs
                    .get(&(*decl_id, *item_index))
                    .ok_or_else(|| ty_custom("extern signature missing", span))?;
                Ok(Ty::Function {
                    params: sig.params.clone(),
                    ret: Box::new(sig.ret.clone()),
                })
            }
            Binding::Const(id) => Ok(self.env.consts.get(id).cloned().unwrap_or(Ty::Error)),
            Binding::Static(id) => Ok(self.env.statics.get(id).cloned().unwrap_or(Ty::Error)),
            Binding::Param(sp) => Ok(self
                .locals
                .get(&BindingKey::param(sp))
                .cloned()
                .unwrap_or(Ty::Error)),
            Binding::Local(sp) => Ok(self
                .locals
                .get(&BindingKey::local(sp))
                .cloned()
                .unwrap_or(Ty::Error)),
            Binding::PatternBinding(sp) => Ok(self
                .locals
                .get(&BindingKey::pattern(sp))
                .cloned()
                .unwrap_or(Ty::Error)),
            Binding::SelfValue => Ok(self
                .self_ty
                .clone()
                .map(|t| Ty::SelfTy(Box::new(t)))
                .unwrap_or(Ty::Error)),
            Binding::SelfType => Ok(self
                .self_ty
                .clone()
                .map(|t| Ty::SelfTy(Box::new(t)))
                .unwrap_or(Ty::Error)),
            Binding::Enum(id) => {
                let e = self
                    .env
                    .enums
                    .get(id)
                    .cloned()
                    .ok_or_else(|| ty_custom("enum signature missing", span))?;
                let args = self.instantiate_type_args(&e.generics, explicit_args, span, &e.name)?;
                Ok(Ty::Enum {
                    id: *id,
                    name: e.name.clone(),
                    args,
                })
            }
            Binding::Struct(id) => {
                let s = self
                    .env
                    .structs
                    .get(id)
                    .cloned()
                    .ok_or_else(|| ty_custom("struct signature missing", span))?;
                let args = self.instantiate_type_args(&s.generics, explicit_args, span, &s.name)?;
                Ok(Ty::Struct {
                    id: *id,
                    name: s.name.clone(),
                    args,
                })
            }
            Binding::Trait(_) => Err(ty_custom(
                "trait values are not first class without `dyn` (deferred to issue #66)",
                span,
            )),
            Binding::GenericParam { owner, name } => {
                // The use refers to a declared generic parameter. The
                // lexical scope built by `check_function` already keyed
                // ParamId via `resolve_ast_ty`. Reconstruct the
                // parameter id from the resolver binding here so a use
                // outside of `resolve_ast_ty` still works.
                if let Some(p) = self.generic_scope.lookup(name) {
                    return Ok(Ty::Param(p.clone()));
                }
                // Fallback: build a ParamId from the binding.
                Ok(Ty::Param(ParamId::new(owner, 0, name.clone())))
            }
            Binding::Variant { .. } | Binding::ImportAlias(_) | Binding::ImportedItem { .. } => {
                // These are not yet given a usable type by the type
                // checker; surface a clear error so users see what is
                // missing rather than a panic.
                Err(ty_custom(
                    "name does not yet have a type in this release",
                    span,
                ))
            }
        }
    }

    /// Allocate fresh inference variables for each declared generic
    /// parameter, optionally unifying them with explicit type
    /// arguments supplied at the use site.
    fn instantiate_type_args(
        &mut self,
        generics: &[GenericParamSig],
        explicit: &[Ty],
        span: &Span,
        decl_name: &str,
    ) -> Result<Vec<Ty>, RavenError> {
        if generics.is_empty() {
            if !explicit.is_empty() {
                return Err(RavenError::ty(
                    TypeError::GenericArityMismatch {
                        decl: decl_name.to_string(),
                        expected: 0,
                        actual: explicit.len(),
                    },
                    span.clone(),
                ));
            }
            return Ok(Vec::new());
        }
        if !explicit.is_empty() && explicit.len() != generics.len() {
            return Err(RavenError::ty(
                TypeError::GenericArityMismatch {
                    decl: decl_name.to_string(),
                    expected: generics.len(),
                    actual: explicit.len(),
                },
                span.clone(),
            ));
        }
        let mut out = Vec::with_capacity(generics.len());
        for (i, p) in generics.iter().enumerate() {
            let v = self.infer.fresh(span.clone());
            for b in &p.bounds {
                self.infer.add_bound(v, b.clone(), span.clone());
            }
            let assigned = Ty::Var(v);
            if let Some(e) = explicit.get(i) {
                self.infer.unify(&assigned, e, span)?;
            }
            out.push(assigned);
        }
        Ok(out)
    }

    fn check_array(&mut self, items: &[Expr], span: &Span) -> Result<Ty, RavenError> {
        if items.is_empty() {
            // An empty `[]` has no element to infer from. Adopt the
            // element type hint from the enclosing `let` binding's
            // declared `List<T>` type when one is present.
            if let Some(elem) = self.array_hint.clone() {
                return Ok(Ty::List(Box::new(elem)));
            }
            return Err(RavenError::ty(
                TypeError::Custom(
                    "empty array literals require a context type; annotate the binding".into(),
                ),
                span.clone(),
            ));
        }
        let first = self.check_expr(&items[0])?;
        for it in &items[1..] {
            let t = self.check_expr(it)?;
            self.unify(&first, &t, &it.span)?;
        }
        Ok(Ty::List(Box::new(first)))
    }

    /// Find a struct declaration by its source name and build a fresh
    /// instantiation `Ty::Struct` whose type arguments are inference
    /// variables carrying the declaration's trait bounds. Used by the set
    /// and map literals, which name the bundled `Set` and `Map` types. A
    /// missing declaration means the collections module is not in scope.
    fn instantiate_named_struct(
        &mut self,
        name: &str,
        span: &Span,
    ) -> Result<(crate::resolve::DeclId, Vec<Ty>), RavenError> {
        let found = self
            .env
            .structs
            .iter()
            .find(|(_, s)| s.name == name)
            .map(|(id, s)| (*id, s.generics.clone()));
        let Some((id, generics)) = found else {
            return Err(RavenError::ty(
                TypeError::Custom(format!(
                    "`{}` literal requires the collections module; add `import std/collections`",
                    name
                )),
                span.clone(),
            ));
        };
        let args = self.instantiate_type_args(&generics, &[], span, name)?;
        Ok((id, args))
    }

    /// Check a set literal `{e1, e2, ...}`. The result is `Set<T>` where
    /// `T` unifies with every element type. The `Eq` bound on `Set`'s type
    /// parameter rides along on the fresh argument variable.
    fn check_set_lit(&mut self, items: &[Expr], span: &Span) -> Result<Ty, RavenError> {
        let (id, args) = self.instantiate_named_struct("Set", span)?;
        let elem = args.first().cloned().unwrap_or(Ty::Error);
        for it in items {
            let t = self.check_expr(it)?;
            self.unify(&elem, &t, &it.span)?;
        }
        Ok(Ty::Struct {
            id,
            name: "Set".to_string(),
            args,
        })
    }

    /// Check a map literal `[k1: v1, ...]` (and the empty `[:]`). The
    /// result is `Map<K, V>` where `K` unifies with every key type and `V`
    /// with every value type. The `Eq` bound on `Map`'s key parameter
    /// rides along on the fresh key argument variable.
    fn check_map_lit(&mut self, pairs: &[(Expr, Expr)], span: &Span) -> Result<Ty, RavenError> {
        let (id, args) = self.instantiate_named_struct("Map", span)?;
        let key_ty = args.first().cloned().unwrap_or(Ty::Error);
        let val_ty = args.get(1).cloned().unwrap_or(Ty::Error);
        for (k, v) in pairs {
            let kt = self.check_expr(k)?;
            self.unify(&key_ty, &kt, &k.span)?;
            let vt = self.check_expr(v)?;
            self.unify(&val_ty, &vt, &v.span)?;
        }
        Ok(Ty::Struct {
            id,
            name: "Map".to_string(),
            args,
        })
    }

    fn check_unary(&mut self, op: UnaryOp, operand: &Expr) -> Result<Ty, RavenError> {
        let t = self.check_expr(operand)?;
        match op {
            UnaryOp::Neg => match t.strip_self() {
                Ty::Int => Ok(Ty::Int),
                Ty::Float => Ok(Ty::Float),
                Ty::Error => Ok(Ty::Error),
                other => Err(RavenError::ty(
                    TypeError::TypeMismatch {
                        expected: "Int or Float".into(),
                        actual: format!("{}", other),
                    },
                    operand.span.clone(),
                )),
            },
            UnaryOp::Not => {
                self.unify(&Ty::Bool, &t, &operand.span)?;
                Ok(Ty::Bool)
            }
            // Raven has no reference or pointer type, so there is nothing for an
            // address-of to produce. Accepting it silently (returning the operand
            // unchanged) made `&x` a misleading no-op, so reject it instead.
            UnaryOp::Ref => Err(RavenError::ty(
                TypeError::Custom(
                    "unary `&` (address-of) is not supported: Raven has no reference \
                     or pointer type for it to produce"
                        .into(),
                ),
                operand.span.clone(),
            )),
        }
    }

    fn check_call(&mut self, callee: &Expr, args: &[Expr], span: &Span) -> Result<Ty, RavenError> {
        // Special case: enum variant construction via a bare ident
        // resolved to an `Enum` binding plus a chained `.variant`
        // method shape would need path parsing. For our scope, the
        // only call form we recognize for variants is `Some(x)`,
        // `Ok(x)`, `Err(x)` which arrive as `Call { callee: Ident, .. }`
        // without a resolver binding.
        if let ExprKind::Ident { name, generics } = &callee.kind {
            if self.resolved.map.lookup(&callee.span).is_none() {
                return self.check_builtin_constructor_call(
                    name,
                    generics,
                    &callee.span,
                    args,
                    span,
                );
            }
        }

        let callee_ty = self.check_expr(callee)?;
        let (params, ret) = match callee_ty.strip_self() {
            Ty::Function { params, ret } => (params.clone(), *ret.clone()),
            Ty::Error => (vec![], Ty::Error),
            other => {
                return Err(RavenError::ty(
                    TypeError::NotCallable(format!("{}", other)),
                    callee.span.clone(),
                ));
            }
        };
        // A variadic `extern` C function accepts extra C-FFI arguments after
        // its fixed parameters; look the flag up from the callee's signature.
        let extern_variadic = match self.resolved.map.lookup(&callee.span) {
            Some(Binding::Extern {
                decl_id,
                item_index,
            }) => self
                .env
                .externs
                .get(&(*decl_id, *item_index))
                .is_some_and(|s| s.variadic),
            _ => false,
        };
        let arity_ok = if extern_variadic {
            args.len() >= params.len()
        } else {
            args.len() == params.len()
        };
        if !arity_ok {
            return Err(RavenError::ty(
                TypeError::WrongArity {
                    func: describe_callee(callee),
                    expected: params.len(),
                    actual: args.len(),
                },
                span.clone(),
            ));
        }
        // The arguments beyond the fixed parameters of a variadic C function
        // must each be a C-FFI integer or pointer type. Floats are rejected:
        // the back end cannot set the System V `al` register or apply the
        // Windows x64 float-shadow rule, so a float vararg would miscompile.
        for arg in args.iter().skip(params.len()) {
            let a = self.check_expr(arg)?;
            let ra = self.infer.resolve(&a);
            if !is_variadic_ffi_arg(ra.strip_self()) {
                return Err(ty_custom(
                    &format!(
                        "a variadic C argument must be a C-FFI integer or pointer type (CInt, CLong, CSize, CStr, CPtr<T>) or a native Int; got `{ra}`. Float varargs are not supported (wrap the call in a C shim)"
                    ),
                    &arg.span,
                ));
            }
        }
        // Whether this call targets a foreign C function; only then is a
        // struct argument marshaled by value and required to be `@repr(C)`.
        let callee_is_extern = matches!(
            self.resolved.map.lookup(&callee.span),
            Some(Binding::Extern { .. })
        );
        for (param_ty, arg) in params.iter().zip(args.iter()) {
            let a = self.check_expr(arg)?;
            // An integer C FFI parameter (`CInt`, `CLong`, `CSize`)
            // accepts a native `Int`, so an integer literal or expression
            // can be passed to a C function (for example `abs(-7)`). The
            // back end converts the i64 to the parameter's machine width
            // at the call. A `c"..."` literal is already typed `CStr`, so
            // it unifies directly with a `CStr` parameter. Any other
            // mismatch falls through to the normal unify diagnostic.
            let resolved_param = self.infer.resolve(param_ty);
            let resolved_arg = self.infer.resolve(&a);
            if is_int_ffi(resolved_param.strip_self())
                && matches!(resolved_arg.strip_self(), Ty::Int)
            {
                continue;
            }
            // A `CDouble` parameter is C `double` (f64), the same
            // representation a Raven `Float` uses, so a `Float` argument
            // passes directly with no conversion at the call. A `CFloat`
            // parameter is C `float` (f32); a `Float` argument is accepted
            // and the back end narrows it to f32 at the call boundary.
            if matches!(
                resolved_param.strip_self(),
                Ty::Ffi(FfiTy::CDouble) | Ty::Ffi(FfiTy::CFloat)
            ) && matches!(resolved_arg.strip_self(), Ty::Float)
            {
                continue;
            }
            // A `CFnPtr` parameter accepts a non-capturing top-level Raven
            // function whose parameters and return are all C-FFI types. The
            // function is passed as its C-ABI address; the signature match
            // is the programmer's responsibility, like C. A captured local
            // (a closure value) or a function with a non-FFI signature is
            // rejected. See `docs/v2/specs/std-ffi.md`.
            if matches!(resolved_param.strip_self(), Ty::Ffi(FfiTy::CFnPtr)) {
                self.check_callback_arg(arg, resolved_arg.strip_self())?;
                continue;
            }
            // A closure value passed where a `CPtr` is expected is the
            // `userdata` pointer for a trampoline callback: the closure object
            // pointer is handed to C, which threads it back to the trampoline.
            if matches!(resolved_param.strip_self(), Ty::Ffi(FfiTy::CPtr(_)))
                && matches!(resolved_arg.strip_self(), Ty::Function { .. })
            {
                self.types.record_closure_userdata(&arg.span);
                continue;
            }
            // A struct argument may cross a C call only when it is a
            // `@repr(C)` struct: the back end marshals its fields by value
            // per the platform ABI. A plain heap struct (a GC pointer) is
            // rejected so it is never handed to C as if it had C layout.
            if let Ty::Struct { id, name, .. } = resolved_param.strip_self() {
                let is_repr_c = self.env.structs.get(id).is_some_and(|s| s.repr_c);
                if callee_is_extern && !is_repr_c {
                    return Err(ty_custom(
                        &format!(
                            "struct `{name}` is passed to a C function but is not `@repr(C)`; mark it `@repr(C)` to cross the FFI by value"
                        ),
                        &arg.span,
                    ));
                }
            }
            self.unify(param_ty, &a, &arg.span)?;
        }
        Ok(ret)
    }

    /// Validate an argument passed where a `CFnPtr` is expected. The
    /// argument must be a bare name of a non-capturing top-level function
    /// (a resolver `Function` binding) whose parameters and return are all
    /// C-FFI scalar or pointer types, so the C ABI of the resulting
    /// function pointer is well defined. A closure value (a captured local
    /// of function type) or a function with a non-FFI signature is
    /// rejected. Capturing closures as callbacks are a follow-up.
    fn check_callback_arg(&mut self, arg: &Expr, arg_ty: &Ty) -> Result<(), RavenError> {
        let (params, ret) = match arg_ty {
            Ty::Function { params, ret } => (params, ret.as_ref()),
            other => {
                return Err(ty_custom(
                    &format!(
                        "a `CFnPtr` argument must be a top-level function, got `{}`",
                        other
                    ),
                    &arg.span,
                ));
            }
        };
        // Every callback parameter and the return must be a C-FFI type, so
        // the resulting function pointer has a well-defined C ABI.
        for p in params {
            let rp = self.infer.resolve(p);
            if !is_ffi_abi_ty(rp.strip_self()) {
                return Err(ty_custom(
                    &format!(
                        "a `CFnPtr` callback parameter must be a C-FFI type (CInt, CLong, CSize, CFloat, CDouble, CStr, CPtr<T>), got `{}`",
                        rp
                    ),
                    &arg.span,
                ));
            }
        }
        let rr = self.infer.resolve(ret);
        if !matches!(rr.strip_self(), Ty::Unit) && !is_ffi_abi_ty(rr.strip_self()) {
            return Err(ty_custom(
                &format!(
                    "a `CFnPtr` callback return must be a C-FFI type or Unit, got `{}`",
                    rr
                ),
                &arg.span,
            ));
        }
        // A bare name of a top-level function lowers to its raw C address,
        // callable by C directly. Any other function-typed value (a lambda or
        // a captured closure) lowers to a generated trampoline whose last
        // argument is a `userdata` pointer C threads back to it; the closure
        // itself must be passed to the C function's `userdata` slot (a
        // `CPtr<Unit>` parameter). See `docs/v2/specs/std-ffi.md`.
        let is_top_level_fn = matches!(&arg.kind, ExprKind::Ident { .. })
            && matches!(
                self.resolved.map.lookup(&arg.span),
                Some(Binding::Function(_))
            );
        if is_top_level_fn {
            self.types.record_callback_fn(&arg.span);
        } else {
            self.types.record_closure_callback(&arg.span);
        }
        Ok(())
    }

    fn check_builtin_constructor_call(
        &mut self,
        name: &str,
        generics: &[crate::ast::Type],
        callee_span: &Span,
        args: &[Expr],
        span: &Span,
    ) -> Result<Ty, RavenError> {
        match name {
            "type_name"
            | "field_names"
            | "field_types"
            | "variant_names"
            | "variant_field_types" => {
                self.check_reflection_builtin(name, generics, callee_span, args, span)
            }
            "__ptr_alloc" | "__ptr_free" | "__ptr_load" | "__ptr_store" | "__ptr_offset"
            | "__ptr_is_null" | "__ptr_null" => {
                self.check_ptr_builtin(name, generics, callee_span, args, span)
            }
            "to_any" | "cast" | "type_name_of" | "field_names_of" | "get_field" | "set_field" => {
                self.check_runtime_reflection_builtin(name, generics, callee_span, args, span)
            }
            "Some" => {
                if args.len() != 1 {
                    return Err(RavenError::ty(
                        TypeError::WrongArity {
                            func: "Some".into(),
                            expected: 1,
                            actual: args.len(),
                        },
                        span.clone(),
                    ));
                }
                let inner = self.check_expr(&args[0])?;
                Ok(Ty::Option(Box::new(inner)))
            }
            "Ok" => {
                if args.len() != 1 {
                    return Err(RavenError::ty(
                        TypeError::WrongArity {
                            func: "Ok".into(),
                            expected: 1,
                            actual: args.len(),
                        },
                        span.clone(),
                    ));
                }
                let t = self.check_expr(&args[0])?;
                Ok(Ty::Result(Box::new(t), Box::new(Ty::Error)))
            }
            "Err" => {
                if args.len() != 1 {
                    return Err(RavenError::ty(
                        TypeError::WrongArity {
                            func: "Err".into(),
                            expected: 1,
                            actual: args.len(),
                        },
                        span.clone(),
                    ));
                }
                let e = self.check_expr(&args[0])?;
                Ok(Ty::Result(Box::new(Ty::Error), Box::new(e)))
            }
            "print" => {
                // The built-in `print` accepts any value whose type
                // implements `ToString`. A `String` is written directly
                // (the allocation-free literal fast path stays available);
                // any other `ToString` value is rendered through its
                // `to_string` method, inserted during HIR lowering. The
                // codegen back end recognizes the mangled `print` name and
                // emits the runtime's `raven_println_str` ABI call on the
                // resulting String.
                if args.len() != 1 {
                    return Err(RavenError::ty(
                        TypeError::WrongArity {
                            func: "print".into(),
                            expected: 1,
                            actual: args.len(),
                        },
                        span.clone(),
                    ));
                }
                let arg_ty = self.check_expr(&args[0])?;
                self.require_to_string(&arg_ty, &args[0].span)?;
                Ok(Ty::Unit)
            }
            // Internal stdlib I/O intrinsics. The bundled `std/io` source
            // calls these to reach the runtime's byte-level output and
            // input symbols; the codegen back end recognizes the mangled
            // names and emits the matching runtime calls. They are not a
            // user-facing surface (the leading `__` marks them internal).
            "__io_print_str" | "__io_println_str" => {
                if args.len() != 1 {
                    return Err(RavenError::ty(
                        TypeError::WrongArity {
                            func: name.to_string(),
                            expected: 1,
                            actual: args.len(),
                        },
                        span.clone(),
                    ));
                }
                let arg_ty = self.check_expr(&args[0])?;
                self.unify(&Ty::Str, &arg_ty, &args[0].span)?;
                Ok(Ty::Unit)
            }
            "__panic" => {
                // Internal `__panic(msg: String)` intrinsic. The bundled
                // `std/test` source calls it to abort on a failed assertion;
                // the back end lowers it to the runtime `raven_panic`. It
                // never returns at runtime, but is typed as `Unit` so a call
                // is a valid statement.
                self.check_intrinsic_arity(name, args, 1, span)?;
                let arg_ty = self.check_expr(&args[0])?;
                self.unify(&Ty::Str, &arg_ty, &args[0].span)?;
                Ok(Ty::Unit)
            }
            "__io_read_line" => {
                if !args.is_empty() {
                    return Err(RavenError::ty(
                        TypeError::WrongArity {
                            func: name.to_string(),
                            expected: 0,
                            actual: args.len(),
                        },
                        span.clone(),
                    ));
                }
                Ok(Ty::Str)
            }
            // Internal stdlib string intrinsics. The bundled `std/string`
            // source calls these byte-level primitives; the codegen back
            // end recognizes the mangled names and emits the matching
            // runtime calls. The leading `__str_` marks them internal.
            "__str_len" => {
                self.check_intrinsic_arity(name, args, 1, span)?;
                let s = self.check_expr(&args[0])?;
                self.unify(&Ty::Str, &s, &args[0].span)?;
                Ok(Ty::Int)
            }
            "__str_byte_at" => {
                self.check_intrinsic_arity(name, args, 2, span)?;
                let s = self.check_expr(&args[0])?;
                self.unify(&Ty::Str, &s, &args[0].span)?;
                let i = self.check_expr(&args[1])?;
                self.unify(&Ty::Int, &i, &args[1].span)?;
                Ok(Ty::Int)
            }
            "__str_substring" => {
                self.check_intrinsic_arity(name, args, 3, span)?;
                let s = self.check_expr(&args[0])?;
                self.unify(&Ty::Str, &s, &args[0].span)?;
                let start = self.check_expr(&args[1])?;
                self.unify(&Ty::Int, &start, &args[1].span)?;
                let end = self.check_expr(&args[2])?;
                self.unify(&Ty::Int, &end, &args[2].span)?;
                Ok(Ty::Str)
            }
            "__str_from_byte" => {
                self.check_intrinsic_arity(name, args, 1, span)?;
                let b = self.check_expr(&args[0])?;
                self.unify(&Ty::Int, &b, &args[0].span)?;
                Ok(Ty::Str)
            }
            "__str_concat" => {
                self.check_intrinsic_arity(name, args, 2, span)?;
                let a = self.check_expr(&args[0])?;
                self.unify(&Ty::Str, &a, &args[0].span)?;
                let b = self.check_expr(&args[1])?;
                self.unify(&Ty::Str, &b, &args[1].span)?;
                Ok(Ty::Str)
            }
            other => Err(RavenError::ty(
                TypeError::Custom(format!("identifier `{}` has no type binding", other)),
                span.clone(),
            )),
        }
    }

    /// Check a compile-time reflection builtin (`type_name<T>()`,
    /// `field_names<T>()`, `field_types<T>()`, `variant_names<T>()`, or
    /// `variant_field_types<T>()`). Each takes exactly one
    /// type argument and no value
    /// arguments. The resolved type argument is recorded under the callee
    /// span so HIR lowering can carry it (a generic parameter `T` resolves
    /// to `Ty::Param`, grounded per monomorphization in MIR). See
    /// `docs/v2/specs/reflection.md`.
    fn check_reflection_builtin(
        &mut self,
        name: &str,
        generics: &[crate::ast::Type],
        callee_span: &Span,
        args: &[Expr],
        span: &Span,
    ) -> Result<Ty, RavenError> {
        if !args.is_empty() {
            return Err(RavenError::ty(
                TypeError::WrongArity {
                    func: name.to_string(),
                    expected: 0,
                    actual: args.len(),
                },
                span.clone(),
            ));
        }
        if generics.len() != 1 {
            return Err(RavenError::ty(
                TypeError::GenericArityMismatch {
                    decl: name.to_string(),
                    expected: 1,
                    actual: generics.len(),
                },
                span.clone(),
            ));
        }
        let arg_ty = self.resolve_ast_ty(&generics[0])?;
        if (name == "field_names" || name == "field_types")
            && !matches!(arg_ty.strip_self(), Ty::Struct { .. } | Ty::Param(_))
        {
            return Err(RavenError::ty(
                TypeError::Custom(format!(
                    "`{}` requires a struct type, got `{}`",
                    name, arg_ty
                )),
                span.clone(),
            ));
        }
        if (name == "variant_names" || name == "variant_field_types")
            && !matches!(arg_ty.strip_self(), Ty::Enum { .. } | Ty::Param(_))
        {
            return Err(RavenError::ty(
                TypeError::Custom(format!(
                    "`{}` requires an enum type, got `{}`",
                    name, arg_ty
                )),
                span.clone(),
            ));
        }
        // Record the resolved type argument at the callee span. HIR lowering
        // reads it to build the reflection node; the call's own result span
        // keeps the result type (`String`, `List<String>`, or
        // `List<List<String>>`).
        self.record(callee_span, arg_ty);
        Ok(match name {
            "field_names" | "field_types" | "variant_names" => Ty::List(Box::new(Ty::Str)),
            "variant_field_types" => Ty::List(Box::new(Ty::List(Box::new(Ty::Str)))),
            _ => Ty::Str,
        })
    }

    /// Check a runtime reflection builtin (`to_any`, `cast`,
    /// `type_name_of`, `field_names_of`, `get_field`). `to_any<T>(v)` boxes
    /// a `T` into `Any`; `cast<T>(a)` is a checked downcast to `Option<T>`.
    /// Both carry the type argument `T` at the callee span so MIR grounds it
    /// per monomorphization (the box site knows the static tag, the cast site
    /// knows the tag to compare). The other three take an `Any` value and no
    /// type argument. See `docs/v2/specs/runtime-reflection.md`.
    fn check_runtime_reflection_builtin(
        &mut self,
        name: &str,
        generics: &[crate::ast::Type],
        callee_span: &Span,
        args: &[Expr],
        span: &Span,
    ) -> Result<Ty, RavenError> {
        let wrong_arity = |actual: usize, expected: usize| {
            RavenError::ty(
                TypeError::WrongArity {
                    func: name.to_string(),
                    expected,
                    actual,
                },
                span.clone(),
            )
        };
        let wrong_generics = |actual: usize, expected: usize| {
            RavenError::ty(
                TypeError::GenericArityMismatch {
                    decl: name.to_string(),
                    expected,
                    actual,
                },
                span.clone(),
            )
        };
        match name {
            "to_any" => {
                if generics.len() != 1 {
                    return Err(wrong_generics(generics.len(), 1));
                }
                if args.len() != 1 {
                    return Err(wrong_arity(args.len(), 1));
                }
                let declared = self.resolve_ast_ty(&generics[0])?;
                let actual = self.check_expr(&args[0])?;
                self.unify(&declared, &actual, &args[0].span)?;
                // Record the boxed type at the callee span so MIR grounds it
                // per monomorphization to pick the runtime type tag.
                self.record(callee_span, declared);
                Ok(Ty::Any)
            }
            "cast" => {
                if generics.len() != 1 {
                    return Err(wrong_generics(generics.len(), 1));
                }
                if args.len() != 1 {
                    return Err(wrong_arity(args.len(), 1));
                }
                let target = self.resolve_ast_ty(&generics[0])?;
                let a = self.check_expr(&args[0])?;
                self.unify(&Ty::Any, &a, &args[0].span)?;
                self.record(callee_span, target.clone());
                Ok(Ty::Option(Box::new(target)))
            }
            "type_name_of" => {
                if !generics.is_empty() {
                    return Err(wrong_generics(generics.len(), 0));
                }
                if args.len() != 1 {
                    return Err(wrong_arity(args.len(), 1));
                }
                let a = self.check_expr(&args[0])?;
                self.unify(&Ty::Any, &a, &args[0].span)?;
                Ok(Ty::Str)
            }
            "field_names_of" => {
                if !generics.is_empty() {
                    return Err(wrong_generics(generics.len(), 0));
                }
                if args.len() != 1 {
                    return Err(wrong_arity(args.len(), 1));
                }
                let a = self.check_expr(&args[0])?;
                self.unify(&Ty::Any, &a, &args[0].span)?;
                Ok(Ty::List(Box::new(Ty::Str)))
            }
            "get_field" => {
                if !generics.is_empty() {
                    return Err(wrong_generics(generics.len(), 0));
                }
                if args.len() != 2 {
                    return Err(wrong_arity(args.len(), 2));
                }
                let a = self.check_expr(&args[0])?;
                self.unify(&Ty::Any, &a, &args[0].span)?;
                let n = self.check_expr(&args[1])?;
                self.unify(&Ty::Str, &n, &args[1].span)?;
                Ok(Ty::Option(Box::new(Ty::Any)))
            }
            "set_field" => {
                if !generics.is_empty() {
                    return Err(wrong_generics(generics.len(), 0));
                }
                if args.len() != 3 {
                    return Err(wrong_arity(args.len(), 3));
                }
                let a = self.check_expr(&args[0])?;
                self.unify(&Ty::Any, &a, &args[0].span)?;
                let n = self.check_expr(&args[1])?;
                self.unify(&Ty::Str, &n, &args[1].span)?;
                let v = self.check_expr(&args[2])?;
                self.unify(&Ty::Any, &v, &args[2].span)?;
                Ok(Ty::Unit)
            }
            _ => unreachable!("unhandled runtime reflection builtin {name}"),
        }
    }

    /// Check a raw-pointer FFI builtin (`__ptr_load`, `__ptr_store`,
    /// `__ptr_offset`, `__ptr_is_null`, `__ptr_null`, `__ptr_alloc`,
    /// `__ptr_free`). Each takes one type argument `T`, the pointee type,
    /// which must be a C scalar (`CInt`, `CLong`, `CSize`, `CFloat`,
    /// `CDouble`, `CStr`) or a native `Int`/`Float`; a generic parameter is
    /// allowed so `std/ffi` can wrap these in generic functions. The pointee
    /// is recorded at the callee span so HIR carries it; MIR grounds it per
    /// monomorphization to pick the load/store width.
    fn check_ptr_builtin(
        &mut self,
        name: &str,
        generics: &[crate::ast::Type],
        callee_span: &Span,
        args: &[Expr],
        span: &Span,
    ) -> Result<Ty, RavenError> {
        if generics.len() != 1 {
            return Err(RavenError::ty(
                TypeError::GenericArityMismatch {
                    decl: name.to_string(),
                    expected: 1,
                    actual: generics.len(),
                },
                span.clone(),
            ));
        }
        let pointee = self.resolve_ast_ty(&generics[0])?;
        if !is_ptr_pointee(pointee.strip_self()) {
            return Err(ty_custom(
                &format!(
                    "`{}` pointee must be a C scalar (CInt, CLong, CSize, CFloat, CDouble, CStr) or Int/Float, got `{}`",
                    name, pointee
                ),
                span,
            ));
        }
        let cptr = Ty::Ffi(FfiTy::CPtr(Box::new(pointee.clone())));
        let expected_args = match name {
            "__ptr_null" => 0,
            "__ptr_alloc" | "__ptr_free" | "__ptr_load" | "__ptr_is_null" => 1,
            _ => 2, // __ptr_store, __ptr_offset
        };
        if args.len() != expected_args {
            return Err(RavenError::ty(
                TypeError::WrongArity {
                    func: name.to_string(),
                    expected: expected_args,
                    actual: args.len(),
                },
                span.clone(),
            ));
        }
        // Record the pointee at the callee span for HIR to carry.
        self.record(callee_span, pointee.clone());
        match name {
            "__ptr_null" => Ok(cptr),
            "__ptr_alloc" => {
                let n = self.check_expr(&args[0])?;
                self.unify(&Ty::Int, &n, &args[0].span)?;
                Ok(cptr)
            }
            "__ptr_free" => {
                let p = self.check_expr(&args[0])?;
                self.unify(&cptr, &p, &args[0].span)?;
                Ok(Ty::Unit)
            }
            "__ptr_is_null" => {
                let p = self.check_expr(&args[0])?;
                self.unify(&cptr, &p, &args[0].span)?;
                Ok(Ty::Bool)
            }
            "__ptr_load" => {
                let p = self.check_expr(&args[0])?;
                self.unify(&cptr, &p, &args[0].span)?;
                Ok(pointee)
            }
            "__ptr_offset" => {
                let p = self.check_expr(&args[0])?;
                self.unify(&cptr, &p, &args[0].span)?;
                let n = self.check_expr(&args[1])?;
                self.unify(&Ty::Int, &n, &args[1].span)?;
                Ok(cptr)
            }
            // __ptr_store
            _ => {
                let p = self.check_expr(&args[0])?;
                self.unify(&cptr, &p, &args[0].span)?;
                let v = self.check_expr(&args[1])?;
                let rv = self.infer.resolve(&v);
                // A native Int/Float is accepted where the pointee is an
                // integer/float C scalar, matching the call-site coercion.
                let ok = (is_int_ffi(pointee.strip_self()) && matches!(rv.strip_self(), Ty::Int))
                    || (is_float_ffi(pointee.strip_self()) && matches!(rv.strip_self(), Ty::Float));
                if !ok {
                    self.unify(&pointee, &v, &args[1].span)?;
                }
                Ok(Ty::Unit)
            }
        }
    }

    /// Reject a stdlib intrinsic call whose argument count differs from
    /// `expected`, with the same `WrongArity` diagnostic the other
    /// intrinsic arms use.
    fn check_intrinsic_arity(
        &self,
        name: &str,
        args: &[Expr],
        expected: usize,
        span: &Span,
    ) -> Result<(), RavenError> {
        if args.len() != expected {
            return Err(RavenError::ty(
                TypeError::WrongArity {
                    func: name.to_string(),
                    expected,
                    actual: args.len(),
                },
                span.clone(),
            ));
        }
        Ok(())
    }

    fn check_method_call(
        &mut self,
        receiver: &Expr,
        name: &str,
        generics: &[crate::ast::Type],
        args: &[Expr],
        span: &Span,
    ) -> Result<Ty, RavenError> {
        // `EnumName.Variant(args)` constructs a payload variant. The
        // parser shapes this as a method call, so it is recognized here
        // (before associated-function routing) when the receiver is an
        // enum name and the called name is one of its variants.
        if let Some(ctor_ty) = self.try_enum_variant_ctor(receiver, name, &receiver.span)? {
            let (params, ret) = match ctor_ty {
                Ty::Function { params, ret } => (params, *ret),
                // A unit variant called with arguments: report the arity.
                other => (Vec::new(), other),
            };
            if params.len() != args.len() {
                return Err(RavenError::ty(
                    TypeError::WrongArity {
                        func: format!("{}", ret),
                        expected: params.len(),
                        actual: args.len(),
                    },
                    span.clone(),
                ));
            }
            for (param_ty, arg) in params.iter().zip(args.iter()) {
                let a = self.check_expr(arg)?;
                self.unify(param_ty, &a, &arg.span)?;
            }
            return Ok(ret);
        }

        // `module.func(args)`: when the receiver is a stdlib import alias
        // (`import std/fs` then `fs.write(...)`), this is a call to the
        // module's namespaced free function, not an instance method.
        if let Some(ret) = self.check_module_qualified_call(receiver, name, args, span)? {
            return Ok(ret);
        }

        // `Type.func(args)`: when the receiver is a type name (a struct or
        // enum, or a built-in type), this is an associated function call,
        // not an instance method call. The named function on that type has
        // no `self`. Distinguished from an instance call by the receiver
        // being a bare type reference rather than a value.
        if let Some(type_ref) = self.type_ref_receiver(receiver)? {
            // Record the named type at the receiver span so HIR lowering
            // can read the concrete implementing type for the static call.
            self.record(&receiver.span, type_ref.clone());
            return self.check_assoc_fn_call(&type_ref, name, args, span);
        }

        let recv_ty = self.check_expr(receiver)?;
        let recv_resolved = self.infer.resolve(&recv_ty);
        let recv_stripped = recv_resolved.strip_self().clone();
        if matches!(recv_stripped, Ty::Error) {
            // Eat the cascade.
            for a in args {
                self.check_expr(a)?;
            }
            return Ok(Ty::Error);
        }

        // A `dyn Trait` receiver dispatches dynamically. The method must
        // be one of the trait's own methods; its signature comes from the
        // trait declaration with `Self` standing for the erased concrete
        // type. The return type is the trait method's return type.
        if let Ty::Dyn {
            name: trait_name, ..
        } = &recv_stripped
        {
            return self.check_dyn_method_call(trait_name, name, args, span);
        }

        // A receiver of generic-parameter type `T` dispatches through one
        // of `T`'s trait bounds. The method must be declared by a bound
        // trait; its signature is the template, with every `Self` (the
        // receiver and any `other: Self` parameter) substituted by `T`.
        // Monomorphization later resolves `T` to a concrete type at each
        // call site and rewrites the call to that type's impl symbol.
        if let Ty::Param(param) = &recv_stripped {
            return self.check_bound_method_call(param, name, args, span);
        }

        // User declared `impl` methods are searched first, including
        // impls on built in receiver types (`impl Int`, `impl String`,
        // `impl<T> List<T>`, ...). This is the same path that resolves
        // methods on user structs; built in receivers fall out of it
        // because their `self_ty` is the matching built in `Ty`.
        //
        // Precedence: a user `impl` method always wins over a hard coded
        // built in fast path method of the same name (the fall back at
        // the end of this function). This keeps the checked signature in
        // step with code generation, where a method call lowers to the
        // per type symbol `<RecvType>$<method>` and any user `impl`
        // method defines exactly that symbol.
        //
        // Gather candidate impls. For each impl, allocate fresh
        // inference variables for its declared generic parameters and
        // unify the substituted self_ty against the receiver type. An
        // impl is a candidate when unification succeeds and the method
        // name exists in its method table.
        let impls_snapshot = self.env.impls.clone();
        let mut inherent_matches: Vec<(usize, FnSig, HashMap<ParamId, Ty>)> = Vec::new();
        let mut trait_matches: Vec<(usize, FnSig, HashMap<ParamId, Ty>, String)> = Vec::new();
        for (idx, imp) in impls_snapshot.iter().enumerate() {
            // Skip impls without this method before allocating inference
            // variables or unifying. A rejected impl whose self type still
            // unifies (for example `impl Eq for Map<K, V: Eq>`, which has no
            // `set`) would otherwise leak its bounds onto the receiver's type
            // variables.
            let Some(msig) = imp.methods.get(name) else {
                continue;
            };
            // Substitute fresh vars for impl generics.
            let mut subst: HashMap<ParamId, Ty> = HashMap::new();
            for p in &imp.generics {
                let v = self.infer.fresh(span.clone());
                for b in &p.bounds {
                    self.infer.add_bound(v, b.clone(), span.clone());
                }
                subst.insert(p.id.clone(), Ty::Var(v));
            }
            let impl_self = substitute(&imp.self_ty, &subst);
            // Try unifying receiver with this impl's self type.
            let probe = self.infer.unify(&impl_self, &recv_stripped, span);
            if probe.is_err() {
                continue;
            }
            if imp.trait_name.is_some() {
                trait_matches.push((
                    idx,
                    msig.clone(),
                    subst.clone(),
                    imp.trait_name.clone().unwrap_or_default(),
                ));
            } else {
                inherent_matches.push((idx, msig.clone(), subst.clone()));
            }
        }

        let total = inherent_matches.len() + trait_matches.len();
        if total == 0 {
            // No user `impl` method matched. Fall back to the hard coded
            // built in fast path methods (Option/Result/List/String).
            // These match directly against the resolved receiver shape;
            // their signatures already substitute the element type.
            if let Some((params, ret)) = builtin::lookup_method(&recv_stripped, name) {
                if params.len() != args.len() {
                    return Err(RavenError::ty(
                        TypeError::WrongArity {
                            func: name.to_string(),
                            expected: params.len(),
                            actual: args.len(),
                        },
                        span.clone(),
                    ));
                }
                for (pt, arg) in params.iter().zip(args.iter()) {
                    let a = self.check_expr(arg)?;
                    self.unify(pt, &a, &arg.span)?;
                }
                return Ok(ret);
            }
            return Err(RavenError::ty(
                TypeError::UndefinedMethod {
                    receiver_ty: format!("{}", recv_stripped),
                    method: name.to_string(),
                },
                span.clone(),
            ));
        }
        // Resolve explicit method type arguments (`recv.method<T>(...)`) once,
        // to bind the method's own generic parameters at the call site.
        let mut explicit: Vec<Ty> = Vec::with_capacity(generics.len());
        for g in generics {
            match self.resolve_ast_ty(g) {
                Ok(t) => explicit.push(t),
                Err(e) => {
                    self.push_error(e);
                    explicit.push(Ty::Error);
                }
            }
        }
        // Prefer inherent over trait if both exist.
        if !inherent_matches.is_empty() && inherent_matches.len() == 1 {
            let (_, sig, subst) = inherent_matches.into_iter().next().unwrap();
            return self.apply_method_call(&sig, &subst, args, name, span, &explicit);
        }
        if inherent_matches.is_empty() && trait_matches.len() == 1 {
            let (_, sig, subst, _) = trait_matches.into_iter().next().unwrap();
            return self.apply_method_call(&sig, &subst, args, name, span, &explicit);
        }
        // Otherwise ambiguous.
        let mut names: Vec<String> = inherent_matches
            .iter()
            .map(|_| "<inherent>".to_string())
            .collect();
        names.extend(trait_matches.iter().map(|(_, _, _, t)| t.clone()));
        Err(RavenError::ty(
            TypeError::AmbiguousMethod {
                receiver_ty: format!("{}", recv_stripped),
                method: name.to_string(),
                candidates: names,
            },
            span.clone(),
        ))
    }

    /// If `receiver` is a bare reference to a type name (a struct or enum
    /// binding, or a built-in type identifier), resolve it to that type.
    /// This marks the call as an associated function call. A value
    /// receiver (a local, parameter, field, or any non-type expression)
    /// returns `None` so it stays an instance method call.
    /// Check a `module.func(args)` call where `module` is a stdlib import
    /// alias. Resolves the call to the module's namespaced free function
    /// (`std.<module>.<func>`, the same symbol a selective import binds) and
    /// checks the arguments against its signature. Returns `None` when the
    /// receiver is not a stdlib module alias, so ordinary method-call
    /// checking continues.
    fn check_module_qualified_call(
        &mut self,
        receiver: &Expr,
        name: &str,
        args: &[Expr],
        span: &Span,
    ) -> Result<Option<Ty>, RavenError> {
        let ExprKind::Ident {
            name: alias,
            generics,
        } = &receiver.kind
        else {
            return Ok(None);
        };
        if !generics.is_empty() {
            return Ok(None);
        }
        let Some(Binding::ImportAlias(import_id)) =
            self.resolved.map.lookup(&receiver.span).cloned()
        else {
            return Ok(None);
        };
        let Some(import) = self.resolved.map.imports.get(import_id.0) else {
            return Ok(None);
        };
        // The expander merged this module's functions under `mangled_prefix`.
        // An alias call `alias.fn()` resolves to `<prefix>.fn`, the same symbol
        // a selective import of `fn` would bind. Works for std, local, and
        // external sources alike.
        let Some(prefix) = import.mangled_prefix.clone() else {
            return Ok(None);
        };
        let mangled = format!(
            "{}{}{}",
            prefix,
            crate::resolve::stdlib::NAMESPACE_SEP,
            name
        );
        let decl = self
            .env
            .functions
            .iter()
            .find(|(_, s)| s.name == mangled)
            .map(|(d, _)| *d);
        let Some(decl) = decl else {
            return Err(ty_custom(
                &format!("module `{}` has no function `{}`", alias, name),
                span,
            ));
        };
        let fn_ty = self.type_of_binding(&Binding::Function(decl), span, &[])?;
        let Ty::Function { params, ret } = fn_ty else {
            return Ok(None);
        };
        if params.len() != args.len() {
            return Err(RavenError::ty(
                TypeError::WrongArity {
                    func: mangled,
                    expected: params.len(),
                    actual: args.len(),
                },
                span.clone(),
            ));
        }
        for (param_ty, arg) in params.iter().zip(args.iter()) {
            let a = self.check_expr(arg)?;
            self.unify(param_ty, &a, &arg.span)?;
        }
        Ok(Some(*ret))
    }

    fn type_ref_receiver(&mut self, receiver: &Expr) -> Result<Option<Ty>, RavenError> {
        let ExprKind::Ident { name, generics } = &receiver.kind else {
            return Ok(None);
        };
        // A built-in type name with no resolver binding (`Int`, `String`,
        // `Array`, ...) resolves to its concrete type with the explicit
        // generic arguments applied.
        let Some(binding) = self.resolved.map.lookup(&receiver.span).cloned() else {
            if let Some(ty) = self.builtin_type_ref(name, generics, &receiver.span)? {
                return Ok(Some(ty));
            }
            return Ok(None);
        };
        match binding {
            Binding::Struct(_) | Binding::Enum(_) => {
                let mut explicit = Vec::with_capacity(generics.len());
                for g in generics {
                    explicit.push(self.resolve_ast_ty(g)?);
                }
                Ok(Some(self.type_of_binding(
                    &binding,
                    &receiver.span,
                    &explicit,
                )?))
            }
            _ => Ok(None),
        }
    }

    /// Resolve a built-in type name to its `Ty` for an associated function
    /// receiver. Returns `None` for a name that is not a built-in type, so
    /// a value identifier still flows to instance dispatch.
    fn builtin_type_ref(
        &mut self,
        name: &str,
        generics: &[crate::ast::Type],
        span: &Span,
    ) -> Result<Option<Ty>, RavenError> {
        let mut args = Vec::with_capacity(generics.len());
        for g in generics {
            args.push(self.resolve_ast_ty(g)?);
        }
        let ty = match name {
            "Int" => Ty::Int,
            "Float" => Ty::Float,
            "Bool" => Ty::Bool,
            "String" => Ty::Str,
            "Char" => Ty::Char,
            "Unit" => Ty::Unit,
            "Array" | "List" | "Vec" => {
                let elem = args.into_iter().next().unwrap_or(Ty::Error);
                Ty::List(Box::new(elem))
            }
            _ => return Ok(None),
        };
        let _ = span;
        Ok(Some(ty))
    }

    /// Check an associated function call `Type.func(args)`. Finds an impl
    /// on the named type with a `name` function that has no `self`, checks
    /// the arguments against its parameters, and returns its declared
    /// return type with the impl's generic parameters instantiated.
    fn check_assoc_fn_call(
        &mut self,
        type_ty: &Ty,
        name: &str,
        args: &[Expr],
        span: &Span,
    ) -> Result<Ty, RavenError> {
        let impls_snapshot = self.env.impls.clone();
        let mut matches: Vec<(FnSig, HashMap<ParamId, Ty>)> = Vec::new();
        for imp in impls_snapshot.iter() {
            // Skip impls that do not provide this associated function before
            // allocating inference variables or unifying. A rejected impl whose
            // self type still unifies (for example `impl Eq for Map<K, V: Eq>`,
            // which has no `new`) would otherwise leak its bounds onto the
            // result's type variables.
            let Some(msig) = imp.methods.get(name) else {
                continue;
            };
            if msig.has_self {
                continue;
            }
            let mut subst: HashMap<ParamId, Ty> = HashMap::new();
            for p in &imp.generics {
                let v = self.infer.fresh(span.clone());
                for b in &p.bounds {
                    self.infer.add_bound(v, b.clone(), span.clone());
                }
                subst.insert(p.id.clone(), Ty::Var(v));
            }
            let impl_self = substitute(&imp.self_ty, &subst);
            if self.infer.unify(&impl_self, type_ty, span).is_err() {
                continue;
            }
            matches.push((msig.clone(), subst));
        }
        match matches.len() {
            0 => Err(RavenError::ty(
                TypeError::UndefinedMethod {
                    receiver_ty: format!("{}", type_ty),
                    method: name.to_string(),
                },
                span.clone(),
            )
            .with_hint(format!(
                "no associated function `{}` on type `{}`",
                name, type_ty
            ))),
            1 => {
                let (sig, subst) = matches.into_iter().next().unwrap();
                self.apply_method_call(&sig, &subst, args, name, span, &[])
            }
            _ => Err(RavenError::ty(
                TypeError::AmbiguousMethod {
                    receiver_ty: format!("{}", type_ty),
                    method: name.to_string(),
                    candidates: vec!["<associated>".to_string()],
                },
                span.clone(),
            )),
        }
    }

    fn apply_method_call(
        &mut self,
        sig: &FnSig,
        subst: &HashMap<ParamId, Ty>,
        args: &[Expr],
        name: &str,
        span: &Span,
        explicit: &[Ty],
    ) -> Result<Ty, RavenError> {
        // Explicit type arguments (`recv.method<T>(...)`), when given, must
        // match the method's own generic arity and bind its parameters.
        if !explicit.is_empty() && explicit.len() != sig.generics.len() {
            return Err(RavenError::ty(
                TypeError::GenericArityMismatch {
                    decl: name.to_string(),
                    expected: sig.generics.len(),
                    actual: explicit.len(),
                },
                span.clone(),
            ));
        }
        // Build a per-call substitution: impl generics already in
        // `subst`, plus fresh variables for method generics. A fresh
        // variable is unified with its explicit type argument when one is
        // given, so a method parameter that appears only in the return type
        // is grounded at the call site.
        let mut full = subst.clone();
        for (i, p) in sig.generics.iter().enumerate() {
            let v = self.infer.fresh(span.clone());
            for b in &p.bounds {
                self.infer.add_bound(v, b.clone(), span.clone());
            }
            if let Some(arg) = explicit.get(i) {
                self.unify(&Ty::Var(v), arg, span)?;
            }
            full.insert(p.id.clone(), Ty::Var(v));
        }
        let user_params: Vec<Ty> = sig
            .params
            .iter()
            .filter(|t| !matches!(t, Ty::SelfTy(_)))
            .map(|t| substitute(t, &full))
            .collect();
        if user_params.len() != args.len() {
            return Err(RavenError::ty(
                TypeError::WrongArity {
                    func: name.to_string(),
                    expected: user_params.len(),
                    actual: args.len(),
                },
                span.clone(),
            ));
        }
        for (pt, arg) in user_params.iter().zip(args.iter()) {
            let a = self.check_expr(arg)?;
            self.unify(pt, &a, &arg.span)?;
        }
        Ok(substitute(&sig.ret, &full))
    }

    /// If `recv` has a `next(self) -> Option<T>` method (from any impl,
    /// trait or inherent), return the element type `T`. This is how a
    /// `for` loop discovers that an arbitrary value is iterable: the
    /// `Iterator<T>` trait declares exactly that method, and concrete
    /// adapter structs implement it. Returns `None` when no matching
    /// `next` method resolves to an `Option`.
    fn iterator_elem_ty(&mut self, recv: &Ty, span: &Span) -> Option<Ty> {
        let impls_snapshot = self.env.impls.clone();
        for imp in impls_snapshot.iter() {
            let Some(msig) = imp.methods.get("next") else {
                continue;
            };
            // Substitute fresh inference vars for the impl's generics, then
            // unify the impl's self type against the receiver. A successful
            // unification fixes those vars, so the substituted return type
            // becomes concrete.
            let mut subst: HashMap<ParamId, Ty> = HashMap::new();
            for p in &imp.generics {
                let v = self.infer.fresh(span.clone());
                subst.insert(p.id.clone(), Ty::Var(v));
            }
            let impl_self = substitute(&imp.self_ty, &subst);
            if self.infer.unify(&impl_self, recv, span).is_err() {
                continue;
            }
            let ret = self.infer.resolve(&substitute(&msig.ret, &subst));
            if let Ty::Option(elem) = ret.strip_self() {
                return Some((**elem).clone());
            }
        }
        None
    }

    /// Element type for a generic parameter bounded by `Iterator<T>`.
    /// Reads the parameter's recorded bounds and returns the type argument
    /// of an `Iterator` bound, which is the element type the `for` loop and
    /// `next()` calls produce once the parameter is grounded.
    fn param_iterator_elem_ty(&self, param: &ParamId) -> Option<Ty> {
        let bounds = self.param_bounds.get(param)?;
        for (name, args) in bounds {
            if name == "Iterator" {
                return args.first().cloned();
            }
        }
        None
    }

    /// Check a method call whose receiver is a `dyn Trait` value. The
    /// method must belong to the trait. Argument types are checked against
    /// the trait method's declared parameter types (with `Self` left
    /// abstract, which is sound because object-safe methods never use
    /// `Self` outside the receiver). The result is the method's return
    /// type, with any `Self` return collapsed to the trait object type so
    /// the value stays usable.
    fn check_dyn_method_call(
        &mut self,
        trait_name: &str,
        method: &str,
        args: &[Expr],
        span: &Span,
    ) -> Result<Ty, RavenError> {
        let sig = self
            .env
            .traits
            .values()
            .find(|t| t.name == trait_name)
            .and_then(|t| t.methods.get(method).cloned());
        let Some(sig) = sig else {
            return Err(RavenError::ty(
                TypeError::UndefinedMethod {
                    receiver_ty: format!("dyn {}", trait_name),
                    method: method.to_string(),
                },
                span.clone(),
            ));
        };
        let user_params: Vec<Ty> = sig
            .params
            .iter()
            .filter(|t| !matches!(t, Ty::SelfTy(_)))
            .cloned()
            .collect();
        if user_params.len() != args.len() {
            return Err(RavenError::ty(
                TypeError::WrongArity {
                    func: method.to_string(),
                    expected: user_params.len(),
                    actual: args.len(),
                },
                span.clone(),
            ));
        }
        for (pt, arg) in user_params.iter().zip(args.iter()) {
            let a = self.check_expr(arg)?;
            self.unify(pt, &a, &arg.span)?;
        }
        Ok(sig.ret.clone())
    }

    /// Resolve a method call whose receiver has generic-parameter type
    /// `param` (for example `x.to_string()` where `x: T` and `T:
    /// ToString`). The method must be declared by one of `param`'s trait
    /// bounds. The trait method signature is the template: every `Self`
    /// (the `self` receiver and any `Self`-typed parameter or return) is
    /// substituted by `Ty::Param(param)` so argument and result types
    /// stay tied to the type parameter. The concrete impl is selected
    /// during monomorphization once `param` is known at each call site.
    fn check_bound_method_call(
        &mut self,
        param: &ParamId,
        name: &str,
        args: &[Expr],
        span: &Span,
    ) -> Result<Ty, RavenError> {
        let recv_ty = Ty::Param(param.clone());
        let bounds = self.param_bounds.get(param).cloned().unwrap_or_default();
        // Find a bound trait whose declaration carries the called method.
        // Alongside the method signature, build a substitution that maps
        // the trait's own generic parameters to the type arguments named
        // in the bound. For a bound `S: Iterator<Int>` calling `next`,
        // `Iterator`'s declared `T` maps to `Int`, so the method's return
        // `Option<T>` resolves to `Option<Int>` instead of staying
        // abstract. Without this, the trait parameter leaks out as a fresh
        // `Param` that cannot unify with the concrete element type.
        let mut found: Option<FnSig> = None;
        let mut trait_subst: HashMap<ParamId, Ty> = HashMap::new();
        for (trait_name, bound_args) in &bounds {
            let trait_sig = self
                .env
                .traits
                .values()
                .find(|t| &t.name == trait_name)
                .cloned();
            let Some(trait_sig) = trait_sig else {
                continue;
            };
            let Some(msig) = trait_sig.methods.get(name).cloned() else {
                continue;
            };
            for (tp, arg) in trait_sig.generics.iter().zip(bound_args.iter()) {
                if !matches!(arg, Ty::Error) {
                    trait_subst.insert(tp.id.clone(), arg.clone());
                }
            }
            found = Some(msig);
            break;
        }
        let Some(sig) = found else {
            // The method is on no bound trait. If the parameter has no
            // bounds at all the message points at the missing bound;
            // otherwise it is a genuine "no such method" on the bounds.
            let hint = if bounds.is_empty() {
                format!(
                    "`{}` has no trait bound; add a bound such as `{}: ToString` to call methods on it",
                    param.name,
                    param.name
                )
            } else {
                let names: Vec<&str> = bounds.iter().map(|(n, _)| n.as_str()).collect();
                format!(
                    "none of the bounds `{}` on `{}` declare a method `{}`",
                    names.join(" + "),
                    param.name,
                    name
                )
            };
            return Err(RavenError::ty(
                TypeError::UndefinedMethod {
                    receiver_ty: param.name.to_string(),
                    method: name.to_string(),
                },
                span.clone(),
            )
            .with_hint(hint));
        };

        // Drop the leading `self` receiver positionally, then substitute
        // every remaining `Self` (for example the `other: Self` of
        // `equals`) with the receiver's parameter type and the trait's own
        // generic parameters with the bound's type arguments. Filtering by
        // `Self` type instead would also drop a real `Self`-typed argument.
        let rest = if sig.has_self && !sig.params.is_empty() {
            &sig.params[1..]
        } else {
            &sig.params[..]
        };
        let user_params: Vec<Ty> = rest
            .iter()
            .map(|t| substitute(&substitute_self(t, &recv_ty), &trait_subst))
            .collect();
        if user_params.len() != args.len() {
            return Err(RavenError::ty(
                TypeError::WrongArity {
                    func: name.to_string(),
                    expected: user_params.len(),
                    actual: args.len(),
                },
                span.clone(),
            ));
        }
        for (pt, arg) in user_params.iter().zip(args.iter()) {
            let a = self.check_expr(arg)?;
            self.unify(pt, &a, &arg.span)?;
        }
        Ok(substitute(
            &substitute_self(&sig.ret, &recv_ty),
            &trait_subst,
        ))
    }

    fn check_field(&mut self, receiver: &Expr, name: &str, span: &Span) -> Result<Ty, RavenError> {
        // `EnumName.Variant` constructs a variant. Intercept before
        // checking the receiver as a value: a bare enum name is a type,
        // not a value, so `check_expr(receiver)` cannot type it.
        if let Some(ty) = self.try_enum_variant_ctor(receiver, name, span)? {
            return Ok(ty);
        }
        let recv = self.check_expr(receiver)?;
        let recv_resolved = self.infer.resolve(&recv);
        let stripped = recv_resolved.strip_self().clone();
        match stripped {
            Ty::Struct {
                id,
                name: sname,
                args,
            } => {
                let sig = self
                    .env
                    .structs
                    .get(&id)
                    .cloned()
                    .ok_or_else(|| ty_custom("struct signature missing", span))?;
                // Build substitution from declared generics to args.
                let mut subst: HashMap<ParamId, Ty> = HashMap::new();
                for (p, a) in sig.generics.iter().zip(args.iter()) {
                    subst.insert(p.id.clone(), a.clone());
                }
                match sig.field(name) {
                    Some((_, ty)) => Ok(substitute(ty, &subst)),
                    None => Err(RavenError::ty(
                        TypeError::UndefinedField {
                            struct_name: sname,
                            field: name.to_string(),
                        },
                        span.clone(),
                    )),
                }
            }
            Ty::Enum { name: ename, .. } => Err(RavenError::ty(
                TypeError::Custom(format!(
                    "enum variant access `{}.{}` is parsed but not yet supported as field syntax; use a `match`",
                    ename, name
                )),
                span.clone(),
            )),
            Ty::Error => Ok(Ty::Error),
            other => Err(RavenError::ty(
                TypeError::Custom(format!("type `{}` has no field `{}`", other, name)),
                span.clone(),
            )),
        }
    }

    /// If `receiver` is a bare enum name and `name` is one of its
    /// variants, return the type produced by constructing it.
    ///
    /// A unit variant yields the enum type directly. A payload variant
    /// yields a function type whose parameters are the payload types and
    /// whose result is the enum type, so the surrounding `check_call`
    /// applies the arguments. For a generic enum the declared generic
    /// parameters become fresh inference variables, solved by unification
    /// against the argument types and the surrounding expected type.
    ///
    /// Returns `None` when the receiver is not an enum name, so ordinary
    /// struct field access is left untouched.
    fn try_enum_variant_ctor(
        &mut self,
        receiver: &Expr,
        name: &str,
        span: &Span,
    ) -> Result<Option<Ty>, RavenError> {
        let ExprKind::Ident { .. } = &receiver.kind else {
            return Ok(None);
        };
        let Some(Binding::Enum(id)) = self.resolved.map.lookup(&receiver.span).cloned() else {
            return Ok(None);
        };
        let sig = self
            .env
            .enums
            .get(&id)
            .cloned()
            .ok_or_else(|| ty_custom("enum signature missing", span))?;
        let Some((_, variant)) = sig.variant(name) else {
            // Not a variant: this may be an associated function call on the
            // enum (for example a derived `Shape.from_json(j)`). Fall through
            // so associated-function resolution can try; it reports its own
            // error if no such function exists.
            return Ok(None);
        };
        // Fresh inference variables for the enum's generic parameters.
        let args = self.instantiate_type_args(&sig.generics, &[], span, &sig.name)?;
        let mut subst: HashMap<ParamId, Ty> = HashMap::new();
        for (p, a) in sig.generics.iter().zip(args.iter()) {
            subst.insert(p.id.clone(), a.clone());
        }
        let enum_ty = Ty::Enum {
            id,
            name: sig.name.clone(),
            args,
        };
        match &variant.payload {
            VariantPayloadSig::Unit => Ok(Some(enum_ty)),
            VariantPayloadSig::Tuple(tys) => {
                let params = tys.iter().map(|t| substitute(t, &subst)).collect();
                Ok(Some(Ty::Function {
                    params,
                    ret: Box::new(enum_ty),
                }))
            }
            VariantPayloadSig::Struct(_) => Err(RavenError::ty(
                TypeError::Custom(format!(
                    "variant `{}` has named fields; struct-variant construction is not yet supported",
                    name
                )),
                span.clone(),
            )),
        }
    }

    fn check_index(
        &mut self,
        receiver: &Expr,
        index: &Expr,
        span: &Span,
    ) -> Result<Ty, RavenError> {
        let recv = self.check_expr(receiver)?;
        let idx = self.check_expr(index)?;
        self.unify(&Ty::Int, &idx, &index.span)?;
        let recv_resolved = self.infer.resolve(&recv);
        match recv_resolved.strip_self() {
            Ty::List(t) => Ok(*t.clone()),
            Ty::Str => Ok(Ty::Char),
            Ty::Error => Ok(Ty::Error),
            Ty::Var(_) => {
                // Unify the receiver with a list of fresh element type.
                let elem = Ty::Var(self.infer.fresh(span.clone()));
                let list_ty = Ty::List(Box::new(elem.clone()));
                self.unify(&list_ty, &recv, &receiver.span)?;
                Ok(elem)
            }
            other => Err(RavenError::ty(
                TypeError::Custom(format!("cannot index into `{}`", other)),
                span.clone(),
            )),
        }
    }

    fn check_if(
        &mut self,
        cond: &Expr,
        then_branch: &Block,
        else_branch: Option<&ElseBranch>,
        span: &Span,
    ) -> Result<Ty, RavenError> {
        let c = self.check_expr(cond)?;
        self.unify(&Ty::Bool, &c, &cond.span)?;
        let t = self.check_block(then_branch)?;
        let e = match else_branch {
            // An `if` without `else` cannot produce a value on every path, so
            // it is a unit expression. The then branch is still checked and
            // evaluated for side effects, but its trailing value is discarded.
            None => return Ok(Ty::Unit),
            Some(ElseBranch::If(expr)) => self.check_expr(expr)?,
            Some(ElseBranch::Block(b)) => self.check_block(b)?,
        };
        self.unify(&t, &e, span)?;
        Ok(self.infer.resolve(&t))
    }

    fn check_match(
        &mut self,
        scrutinee: &Expr,
        arms: &[crate::ast::MatchArm],
        span: &Span,
    ) -> Result<Ty, RavenError> {
        let scrut_ty = self.check_expr(scrutinee)?;
        // Resolve the scrutinee type up front. A nested match binds its
        // scrutinee to an inference variable that is already solved by the time
        // the inner match runs (the inner `v` of `match opt { Some(v) -> match v
        // { ... } }`). Without resolving, the patterns are bound against the
        // bare variable, so a payload binding like `Str(s)` gets a fresh
        // variable instead of its real type and the arms fail to infer, and the
        // exhaustiveness check finds no constructors and wrongly flags the first
        // arm as a catch-all that shadows the rest.
        let scrut_stripped = self.infer.resolve(scrut_ty.strip_self());

        let mut result_ty: Option<Ty> = None;
        let mut pattern_names: Vec<super::match_check::PatternHead> = Vec::new();

        for arm in arms {
            // A constructor pattern whose element is itself a constructor,
            // literal, or range (`Some(Ok(x))`, `Some(0)`) is not supported:
            // the lowering binds only one level and would miscompile. Reject it
            // rather than silently mis-binding.
            if let Some(nested) = unsupported_nesting_span(&arm.pattern) {
                return Err(RavenError::ty(
                    TypeError::Custom(
                        "a nested pattern inside a constructor is not supported yet; bind the inner value and match it separately".into(),
                    ),
                    nested,
                ));
            }
            // Each arm gets its own pattern bindings.
            let mut arm_locals = self.locals.clone();
            pattern::bind(&arm.pattern, &scrut_stripped, self.env, &mut arm_locals)?;
            let saved_locals = std::mem::replace(&mut self.locals, arm_locals);

            if let Some(g) = &arm.guard {
                let gt = self.check_expr(g)?;
                self.unify(&Ty::Bool, &gt, &g.span)?;
            }

            let body_ty = self.check_expr(&arm.body)?;
            self.locals = saved_locals;

            result_ty = Some(match result_ty.take() {
                None => body_ty,
                Some(prev) => {
                    self.unify(&prev, &body_ty, &arm.span)?;
                    self.infer.resolve(&prev)
                }
            });
            // A guarded arm only matches when its guard holds, so it neither
            // covers its pattern for exhaustiveness nor shadows a later arm with
            // the same pattern. Treat it as a non-covering specific head.
            let head = if arm.guard.is_some() {
                super::match_check::PatternHead::Other
            } else {
                super::match_check::pattern_head(&arm.pattern)
            };
            pattern_names.push(head);
        }

        super::match_check::check(&scrut_stripped, &pattern_names, span, self.env)?;
        Ok(result_ty.unwrap_or(Ty::Unit))
    }

    fn check_lambda(
        &mut self,
        params: &[crate::ast::LambdaParam],
        ret: Option<&crate::ast::Type>,
        body: &LambdaBody,
        params_inferred: bool,
        _span: &Span,
    ) -> Result<Ty, RavenError> {
        // Lambdas require parameter type annotations in this release;
        // shorthand `{ x, y -> body }` syntax is parsed but not typed.
        if params_inferred {
            return Err(RavenError::ty(
                TypeError::Custom(
                    "shorthand lambdas without parameter annotations require a context type; \
                     full inference lands with issue #59"
                        .into(),
                ),
                _span.clone(),
            ));
        }
        let mut param_tys = Vec::with_capacity(params.len());
        for p in params {
            let t = match &p.ty {
                Some(t) => self.resolve_ast_ty(t)?,
                None => {
                    return Err(RavenError::ty(
                        TypeError::Custom(format!(
                            "lambda parameter `{}` needs a type annotation",
                            p.name
                        )),
                        p.span.clone(),
                    ));
                }
            };
            self.locals.insert(BindingKey::param(&p.span), t.clone());
            param_tys.push(t);
        }
        let declared_ret = match ret {
            Some(t) => Some(self.resolve_ast_ty(t)?),
            None => None,
        };
        // A lambda is its own function: an enclosing loop does not extend into
        // it, so `break`/`continue` in the body are outside any loop. Check the
        // body with a fresh loop stack, then restore the caller's.
        let saved_loops = std::mem::take(&mut self.loop_kinds);
        let body_ty = match body {
            LambdaBody::Block(b) => self.check_block(b),
            LambdaBody::Expr(e) => self.check_expr(e),
        };
        self.loop_kinds = saved_loops;
        let body_ty = body_ty?;
        let final_ret = match declared_ret {
            Some(d) => {
                let discards_block_tail =
                    matches!(body, LambdaBody::Block(_)) && matches!(d.strip_self(), Ty::Unit);
                if !discards_block_tail {
                    self.unify(
                        &d,
                        &body_ty,
                        match body {
                            LambdaBody::Block(b) => &b.span,
                            LambdaBody::Expr(e) => &e.span,
                        },
                    )?;
                }
                d
            }
            None => body_ty,
        };
        Ok(Ty::Function {
            params: param_tys,
            ret: Box::new(final_ret),
        })
    }

    fn check_struct_lit(
        &mut self,
        name: &str,
        fields: &[FieldInit],
        span: &Span,
    ) -> Result<Ty, RavenError> {
        // Look up the struct binding the resolver recorded under the
        // literal's span (resolver binds the literal's whole span to
        // the struct decl).
        let binding = self.resolved.map.lookup(span).ok_or_else(|| {
            RavenError::ty(
                TypeError::Custom(format!("struct `{}` is not in scope", name)),
                span.clone(),
            )
        })?;
        let id = match binding {
            Binding::Struct(id) => *id,
            _ => {
                return Err(RavenError::ty(
                    TypeError::Custom(format!("`{}` is not a struct type", name)),
                    span.clone(),
                ));
            }
        };
        let sig = self
            .env
            .structs
            .get(&id)
            .cloned()
            .ok_or_else(|| ty_custom("struct signature missing", span))?;

        // Instantiate the struct's generic parameters with fresh
        // inference variables, so each field type can be substituted.
        let mut subst: HashMap<ParamId, Ty> = HashMap::new();
        for p in &sig.generics {
            let v = self.infer.fresh(span.clone());
            for b in &p.bounds {
                self.infer.add_bound(v, b.clone(), span.clone());
            }
            subst.insert(p.id.clone(), Ty::Var(v));
        }

        let mut seen = std::collections::HashSet::new();
        for fi in fields {
            let (_, field_ty) = sig.field(&fi.name).ok_or_else(|| {
                RavenError::ty(
                    TypeError::UndefinedField {
                        struct_name: sig.name.clone(),
                        field: fi.name.clone(),
                    },
                    fi.span.clone(),
                )
            })?;
            let field_ty_inst = substitute(field_ty, &subst);
            let value_ty = self.check_expr(&fi.value)?;
            // An integer-class C FFI field (`CInt`, `CLong`, `CSize`)
            // accepts a native `Int` literal, the same coercion a C call
            // applies, so a `@repr(C)` struct can be built with plain
            // integer literals (`Point { x: 3, y: 4 }`). The back end
            // reduces the i64 to the field's C width at the boundary.
            let resolved_field = self.infer.resolve(&field_ty_inst);
            let resolved_value = self.infer.resolve(&value_ty);
            if is_int_ffi(resolved_field.strip_self())
                && matches!(resolved_value.strip_self(), Ty::Int)
            {
                seen.insert(fi.name.clone());
                continue;
            }
            // A floating C FFI field (`CFloat`, `CDouble`) accepts a native
            // `Float` literal, the same coercion a C call applies. The back
            // end narrows an f64 to f32 for a `CFloat` field at the boundary.
            if is_float_ffi(resolved_field.strip_self())
                && matches!(resolved_value.strip_self(), Ty::Float)
            {
                seen.insert(fi.name.clone());
                continue;
            }
            self.unify(&field_ty_inst, &value_ty, &fi.value.span)?;
            seen.insert(fi.name.clone());
        }
        let missing: Vec<&str> = sig
            .fields
            .iter()
            .filter_map(|f| {
                if seen.contains(&f.name) {
                    None
                } else {
                    Some(f.name.as_str())
                }
            })
            .collect();
        if !missing.is_empty() {
            return Err(RavenError::ty(
                TypeError::Custom(format!(
                    "struct `{}` is missing field(s): {}",
                    sig.name,
                    missing.join(", ")
                )),
                span.clone(),
            ));
        }
        // Build the type with the inference variables in the args list.
        let args: Vec<Ty> = sig
            .generics
            .iter()
            .map(|p| subst.get(&p.id).cloned().unwrap_or(Ty::Error))
            .collect();
        Ok(Ty::Struct {
            id,
            name: sig.name.clone(),
            args,
        })
    }

    fn record(&mut self, span: &Span, ty: Ty) {
        let key = UseKey::from_span(span);
        self.recorded.push(key.clone());
        self.types.types.insert(key, ty);
    }

    fn record_type_args(&mut self, span: &Span, args: Vec<Ty>) {
        self.types.record_type_args(span, args);
    }

    /// Type an interpolated string literal. The whole literal has type
    /// `String`. Every embedded `${expr}` must resolve to a type that can
    /// be converted to a string. The built-in scalars (`String`, `Int`,
    /// `Bool`, `Float`, `Char`) convert through their per-type runtime
    /// rendering; any other type converts through its `ToString` impl, so
    /// a user struct that implements `ToString` interpolates. A type with
    /// neither is rejected with a hint to implement `ToString`.
    ///
    /// Each embedded expression is checked through `check_expr`, which
    /// records its resolved type under its (synthetic, per-fragment)
    /// span so HIR lowering can pick the right conversion.
    fn check_interpolated_string(&mut self, fragments: &[StrFragment]) -> Result<Ty, RavenError> {
        for frag in fragments {
            let StrFragment::Expr(e) = frag else {
                continue;
            };
            let ty = self.check_expr(e)?;
            let resolved = self.infer.resolve(&ty);
            let stripped = resolved.strip_self();
            if matches!(stripped, Ty::Error) {
                // Eat the cascade; an earlier error was already reported.
                continue;
            }
            // The built-in scalars convert through their dedicated runtime
            // rendering. Any other type must implement `ToString`.
            if is_interpolatable(stripped) {
                continue;
            }
            // The integer C FFI types widen to `Int` and render through the
            // `Int` to-string path (HIR lowering inserts the cast). The
            // float C FFI types render through the `Float` path.
            if is_int_ffi(stripped) || is_float_ffi(stripped) {
                continue;
            }
            if let Ty::Param(p) = stripped {
                let ok = self
                    .param_bounds
                    .get(p)
                    .map(|bs| bs.iter().any(|(name, _)| name == "ToString"))
                    .unwrap_or(false);
                if ok {
                    continue;
                }
            } else if self.implements_trait(stripped, "ToString") {
                continue;
            }
            return Err(RavenError::ty(
                TypeError::Custom(format!(
                    "values of type `{}` cannot be interpolated into a string",
                    resolved
                )),
                e.span.clone(),
            )
            .with_hint(format!(
                "implement `ToString` for `{}` to interpolate it, or convert it to a `String` first",
                resolved
            )));
        }
        Ok(Ty::Str)
    }
}

/// True when a value of type `ty` can be interpolated into a string via
/// a known per-type conversion. The set is kept in sync with the runtime
/// `raven_*_to_string` conversions wired up in codegen.
fn is_interpolatable(ty: &Ty) -> bool {
    matches!(ty, Ty::Str | Ty::Int | Ty::Bool | Ty::Float | Ty::Char)
}

/// Replace every `Self` placeholder in `ty` with `target`. Used when a
/// trait method signature is applied to a generic-parameter receiver: a
/// `Self`-typed parameter or return becomes the receiver's type. Walks
/// the type structurally so a `Self` nested in `Option<Self>` and the
/// like is substituted too.
fn substitute_self(ty: &Ty, target: &Ty) -> Ty {
    match ty {
        Ty::SelfTy(_) => target.clone(),
        Ty::Option(t) => Ty::Option(Box::new(substitute_self(t, target))),
        Ty::List(t) => Ty::List(Box::new(substitute_self(t, target))),
        Ty::Result(a, b) => Ty::Result(
            Box::new(substitute_self(a, target)),
            Box::new(substitute_self(b, target)),
        ),
        Ty::Struct { id, name, args } => Ty::Struct {
            id: *id,
            name: name.clone(),
            args: args.iter().map(|a| substitute_self(a, target)).collect(),
        },
        Ty::Enum { id, name, args } => Ty::Enum {
            id: *id,
            name: name.clone(),
            args: args.iter().map(|a| substitute_self(a, target)).collect(),
        },
        Ty::Function { params, ret } => Ty::Function {
            params: params.iter().map(|a| substitute_self(a, target)).collect(),
            ret: Box::new(substitute_self(ret, target)),
        },
        other => other.clone(),
    }
}

/// Type rules for binary operators. Exposed so the assignment helper
/// can reuse the table.
pub fn check_binary(l: &Ty, r: &Ty, op: BinaryOp, span: &Span) -> Result<Ty, RavenError> {
    use BinaryOp::*;
    if l.is_error() || r.is_error() {
        return Ok(Ty::Error);
    }
    let ls = l.strip_self();
    let rs = r.strip_self();
    match op {
        Add | Sub | Mul | Div | Mod => match (ls, rs) {
            (Ty::Int, Ty::Int) => Ok(Ty::Int),
            (Ty::Float, Ty::Float) => Ok(Ty::Float),
            // Arithmetic on two equal integer C FFI types stays in that
            // type (the back end emits the op at the type's machine
            // width). This lets an FFI callback such as a `qsort`
            // comparator compute `load<CInt>(a) - load<CInt>(b)` directly.
            (a, b) if is_int_ffi(a) && a == b => Ok(a.clone()),
            _ => Err(RavenError::ty(
                TypeError::TypeMismatch {
                    expected: format!("{} and {}", ls, ls),
                    actual: format!("{} and {}", ls, rs),
                },
                span.clone(),
            )),
        },
        Eq | Ne => {
            if assignable(ls, rs) || assignable(rs, ls) {
                Ok(Ty::Bool)
            } else {
                Err(RavenError::ty(
                    TypeError::TypeMismatch {
                        expected: format!("{}", ls),
                        actual: format!("{}", rs),
                    },
                    span.clone(),
                ))
            }
        }
        Lt | Le | Gt | Ge => match (ls, rs) {
            (Ty::Int, Ty::Int)
            | (Ty::Float, Ty::Float)
            | (Ty::Char, Ty::Char)
            | (Ty::Str, Ty::Str) => Ok(Ty::Bool),
            _ => Err(RavenError::ty(
                TypeError::TypeMismatch {
                    expected: "orderable types".into(),
                    actual: format!("{} and {}", ls, rs),
                },
                span.clone(),
            )),
        },
        And | Or => match (ls, rs) {
            (Ty::Bool, Ty::Bool) => Ok(Ty::Bool),
            _ => Err(RavenError::ty(
                TypeError::TypeMismatch {
                    expected: "Bool and Bool".into(),
                    actual: format!("{} and {}", ls, rs),
                },
                span.clone(),
            )),
        },
        BitAnd | BitOr | BitXor | Shl | Shr => match (ls, rs) {
            (Ty::Int, Ty::Int) => Ok(Ty::Int),
            _ => Err(RavenError::ty(
                TypeError::TypeMismatch {
                    expected: "Int and Int".into(),
                    actual: format!("{} and {}", ls, rs),
                },
                span.clone(),
            )),
        },
    }
}

fn compound_binary_op(op: AssignOp) -> BinaryOp {
    match op {
        AssignOp::Add => BinaryOp::Add,
        AssignOp::Sub => BinaryOp::Sub,
        AssignOp::Mul => BinaryOp::Mul,
        AssignOp::Div => BinaryOp::Div,
        AssignOp::Mod => BinaryOp::Mod,
        AssignOp::BitAnd => BinaryOp::BitAnd,
        AssignOp::BitOr => BinaryOp::BitOr,
        AssignOp::BitXor => BinaryOp::BitXor,
        AssignOp::Shl => BinaryOp::Shl,
        AssignOp::Shr => BinaryOp::Shr,
        AssignOp::Assign => BinaryOp::Add, // unreachable in callers
    }
}

/// The span of the first constructor-pattern element that is itself a
/// constructor, literal, or range (an unsupported nested pattern), or `None`
/// when the pattern nests only wildcards and bindings. A bare identifier is
/// allowed because it lowers to a simple binding.
fn unsupported_nesting_span(pat: &crate::ast::Pattern) -> Option<Span> {
    use crate::ast::PatternKind;
    let trivial =
        |p: &crate::ast::Pattern| matches!(p.kind, PatternKind::Wildcard | PatternKind::Ident(_));
    match &pat.kind {
        PatternKind::Tuple { elements, .. } => elements
            .iter()
            .find(|e| !trivial(e))
            .map(|e| e.span.clone()),
        PatternKind::Struct { fields, .. } => fields
            .iter()
            .filter_map(|f| f.pattern.as_ref())
            .find(|p| !trivial(p))
            .map(|p| p.span.clone()),
        _ => None,
    }
}

/// Whether a type mentions a generic parameter anywhere.
fn ty_mentions_param(ty: &Ty) -> bool {
    match ty {
        Ty::Param(_) => true,
        Ty::Option(t) | Ty::List(t) | Ty::SelfTy(t) => ty_mentions_param(t),
        Ty::Result(a, b) => ty_mentions_param(a) || ty_mentions_param(b),
        Ty::Struct { args, .. } | Ty::Enum { args, .. } => args.iter().any(ty_mentions_param),
        Ty::Function { params, ret } => {
            params.iter().any(ty_mentions_param) || ty_mentions_param(ret)
        }
        _ => false,
    }
}

/// Whether `ty` is a non-simple, fully concrete type: a generic container or a
/// struct/enum with type arguments, with no inference variable or generic
/// parameter inside. These are exactly the instantiations the eager,
/// simple-only bound check deferred.
fn is_nonsimple_concrete(ty: &Ty) -> bool {
    if ty.has_var() || ty_mentions_param(ty) {
        return false;
    }
    match ty {
        Ty::List(_) | Ty::Option(_) | Ty::Result(_, _) | Ty::Function { .. } => true,
        Ty::Struct { args, .. } | Ty::Enum { args, .. } => !args.is_empty(),
        _ => false,
    }
}

fn describe_callee(expr: &Expr) -> String {
    match &expr.kind {
        ExprKind::Ident { name, .. } => name.clone(),
        _ => "<expression>".to_string(),
    }
}

/// True for the integer C FFI types (`CInt`, `CLong`, `CSize`). A native
/// `Int` may be passed where one of these is expected at a C call.
fn is_int_ffi(ty: &Ty) -> bool {
    matches!(
        ty,
        Ty::Ffi(FfiTy::CInt) | Ty::Ffi(FfiTy::CLong) | Ty::Ffi(FfiTy::CSize)
    )
}

fn is_float_ffi(ty: &Ty) -> bool {
    matches!(ty, Ty::Ffi(FfiTy::CFloat) | Ty::Ffi(FfiTy::CDouble))
}

/// True for a type allowed as a variadic C argument: a C-FFI integer or
/// pointer, or a native `Int`. Floats are excluded because the back end
/// cannot honor the variadic float ABI (the System V `al` count and the
/// Windows x64 float-shadow rule).
fn is_variadic_ffi_arg(ty: &Ty) -> bool {
    matches!(
        ty,
        Ty::Int
            | Ty::Ffi(FfiTy::CInt)
            | Ty::Ffi(FfiTy::CLong)
            | Ty::Ffi(FfiTy::CSize)
            | Ty::Ffi(FfiTy::CStr)
            | Ty::Ffi(FfiTy::CPtr(_))
    )
}

/// True for a C-FFI scalar or pointer type with a well-defined C ABI:
/// `CInt`, `CLong`, `CSize`, `CFloat`, `CDouble`, `CStr`, `CPtr<T>`, or
/// `CFnPtr`. Used to validate that a function passed as a `CFnPtr`
/// callback has a signature C can call.
fn is_ffi_abi_ty(ty: &Ty) -> bool {
    is_int_ffi(ty)
        || is_float_ffi(ty)
        || matches!(
            ty,
            Ty::Ffi(FfiTy::CStr) | Ty::Ffi(FfiTy::CPtr(_)) | Ty::Ffi(FfiTy::CFnPtr)
        )
}

/// True if `ty` is a valid pointee for the raw-pointer FFI builtins: a C
/// scalar, a native `Int`/`Float`, or a generic parameter (resolved per
/// monomorphization). The pointer load/store width is the Cranelift type of
/// this pointee.
fn is_ptr_pointee(ty: &Ty) -> bool {
    is_int_ffi(ty)
        || is_float_ffi(ty)
        || matches!(
            ty,
            Ty::Ffi(FfiTy::CStr) | Ty::Int | Ty::Float | Ty::Param(_)
        )
}

pub(crate) fn ty_custom(msg: &str, span: &Span) -> RavenError {
    RavenError::ty(TypeError::Custom(msg.into()), span.clone())
}

/// Find the `Iterator` element type of a fully concrete type by matching
/// it against the `next` method of every impl. The impl's `self_ty`
/// (which carries `Ty::Param`s) is structurally matched against the
/// concrete type to bind those parameters, and the bound substitution is
/// applied to `next`'s `Option<T>` return. Pure: it allocates no
/// inference variables, so it is safe to call from `finalize`.
fn iterator_elem_concrete(impls: &[super::env::ImplSig], ty: &Ty) -> Option<Ty> {
    for imp in impls {
        let Some(msig) = imp.methods.get("next") else {
            continue;
        };
        let mut subst: HashMap<ParamId, Ty> = HashMap::new();
        if !structural_match(&imp.self_ty, ty, &mut subst) {
            continue;
        }
        let ret = substitute(&msig.ret, &subst);
        if let Ty::Option(elem) = ret.strip_self() {
            if !elem.has_var() {
                return Some((**elem).clone());
            }
        }
    }
    None
}

/// Structurally match a declared type (which may contain `Ty::Param`)
/// against a concrete type, recording each parameter's binding. Returns
/// false on a shape mismatch. Used to ground an impl's parameters from a
/// concrete receiver without the inference machinery.
fn structural_match(decl: &Ty, concrete: &Ty, out: &mut HashMap<ParamId, Ty>) -> bool {
    match (decl, concrete) {
        (Ty::Param(p), c) => {
            out.insert(p.clone(), c.clone());
            true
        }
        (Ty::Option(a), Ty::Option(b))
        | (Ty::List(a), Ty::List(b))
        | (Ty::SelfTy(a), Ty::SelfTy(b)) => structural_match(a, b, out),
        (Ty::Result(a1, a2), Ty::Result(b1, b2)) => {
            structural_match(a1, b1, out) && structural_match(a2, b2, out)
        }
        (
            Ty::Struct {
                id: ia, args: a, ..
            },
            Ty::Struct {
                id: ib, args: b, ..
            },
        )
        | (
            Ty::Enum {
                id: ia, args: a, ..
            },
            Ty::Enum {
                id: ib, args: b, ..
            },
        ) => {
            ia == ib
                && a.len() == b.len()
                && a.iter()
                    .zip(b.iter())
                    .all(|(x, y)| structural_match(x, y, out))
        }
        (
            Ty::Function {
                params: ap,
                ret: ar,
            },
            Ty::Function {
                params: bp,
                ret: br,
            },
        ) => {
            ap.len() == bp.len()
                && ap
                    .iter()
                    .zip(bp.iter())
                    .all(|(x, y)| structural_match(x, y, out))
                && structural_match(ar, br, out)
        }
        (a, b) => a == b,
    }
}
