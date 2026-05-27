//! Match arm exhaustiveness analysis.
//!
//! For each known closed type (`Bool`, user enums, `Option`, `Result`),
//! the arms must cover every variant or include a wildcard. For other
//! types a wildcard is required.

use crate::ast::{Pattern, PatternKind};
use crate::error::{RavenError, TypeError};
use crate::span::Span;

use super::env::TypeEnv;
use super::ty::Ty;

/// What a single pattern contributes to the variant cover. We only
/// need a coarse view: was it a wildcard, a constructor name, or a
/// literal that we cannot easily reason about?
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatternHead {
    Wildcard,
    /// A constructor identifier (`None`, `Some`, `Ok`, `Err`, a user
    /// enum variant).
    Ctor(String),
    /// A literal pattern. Treated as "specific value" without further
    /// reasoning; a wildcard is required for exhaustiveness unless the
    /// scrutinee is `Bool`.
    Literal(LiteralKind),
    /// Anything else (struct destructuring, range, etc.). Requires a
    /// wildcard arm for exhaustiveness.
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiteralKind {
    Bool(bool),
    Int(i64),
    Str(String),
    Char(char),
    Float,
}

/// Compute the coarse head of a pattern for exhaustiveness purposes.
pub fn pattern_head(pat: &Pattern) -> PatternHead {
    match &pat.kind {
        PatternKind::Wildcard => PatternHead::Wildcard,
        PatternKind::Ident(name) => {
            // A bare identifier may be a binding (catch all) or a
            // nullary constructor. We err on the side of "catch all"
            // for built in nullary constructors that are known
            // covers (`None`, user enum unit variants). For unknown
            // names we keep them as Ctor; the body checker has
            // already validated the name.
            //
            // The body checker uses `is_nullary_constructor` which
            // needs the scrutinee type. To keep this side cheap, we
            // record the name as Ctor and let `check` resolve it
            // against the scrutinee.
            PatternHead::Ctor(name.clone())
        }
        PatternKind::Tuple { name, .. } => match name {
            Some(n) => PatternHead::Ctor(n.clone()),
            None => PatternHead::Other,
        },
        PatternKind::Struct { name, .. } => PatternHead::Ctor(name.clone()),
        PatternKind::Literal(lit) => PatternHead::Literal(match lit {
            crate::ast::LiteralPattern::Bool(b) => LiteralKind::Bool(*b),
            crate::ast::LiteralPattern::Int(i) => LiteralKind::Int(*i),
            crate::ast::LiteralPattern::String(s) => LiteralKind::Str(s.clone()),
            crate::ast::LiteralPattern::Char(c) => LiteralKind::Char(*c),
            crate::ast::LiteralPattern::Float(_) => LiteralKind::Float,
        }),
        PatternKind::Range { .. } => PatternHead::Other,
    }
}

/// Run the exhaustiveness check on `arms` over `scrut`.
pub fn check(
    scrut: &Ty,
    arms: &[PatternHead],
    span: &Span,
    env: &TypeEnv,
) -> Result<(), RavenError> {
    if arms.is_empty() {
        return Err(RavenError::ty(
            TypeError::NonExhaustiveMatch {
                missing: vec!["any value".into()],
            },
            span.clone(),
        ));
    }

    // Redundancy: first wildcard renders every subsequent arm
    // unreachable. We use the existence of a wildcard or a catch all
    // binding ident later to count for exhaustiveness.
    if let Some(idx) = arms.iter().position(is_catch_all) {
        if idx + 1 < arms.len() {
            return Err(RavenError::ty(TypeError::RedundantPattern, span.clone()));
        }
        return Ok(());
    }

    match scrut.strip_self() {
        Ty::Bool => check_bool(arms, span),
        Ty::Option(_) => check_variant_set(arms, &["None", "Some"], span),
        Ty::Result(_, _) => check_variant_set(arms, &["Ok", "Err"], span),
        Ty::Enum { id, .. } => match env.enums.get(id) {
            Some(esig) => {
                let names: Vec<String> = esig.variants.iter().map(|v| v.name.clone()).collect();
                let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
                check_variant_set(arms, &refs, span)
            }
            None => Err(RavenError::ty(
                TypeError::Custom("enum signature missing for exhaustiveness check".into()),
                span.clone(),
            )),
        },
        Ty::Error => Ok(()),
        _ => Err(RavenError::ty(
            TypeError::NonExhaustiveMatch {
                missing: vec!["_ (wildcard arm required)".into()],
            },
            span.clone(),
        )),
    }
}

fn check_bool(arms: &[PatternHead], span: &Span) -> Result<(), RavenError> {
    let mut has_true = false;
    let mut has_false = false;
    for h in arms {
        match h {
            PatternHead::Literal(LiteralKind::Bool(true)) => has_true = true,
            PatternHead::Literal(LiteralKind::Bool(false)) => has_false = true,
            _ => {}
        }
    }
    if has_true && has_false {
        return Ok(());
    }
    let mut missing = Vec::new();
    if !has_true {
        missing.push("true".to_string());
    }
    if !has_false {
        missing.push("false".to_string());
    }
    Err(RavenError::ty(
        TypeError::NonExhaustiveMatch { missing },
        span.clone(),
    ))
}

fn check_variant_set(
    arms: &[PatternHead],
    expected: &[&str],
    span: &Span,
) -> Result<(), RavenError> {
    let covered: std::collections::HashSet<&str> = arms
        .iter()
        .filter_map(|h| match h {
            PatternHead::Ctor(name) => Some(name.as_str()),
            _ => None,
        })
        .collect();
    let missing: Vec<String> = expected
        .iter()
        .filter(|v| !covered.contains(*v))
        .map(|v| (*v).to_string())
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(RavenError::ty(
            TypeError::NonExhaustiveMatch { missing },
            span.clone(),
        ))
    }
}

/// True when the head matches everything: a wildcard, or a bare
/// identifier we recognized as a binding (the body checker already
/// resolved any constructor identifier semantics; what remains here as
/// `Ctor` may still be a binding). To stay sound for exhaustiveness,
/// we treat a `Ctor(name)` that contains lower case ascii as a
/// catch all, mirroring Rust's convention. Constructor names in
/// Raven are upper case (`Some`, `Ok`, enum variants).
fn is_catch_all(h: &PatternHead) -> bool {
    match h {
        PatternHead::Wildcard => true,
        PatternHead::Ctor(name) => name
            .chars()
            .next()
            .map(|c| c.is_lowercase())
            .unwrap_or(false),
        _ => false,
    }
}
