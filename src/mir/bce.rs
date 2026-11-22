//! Bounds-check elimination by loop versioning.
//!
//! A rotated loop whose body is one block,
//!
//! ```text
//! pre:   c0 = i < n            switch c0 -> body / exit
//! body:  ... xs[i] ...  i = i + step   goto latch
//! latch: c = i < n             switch c -> body / exit
//! ```
//!
//! where nothing in the body can resize `xs` or change `n` or `step`, reads
//! and writes `xs[i]` before `i` moves only at indexes in `start..=n` (or
//! `start..n`). One test before the loop, `0 <= start`, `0 <= step`, and `n`
//! inside `len(xs)`, proves every such access in range, so a copy of the
//! loop with those accesses unchecked runs when it passes. When it fails the
//! original, checked loop runs, so a program that would panic still panics
//! at the same access.

use std::collections::HashSet;

use super::ir::{
    ListMethodOp, MirBinOp, MirBlock, MirBlockId, MirConstant, MirFunction, MirLocal, MirLocalDecl,
    MirOperand, MirProgram, MirRvalue, MirStatement, MirTerminator,
};
use super::ty::MirType;
use super::visit::assigned_local;

pub fn version_loops(program: &mut MirProgram) {
    // Math calls count as pure only when they reach the C function itself:
    // a declared `float -> float` extern, and no Raven function of that name.
    let defined: HashSet<String> = program.functions.iter().map(|f| f.name.clone()).collect();
    let math: HashSet<String> = program
        .externs
        .iter()
        .filter(|e| {
            matches!(
                e.name.as_str(),
                "sqrt" | "floor" | "ceil" | "trunc" | "fabs"
            ) && e.params == [MirType::Float]
                && e.ret == MirType::Float
                && !defined.contains(&e.name)
        })
        .map(|e| e.name.clone())
        .collect();
    for f in &mut program.functions {
        if f.blocks
            .iter()
            .enumerate()
            .all(|(i, b)| b.id.0 as usize == i)
        {
            version_in(f, &math);
        }
    }
}

fn version_in(f: &mut MirFunction, math: &HashSet<String>) {
    let headers: Vec<usize> = (0..f.blocks.len())
        .filter(|&i| f.blocks[i].is_loop_header)
        .collect();
    for body in headers {
        if let Some(plan) = analyze(f, body, math) {
            apply(f, plan);
        }
    }
}

struct Plan {
    pre: usize,
    body: usize,
    /// The body's blocks in order, starting with `body`, joined by `goto`.
    chain: Vec<usize>,
    latch: usize,
    exit: MirBlockId,
    /// The index local, its bound, and the comparison (`Lt` or `Le`).
    index: MirLocal,
    bound: MirOperand,
    cmp: MirBinOp,
    step: MirOperand,
    /// For `while lo < hi` with `hi` stepping down: `hi` and its step.
    pair: Option<(MirLocal, MirOperand)>,
    /// (block, statement) positions whose access becomes unchecked, and the
    /// lists they index.
    sites: Vec<(usize, usize)>,
    /// Each indexed list with the largest constant offset used on it.
    lists: Vec<(MirLocal, i64)>,
}

/// The `(cond, cmp, index, bound)` a block's last statement computes for its
/// switch, when the switch sends 1 to `body` and 0 to one exit.
fn loop_test(b: &MirBlock, body: usize) -> Option<(MirBinOp, MirLocal, MirOperand, MirBlockId)> {
    let MirTerminator::SwitchInt {
        discriminant: MirOperand::Copy(c),
        targets,
        otherwise,
    } = &b.terminator
    else {
        return None;
    };
    let take = targets.iter().find(|(v, _)| *v == 1)?.1;
    let skip = targets.iter().find(|(v, _)| *v == 0)?.1;
    if take.0 as usize != body || skip != *otherwise {
        return None;
    }
    let Some(MirStatement::Assign {
        dst,
        rvalue: MirRvalue::BinaryOp(op @ (MirBinOp::Lt | MirBinOp::Le), MirOperand::Copy(i), n),
    }) = b.statements.last()
    else {
        return None;
    };
    if dst != c {
        return None;
    }
    Some((*op, *i, n.clone(), skip))
}

/// A call to one of the C math functions the back end emits as a single
/// instruction; it touches no memory and cannot reach a collection.
fn pure_math_call(r: &MirRvalue, math: &HashSet<String>) -> bool {
    matches!(r, MirRvalue::Call { callee, .. } if math.contains(&callee.mangled))
}

fn invariant(op: &MirOperand, assigned: &HashSet<MirLocal>) -> bool {
    match op {
        MirOperand::Copy(l) => !assigned.contains(l),
        MirOperand::Const(MirConstant::Int(_)) => true,
        _ => false,
    }
}

fn preds_of(f: &MirFunction, target: usize) -> Vec<usize> {
    (0..f.blocks.len())
        .filter(|&p| {
            f.blocks[p]
                .terminator
                .successors()
                .iter()
                .any(|s| s.0 as usize == target)
        })
        .collect()
}

fn analyze(f: &MirFunction, body: usize, math: &HashSet<String>) -> Option<Plan> {
    // Follow the body's straight-line chain of `goto`s to the block that
    // jumps to the latch: the latch is the first block whose only statement
    // is the loop test feeding back to `body`.
    let mut chain = vec![body];
    let latch;
    loop {
        let cur = *chain.last().expect("chain starts non-empty");
        let MirTerminator::Goto(next) = f.blocks[cur].terminator else {
            return None;
        };
        let next = next.0 as usize;
        if next == body || chain.contains(&next) || chain.len() > 32 {
            return None;
        }
        if loop_test(&f.blocks[next], body).is_some() && f.blocks[next].statements.len() == 1 {
            latch = next;
            break;
        }
        if preds_of(f, next) != vec![cur] {
            return None;
        }
        chain.push(next);
    }
    let stmts: Vec<(usize, usize, &MirStatement)> = chain
        .iter()
        .flat_map(|&bi| {
            f.blocks[bi]
                .statements
                .iter()
                .enumerate()
                .map(move |(k, st)| (bi, k, st))
        })
        .collect();
    let (cmp, index, bound, exit) = loop_test(&f.blocks[latch], body)?;
    // The latch only recomputes the test.
    if f.blocks[latch].statements.len() != 1 {
        return None;
    }
    // Exactly two ways in: the pre-test and the latch.
    let preds = preds_of(f, body);
    let pre = *preds.iter().find(|&&p| p != latch)?;
    if preds.len() != 2 || pre == body {
        return None;
    }
    let (pcmp, pindex, pbound, pexit) = loop_test(&f.blocks[pre], body)?;
    if pcmp != cmp || pindex != index || pbound != bound || pexit != exit {
        return None;
    }

    // Every statement must be plain data movement: nothing that can call
    // out, allocate, or resize a list.
    let mut assigned: HashSet<MirLocal> = HashSet::new();
    for &(_, _, st) in &stmts {
        let plain = match st {
            MirStatement::Assign { rvalue, .. } => {
                matches!(
                    rvalue,
                    MirRvalue::Use(_)
                        | MirRvalue::BinaryOp(..)
                        | MirRvalue::UnaryOp(..)
                        | MirRvalue::FieldAccess { .. }
                        | MirRvalue::IndexAccess { .. }
                        | MirRvalue::Cast { .. }
                ) || pure_math_call(rvalue, math)
            }
            MirStatement::StoreField { .. }
            | MirStatement::StoreIndex { .. }
            | MirStatement::StorageLive(_)
            | MirStatement::StorageDead(_)
            | MirStatement::Nop => true,
            _ => false,
        };
        if !plain {
            return None;
        }
        if let Some(l) = assigned_local(st) {
            assigned.insert(l);
        }
    }
    if let Some(l) = assigned_local(&f.blocks[latch].statements[0]) {
        assigned.insert(l);
    }

    // The index moves once, by a step that cannot be negative once checked:
    // `t = i + step; i = t` (or `i = i + step`).
    let (update, step) = find_step(&stmts, index, MirBinOp::Add)?;
    if !invariant(&step, &assigned) {
        return None;
    }
    // `while lo < hi` where `hi` steps down: every access stays between the
    // entry values of `lo` and `hi`. Otherwise the bound must not change.
    let mut pair = None;
    let mut last = update;
    if let MirOperand::Copy(hi) = bound {
        if assigned.contains(&hi) {
            if cmp != MirBinOp::Lt {
                return None;
            }
            let (hi_update, hi_step) = find_step(&stmts, hi, MirBinOp::Sub)?;
            if !invariant(&hi_step, &assigned) {
                return None;
            }
            last = last.min(hi_update);
            pair = Some((hi, hi_step));
        }
    }
    if pair.is_none() && !invariant(&bound, &assigned) {
        return None;
    }
    let hi_local = pair.as_ref().map(|(h, _)| *h);

    // Accesses `xs[i]` before the update, on lists the body never rebinds.
    let mut sites = Vec::new();
    let mut lists: Vec<(MirLocal, i64)> = Vec::new();
    // An index `t` set once to `index + c` (constant `c >= 0`) earlier in
    // the same iteration reads at a fixed offset from `index`. It counts only
    // after its assignment, so a read of a stale `t` stays checked.
    let single_def = |l: MirLocal| {
        stmts
            .iter()
            .filter(|(_, _, s2)| assigned_local(s2) == Some(l))
            .count()
            == 1
    };
    let mut offsets: std::collections::HashMap<MirLocal, i64> = Default::default();
    for &(bi, k, st) in stmts.iter().take(last) {
        if let MirStatement::Assign {
            dst,
            rvalue:
                MirRvalue::BinaryOp(
                    MirBinOp::Add,
                    MirOperand::Copy(i),
                    MirOperand::Const(MirConstant::Int(c)),
                ),
        } = st
        {
            if pair.is_none() && *i == index && (0..1 << 20).contains(c) && single_def(*dst) {
                offsets.insert(*dst, *c);
            }
        }
        let offset_of = |i: &MirLocal| -> Option<i64> {
            if *i == index || Some(*i) == hi_local {
                Some(0)
            } else {
                offsets.get(i).copied()
            }
        };
        let (base, off) = match st {
            MirStatement::Assign {
                rvalue:
                    MirRvalue::IndexAccess {
                        base: MirOperand::Copy(xs),
                        index: MirOperand::Copy(i),
                        ..
                    },
                ..
            } => match offset_of(i) {
                Some(o) => (*xs, o),
                None => continue,
            },
            MirStatement::StoreIndex {
                base: MirOperand::Copy(xs),
                index: MirOperand::Copy(i),
                ..
            } => match offset_of(i) {
                Some(o) => (*xs, o),
                None => continue,
            },
            _ => continue,
        };
        if assigned.contains(&base) || !matches!(f.locals[base.0 as usize].ty, MirType::List(_)) {
            continue;
        }
        sites.push((bi, k));
        match lists.iter_mut().find(|(l, _)| *l == base) {
            Some(entry) => entry.1 = entry.1.max(off),
            None => lists.push((base, off)),
        }
    }
    if sites.is_empty() {
        return None;
    }
    Some(Plan {
        pre,
        body,
        chain,
        latch,
        exit,
        index,
        bound,
        cmp,
        step,
        pair,
        sites,
        lists,
    })
}

/// The one statement that updates `local`, as `local = local op step` or
/// through a temporary, with its position among `stmts`.
fn find_step(
    stmts: &[(usize, usize, &MirStatement)],
    local: MirLocal,
    op: MirBinOp,
) -> Option<(usize, MirOperand)> {
    let defs: Vec<usize> = stmts
        .iter()
        .enumerate()
        .filter(|(_, (_, _, st))| assigned_local(st) == Some(local))
        .map(|(k, _)| k)
        .collect();
    let [update] = defs[..] else {
        return None;
    };
    let step_of = |st: &MirStatement| match st {
        MirStatement::Assign {
            rvalue: MirRvalue::BinaryOp(o, MirOperand::Copy(i), s),
            ..
        } if *i == local && *o == op => Some(s.clone()),
        _ => None,
    };
    let step = match stmts[update].2 {
        MirStatement::Assign {
            rvalue: MirRvalue::Use(MirOperand::Copy(t)),
            ..
        } => {
            let k = (0..update)
                .rev()
                .find(|&k| assigned_local(stmts[k].2) == Some(*t))?;
            step_of(stmts[k].2)?
        }
        st => step_of(st)?,
    };
    Some((update, step))
}

fn apply(f: &mut MirFunction, plan: Plan) {
    let new_local = |f: &mut MirFunction, name: &str, ty: MirType| {
        f.locals.push(MirLocalDecl {
            name: name.to_string(),
            ty,
            is_param: false,
        });
        MirLocal(f.locals.len() as u32 - 1)
    };
    let check_id = MirBlockId(f.blocks.len() as u32);
    let fast_ids: Vec<MirBlockId> = (0..plan.chain.len())
        .map(|k| MirBlockId(check_id.0 + 1 + k as u32))
        .collect();
    let fast_body_id = fast_ids[0];
    let fast_latch_id = MirBlockId(check_id.0 + 1 + plan.chain.len() as u32);

    // check: all of the conditions folded into one flag.
    let mut stmts = Vec::new();
    let mut flag = {
        let ok = new_local(f, "bce_ok", MirType::Bool);
        stmts.push(MirStatement::Assign {
            dst: ok,
            rvalue: MirRvalue::BinaryOp(
                MirBinOp::Ge,
                MirOperand::Copy(plan.index),
                MirOperand::Const(MirConstant::Int(0)),
            ),
        });
        ok
    };
    let mut and = |f: &mut MirFunction, stmts: &mut Vec<MirStatement>, rv: MirRvalue| {
        let cond = new_local(f, "bce_cond", MirType::Bool);
        stmts.push(MirStatement::Assign {
            dst: cond,
            rvalue: rv,
        });
        let next = new_local(f, "bce_ok", MirType::Bool);
        stmts.push(MirStatement::Assign {
            dst: next,
            rvalue: MirRvalue::BinaryOp(
                MirBinOp::And,
                MirOperand::Copy(flag),
                MirOperand::Copy(cond),
            ),
        });
        flag = next;
    };
    and(
        f,
        &mut stmts,
        MirRvalue::BinaryOp(
            MirBinOp::Ge,
            plan.step.clone(),
            MirOperand::Const(MirConstant::Int(0)),
        ),
    );
    if let Some((_, hi_step)) = &plan.pair {
        and(
            f,
            &mut stmts,
            MirRvalue::BinaryOp(
                MirBinOp::Ge,
                hi_step.clone(),
                MirOperand::Const(MirConstant::Int(0)),
            ),
        );
    }
    for (xs, off) in &plan.lists {
        let elem = match &f.locals[xs.0 as usize].ty {
            MirType::List(e) => (**e).clone(),
            _ => unreachable!("only list locals are versioned"),
        };
        let full_len = new_local(f, "bce_len", MirType::Int);
        stmts.push(MirStatement::Assign {
            dst: full_len,
            rvalue: MirRvalue::ListMethod {
                op: ListMethodOp::Len,
                receiver: MirOperand::Copy(*xs),
                arg: None,
                elem_ty: elem,
            },
        });
        // Offset accesses `xs[i + c]` need `n` within `len - c`; subtracting
        // from the length cannot wrap, unlike adding to `n`.
        let len = if *off > 0 {
            let l = new_local(f, "bce_len", MirType::Int);
            stmts.push(MirStatement::Assign {
                dst: l,
                rvalue: MirRvalue::BinaryOp(
                    MirBinOp::Sub,
                    MirOperand::Copy(full_len),
                    MirOperand::Const(MirConstant::Int(*off)),
                ),
            });
            l
        } else {
            full_len
        };
        // `i <= n` touches index n, so n must be below len; `i < n` stops
        // short of it, so n may equal len. A falling `hi` is itself indexed,
        // so it must be below len.
        let op = if plan.cmp == MirBinOp::Le || plan.pair.is_some() {
            MirBinOp::Lt
        } else {
            MirBinOp::Le
        };
        and(
            f,
            &mut stmts,
            MirRvalue::BinaryOp(op, plan.bound.clone(), MirOperand::Copy(len)),
        );
        // A step no larger than the length keeps `i + step` from wrapping.
        and(
            f,
            &mut stmts,
            MirRvalue::BinaryOp(MirBinOp::Le, plan.step.clone(), MirOperand::Copy(len)),
        );
        if let Some((_, hi_step)) = &plan.pair {
            and(
                f,
                &mut stmts,
                MirRvalue::BinaryOp(MirBinOp::Le, hi_step.clone(), MirOperand::Copy(len)),
            );
        }
    }
    let original_body = MirBlockId(plan.body as u32);
    let check = MirBlock {
        id: check_id,
        statements: stmts,
        terminator: MirTerminator::SwitchInt {
            discriminant: MirOperand::Copy(flag),
            targets: vec![(0, original_body), (1, fast_body_id)],
            otherwise: original_body,
        },
        is_loop_header: false,
    };

    // The unchecked copy of the loop body, block for block.
    let mut fast_blocks = Vec::new();
    for (pos, &bi) in plan.chain.iter().enumerate() {
        let mut blk = f.blocks[bi].clone();
        blk.id = fast_ids[pos];
        blk.is_loop_header = false;
        for &(sb, k) in &plan.sites {
            if sb != bi {
                continue;
            }
            match &mut blk.statements[k] {
                MirStatement::Assign {
                    rvalue: MirRvalue::IndexAccess { checked, .. },
                    ..
                }
                | MirStatement::StoreIndex { checked, .. } => *checked = false,
                _ => {}
            }
        }
        let next = fast_ids.get(pos + 1).copied().unwrap_or(fast_latch_id);
        blk.terminator = MirTerminator::Goto(next);
        fast_blocks.push(blk);
    }
    fast_blocks[0].is_loop_header = true;
    let mut fast_latch = f.blocks[plan.latch].clone();
    fast_latch.id = fast_latch_id;
    retarget(&mut fast_latch.terminator, original_body, fast_body_id);

    // The pre-test enters through the check instead of the loop.
    retarget(&mut f.blocks[plan.pre].terminator, original_body, check_id);
    let _ = plan.exit;
    f.blocks.push(check);
    f.blocks.extend(fast_blocks);
    f.blocks.push(fast_latch);
}

fn retarget(t: &mut MirTerminator, from: MirBlockId, to: MirBlockId) {
    if let MirTerminator::SwitchInt {
        targets, otherwise, ..
    } = t
    {
        for (_, b) in targets.iter_mut() {
            if *b == from {
                *b = to;
            }
        }
        if *otherwise == from {
            *otherwise = to;
        }
    }
}
