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

    // The constructor names this scrutinee actually has. A bare identifier
    // pattern is a constructor only when it names one of these; any other
    // identifier is a binding that matches everything. Deciding this by the
    // real variant set, rather than by the identifier's letter case, is what
    // lets a lowercase enum variant (`pos`) stay a constructor and an
    // uppercase binding (`Found`) stay a binding.
    let ctors = scrutinee_ctors(scrut, env);
    let is_catch_all = |h: &PatternHead| match h {
        PatternHead::Wildcard => true,
        PatternHead::Ctor(name) => !ctors.iter().any(|c| c == name),
        _ => false,
    };

    // Redundancy: an arm whose head exactly repeats an earlier arm's can never
    // match. A duplicate literal value, or a real variant named twice, is
    // unreachable. (A `Float` literal carries no value here, so floats are not
    // compared; a binding identifier matches everything and is handled by the
    // catch-all check below; a range is `Other` and not compared.)
    let mut seen: Vec<&PatternHead> = Vec::new();
    for h in arms {
        let compares = match h {
            PatternHead::Literal(LiteralKind::Float) => false,
            PatternHead::Literal(_) => true,
            PatternHead::Ctor(name) => ctors.iter().any(|c| c == name),
            _ => false,
        };
        if compares {
            if seen.iter().any(|s| *s == h) {
                return Err(RavenError::ty(TypeError::RedundantPattern, span.clone()));
            }
            seen.push(h);
        }
    }

    // Redundancy: a catch-all arm renders every arm after it unreachable. A
    // catch-all anywhere also makes the match exhaustive.
    if let Some(idx) = arms.iter().position(&is_catch_all) {
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

/// The constructor names a scrutinee type offers, used to tell a real
/// constructor pattern from a binding. An open type (`Int`, `String`, a
/// struct, ...) has none, so any bare identifier over it is a binding.
fn scrutinee_ctors(scrut: &Ty, env: &TypeEnv) -> Vec<String> {
    match scrut.strip_self() {
        Ty::Option(_) => vec!["None".to_string(), "Some".to_string()],
        Ty::Result(_, _) => vec!["Ok".to_string(), "Err".to_string()],
        Ty::Enum { id, .. } => env
            .enums
            .get(id)
            .map(|esig| esig.variants.iter().map(|v| v.name.clone()).collect())
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}
