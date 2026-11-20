//! Forward copies of locals that never change.
//!
//! A local assigned exactly once from another local `s` is replaced by `s`
//! when `s` holds one value for the whole function: a parameter that is
//! never reassigned, or a local assigned exactly once outside any loop.
//! Inlining leaves many such copies (each inlined parameter), and every
//! GC pointer copy would otherwise cost its own root slot.

use std::collections::HashMap;

use super::ir::{MirFunction, MirLocal, MirOperand, MirProgram, MirRvalue, MirStatement};
use super::visit::{assigned_local, statement_operands_mut, terminator_operands_mut};

pub fn propagate_copies(program: &mut MirProgram) {
    for f in &mut program.functions {
        propagate_in(f);
    }
}

fn propagate_in(f: &mut MirFunction) {
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
