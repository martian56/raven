//! Lowering AST patterns to HIR patterns.

use crate::ast::{LiteralPattern, Pattern, PatternKind};
use crate::error::RavenError;

use crate::hir::pattern::{HirFieldPat, HirLiteralPat, HirPattern, HirPatternKind};
use crate::resolve::Binding;

use super::LowerCtx;

/// Lower one pattern.
pub(crate) fn lower_pattern(pat: &Pattern, cx: &LowerCtx<'_>) -> Result<HirPattern, RavenError> {
    let kind = match &pat.kind {
        PatternKind::Wildcard => HirPatternKind::Wildcard,
        PatternKind::Literal(lit) => HirPatternKind::Literal(match lit {
            LiteralPattern::Int(i) => HirLiteralPat::Int(*i),
            LiteralPattern::Float(f) => HirLiteralPat::Float(*f),
            LiteralPattern::Bool(b) => HirLiteralPat::Bool(*b),
            LiteralPattern::String(s) => HirLiteralPat::Str(s.clone()),
            LiteralPattern::Char(c) => HirLiteralPat::Char(*c),
        }),
        PatternKind::Ident(name) => {
            // An ident pattern is a constructor if the resolver bound
            // it to an enum variant; otherwise it is a fresh binding.
            match cx.resolved.map.lookup(&pat.span) {
                Some(Binding::Variant { .. }) => HirPatternKind::Constructor {
                    name: Some(name.clone()),
                    elements: Vec::new(),
                },
                _ => HirPatternKind::Binding(name.clone()),
            }
        }
        PatternKind::Tuple { name, elements } => {
            let mut elems = Vec::with_capacity(elements.len());
            for e in elements {
                elems.push(lower_pattern(e, cx)?);
            }
            HirPatternKind::Constructor {
                name: name.clone(),
                elements: elems,
            }
        }
        PatternKind::Struct { name, fields } => {
            let mut out = Vec::with_capacity(fields.len());
            for f in fields {
                let sub = match &f.pattern {
                    Some(p) => Some(lower_pattern(p, cx)?),
                    None => None,
                };
                out.push(HirFieldPat {
                    name: f.name.clone(),
                    pattern: sub,
                    span: f.span.clone(),
                });
            }
            HirPatternKind::Struct {
                name: name.clone(),
                fields: out,
            }
        }
        PatternKind::Range { lo, hi, inclusive } => HirPatternKind::Range {
            lo: *lo,
            hi: *hi,
            inclusive: *inclusive,
        },
    };
    Ok(HirPattern {
        kind,
        span: pat.span.clone(),
    })
}
