//! Well-formedness of declared types.
//!
//! A generic type written with concrete arguments must satisfy the trait bounds
//! its declaration requires. The inference path only verifies bounds on
//! inference variables; a type written in a declaration (a struct field, a
//! function parameter or return, an enum payload) resolves straight to a
//! concrete `Ty` with no variable, so its arguments were never checked. That is
//! why `Map<Uuid, V>` whose key has no `Hash` impl reached the back end as an
//! unresolved `Uuid$hash` callee instead of a clear error.
//!
//! This pass runs after declaration collection (so every impl, including derived
//! ones, is known) and reports a `BoundNotSatisfied` at the offending type. To
//! stay free of false positives it only judges *simple* concrete arguments,
//! primitives and monomorphic structs/enums, whose impl can be matched exactly;
//! generic arguments (`List<Int>`, a type parameter, an inference variable) are
//! left to the existing call-site machinery.

use super::env::{tys_equal, GenericParamSig, TypeEnv, VariantPayloadSig};
use super::ty::Ty;
use crate::error::{RavenError, TypeError};
use crate::span::Span;

/// Check every declared type in `env`, returning all bound violations.
pub fn check_declared_types(env: &TypeEnv) -> Vec<RavenError> {
    let mut errs = Vec::new();
    for s in env.structs.values() {
        for f in &s.fields {
            push_errs(env, &f.ty, &f.span, &mut errs);
        }
    }
    for e in env.enums.values() {
        for v in &e.variants {
            match &v.payload {
                VariantPayloadSig::Unit => {}
                VariantPayloadSig::Tuple(tys) => {
                    for t in tys {
                        push_errs(env, t, &v.span, &mut errs);
                    }
                }
                VariantPayloadSig::Struct(fields) => {
                    for f in fields {
                        push_errs(env, &f.ty, &f.span, &mut errs);
                    }
                }
            }
        }
    }
    for f in env.functions.values() {
        for p in &f.params {
            push_errs(env, p, &f.span, &mut errs);
        }
        push_errs(env, &f.ret, &f.span, &mut errs);
    }
    for imp in &env.impls {
        for m in imp.methods.values() {
            for p in &m.params {
                push_errs(env, p, &m.span, &mut errs);
            }
            push_errs(env, &m.ret, &m.span, &mut errs);
        }
    }
    errs
}

fn push_errs(env: &TypeEnv, ty: &Ty, span: &Span, errs: &mut Vec<RavenError>) {
    if let Err(e) = check_type(env, ty, span) {
        errs.push(e);
    }
}

/// Recursively check that `ty`'s generic instantiations satisfy their bounds.
/// Returns the first violation found. Used by the declared-type pass and by the
/// body checker for explicit `let`/`const` type annotations.
pub fn check_type(env: &TypeEnv, ty: &Ty, span: &Span) -> Result<(), RavenError> {
    match ty {
        Ty::Struct { id, args, .. } => {
            let generics = env.structs.get(id).map(|s| &s.generics);
            check_instantiation(env, generics, args, span)?;
        }
        Ty::Enum { id, args, .. } => {
            let generics = env.enums.get(id).map(|e| &e.generics);
            check_instantiation(env, generics, args, span)?;
        }
        Ty::List(t) | Ty::Option(t) | Ty::SelfTy(t) => check_type(env, t, span)?,
        Ty::Result(a, b) => {
            check_type(env, a, span)?;
            check_type(env, b, span)?;
        }
        Ty::Function { params, ret } => {
            for p in params {
                check_type(env, p, span)?;
            }
            check_type(env, ret, span)?;
        }
        _ => {}
    }
    Ok(())
}

fn check_instantiation(
    env: &TypeEnv,
    generics: Option<&Vec<GenericParamSig>>,
    args: &[Ty],
    span: &Span,
) -> Result<(), RavenError> {
    if let Some(generics) = generics {
        for (arg, param) in args.iter().zip(generics.iter()) {
            for bound in &param.bounds {
                check_arg_bound(env, arg, bound, span)?;
            }
        }
    }
    for arg in args {
        check_type(env, arg, span)?;
    }
    Ok(())
}

/// Require that `arg` satisfies `bound`, but only when `arg` is a *simple*
/// concrete type whose impl can be matched exactly. Type parameters, inference
/// variables, the error placeholder, and generic instantiations are passed over
/// (the call-site machinery handles those, and matching a generic impl by
/// structural equality would produce false positives).
fn check_arg_bound(env: &TypeEnv, arg: &Ty, bound: &str, span: &Span) -> Result<(), RavenError> {
    let simple = match arg {
        Ty::Error | Ty::Var(_) | Ty::Param(_) => false,
        Ty::Struct { args, .. } | Ty::Enum { args, .. } => args.is_empty(),
        Ty::List(_) | Ty::Option(_) | Ty::Result(_, _) | Ty::Function { .. } => false,
        Ty::SelfTy(_) => false,
        _ => true, // primitives: Int, Float, Bool, Char, Str, ...
    };
    if !simple || type_satisfies(env, arg, bound) {
        return Ok(());
    }
    let ty_name = format!("{}", arg);
    Err(RavenError::ty(
        TypeError::BoundNotSatisfied {
            ty: ty_name.clone(),
            trait_name: bound.to_string(),
        },
        span.clone(),
    )
    .with_hint(format!(
        "`{ty_name}` must implement `{bound}` to be used here; add `@derive({bound})` to its definition, or write an `impl {bound} for {ty_name}`"
    )))
}

/// Whether the environment holds an impl of `trait_name` whose self type equals
/// `concrete` (the same match the checker uses for `implements_trait`).
fn type_satisfies(env: &TypeEnv, concrete: &Ty, trait_name: &str) -> bool {
    env.impls.iter().any(|imp| {
        imp.trait_name.as_deref() == Some(trait_name) && tys_equal(&imp.self_ty, concrete)
    })
}
