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
    // Apply the monomorphization substitution so a scrutinee whose type
    // mentions a generic parameter (for example `Option<T>` returned by a
    // bound iterator's `next`) yields concrete payload-binding types. Using
    // the raw HIR type would leave `T` abstract, which lowers to a unit
    // slot and produces a type mismatch in the generated code. Strip the
    // `SelfTy(T)` wrapper a method receiver carries so the match dispatches
    // on the underlying enum and projects payloads the same way a plain
    // value scrutinee does.
    let scrut_ty = super::substitute(&scrutinee.ty, cx.subst)
        .strip_self()
        .clone();
    let result_local = cx.builder.fresh_temp("match", result_ty);
    let cont = cx.builder.new_block();

    // A guard needs sequential fall-through, which the switch-based paths
    // cannot express, so any match with a guarded arm takes the sequential
    // lowering regardless of scrutinee type.
    if arms.iter().any(|a| a.guard.is_some()) {
        lower_sequential_match(cx, scrut_op, &scrut_ty, arms, result_local, cont);
    } else if is_enum_like(&scrut_ty) {
        lower_enum_match(cx, scrut_op, &scrut_ty, arms, result_local, cont);
    } else if matches!(scrut_ty, Ty::Int | Ty::Bool | Ty::Char) {
        // The switch-based int path keys on single values, so a match with a
        // range pattern takes the sequential lowering, which tests bounds.
        if arms
            .iter()
            .any(|a| matches!(a.pattern.kind, HirPatternKind::Range { .. }))
        {
            lower_sequential_match(cx, scrut_op, &scrut_ty, arms, result_local, cont);
        } else {
            lower_int_match(cx, scrut_op, &scrut_ty, arms, result_local, cont);
        }
    } else {
        lower_fallback_match(cx, scrut_op, &scrut_ty, arms, result_local, cont);
    }

    cx.current = cont;
    cx.diverged = false;
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
        match variant_index_of(&arm.pattern, scrut_ty, cx) {
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
    scrut_ty: &Ty,
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
            discriminant: scrut.clone(),
            targets,
            otherwise,
        },
    );

    for (arm, block) in arms.iter().zip(arm_blocks.iter()) {
        cx.current = *block;
        // A binding arm (`other -> ...`) over an int/bool/char scrutinee binds
        // the name to the scrutinee value. Without this the name was unbound in
        // the arm body and read a garbage slot.
        if let HirPatternKind::Binding(name) = &arm.pattern.kind {
            let local = cx
                .builder
                .named_local(name.clone(), super::super::ty::MirType::from_ty(scrut_ty));
            cx.builder
                .assign(*block, local, MirRvalue::Use(scrut.clone()));
            cx.bind(name.clone(), local);
        }
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
    scrut_ty: &Ty,
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
                let cmp_local = cx
                    .builder
                    .fresh_temp("cmp", super::super::ty::MirType::Bool);
                // A `String` literal pattern compares by content, not by
                // heap-pointer identity: route it through the string-equality
                // intrinsic. A plain `BinaryOp::Eq` would compare the literal
                // constant's address against the scrutinee's and never match,
                // silently falling through to the wildcard arm.
                let test = if let HirLiteralPat::Str(s) = lit {
                    MirRvalue::Call {
                        callee: super::super::ir::MirFnRef {
                            mangled: super::super::intrinsics::STR_EQ.into(),
                            origin: None,
                        },
                        args: vec![
                            scrut.clone(),
                            MirOperand::Const(MirConstant::Str(s.clone())),
                        ],
                    }
                } else {
                    MirRvalue::BinaryOp(
                        super::super::ir::MirBinOp::Eq,
                        scrut.clone(),
                        MirOperand::Const(literal_to_const(lit)),
                    )
                };
                cx.builder.assign(next_test, cmp_local, test);
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
            // Bind the name to the scrutinee value at its real type. Using a
            // Unit slot here dropped the value: a `String`, `Float`, or struct
            // scrutinee came back empty inside the arm body.
            let local = cx
                .builder
                .named_local(name.clone(), super::super::ty::MirType::from_ty(scrut_ty));
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

/// Lower a match where at least one arm has a guard. Guards need a sequential
/// fall-through that a `SwitchEnum`/`SwitchInt` cannot express: an arm whose
/// pattern matches but whose guard is false must drop through to the next arm,
/// and two arms may share a pattern with different guards. Each arm is tested in
/// order (a per-arm `SwitchEnum`/`SwitchInt` for a constructor or literal, an
/// unconditional jump for a binding or wildcard); on a match the pattern is
/// bound and the guard, if any, is evaluated, falling through to the next arm
/// when it is false.
fn lower_sequential_match(
    cx: &mut LowerCx<'_>,
    scrut: MirOperand,
    scrut_ty: &Ty,
    arms: &[HirArm],
    result: super::super::ir::MirLocal,
    cont: MirBlockId,
) {
    let mut next_test = cx.current;
    for arm in arms {
        let arm_block = cx.builder.new_block();
        let test_next = cx.builder.new_block();

        match &arm.pattern.kind {
            HirPatternKind::Wildcard | HirPatternKind::Binding(_) => {
                if !cx.builder.is_closed(next_test) {
                    cx.builder
                        .close_block(next_test, MirTerminator::Goto(arm_block));
                }
            }
            HirPatternKind::Literal(lit) => {
                let cmp_local = cx
                    .builder
                    .fresh_temp("cmp", super::super::ty::MirType::Bool);
                let test = if let HirLiteralPat::Str(s) = lit {
                    MirRvalue::Call {
                        callee: super::super::ir::MirFnRef {
                            mangled: super::super::intrinsics::STR_EQ.into(),
                            origin: None,
                        },
                        args: vec![
                            scrut.clone(),
                            MirOperand::Const(MirConstant::Str(s.clone())),
                        ],
                    }
                } else {
                    MirRvalue::BinaryOp(
                        super::super::ir::MirBinOp::Eq,
                        scrut.clone(),
                        MirOperand::Const(literal_to_const(lit)),
                    )
                };
                cx.builder.assign(next_test, cmp_local, test);
                cx.builder.close_block(
                    next_test,
                    MirTerminator::SwitchInt {
                        discriminant: MirOperand::Copy(cmp_local),
                        targets: vec![(1, arm_block)],
                        otherwise: test_next,
                    },
                );
            }
            HirPatternKind::Constructor { .. } | HirPatternKind::Struct { .. } => {
                match variant_index_of(&arm.pattern, scrut_ty, cx) {
                    Some(idx) => cx.builder.close_block(
                        next_test,
                        MirTerminator::SwitchEnum {
                            discriminant: scrut.clone(),
                            targets: vec![(idx, arm_block)],
                            otherwise: Some(test_next),
                        },
                    ),
                    None => {
                        if !cx.builder.is_closed(next_test) {
                            cx.builder
                                .close_block(next_test, MirTerminator::Goto(arm_block));
                        }
                    }
                }
            }
            HirPatternKind::Range { lo, hi, inclusive } => {
                // Take the arm only when `lo <= scrut` and `scrut < hi` (or
                // `<= hi` when inclusive). Without this the bounds were ignored
                // and the range arm matched every value.
                let ge_local = cx.builder.fresh_temp("ge", MirType::Bool);
                let hi_local = cx.builder.fresh_temp("lt", MirType::Bool);
                let in_local = cx.builder.fresh_temp("inrange", MirType::Bool);
                cx.builder.assign(
                    next_test,
                    ge_local,
                    MirRvalue::BinaryOp(
                        super::super::ir::MirBinOp::Ge,
                        scrut.clone(),
                        MirOperand::Const(MirConstant::Int(*lo)),
                    ),
                );
                let hi_op = if *inclusive {
                    super::super::ir::MirBinOp::Le
                } else {
                    super::super::ir::MirBinOp::Lt
                };
                cx.builder.assign(
                    next_test,
                    hi_local,
                    MirRvalue::BinaryOp(
                        hi_op,
                        scrut.clone(),
                        MirOperand::Const(MirConstant::Int(*hi)),
                    ),
                );
                cx.builder.assign(
                    next_test,
                    in_local,
                    MirRvalue::BinaryOp(
                        super::super::ir::MirBinOp::And,
                        MirOperand::Copy(ge_local),
                        MirOperand::Copy(hi_local),
                    ),
                );
                cx.builder.close_block(
                    next_test,
                    MirTerminator::SwitchInt {
                        discriminant: MirOperand::Copy(in_local),
                        targets: vec![(1, arm_block)],
                        otherwise: test_next,
                    },
                );
            }
            _ => {
                if !cx.builder.is_closed(next_test) {
                    cx.builder
                        .close_block(next_test, MirTerminator::Goto(arm_block));
                }
            }
        }

        cx.current = arm_block;
        bind_pattern(cx, &arm.pattern, scrut_ty, &scrut);
        // A false guard drops through to the next arm's test.
        if let Some(guard) = &arm.guard {
            let g = super::expr::lower_expr(cx, guard);
            let run = cx.builder.new_block();
            cx.builder.close_block(
                cx.current,
                MirTerminator::SwitchInt {
                    discriminant: g,
                    targets: vec![(1, run)],
                    otherwise: test_next,
                },
            );
            cx.current = run;
        }
        let v = super::expr::lower_expr(cx, &arm.body);
        cx.builder.assign(cx.current, result, MirRvalue::Use(v));
        if !cx.builder.is_closed(cx.current) {
            cx.builder
                .close_block(cx.current, MirTerminator::Goto(cont));
        }

        next_test = test_next;
    }
    if !cx.builder.is_closed(next_test) {
        cx.builder.close_block(next_test, MirTerminator::Goto(cont));
    }
}

/// Variant index of a constructor pattern against an enum-like type.
/// Returns `None` for wildcard, binding, or unrecognized cases (the
/// caller treats those as the `otherwise` arm).
fn variant_index_of(pat: &HirPattern, scrut_ty: &Ty, cx: &LowerCx<'_>) -> Option<usize> {
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
        Ty::Enum { name: ename, .. } => {
            let decl = cx.decls.enums.get(ename)?;
            decl.variants.iter().position(|v| v.name == *name)
        }
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
        HirPatternKind::Constructor {
            elements,
            name: variant,
        } => {
            // Each element binds the payload at its positional index. The
            // enum value stores its discriminant in slot 0, so payload
            // field `i` lives in slot `i + 1`.
            for (i, element) in elements.iter().enumerate() {
                match &element.kind {
                    HirPatternKind::Binding(name) => {
                        let payload_ty = payload_type_at(scrut_ty, variant.as_deref(), i, cx);
                        let mty = MirType::from_ty(&payload_ty);
                        let local = cx.builder.named_local(name.clone(), mty);
                        cx.builder.assign(
                            cx.current,
                            local,
                            MirRvalue::FieldAccess {
                                base: scrut.clone(),
                                index: i + 1,
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
        HirPatternKind::Struct { fields, name } => {
            for (i, field) in fields.iter().enumerate() {
                let payload_ty = payload_type_at(scrut_ty, Some(name), i, cx);
                let mty = MirType::from_ty(&payload_ty);
                if let Some(p) = &field.pattern {
                    if let HirPatternKind::Binding(bname) = &p.kind {
                        let local = cx.builder.named_local(bname.clone(), mty);
                        cx.builder.assign(
                            cx.current,
                            local,
                            MirRvalue::FieldAccess {
                                base: scrut.clone(),
                                index: i + 1,
                            },
                        );
                        cx.bind(bname.clone(), local);
                    }
                } else {
                    // Shorthand `{ name }` binds the field name to a
                    // local of the same name.
                    let local = cx.builder.named_local(field.name.clone(), mty);
                    cx.builder.assign(
                        cx.current,
                        local,
                        MirRvalue::FieldAccess {
                            base: scrut.clone(),
                            index: i + 1,
                        },
                    );
                    cx.bind(field.name.clone(), local);
                }
            }
        }
    }
}

fn payload_type_at(scrut_ty: &Ty, variant: Option<&str>, index: usize, cx: &LowerCx<'_>) -> Ty {
    match scrut_ty {
        // `Some(x)`, `Ok(x)`, and `Err(e)` each bind their single payload at
        // index 0 of their own pattern, so the binding type is selected by the
        // variant, not the positional index. (Selecting by index alone typed
        // `Err(e)` as the `Ok` payload; it only worked while both payloads
        // shared the i64 slot, and produced a bad cast, a Cranelift verifier
        // error, or a segfault once they differed, e.g. `Result<Bool, E>`.)
        Ty::Option(inner) if variant == Some("Some") && index == 0 => (**inner).clone(),
        Ty::Result(t, _) if variant == Some("Ok") && index == 0 => (**t).clone(),
        Ty::Result(_, e) if variant == Some("Err") && index == 0 => (**e).clone(),
        Ty::Enum { name, .. } => {
            let Some(vname) = variant else {
                return Ty::Error;
            };
            let Some(decl) = cx.decls.enums.get(name) else {
                return Ty::Error;
            };
            decl.variants
                .iter()
                .find(|v| v.name == vname)
                .and_then(|v| v.fields.get(index))
                .map(|(_, ty, _)| ty.clone())
                .unwrap_or(Ty::Error)
        }
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
