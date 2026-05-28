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
use super::env::{FnSig, GenericParamSig, TypeEnv};
use super::infer::{substitute, InferCtx};
use super::pattern;
use super::ty::{FfiTy, ParamId, Ty};
use super::unify::assignable;
use super::TypeMap;

/// Walk every function body and module level expression in `resolved`,
/// recording each expression's inferred type in `types`.
pub fn check_bodies(
    resolved: &ResolvedFile<'_>,
    env: &TypeEnv,
    types: &mut TypeMap,
) -> Result<(), RavenError> {
    for decl in &resolved.file.items {
        check_decl_body(decl, resolved, env, types)?;
    }
    Ok(())
}

fn check_decl_body(
    decl: &Decl,
    resolved: &ResolvedFile<'_>,
    env: &TypeEnv,
    types: &mut TypeMap,
) -> Result<(), RavenError> {
    match &decl.kind {
        DeclKind::Function(f) => check_function(f, None, &[], resolved, env, types),
        DeclKind::Impl(i) => {
            let impl_id_idx = resolved
                .file
                .items
                .iter()
                .position(|d| std::ptr::eq(d, decl))
                .unwrap_or(0);
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
            let self_ty = resolve_ty(
                &crate::ast::Type {
                    kind: crate::ast::TypeKind::Path(impl_path.clone()),
                    span: impl_path.span.clone(),
                },
                resolved,
                env,
                None,
                &scope,
            )?;
            for f in &i.items {
                check_function(f, Some(&self_ty), &impl_generics, resolved, env, types)?;
            }
            let _ = impl_id_idx;
            Ok(())
        }
        DeclKind::Trait(t) => {
            // Default bodies in trait declarations: walk them without
            // a concrete Self because we treat `Self` as an error
            // marker for now; trait default bodies that reference Self
            // remain limited in this release.
            for m in &t.members {
                if matches!(m.body, FunctionBody::None) {
                    continue;
                }
                check_function(m, None, &[], resolved, env, types)?;
            }
            Ok(())
        }
        DeclKind::Const(c) => {
            let expected = env
                .consts
                .get(&const_id_of(decl, resolved))
                .cloned()
                .unwrap_or(Ty::Error);
            let mut cx = Checker::new(resolved, env, types, None, expected.clone());
            let actual = cx.check_expr(&c.value)?;
            if !matches!(expected, Ty::Error) {
                cx.unify(&expected, &actual, &c.value.span)?;
            }
            cx.finalize_types()?;
            Ok(())
        }
        DeclKind::Let(l) => {
            let scope = GenericScope::new();
            let expected = match &l.ty {
                Some(t) => resolve_ty(t, resolved, env, None, &scope)?,
                None => Ty::Error,
            };
            if let Some(init) = &l.init {
                let mut cx = Checker::new(resolved, env, types, None, expected.clone());
                let actual = cx.check_expr(init)?;
                if !matches!(expected, Ty::Error) {
                    cx.unify(&expected, &actual, &init.span)?;
                }
                cx.finalize_types()?;
            }
            Ok(())
        }
        DeclKind::Struct(_) | DeclKind::Enum(_) | DeclKind::Extern(_) | DeclKind::Import(_) => {
            Ok(())
        }
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
) -> Result<(), RavenError> {
    // Build a generic scope: enclosing impl generics, then this
    // function's own generics.
    let fn_generics = super::collect::scope_from_params(extra_generics);
    // Layer the function's own generics on top.
    let f_params = super::collect::collect_generic_params_for_owner(&f.generics, &f.span);
    let mut full_scope = fn_generics;
    super::collect::push_into_scope(&mut full_scope, &f_params);

    let ret_ty = match &f.ret {
        Some(t) => resolve_ty(t, resolved, env, self_ty, &full_scope)?,
        None => Ty::Unit,
    };

    let mut cx =
        Checker::new(resolved, env, types, self_ty.cloned(), ret_ty.clone()).with_scope(full_scope);

    // Bind parameters into the local scope. The resolver records
    // `Binding::Param(span)` for parameter sites; we mirror that key.
    for p in &f.params {
        let ty = if p.name == "self" {
            self_ty
                .cloned()
                .map(|t| Ty::SelfTy(Box::new(t)))
                .unwrap_or(Ty::Error)
        } else {
            cx.resolve_ast_ty(&p.ty)?
        };
        cx.locals.insert(BindingKey::param(&p.span), ty);
    }

    match &f.body {
        FunctionBody::Block(b) => {
            let body_ty = cx.check_block(b)?;
            if b.trailing.is_some() {
                cx.unify(&ret_ty, &body_ty, &b.span)?;
            } else if !matches!(ret_ty, Ty::Unit | Ty::Error) {
                // No trailing expression and a non unit return type.
                // Acceptable as long as the body contains explicit
                // returns; we do not analyze control flow here.
            }
        }
        FunctionBody::Expr(e) => {
            let body_ty = cx.check_expr(e)?;
            cx.unify(&ret_ty, &body_ty, &e.span)?;
        }
        FunctionBody::None => {}
    }
    cx.finalize_types()?;
    Ok(())
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
    /// Inference context for this body. Holds variables, their
    /// solutions, and any pending trait bounds.
    infer: InferCtx,
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
        Self {
            resolved,
            env,
            types,
            self_ty,
            return_ty,
            locals: HashMap::new(),
            generic_scope: GenericScope::new(),
            infer: InferCtx::new(),
        }
    }

    fn with_scope(mut self, scope: GenericScope) -> Self {
        self.generic_scope = scope;
        self
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

    /// After body checking, walk every recorded type and resolve any
    /// inference variables. Unsolved variables surface as
    /// `CannotInferType` errors. Also resolves locals so subsequent
    /// stages see concrete types.
    fn finalize_types(&mut self) -> Result<(), RavenError> {
        // Resolve every entry in the type map. We do this in place by
        // replacing each entry's value with its resolved form, raising
        // CannotInferType if a variable remains.
        let keys: Vec<crate::resolve::UseKey> = self.types.types.keys().cloned().collect();
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
        if let Some(e) = first_err {
            return Err(e);
        }
        Ok(())
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
        let mut out: HashMap<ParamId, Ty> = HashMap::new();
        for (i, p) in generics.iter().enumerate() {
            let v = self.infer.fresh(span.clone());
            for b in &p.bounds {
                self.infer.add_bound(v, b.clone(), span.clone());
            }
            let assigned = Ty::Var(v);
            if let Some(explicit) = explicit_args.get(i) {
                self.infer.unify(&assigned, explicit, span)?;
            }
            out.insert(p.id.clone(), assigned);
        }
        Ok(out)
    }

    fn check_block(&mut self, block: &Block) -> Result<Ty, RavenError> {
        for stmt in &block.stmts {
            self.check_stmt(stmt)?;
        }
        let ty = match &block.trailing {
            Some(e) => self.check_expr(e)?,
            None => Ty::Unit,
        };
        self.record(&block.span, ty.clone());
        Ok(ty)
    }

    fn check_stmt(&mut self, stmt: &Stmt) -> Result<(), RavenError> {
        match &stmt.kind {
            StmtKind::Let { name: _, ty, init } => {
                let declared = match ty {
                    Some(t) => Some(self.resolve_ast_ty(t)?),
                    None => None,
                };
                let init_ty = match init {
                    Some(e) => Some(self.check_expr(e)?),
                    None => None,
                };
                let final_ty = match (declared, init_ty) {
                    (Some(d), Some(i)) => {
                        self.unify(&d, &i, &init.as_ref().unwrap().span)?;
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
                    Some(expr) => self.check_expr(expr)?,
                    None => Ty::Unit,
                };
                let ret = self.return_ty.clone();
                self.unify(&ret, &actual, &stmt.span)?;
            }
            StmtKind::Break(e) => {
                if let Some(expr) = e {
                    self.check_expr(expr)?;
                }
            }
            StmtKind::Continue => {}
            StmtKind::Defer(e) => {
                self.check_expr(e)?;
            }
            StmtKind::Assign { target, op, value } => {
                let target_ty = self.check_expr(target)?;
                let value_ty = self.check_expr(value)?;
                match op {
                    AssignOp::Assign => {
                        self.unify(&target_ty, &value_ty, &value.span)?;
                    }
                    _ => {
                        // Compound: target op= value behaves like
                        // target = target op value. Reuse the binary
                        // op checker to pin the rule down.
                        let bin = compound_binary_op(*op);
                        let _ = super::expr::check_binary(&target_ty, &value_ty, bin, &stmt.span)?;
                    }
                }
            }
            StmtKind::Expr(e) => {
                self.check_expr(e)?;
            }
        }
        Ok(())
    }

    fn check_expr(&mut self, expr: &Expr) -> Result<Ty, RavenError> {
        let ty = self.check_expr_inner(expr)?;
        self.record(&expr.span, ty.clone());
        Ok(ty)
    }

    fn check_expr_inner(&mut self, expr: &Expr) -> Result<Ty, RavenError> {
        match &expr.kind {
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
                args,
                ..
            } => self.check_method_call(receiver, name, args, &expr.span),
            ExprKind::Field { receiver, name } => self.check_field(receiver, name, &expr.span),
            ExprKind::Index { receiver, index } => self.check_index(receiver, index, &expr.span),
            ExprKind::Try(inner) => {
                let inner_ty = self.check_expr(inner)?;
                let resolved = self.infer.resolve(&inner_ty);
                match resolved {
                    Ty::Result(t, _) => Ok(*t),
                    Ty::Option(t) => Ok(*t),
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
                self.check_block(b)?;
                Ok(Ty::Unit)
            }
            ExprKind::While { cond, body } => {
                let c = self.check_expr(cond)?;
                self.unify(&Ty::Bool, &c, &cond.span)?;
                self.check_block(body)?;
                Ok(Ty::Unit)
            }
            ExprKind::For {
                pattern: pat,
                iter,
                body,
            } => {
                let iter_ty = self.check_expr(iter)?;
                let elem = match iter_ty.strip_self() {
                    Ty::List(t) => *t.clone(),
                    Ty::Error => Ty::Error,
                    other => {
                        return Err(RavenError::ty(
                            TypeError::Custom(format!(
                                "cannot iterate over `{}`; expected a `List<T>`",
                                other
                            )),
                            iter.span.clone(),
                        ));
                    }
                };
                pattern::bind(pat, &elem, self.env, &mut self.locals)?;
                self.check_block(body)?;
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
        if let Some(binding) = self.resolved.map.lookup(span).cloned() {
            // Resolve any explicit type arguments first. We pass them
            // through to the instantiation step below.
            let mut explicit_args = Vec::with_capacity(generics.len());
            for g in generics {
                explicit_args.push(self.resolve_ast_ty(g)?);
            }
            self.type_of_binding(&binding, span, &explicit_args)
        } else if let Some(t) = recognize_constructor_ident(name) {
            Ok(t)
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
            UnaryOp::Ref => Ok(t),
        }
    }

    fn check_call(&mut self, callee: &Expr, args: &[Expr], span: &Span) -> Result<Ty, RavenError> {
        // Special case: enum variant construction via a bare ident
        // resolved to an `Enum` binding plus a chained `.variant`
        // method shape would need path parsing. For our scope, the
        // only call form we recognize for variants is `Some(x)`,
        // `Ok(x)`, `Err(x)` which arrive as `Call { callee: Ident, .. }`
        // without a resolver binding.
        if let ExprKind::Ident { name, .. } = &callee.kind {
            if self.resolved.map.lookup(&callee.span).is_none() {
                return self.check_builtin_constructor_call(name, args, span);
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
        if params.len() != args.len() {
            return Err(RavenError::ty(
                TypeError::WrongArity {
                    func: describe_callee(callee),
                    expected: params.len(),
                    actual: args.len(),
                },
                span.clone(),
            ));
        }
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
            self.unify(param_ty, &a, &arg.span)?;
        }
        Ok(ret)
    }

    fn check_builtin_constructor_call(
        &mut self,
        name: &str,
        args: &[Expr],
        span: &Span,
    ) -> Result<Ty, RavenError> {
        match name {
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
                // Built in `print(s: String)` intrinsic. The codegen
                // backend recognizes the mangled name and emits a call
                // to the runtime's `raven_println_str` ABI symbol.
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
                self.unify(&Ty::Str, &arg_ty, &args[0].span)?;
                Ok(Ty::Unit)
            }
            "print_int" => {
                // Built in `print_int(n: Int)` intrinsic. The codegen
                // backend recognizes the mangled name and emits a call
                // to the runtime's `raven_println_int` ABI symbol so a
                // program can observe a computed integer. The integer C
                // FFI types (`CInt`, `CLong`, `CSize`) are also accepted
                // so the result of a C call (for example `strlen`) can be
                // printed directly; the back end widens narrower ones to
                // the i64 the runtime expects.
                if args.len() != 1 {
                    return Err(RavenError::ty(
                        TypeError::WrongArity {
                            func: "print_int".into(),
                            expected: 1,
                            actual: args.len(),
                        },
                        span.clone(),
                    ));
                }
                let arg_ty = self.check_expr(&args[0])?;
                let resolved = self.infer.resolve(&arg_ty);
                let is_int_ffi = matches!(
                    resolved.strip_self(),
                    Ty::Ffi(FfiTy::CInt) | Ty::Ffi(FfiTy::CLong) | Ty::Ffi(FfiTy::CSize)
                );
                if !is_int_ffi {
                    self.unify(&Ty::Int, &arg_ty, &args[0].span)?;
                }
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
        args: &[Expr],
        span: &Span,
    ) -> Result<Ty, RavenError> {
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

        // Built in methods first (Option/Result/List/String). These are
        // matched directly against the resolved receiver shape; their
        // signatures already substitute the element type.
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

        // Gather candidate impls. For each impl, allocate fresh
        // inference variables for its declared generic parameters and
        // unify the substituted self_ty against the receiver type. An
        // impl is a candidate when unification succeeds and the method
        // name exists in its method table.
        let impls_snapshot = self.env.impls.clone();
        let mut inherent_matches: Vec<(usize, FnSig, HashMap<ParamId, Ty>)> = Vec::new();
        let mut trait_matches: Vec<(usize, FnSig, HashMap<ParamId, Ty>, String)> = Vec::new();
        for (idx, imp) in impls_snapshot.iter().enumerate() {
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
            // Method must exist on this impl.
            let Some(msig) = imp.methods.get(name) else {
                continue;
            };
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
            return Err(RavenError::ty(
                TypeError::UndefinedMethod {
                    receiver_ty: format!("{}", recv_stripped),
                    method: name.to_string(),
                },
                span.clone(),
            ));
        }
        // Prefer inherent over trait if both exist.
        if !inherent_matches.is_empty() && inherent_matches.len() == 1 {
            let (_, sig, subst) = inherent_matches.into_iter().next().unwrap();
            return self.apply_method_call(&sig, &subst, args, name, span);
        }
        if inherent_matches.is_empty() && trait_matches.len() == 1 {
            let (_, sig, subst, _) = trait_matches.into_iter().next().unwrap();
            return self.apply_method_call(&sig, &subst, args, name, span);
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

    fn apply_method_call(
        &mut self,
        sig: &FnSig,
        subst: &HashMap<ParamId, Ty>,
        args: &[Expr],
        name: &str,
        span: &Span,
    ) -> Result<Ty, RavenError> {
        // Build a per-call substitution: impl generics already in
        // `subst`, plus fresh variables for method generics.
        let mut full = subst.clone();
        for p in &sig.generics {
            let v = self.infer.fresh(span.clone());
            for b in &p.bounds {
                self.infer.add_bound(v, b.clone(), span.clone());
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

    fn check_field(&mut self, receiver: &Expr, name: &str, span: &Span) -> Result<Ty, RavenError> {
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
            None => Ty::Unit,
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
        let scrut_stripped = scrut_ty.strip_self().clone();

        let mut result_ty: Option<Ty> = None;
        let mut pattern_names: Vec<super::match_check::PatternHead> = Vec::new();

        for arm in arms {
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
            pattern_names.push(super::match_check::pattern_head(&arm.pattern));
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
        let body_ty = match body {
            LambdaBody::Block(b) => self.check_block(b)?,
            LambdaBody::Expr(e) => self.check_expr(e)?,
        };
        let final_ret = match declared_ret {
            Some(d) => {
                self.unify(
                    &d,
                    &body_ty,
                    match body {
                        LambdaBody::Block(b) => &b.span,
                        LambdaBody::Expr(e) => &e.span,
                    },
                )?;
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
        self.types.types.insert(UseKey::from_span(span), ty);
    }

    /// Type an interpolated string literal. The whole literal has type
    /// `String`. Every embedded `${expr}` must resolve to a type that
    /// can be converted to a string: `String` (identity), `Int`, `Bool`,
    /// `Float`, or `Char`. Any other type is rejected with a hint that
    /// points the user at converting to a `String` first.
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
            if !is_interpolatable(stripped) {
                return Err(RavenError::ty(
                    TypeError::Custom(format!(
                        "values of type `{}` cannot be interpolated into a string",
                        resolved
                    )),
                    e.span.clone(),
                )
                .with_hint(format!(
                    "only `String`, `Int`, `Bool`, `Float`, and `Char` interpolate today; convert the `{}` value to a `String` first",
                    resolved
                )));
            }
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
            (Ty::Int, Ty::Int) | (Ty::Float, Ty::Float) | (Ty::Char, Ty::Char) => Ok(Ty::Bool),
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

fn ty_custom(msg: &str, span: &Span) -> RavenError {
    RavenError::ty(TypeError::Custom(msg.into()), span.clone())
}

/// Recognize the bare constructor identifier `None` when the resolver
/// could not bind it. `Some`, `Ok`, `Err` reach the type checker as
/// calls; only `None` is a bare identifier.
fn recognize_constructor_ident(name: &str) -> Option<Ty> {
    match name {
        "None" => Some(Ty::Option(Box::new(Ty::Error))),
        _ => None,
    }
}
