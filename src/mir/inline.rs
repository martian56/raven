//! Inline small leaf functions into their callers.
//!
//! Cranelift does not inline across functions, so tiny helpers (field
//! getters, `Int` hashing and equality, hash table probes) cost a full call
//! and a GC root frame each. A callee qualifies when it is small, not
//! recursive, and has no defers.

use std::collections::HashMap;

use super::ir::{
    MirBlock, MirBlockId, MirFunction, MirLocal, MirLocalDecl, MirOperand, MirProgram, MirRvalue,
    MirStatement, MirTerminator,
};

const MAX_STATEMENTS: usize = 40;
const MAX_BLOCKS: usize = 24;
const ROUNDS: usize = 2;

pub fn inline_small_functions(program: &mut MirProgram) {
    for _ in 0..ROUNDS {
        let candidates: HashMap<String, MirFunction> = program
            .functions
            .iter()
            .filter(|f| inlinable(f))
            .map(|f| (f.name.clone(), f.clone()))
            .collect();
        if candidates.is_empty() {
            return;
        }
        let mut changed = false;
        for f in &mut program.functions {
            changed |= inline_into(f, &candidates);
        }
        if !changed {
            return;
        }
    }
}

fn inlinable(f: &MirFunction) -> bool {
    if f.has_defer || f.blocks.len() > MAX_BLOCKS || f.origin == "main" {
        return false;
    }
    let mut count = 0;
    for b in &f.blocks {
        if !matches!(
            b.terminator,
            MirTerminator::Goto(_)
                | MirTerminator::SwitchInt { .. }
                | MirTerminator::Return(_)
                | MirTerminator::Unreachable
        ) {
            return false;
        }
        for st in &b.statements {
            count += 1;
            let ok = match st {
                MirStatement::Assign { rvalue, .. } => simple_rvalue(rvalue, &f.name),
                MirStatement::StoreField { .. }
                | MirStatement::StoreIndex { .. }
                | MirStatement::StorageLive(_)
                | MirStatement::StorageDead(_)
                | MirStatement::Nop => true,
                _ => false,
            };
            if !ok {
                return false;
            }
        }
    }
    count <= MAX_STATEMENTS
}

fn simple_rvalue(r: &MirRvalue, own: &str) -> bool {
    match r {
        MirRvalue::Call { callee, .. } => callee.mangled != own,
        _ => matches!(
            r,
            MirRvalue::Use(_)
                | MirRvalue::BinaryOp(..)
                | MirRvalue::UnaryOp(..)
                | MirRvalue::FieldAccess { .. }
                | MirRvalue::Cast { .. }
                | MirRvalue::IndexAccess { .. }
                | MirRvalue::ListMethod { .. }
        ),
    }
}

fn inline_into(caller: &mut MirFunction, candidates: &HashMap<String, MirFunction>) -> bool {
    let mut changed = false;
    let mut bi = 0;
    while bi < caller.blocks.len() {
        let site = caller.blocks[bi].statements.iter().position(|st| match st {
            MirStatement::Assign {
                rvalue: MirRvalue::Call { callee, .. },
                ..
            } => callee.mangled != caller.name && candidates.contains_key(&callee.mangled),
            _ => false,
        });
        let Some(k) = site else {
            bi += 1;
            continue;
        };
        let MirStatement::Assign {
            dst,
            rvalue: MirRvalue::Call { callee, args },
        } = caller.blocks[bi].statements[k].clone()
        else {
            unreachable!()
        };
        let callee_fn = &candidates[&callee.mangled];
        splice(caller, bi, k, dst, &args, callee_fn);
        changed = true;
    }
    changed
}

/// Replace the call at statement `k` of block `bi` with the callee's body.
fn splice(
    caller: &mut MirFunction,
    bi: usize,
    k: usize,
    dst: MirLocal,
    args: &[MirOperand],
    callee: &MirFunction,
) {
    let local_base = caller.locals.len() as u32;
    for decl in &callee.locals {
        caller.locals.push(MirLocalDecl {
            is_param: false,
            ..decl.clone()
        });
    }
    let block_base = caller.blocks.len() as u32;
    let cont_id = MirBlockId(block_base + callee.blocks.len() as u32);
    let map_local = |l: MirLocal| MirLocal(l.0 + local_base);
    let map_block = |b: MirBlockId| MirBlockId(b.0 + block_base);

    let block = &mut caller.blocks[bi];
    let rest: Vec<MirStatement> = block.statements.drain(k..).skip(1).collect();
    let after = std::mem::replace(
        &mut block.terminator,
        MirTerminator::Goto(map_block(callee.entry)),
    );
    for (param, arg) in callee.params.iter().zip(args) {
        block.statements.push(MirStatement::Assign {
            dst: map_local(*param),
            rvalue: MirRvalue::Use(arg.clone()),
        });
    }

    for cb in &callee.blocks {
        let mut statements: Vec<MirStatement> = cb
            .statements
            .iter()
            .map(|st| map_statement(st, &map_local))
            .collect();
        let terminator = match &cb.terminator {
            MirTerminator::Return(op) => {
                statements.push(MirStatement::Assign {
                    dst,
                    rvalue: MirRvalue::Use(map_operand(op, &map_local)),
                });
                MirTerminator::Goto(cont_id)
            }
            MirTerminator::Goto(t) => MirTerminator::Goto(map_block(*t)),
            MirTerminator::SwitchInt {
                discriminant,
                targets,
                otherwise,
            } => MirTerminator::SwitchInt {
                discriminant: map_operand(discriminant, &map_local),
                targets: targets.iter().map(|(v, t)| (*v, map_block(*t))).collect(),
                otherwise: map_block(*otherwise),
            },
            other => other.clone(),
        };
        caller.blocks.push(MirBlock {
            id: map_block(cb.id),
            statements,
            terminator,
            is_loop_header: cb.is_loop_header,
        });
    }
    caller.blocks.push(MirBlock {
        id: cont_id,
        statements: rest,
        terminator: after,
        is_loop_header: false,
    });
}

fn map_operand(op: &MirOperand, f: &dyn Fn(MirLocal) -> MirLocal) -> MirOperand {
    match op {
        MirOperand::Copy(l) => MirOperand::Copy(f(*l)),
        other => other.clone(),
    }
}

fn map_statement(st: &MirStatement, f: &dyn Fn(MirLocal) -> MirLocal) -> MirStatement {
    match st {
        MirStatement::Assign { dst, rvalue } => MirStatement::Assign {
            dst: f(*dst),
            rvalue: map_rvalue(rvalue, f),
        },
        MirStatement::StoreField { base, index, value } => MirStatement::StoreField {
            base: map_operand(base, f),
            index: *index,
            value: map_operand(value, f),
        },
        MirStatement::StoreIndex { base, index, value } => MirStatement::StoreIndex {
            base: map_operand(base, f),
            index: map_operand(index, f),
            value: map_operand(value, f),
        },
        MirStatement::StorageLive(l) => MirStatement::StorageLive(f(*l)),
        MirStatement::StorageDead(l) => MirStatement::StorageDead(f(*l)),
        other => other.clone(),
    }
}

fn map_rvalue(r: &MirRvalue, f: &dyn Fn(MirLocal) -> MirLocal) -> MirRvalue {
    match r {
        MirRvalue::Use(op) => MirRvalue::Use(map_operand(op, f)),
        MirRvalue::BinaryOp(op, a, b) => {
            MirRvalue::BinaryOp(*op, map_operand(a, f), map_operand(b, f))
        }
        MirRvalue::UnaryOp(op, a) => MirRvalue::UnaryOp(*op, map_operand(a, f)),
        MirRvalue::FieldAccess { base, index } => MirRvalue::FieldAccess {
            base: map_operand(base, f),
            index: *index,
        },
        MirRvalue::Call { callee, args } => MirRvalue::Call {
            callee: callee.clone(),
            args: args.iter().map(|a| map_operand(a, f)).collect(),
        },
        MirRvalue::IndexAccess { base, index } => MirRvalue::IndexAccess {
            base: map_operand(base, f),
            index: map_operand(index, f),
        },
        MirRvalue::ListMethod {
            op,
            receiver,
            arg,
            elem_ty,
        } => MirRvalue::ListMethod {
            op: *op,
            receiver: map_operand(receiver, f),
            arg: arg.as_ref().map(|a| map_operand(a, f)),
            elem_ty: elem_ty.clone(),
        },
        MirRvalue::Cast { operand, target } => MirRvalue::Cast {
            operand: map_operand(operand, f),
            target: target.clone(),
        },
        other => other.clone(),
    }
}
