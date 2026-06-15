//! Pattern checking and binding extraction.
//!
//! Patterns appear in `match` arms, `for` heads, and `let` bindings.
//! This module validates that a pattern is compatible with the
//! scrutinee's type and binds the names the pattern introduces.

use std::collections::HashMap;

use crate::ast::{LiteralPattern, Pattern, PatternKind};
use crate::error::{RavenError, TypeError};

use super::env::{TypeEnv, VariantPayloadSig};
use super::expr::BindingKey;
use super::infer::substitute;
use super::ty::{ParamId, Ty};

/// Bind every name introduced by `pat` into `locals`, using `scrut_ty`
/// as the scrutinee's type.
///
/// The function checks that literal patterns and constructor patterns
/// are compatible with the scrutinee's type and surfaces a
/// `TypeMismatch` if they are not.
pub fn bind(
    pat: &Pattern,
    scrut_ty: &Ty,
    env: &TypeEnv,
    locals: &mut HashMap<BindingKey, Ty>,
) -> Result<(), RavenError> {
    match &pat.kind {
        PatternKind::Wildcard => Ok(()),
        PatternKind::Literal(lit) => {
            let lit_ty = literal_type(lit);
            if !super::unify::assignable(scrut_ty, &lit_ty) && !scrut_ty.is_error() {
                return Err(RavenError::ty(
                    TypeError::TypeMismatch {
                        expected: format!("{}", scrut_ty),
                        actual: format!("{}", lit_ty),
                    },
                    pat.span.clone(),
                ));
            }
            Ok(())
        }
        PatternKind::Range { .. } => {
            if !matches!(scrut_ty.strip_self(), Ty::Int | Ty::Error) {
                return Err(RavenError::ty(
                    TypeError::TypeMismatch {
                        expected: format!("{}", scrut_ty),
                        actual: "Int range".into(),
                    },
                    pat.span.clone(),
                ));
            }
            Ok(())
        }
        PatternKind::Ident(name) => {
            // Bare identifier: a nullary constructor when the scrutinee
            // is an enum with a unit variant by that name, otherwise a
            // fresh binding.
            if is_nullary_constructor(name, scrut_ty, env) {
                return Ok(());
            }
            locals.insert(BindingKey::pattern(&pat.span), scrut_ty.clone());
            Ok(())
        }
        PatternKind::Tuple { name, elements } => match (name.as_deref(), scrut_ty.strip_self()) {
            (Some("Some"), Ty::Option(t)) => {
                ensure_arity(name.as_deref().unwrap(), 1, elements.len(), &pat.span)?;
                bind(&elements[0], t, env, locals)
            }
            (Some("Ok"), Ty::Result(t, _)) => {
                ensure_arity(name.as_deref().unwrap(), 1, elements.len(), &pat.span)?;
                bind(&elements[0], t, env, locals)
            }
            (Some("Err"), Ty::Result(_, e)) => {
                ensure_arity(name.as_deref().unwrap(), 1, elements.len(), &pat.span)?;
                bind(&elements[0], e, env, locals)
            }
            (Some(variant), Ty::Enum { id, args, .. }) => {
                let sig = env.enums.get(id).ok_or_else(|| {
                    RavenError::ty(
                        TypeError::Custom("enum signature missing".into()),
                        pat.span.clone(),
                    )
                })?;
                // Build a substitution from declared params to args.
                let mut subst: HashMap<ParamId, Ty> = HashMap::new();
                for (p, a) in sig.generics.iter().zip(args.iter()) {
                    subst.insert(p.id.clone(), a.clone());
                }
                let (_, v) = sig.variant(variant).ok_or_else(|| {
                    RavenError::ty(
                        TypeError::Custom(format!(
                            "enum `{}` has no variant `{}`",
                            sig.name, variant
                        )),
                        pat.span.clone(),
                    )
                })?;
                match &v.payload {
                    VariantPayloadSig::Tuple(tys) => {
                        if tys.len() != elements.len() {
                            return Err(RavenError::ty(
                                TypeError::Custom(format!(
                                    "variant `{}` expects {} payload(s), got {}",
                                    variant,
                                    tys.len(),
                                    elements.len()
                                )),
                                pat.span.clone(),
                            ));
                        }
                        for (sub_pat, sub_ty) in elements.iter().zip(tys.iter()) {
                            let substituted = substitute(sub_ty, &subst);
                            bind(sub_pat, &substituted, env, locals)?;
                        }
                        Ok(())
                    }
                    VariantPayloadSig::Unit => {
                        if !elements.is_empty() {
                            return Err(RavenError::ty(
                                TypeError::Custom(format!("variant `{}` has no payload", variant)),
                                pat.span.clone(),
                            ));
                        }
                        Ok(())
                    }
                    VariantPayloadSig::Struct(_) => Err(RavenError::ty(
                        TypeError::Custom(format!(
                            "variant `{}` has named fields, use a struct pattern",
                            variant
                        )),
                        pat.span.clone(),
                    )),
                }
            }
            // Suppress a cascade: a constructor pattern over an already-failed
            // scrutinee binds its parts to Error without a second diagnostic.
            (Some(_), Ty::Error) => {
                for sub_pat in elements {
                    bind(sub_pat, &Ty::Error, env, locals)?;
                }
                Ok(())
            }
            // A named constructor pattern (`Nope(x)`) over a non-enum value (an
            // `Int`, a `List`, ...) is a type error, not a silent Error binding
            // that would otherwise reach codegen and crash Cranelift.
            (Some(ctor), other) => Err(RavenError::ty(
                TypeError::Custom(format!(
                    "`{}` is not a constructor of `{}`; a constructor pattern needs an enum, Option, or Result value",
                    ctor, other
                )),
                pat.span.clone(),
            )),
            _ => {
                // A nameless tuple pattern; bind sub patterns as Error to
                // suppress cascading errors.
                for sub_pat in elements {
                    bind(sub_pat, &Ty::Error, env, locals)?;
                }
                Ok(())
            }
        },
        PatternKind::Struct { name, fields } => {
            if let Ty::Struct { id, .. } = scrut_ty.strip_self() {
                if let Some(sig) = env.structs.get(id) {
                    if sig.name != *name {
                        return Err(RavenError::ty(
                            TypeError::TypeMismatch {
                                expected: sig.name.clone(),
                                actual: name.clone(),
                            },
                            pat.span.clone(),
                        ));
                    }
                    for fpat in fields {
                        let field_ty =
                            sig.field(&fpat.name)
                                .map(|(_, t)| t.clone())
                                .ok_or_else(|| {
                                    RavenError::ty(
                                        TypeError::UndefinedField {
                                            struct_name: sig.name.clone(),
                                            field: fpat.name.clone(),
                                        },
                                        fpat.span.clone(),
                                    )
                                })?;
                        if let Some(p) = &fpat.pattern {
                            bind(p, &field_ty, env, locals)?;
                        } else {
                            locals.insert(BindingKey::pattern(&fpat.span), field_ty);
                        }
                    }
                    return Ok(());
                }
            }
            // A struct pattern whose scrutinee is an enum is a named-field
            // variant match (`Point { x, y }`). Struct-variants are only
            // partially implemented (construction is rejected too), and the MIR
            // path crashes Cranelift on this pattern, so reject it up front with
            // a diagnostic instead of binding fields to Error and reaching
            // codegen. (Matching by position with `Point(x, y)` works.)
            if let Ty::Enum { id, .. } = scrut_ty.strip_self() {
                if env.enums.contains_key(id) {
                    return Err(RavenError::ty(
                        TypeError::Custom(format!(
                            "matching the named-field variant `{}` with `{{ ... }}` is not yet supported; match it positionally as `{}(...)`",
                            name, name
                        )),
                        pat.span.clone(),
                    ));
                }
            }
            // Fallthrough (for example an already-failed scrutinee): bind to
            // Error to suppress a cascade.
            for fpat in fields {
                if let Some(p) = &fpat.pattern {
                    bind(p, &Ty::Error, env, locals)?;
                }
            }
            Ok(())
        }
    }
}

fn ensure_arity(
    ctor: &str,
    expected: usize,
    actual: usize,
    span: &crate::span::Span,
) -> Result<(), RavenError> {
    if expected == actual {
        Ok(())
    } else {
        Err(RavenError::ty(
            TypeError::Custom(format!(
                "`{}` expects {} payload(s), got {}",
                ctor, expected, actual
            )),
            span.clone(),
        ))
    }
}

fn literal_type(lit: &LiteralPattern) -> Ty {
    match lit {
        LiteralPattern::Int(_) => Ty::Int,
        LiteralPattern::Float(_) => Ty::Float,
        LiteralPattern::Bool(_) => Ty::Bool,
        LiteralPattern::String(_) => Ty::Str,
        LiteralPattern::Char(_) => Ty::Char,
    }
}

fn is_nullary_constructor(name: &str, scrut_ty: &Ty, env: &TypeEnv) -> bool {
    if name == "None" && matches!(scrut_ty.strip_self(), Ty::Option(_)) {
        return true;
    }
    if let Ty::Enum { id, .. } = scrut_ty.strip_self() {
        if let Some(sig) = env.enums.get(id) {
            if let Some((_, v)) = sig.variant(name) {
                return matches!(v.payload, VariantPayloadSig::Unit);
            }
        }
    }
    false
}
