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
    FunctionBody, LambdaBody, Stmt, StmtKind, UnaryOp,
};
use crate::error::{RavenError, TypeError};
use crate::resolve::{Binding, ResolvedFile, UseKey};
use crate::span::Span;

use super::builtin;
use super::collect::resolve_ty;
use super::env::{FnSig, ImplSig, TypeEnv};
use super::pattern;
use super::ty::Ty;
use super::unify::{assignable, unify_branches};
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
        DeclKind::Function(f) => check_function(f, None, resolved, env, types),
        DeclKind::Impl(i) => {
            let (impl_path, _) = match &i.for_type {
                Some(t) => (t, Some(())),
                None => (&i.trait_or_type, None),
            };
            let self_ty = resolve_ty(
                &crate::ast::Type {
                    kind: crate::ast::TypeKind::Path(impl_path.clone()),
                    span: impl_path.span.clone(),
                },
                resolved,
                env,
                None,
            )?;
            for f in &i.items {
                check_function(f, Some(&self_ty), resolved, env, types)?;
            }
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
                check_function(m, None, resolved, env, types)?;
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
            expect_assignable(&expected, &actual, &c.value.span)?;
            Ok(())
        }
        DeclKind::Let(l) => {
            let expected = match &l.ty {
                Some(t) => resolve_ty(t, resolved, env, None)?,
                None => Ty::Error,
            };
            if let Some(init) = &l.init {
                let mut cx = Checker::new(resolved, env, types, None, expected.clone());
                let actual = cx.check_expr(init)?;
                if !matches!(expected, Ty::Error) {
                    expect_assignable(&expected, &actual, &init.span)?;
                }
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
    resolved: &ResolvedFile<'_>,
    env: &TypeEnv,
    types: &mut TypeMap,
) -> Result<(), RavenError> {
    let ret_ty = match &f.ret {
        Some(t) => resolve_ty(t, resolved, env, self_ty)?,
        None => Ty::Unit,
    };

    let mut cx = Checker::new(resolved, env, types, self_ty.cloned(), ret_ty.clone());

    // Bind parameters into the local scope. The resolver records
    // `Binding::Param(span)` for parameter sites; we mirror that key.
    for p in &f.params {
        let ty = if p.name == "self" {
            self_ty
                .cloned()
                .map(|t| Ty::SelfTy(Box::new(t)))
                .unwrap_or(Ty::Error)
        } else {
            resolve_ty(&p.ty, resolved, env, self_ty)?
        };
        cx.locals.insert(BindingKey::param(&p.span), ty);
    }

    match &f.body {
        FunctionBody::Block(b) => {
            let body_ty = cx.check_block(b)?;
            // A block body with a trailing expression must match the
            // declared return type. Without a trailing expression the
            // block evaluates to `()`; that is fine when the return
            // type is `()` too, and when it is not, the body is
            // expected to exit via `return` (whose statement is
            // checked against `ret_ty` in `check_stmt`).
            if b.trailing.is_some() {
                expect_assignable(&ret_ty, &body_ty, &b.span)?;
            } else if !matches!(ret_ty, Ty::Unit | Ty::Error) {
                // No trailing expression and a non unit return type.
                // Acceptable as long as the body contains explicit
                // returns; we do not analyze control flow here.
            }
        }
        FunctionBody::Expr(e) => {
            let body_ty = cx.check_expr(e)?;
            expect_assignable(&ret_ty, &body_ty, &e.span)?;
        }
        FunctionBody::None => {}
    }
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
        }
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
                    Some(t) => Some(resolve_ty(
                        t,
                        self.resolved,
                        self.env,
                        self.self_ty.as_ref(),
                    )?),
                    None => None,
                };
                let init_ty = match init {
                    Some(e) => Some(self.check_expr(e)?),
                    None => None,
                };
                let final_ty = match (declared, init_ty) {
                    (Some(d), Some(i)) => {
                        expect_assignable(&d, &i, &init.as_ref().unwrap().span)?;
                        d
                    }
                    (Some(d), None) => d,
                    (None, Some(i)) => i,
                    (None, None) => Ty::Error,
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
                expect_assignable(&ret, &actual, &stmt.span)?;
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
                        expect_assignable(&target_ty, &value_ty, &value.span)?;
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
            ExprKind::Str(_) | ExprKind::BlockStr(_) | ExprKind::CStr(_) => Ok(Ty::Str),
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
            ExprKind::Try(_) => Err(RavenError::ty(
                TypeError::Custom(
                    "`?` operator is not yet supported (HIR lowering, issue #60)".into(),
                ),
                expr.span.clone(),
            )),
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
                expect_assignable(&Ty::Bool, &c, &cond.span)?;
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
        // Built in variants `None`, `Some`, `Ok`, `Err` are recognized
        // when the resolver could not find them in scope. Those names
        // are not in any scope at the moment, so they'd fail in the
        // resolver before reaching here. To allow the user to write
        // `None` or `Some(x)`, the resolver would need to know about
        // them; we handle that by looking up the binding and falling
        // through to a builtin recognizer if the resolver did not
        // record one.
        if let Some(binding) = self.resolved.map.lookup(span) {
            for g in generics {
                let _ = resolve_ty(g, self.resolved, self.env, self.self_ty.as_ref())?;
            }
            self.type_of_binding(binding, span)
        } else if let Some(t) = recognize_constructor_ident(name) {
            Ok(t)
        } else {
            Err(RavenError::ty(
                TypeError::Custom(format!("identifier `{}` has no type binding", name)),
                span.clone(),
            ))
        }
    }

    fn type_of_binding(&self, binding: &Binding, span: &Span) -> Result<Ty, RavenError> {
        match binding {
            Binding::Function(id) => {
                let sig = self
                    .env
                    .functions
                    .get(id)
                    .ok_or_else(|| ty_custom("function signature missing", span))?;
                Ok(Ty::Function {
                    params: sig.params.clone(),
                    ret: Box::new(sig.ret.clone()),
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
                // Bare enum name in expression position is only useful
                // as a path head (`Color.Red`); treat the type as the
                // enum itself for now.
                let e = self
                    .env
                    .enums
                    .get(id)
                    .ok_or_else(|| ty_custom("enum signature missing", span))?;
                Ok(Ty::Enum {
                    id: *id,
                    name: e.name.clone(),
                    args: Vec::new(),
                })
            }
            Binding::Struct(id) => {
                let s = self
                    .env
                    .structs
                    .get(id)
                    .ok_or_else(|| ty_custom("struct signature missing", span))?;
                Ok(Ty::Struct {
                    id: *id,
                    name: s.name.clone(),
                    args: Vec::new(),
                })
            }
            Binding::Trait(_) => Err(ty_custom(
                "trait values are not first class without `dyn` (deferred to issue #66)",
                span,
            )),
            Binding::GenericParam { .. } => Err(RavenError::ty(
                TypeError::GenericsNotYetSupported,
                span.clone(),
            )),
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
            expect_assignable(&first, &t, &it.span)?;
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
                expect_assignable(&Ty::Bool, &t, &operand.span)?;
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
            expect_assignable(param_ty, &a, &arg.span)?;
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
            other => Err(RavenError::ty(
                TypeError::Custom(format!("identifier `{}` has no type binding", other)),
                span.clone(),
            )),
        }
    }

    fn check_method_call(
        &mut self,
        receiver: &Expr,
        name: &str,
        args: &[Expr],
        span: &Span,
    ) -> Result<Ty, RavenError> {
        let recv_ty = self.check_expr(receiver)?;
        let recv_stripped = recv_ty.strip_self().clone();
        if matches!(recv_stripped, Ty::Error) {
            // Eat the cascade.
            for a in args {
                self.check_expr(a)?;
            }
            return Ok(Ty::Error);
        }

        // Built in methods first (Option/Result/List/String).
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
                expect_assignable(pt, &a, &arg.span)?;
            }
            return Ok(ret);
        }

        // Inherent impl methods.
        let inherents: Vec<&ImplSig> = self
            .env
            .impls
            .iter()
            .filter(|i| {
                i.trait_name.is_none()
                    && super::env::tys_equal(&i.self_ty, &recv_stripped)
                    && i.methods.contains_key(name)
            })
            .collect();
        // Trait impl methods.
        let traits: Vec<&ImplSig> = self
            .env
            .impls
            .iter()
            .filter(|i| {
                i.trait_name.is_some()
                    && super::env::tys_equal(&i.self_ty, &recv_stripped)
                    && i.methods.contains_key(name)
            })
            .collect();

        let candidates: Vec<&FnSig> = inherents
            .iter()
            .chain(traits.iter())
            .filter_map(|i| i.methods.get(name))
            .collect();
        match candidates.len() {
            0 => Err(RavenError::ty(
                TypeError::UndefinedMethod {
                    receiver_ty: format!("{}", recv_stripped),
                    method: name.to_string(),
                },
                span.clone(),
            )),
            1 => {
                let sig = candidates[0];
                self.check_method_args(sig, args, name, span)?;
                Ok(sig.ret.clone())
            }
            _ => {
                // Ambiguous: list the trait names that provide it.
                let candidate_names: Vec<String> = inherents
                    .iter()
                    .map(|_| "<inherent>".to_string())
                    .chain(
                        traits
                            .iter()
                            .map(|i| i.trait_name.clone().unwrap_or_default()),
                    )
                    .collect();
                Err(RavenError::ty(
                    TypeError::AmbiguousMethod {
                        receiver_ty: format!("{}", recv_stripped),
                        method: name.to_string(),
                        candidates: candidate_names,
                    },
                    span.clone(),
                ))
            }
        }
    }

    fn check_method_args(
        &mut self,
        sig: &FnSig,
        args: &[Expr],
        name: &str,
        span: &Span,
    ) -> Result<(), RavenError> {
        // `params` includes the `self` slot when present. We skip it
        // for arity checking against caller arguments.
        let user_params: Vec<&Ty> = sig
            .params
            .iter()
            .filter(|t| !matches!(t, Ty::SelfTy(_)))
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
            expect_assignable(pt, &a, &arg.span)?;
        }
        Ok(())
    }

    fn check_field(&mut self, receiver: &Expr, name: &str, span: &Span) -> Result<Ty, RavenError> {
        let recv = self.check_expr(receiver)?;
        let stripped = recv.strip_self().clone();
        match stripped {
            Ty::Struct { id, name: sname, .. } => {
                let sig = self
                    .env
                    .structs
                    .get(&id)
                    .ok_or_else(|| ty_custom("struct signature missing", span))?;
                match sig.field(name) {
                    Some((_, ty)) => Ok(ty.clone()),
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
        expect_assignable(&Ty::Int, &idx, &index.span)?;
        match recv.strip_self() {
            Ty::List(t) => Ok(*t.clone()),
            Ty::Str => Ok(Ty::Char),
            Ty::Error => Ok(Ty::Error),
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
        expect_assignable(&Ty::Bool, &c, &cond.span)?;
        let t = self.check_block(then_branch)?;
        let e = match else_branch {
            None => Ty::Unit,
            Some(ElseBranch::If(expr)) => self.check_expr(expr)?,
            Some(ElseBranch::Block(b)) => self.check_block(b)?,
        };
        unify_branches(&t, &e).ok_or_else(|| {
            RavenError::ty(
                TypeError::TypeMismatch {
                    expected: format!("{}", t),
                    actual: format!("{}", e),
                },
                span.clone(),
            )
        })
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
                expect_assignable(&Ty::Bool, &gt, &g.span)?;
            }

            let body_ty = self.check_expr(&arm.body)?;
            self.locals = saved_locals;

            result_ty = Some(match result_ty.take() {
                None => body_ty,
                Some(prev) => unify_branches(&prev, &body_ty).ok_or_else(|| {
                    RavenError::ty(
                        TypeError::TypeMismatch {
                            expected: format!("{}", prev),
                            actual: format!("{}", body_ty),
                        },
                        arm.span.clone(),
                    )
                })?,
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
                Some(t) => resolve_ty(t, self.resolved, self.env, self.self_ty.as_ref())?,
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
            Some(t) => Some(resolve_ty(
                t,
                self.resolved,
                self.env,
                self.self_ty.as_ref(),
            )?),
            None => None,
        };
        let body_ty = match body {
            LambdaBody::Block(b) => self.check_block(b)?,
            LambdaBody::Expr(e) => self.check_expr(e)?,
        };
        let final_ret = match declared_ret {
            Some(d) => {
                expect_assignable(
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
            .ok_or_else(|| ty_custom("struct signature missing", span))?;

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
            let value_ty = self.check_expr(&fi.value)?;
            expect_assignable(field_ty, &value_ty, &fi.value.span)?;
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
        Ok(Ty::Struct {
            id,
            name: sig.name.clone(),
            args: Vec::new(),
        })
    }

    fn record(&mut self, span: &Span, ty: Ty) {
        self.types.types.insert(UseKey::from_span(span), ty);
    }
}

/// True if `actual` is acceptable where `expected` is wanted; otherwise
/// a `TypeMismatch` is raised at `span`.
fn expect_assignable(expected: &Ty, actual: &Ty, span: &Span) -> Result<(), RavenError> {
    if assignable(expected, actual) {
        return Ok(());
    }
    let mut err = RavenError::ty(
        TypeError::TypeMismatch {
            expected: format!("{}", expected),
            actual: format!("{}", actual),
        },
        span.clone(),
    );
    if matches!(expected, Ty::Int) && matches!(actual, Ty::Float) {
        err = err.with_hint("did you mean to call `.to_int()`?");
    } else if matches!(expected, Ty::Float) && matches!(actual, Ty::Int) {
        err = err.with_hint("did you mean to call `.to_float()`?");
    }
    Err(err)
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
