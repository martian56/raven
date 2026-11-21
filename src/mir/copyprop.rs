//! Forward copies of locals that never change.
//!
//! A local assigned exactly once from another local `s` is replaced by `s`
//! when `s` holds one value for the whole function: a parameter that is
//! never reassigned, or a local assigned exactly once outside any loop.
//! Inlining leaves many such copies (each inlined parameter), and every
//! GC pointer copy would otherwise cost its own root slot.

use std::collections::HashMap;

use super::ir::{MirFunction, MirLocal, MirOperand, MirProgram, MirRvalue, MirStatement};
use super::visit::{
    assigned_local, statement_operands, statement_operands_mut, terminator_operands,
    terminator_operands_mut,
};

pub fn propagate_copies(program: &mut MirProgram) {
    for f in &mut program.functions {
        propagate_in(f);
    }
}

fn propagate_in(f: &mut MirFunction) {
    propagate_global(f);
    if std::env::var_os("RAVEN_NO_FIELD_FORWARD").is_none() {
        forward_field_loads(f);
    }
    propagate_local(f);
}

/// Within one block, reuse a field's known value instead of loading it
/// again: `t = b.m` after an earlier `u = b.m`, or after `b.m = v`, becomes
/// a copy. Objects never overlap, so only a store to the same field index,
/// a reassignment of a local involved, or a statement that may run user
/// code or write through a raw pointer forgets a known field.
fn forward_field_loads(f: &mut MirFunction) {
    for b in &mut f.blocks {
        // (base local, field index) -> operand holding the field's value.
        let mut known: HashMap<(MirLocal, usize), MirOperand> = HashMap::new();
        for st in &mut b.statements {
            let mut learn: Option<((MirLocal, usize), MirOperand)> = None;
            let mut clear = false;
            match st {
                MirStatement::Assign { dst, rvalue } => {
                    if let MirRvalue::FieldAccess {
                        base: MirOperand::Copy(base),
                        index,
                    } = rvalue
                    {
                        let key = (*base, *index);
                        if let Some(op) = known.get(&key) {
                            if !matches!(op, MirOperand::Copy(l) if l == dst) {
                                *rvalue = MirRvalue::Use(op.clone());
                            }
                        } else if base != dst {
                            learn = Some((key, MirOperand::Copy(*dst)));
                        }
                    } else if !field_safe(rvalue) {
                        clear = true;
                    }
                }
                MirStatement::StoreField {
                    base: MirOperand::Copy(base),
                    index,
                    value,
                } => {
                    let i = *index;
                    known.retain(|(_, k), _| *k != i);
                    // A string constant re-evaluated at each use could allocate.
                    let reusable = match value {
                        MirOperand::Copy(_) => true,
                        MirOperand::Const(c) => !matches!(c, super::ir::MirConstant::Str(_)),
                    };
                    if reusable {
                        learn = Some(((*base, i), value.clone()));
                    }
                }
                MirStatement::StoreField { index, .. } => {
                    let i = *index;
                    known.retain(|(_, k), _| *k != i);
                }
                MirStatement::StoreIndex { .. }
                | MirStatement::StoreGlobal { .. }
                | MirStatement::StorageLive(_)
                | MirStatement::Nop => {}
                _ => clear = true,
            }
            if clear {
                known.clear();
            }
            if let Some(d) = assigned_local(st) {
                known.retain(|(base, _), op| {
                    *base != d && !matches!(op, MirOperand::Copy(l) if *l == d)
                });
            }
            if let MirStatement::StorageDead(d) = st {
                let d = *d;
                known.retain(|(base, _), op| {
                    *base != d && !matches!(op, MirOperand::Copy(l) if *l == d)
                });
            }
            if let Some((key, op)) = learn {
                known.insert(key, op);
            }
        }
    }
}

/// Whether evaluating `rv` leaves every object field as it was.
fn field_safe(rv: &MirRvalue) -> bool {
    matches!(
        rv,
        MirRvalue::Use(_)
            | MirRvalue::BinaryOp(..)
            | MirRvalue::UnaryOp(..)
            | MirRvalue::GlobalLoad { .. }
            | MirRvalue::StructCreate { .. }
            | MirRvalue::EnumCreate { .. }
            | MirRvalue::FieldAccess { .. }
            | MirRvalue::IndexAccess { .. }
            | MirRvalue::ArrayLit { .. }
            | MirRvalue::Cast { .. }
            | MirRvalue::EnvLoad { .. }
            | MirRvalue::DynCoerce { .. }
            | MirRvalue::PtrIsNull { .. }
            | MirRvalue::PtrNull { .. }
            | MirRvalue::PtrOffset { .. }
            | MirRvalue::FnAddr { .. }
    )
}

/// Within one block, a copy `d = s` stands for `s` until either is
/// reassigned: `tmp = a; tmp.x = v` writes through `a` directly. A copy
/// left with no reads anywhere is dropped, which saves its GC root slot.
fn propagate_local(f: &mut MirFunction) {
    let n = f.locals.len();
    let mut defs = vec![0u32; n];
    for b in &f.blocks {
        for st in &b.statements {
            if let Some(l) = assigned_local(st) {
                defs[l.0 as usize] += 1;
            }
        }
    }
    let mut changed = false;
    for b in &mut f.blocks {
        let mut alias: HashMap<MirLocal, MirLocal> = HashMap::new();
        for st in &mut b.statements {
            if !alias.is_empty() {
                statement_operands_mut(st, &mut |op: &mut MirOperand| {
                    if let MirOperand::Copy(l) = op {
                        if let Some(&s) = alias.get(l) {
                            *l = s;
                            changed = true;
                        }
                    }
                });
            }
            if let Some(d) = assigned_local(st) {
                alias.retain(|k, v| *k != d && *v != d);
                if let MirStatement::Assign {
                    rvalue: MirRvalue::Use(MirOperand::Copy(src)),
                    ..
                } = st
                {
                    let (di, si) = (d.0 as usize, src.0 as usize);
                    if d != *src
                        && !f.locals[di].is_param
                        && defs[di] == 1
                        && f.locals[di].ty == f.locals[si].ty
                    {
                        alias.insert(d, *src);
                    }
                }
            }
        }
        if !alias.is_empty() {
            terminator_operands_mut(&mut b.terminator, &mut |op: &mut MirOperand| {
                if let MirOperand::Copy(l) = op {
                    if let Some(&s) = alias.get(l) {
                        *l = s;
                        changed = true;
                    }
                }
            });
        }
    }
    if !changed {
        return;
    }
    let mut used = vec![false; n];
    for b in &f.blocks {
        for st in &b.statements {
            statement_operands(st, &mut |op: &MirOperand| {
                if let MirOperand::Copy(l) = op {
                    used[l.0 as usize] = true;
                }
            });
        }
        terminator_operands(&b.terminator, &mut |op: &MirOperand| {
            if let MirOperand::Copy(l) = op {
                used[l.0 as usize] = true;
            }
        });
    }
    for b in &mut f.blocks {
        for st in &mut b.statements {
            let dead = match st {
                MirStatement::Assign {
                    dst,
                    rvalue: MirRvalue::Use(MirOperand::Copy(_)),
                } => !used[dst.0 as usize] && !f.locals[dst.0 as usize].is_param,
                _ => false,
            };
            if dead {
                *st = MirStatement::Nop;
            }
        }
    }
}

fn propagate_global(f: &mut MirFunction) {
    let n = f.locals.len();
    let mut defs = vec![0u32; n];
    let mut def_block = vec![usize::MAX; n];
    for (bi, b) in f.blocks.iter().enumerate() {
        for st in &b.statements {
            if let Some(l) = assigned_local(st) {
                defs[l.0 as usize] += 1;
                def_block[l.0 as usize] = bi;
            }
        }
    }
    let in_loop = blocks_in_cycles(f);
    let stable = |l: MirLocal| {
        let i = l.0 as usize;
        if f.locals[i].is_param {
            defs[i] == 0
        } else {
            defs[i] == 1 && !in_loop[def_block[i]]
        }
    };

    let mut alias: HashMap<MirLocal, MirLocal> = HashMap::new();
    for b in &f.blocks {
        for st in &b.statements {
            let MirStatement::Assign {
                dst,
                rvalue: MirRvalue::Use(MirOperand::Copy(src)),
            } = st
            else {
                continue;
            };
            let d = dst.0 as usize;
            if dst == src
                || f.locals[d].is_param
                || defs[d] != 1
                || !stable(*src)
                || f.locals[d].ty != f.locals[src.0 as usize].ty
            {
                continue;
            }
            alias.insert(*dst, *src);
        }
    }
    if alias.is_empty() {
        return;
    }
    let resolve = |mut l: MirLocal| {
        while let Some(&next) = alias.get(&l) {
            l = next;
        }
        l
    };
    let mut rename = |op: &mut MirOperand| {
        if let MirOperand::Copy(l) = op {
            *l = resolve(*l);
        }
    };
    for b in &mut f.blocks {
        for st in &mut b.statements {
            let dead = match st {
                MirStatement::Assign { dst, .. } => alias.contains_key(dst),
                MirStatement::StorageLive(l) | MirStatement::StorageDead(l) => {
                    alias.contains_key(l)
                }
                _ => false,
            };
            if dead {
                *st = MirStatement::Nop;
            } else {
                statement_operands_mut(st, &mut rename);
            }
        }
        terminator_operands_mut(&mut b.terminator, &mut rename);
    }
}

/// Whether each block lies on a cycle of the control flow graph.
fn blocks_in_cycles(f: &MirFunction) -> Vec<bool> {
    let n = f.blocks.len();
    let index: HashMap<u32, usize> = f
        .blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (b.id.0, i))
        .collect();
    let succ: Vec<Vec<usize>> = f
        .blocks
        .iter()
        .map(|b| {
            b.terminator
                .successors()
                .into_iter()
                .filter_map(|s| index.get(&s.0).copied())
                .collect()
        })
        .collect();

    // Iterative Tarjan strongly connected components.
    let mut order = vec![usize::MAX; n];
    let mut low = vec![0usize; n];
    let mut on_stack = vec![false; n];
    let mut stack = Vec::new();
    let mut in_cycle = vec![false; n];
    let mut counter = 0;
    for root in 0..n {
        if order[root] != usize::MAX {
            continue;
        }
        let mut work = vec![(root, 0usize)];
        order[root] = counter;
        low[root] = counter;
        counter += 1;
        stack.push(root);
        on_stack[root] = true;
        while let Some(&mut (v, ref mut next)) = work.last_mut() {
            if *next < succ[v].len() {
                let w = succ[v][*next];
                *next += 1;
                if order[w] == usize::MAX {
                    order[w] = counter;
                    low[w] = counter;
                    counter += 1;
                    stack.push(w);
                    on_stack[w] = true;
                    work.push((w, 0));
                } else if on_stack[w] {
                    low[v] = low[v].min(order[w]);
                }
                continue;
            }
            work.pop();
            if let Some(&(parent, _)) = work.last() {
                low[parent] = low[parent].min(low[v]);
            }
            if low[v] == order[v] {
                let mut members = Vec::new();
                loop {
                    let w = stack.pop().expect("tarjan stack");
                    on_stack[w] = false;
                    members.push(w);
                    if w == v {
                        break;
                    }
                }
                let cyclic = members.len() > 1 || succ[v].contains(&v);
                for m in members {
                    in_cycle[m] = cyclic;
                }
            }
        }
    }
    in_cycle
}
