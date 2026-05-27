//! Pattern lowering for `match` arms.
//!
//! Patterns reach MIR as a flat list of arms. The lowering rule
//! depends on the discriminant kind:
//!
//! * For an enum or `Option` / `Result` scrutinee, MIR emits a single
//!   `SwitchEnum` terminator. Each arm block opens with assignments
//!   that bind the variant's payload positions to local names.
//! * For an integer literal scrutinee (and pattern set), MIR emits a
//!   `SwitchInt`. The arms have no payload bindings.
//! * For richer shapes (strings, nested constructors), the lowering
//!   falls back to a chain of `SwitchInt` blocks driven by equality
//!   comparisons. The unit tests pin the basic cases; the wider
//!   feature set is shared with issue #66 (exhaustiveness).

use crate::hir::expr::{HirArm, HirExpr};
use crate::hir::pattern::{HirLiteralPat, HirPattern, HirPatternKind};
use crate::tycheck::Ty;

use super::super::ir::{
    MirBlockId, MirConstant, MirOperand, MirRvalue, MirStatement, MirTerminator,
};
use super::super::ty::MirType;
use super::LowerCx;

/// Lower a HIR match expression. Returns the operand that holds the
/// match's result. `result_ty` is the MIR type of the match value.
pub fn lower_match(
    cx: &mut LowerCx<'_>,
    scrutinee: &HirExpr,
    arms: &[HirArm],
    result_ty: MirType,
) -> MirOperand {
    let scrut_op = super::expr::lower_expr(cx, scrutinee);
    let scrut_ty = scrutinee.ty.clone();
    let result_local = cx.builder.fresh_temp("match", result_ty);
    let cont = cx.builder.new_block();

    if is_enum_like(&scrut_ty) {
        lower_enum_match(cx, scrut_op, &scrut_ty, arms, result_local, cont);
    } else if matches!(scrut_ty, Ty::Int | Ty::Bool | Ty::Char) {
        lower_int_match(cx, scrut_op, arms, result_local, cont);
    } else {
        lower_fallback_match(cx, scrut_op, arms, result_local, cont);
    }

    cx.current = cont;
    MirOperand::Copy(result_local)
}

fn is_enum_like(ty: &Ty) -> bool {
    matches!(ty, Ty::Enum { .. } | Ty::Option(_) | Ty::Result(..))
}

fn lower_enum_match(
    cx: &mut LowerCx<'_>,
    scrut: MirOperand,
    scrut_ty: &Ty,
    arms: &[HirArm],
    result: super::super::ir::MirLocal,
    cont: MirBlockId,
) {
    // Allocate one block per arm, then attach the SwitchEnum
    // terminator to the current block.
    let mut targets: Vec<(usize, MirBlockId)> = Vec::new();
    let mut otherwise: Option<MirBlockId> = None;

    // Walk arms once to allocate blocks.
    let mut arm_blocks: Vec<MirBlockId> = Vec::with_capacity(arms.len());
    for _ in arms {
        arm_blocks.push(cx.builder.new_block());
    }

    for (i, arm) in arms.iter().enumerate() {
        match variant_index_of(&arm.pattern, scrut_ty) {
            Some(idx) => targets.push((idx, arm_blocks[i])),
            None => otherwise = Some(arm_blocks[i]),
        }
    }

    cx.builder.close_block(
        cx.current,
        MirTerminator::SwitchEnum {
            discriminant: scrut.clone(),
            targets,
            otherwise,
        },
    );

    for (arm, block) in arms.iter().zip(arm_blocks.iter()) {
        cx.current = *block;
        bind_pattern(cx, &arm.pattern, scrut_ty, &scrut);
        let v = super::expr::lower_expr(cx, &arm.body);
        cx.builder.assign(cx.current, result, MirRvalue::Use(v));
        if !cx.builder.is_closed(cx.current) {
            cx.builder
                .close_block(cx.current, MirTerminator::Goto(cont));
        }
    }
}

fn lower_int_match(
    cx: &mut LowerCx<'_>,
    scrut: MirOperand,
    arms: &[HirArm],
    result: super::super::ir::MirLocal,
    cont: MirBlockId,
) {
    let mut arm_blocks: Vec<MirBlockId> = Vec::with_capacity(arms.len());
    for _ in arms {
        arm_blocks.push(cx.builder.new_block());
    }
    let mut targets: Vec<(i64, MirBlockId)> = Vec::new();
    let mut otherwise: MirBlockId = cont; // fallback to continuation
    for (i, arm) in arms.iter().enumerate() {
        match int_value_of(&arm.pattern) {
            Some(v) => targets.push((v, arm_blocks[i])),
            None => otherwise = arm_blocks[i],
        }
    }

    cx.builder.close_block(
        cx.current,
        MirTerminator::SwitchInt {
            discriminant: scrut,
            targets,
            otherwise,
        },
    );

    for (arm, block) in arms.iter().zip(arm_blocks.iter()) {
        cx.current = *block;
        let v = super::expr::lower_expr(cx, &arm.body);
        cx.builder.assign(cx.current, result, MirRvalue::Use(v));
        if !cx.builder.is_closed(cx.current) {
            cx.builder
                .close_block(cx.current, MirTerminator::Goto(cont));
        }
    }
}

fn lower_fallback_match(
    cx: &mut LowerCx<'_>,
    scrut: MirOperand,
    arms: &[HirArm],
    result: super::super::ir::MirLocal,
    cont: MirBlockId,
) {
    // Chain of equality checks: for each arm, compare scrut to the
    // pattern literal (when possible) and branch on the result. Bind
    // patterns (the trivial binding case) directly when matched.
    let mut next_test = cx.current;
    for arm in arms {
        let arm_block = cx.builder.new_block();
        let test_next = cx.builder.new_block();

        match &arm.pattern.kind {
            HirPatternKind::Wildcard | HirPatternKind::Binding(_) => {
                // Always matches; goto arm directly.
                if !cx.builder.is_closed(next_test) {
                    cx.builder
                        .close_block(next_test, MirTerminator::Goto(arm_block));
                }
            }
            HirPatternKind::Literal(lit) => {
                let cmp = literal_to_const(lit);
                let cmp_local = cx
                    .builder
                    .fresh_temp("cmp", super::super::ty::MirType::Bool);
                cx.builder.assign(
                    next_test,
                    cmp_local,
                    MirRvalue::BinaryOp(
                        super::super::ir::MirBinOp::Eq,
                        scrut.clone(),
                        MirOperand::Const(cmp),
                    ),
                );
                cx.builder.close_block(
                    next_test,
                    MirTerminator::SwitchInt {
                        discriminant: MirOperand::Copy(cmp_local),
                        targets: vec![(0, test_next), (1, arm_block)],
                        otherwise: test_next,
                    },
                );
            }
            _ => {
                // Treat anything else as always-matches for now.
                if !cx.builder.is_closed(next_test) {
                    cx.builder
                        .close_block(next_test, MirTerminator::Goto(arm_block));
                }
            }
        }

        cx.current = arm_block;
        // Bind name for `Binding` patterns.
        if let HirPatternKind::Binding(name) = &arm.pattern.kind {
            // We do not know the scrutinee type here at MIR level
            // beyond what is in HIR; rely on the type checker having
            // recorded it on the arm body's binding instead.
            // Materialize the binding as a Use of the scrutinee.
            let local = cx
                .builder
                .named_local(name.clone(), super::super::ty::MirType::Unit);
            cx.builder
                .assign(arm_block, local, MirRvalue::Use(scrut.clone()));
            cx.bind(name.clone(), local);
        }
        let v = super::expr::lower_expr(cx, &arm.body);
        cx.builder.assign(cx.current, result, MirRvalue::Use(v));
        if !cx.builder.is_closed(cx.current) {
            cx.builder
                .close_block(cx.current, MirTerminator::Goto(cont));
        }

        next_test = test_next;
    }
    // Trailing chain: if no arm matched, fall through to continuation
    // with whatever placeholder the result local already holds.
    if !cx.builder.is_closed(next_test) {
        cx.builder.close_block(next_test, MirTerminator::Goto(cont));
    }
}

/// Variant index of a constructor pattern against an enum-like type.
/// Returns `None` for wildcard, binding, or unrecognized cases (the
/// caller treats those as the `otherwise` arm).
fn variant_index_of(pat: &HirPattern, scrut_ty: &Ty) -> Option<usize> {
    let name = match &pat.kind {
        HirPatternKind::Constructor { name: Some(n), .. } => n,
        HirPatternKind::Struct { name, .. } => name,
        _ => return None,
    };
    match scrut_ty {
        Ty::Option(_) => match name.as_str() {
            "Some" => Some(0),
            "None" => Some(1),
            _ => None,
        },
        Ty::Result(_, _) => match name.as_str() {
            "Ok" => Some(0),
            "Err" => Some(1),
            _ => None,
        },
        Ty::Enum { .. } => None,
        _ => None,
    }
}

/// Bind the pattern's variables in the current block. For payload
/// positions, emit `EnumCreate`-style projections via field access
/// rvalues with the payload index.
fn bind_pattern(cx: &mut LowerCx<'_>, pat: &HirPattern, scrut_ty: &Ty, scrut: &MirOperand) {
    match &pat.kind {
        HirPatternKind::Wildcard | HirPatternKind::Literal(_) | HirPatternKind::Range { .. } => {}
        HirPatternKind::Binding(name) => {
            let mty = MirType::from_ty(scrut_ty);
            let local = cx.builder.named_local(name.clone(), mty);
            cx.builder
                .assign(cx.current, local, MirRvalue::Use(scrut.clone()));
            cx.bind(name.clone(), local);
        }
        HirPatternKind::Constructor { elements, .. } => {
            // Each element binds the payload at its positional index.
            for (i, element) in elements.iter().enumerate() {
                match &element.kind {
                    HirPatternKind::Binding(name) => {
                        let payload_ty = payload_type_at(scrut_ty, i);
                        let mty = MirType::from_ty(&payload_ty);
                        let local = cx.builder.named_local(name.clone(), mty);
                        cx.builder.assign(
                            cx.current,
                            local,
                            MirRvalue::FieldAccess {
                                base: scrut.clone(),
                                index: i,
                            },
                        );
                        cx.bind(name.clone(), local);
                    }
                    HirPatternKind::Wildcard | HirPatternKind::Literal(_) => {}
                    _ => {
                        // Nested patterns are not implemented yet; the
                        // type checker rejects them today.
                        cx.builder.emit(cx.current, MirStatement::Nop);
                    }
                }
            }
        }
        HirPatternKind::Struct { fields, .. } => {
            for (i, field) in fields.iter().enumerate() {
                if let Some(p) = &field.pattern {
                    if let HirPatternKind::Binding(name) = &p.kind {
                        let payload_ty = payload_type_at(scrut_ty, i);
                        let mty = MirType::from_ty(&payload_ty);
                        let local = cx.builder.named_local(name.clone(), mty);
                        cx.builder.assign(
                            cx.current,
                            local,
                            MirRvalue::FieldAccess {
                                base: scrut.clone(),
                                index: i,
                            },
                        );
                        cx.bind(name.clone(), local);
                    }
                } else {
                    // Shorthand `{ name }` binds the field name to a
                    // local of the same name.
                    let payload_ty = payload_type_at(scrut_ty, i);
                    let mty = MirType::from_ty(&payload_ty);
                    let local = cx.builder.named_local(field.name.clone(), mty);
                    cx.builder.assign(
                        cx.current,
                        local,
                        MirRvalue::FieldAccess {
                            base: scrut.clone(),
                            index: i,
                        },
                    );
                    cx.bind(field.name.clone(), local);
                }
            }
        }
    }
}

fn payload_type_at(scrut_ty: &Ty, index: usize) -> Ty {
    match scrut_ty {
        Ty::Option(inner) if index == 0 => (**inner).clone(),
        Ty::Result(t, _) if index == 0 => (**t).clone(),
        Ty::Result(_, e) if index == 1 => (**e).clone(),
        _ => Ty::Error,
    }
}

fn int_value_of(pat: &HirPattern) -> Option<i64> {
    match &pat.kind {
        HirPatternKind::Literal(HirLiteralPat::Int(v)) => Some(*v),
        HirPatternKind::Literal(HirLiteralPat::Bool(b)) => Some(*b as i64),
        HirPatternKind::Literal(HirLiteralPat::Char(c)) => Some(*c as i64),
        _ => None,
    }
}

fn literal_to_const(lit: &HirLiteralPat) -> MirConstant {
    match lit {
        HirLiteralPat::Int(i) => MirConstant::Int(*i),
        HirLiteralPat::Float(v) => MirConstant::Float(*v),
        HirLiteralPat::Bool(b) => MirConstant::Bool(*b),
        HirLiteralPat::Str(s) => MirConstant::Str(s.clone()),
        HirLiteralPat::Char(c) => MirConstant::Char(*c),
    }
}
