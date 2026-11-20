//! Operand traversal over MIR statements and terminators.

use super::ir::{MirLocal, MirOperand, MirRvalue, MirStatement, MirTerminator};

macro_rules! operand_visitors {
    ($rv:ident, $st:ident, $tm:ident $(, $m:tt)?) => {
        pub fn $rv(r: &$($m)* MirRvalue, f: &mut dyn FnMut(&$($m)* MirOperand)) {
            match r {
                MirRvalue::Use(a) | MirRvalue::UnaryOp(_, a) => f(a),
                MirRvalue::BinaryOp(_, a, b) => {
                    f(a);
                    f(b);
                }
                MirRvalue::Call { args, .. }
                | MirRvalue::StructCreate { fields: args, .. }
                | MirRvalue::EnumCreate { payload: args, .. }
                | MirRvalue::ArrayLit { elements: args, .. }
                | MirRvalue::ClosureCreate { captures: args, .. } => args.into_iter().for_each(&mut *f),
                MirRvalue::FieldAccess { base, .. } => f(base),
                MirRvalue::IndexAccess { base, index } => {
                    f(base);
                    f(index);
                }
                MirRvalue::ListMethod { receiver, arg, .. } => {
                    f(receiver);
                    if let Some(a) = arg {
                        f(a);
                    }
                }
                MirRvalue::Cast { operand, .. } => f(operand),
                MirRvalue::EnvLoad { env, .. } => f(env),
                MirRvalue::ClosureCall { closure, args, .. } => {
                    f(closure);
                    args.into_iter().for_each(&mut *f);
                }
                MirRvalue::VirtualCall { receiver, args, .. } => {
                    f(receiver);
                    args.into_iter().for_each(&mut *f);
                }
                MirRvalue::DynCoerce { value, .. } | MirRvalue::AnyBox { value, .. } => f(value),
                MirRvalue::PtrLoad { addr, .. } | MirRvalue::PtrIsNull { addr } => f(addr),
                MirRvalue::PtrOffset { addr, count, .. } => {
                    f(addr);
                    f(count);
                }
                MirRvalue::PtrAlloc { count, .. } => f(count),
                MirRvalue::AnyCast { any, .. }
                | MirRvalue::AnyTypeName { any }
                | MirRvalue::AnyFieldNames { any } => f(any),
                MirRvalue::AnyGetField { any, name, .. } => {
                    f(any);
                    f(name);
                }
                MirRvalue::AnySetField { any, name, value } => {
                    f(any);
                    f(name);
                    f(value);
                }
                MirRvalue::GlobalLoad { .. } | MirRvalue::PtrNull | MirRvalue::FnAddr { .. } => {}
            }
        }

        /// Visit the operands a statement reads. Assignment destinations and
        /// storage markers are not operands.
        pub fn $st(st: &$($m)* MirStatement, f: &mut dyn FnMut(&$($m)* MirOperand)) {
            match st {
                MirStatement::Assign { rvalue, .. } => $rv(rvalue, f),
                MirStatement::StoreField { base, value, .. } => {
                    f(base);
                    f(value);
                }
                MirStatement::StoreIndex { base, index, value } => {
                    f(base);
                    f(index);
                    f(value);
                }
                MirStatement::PtrStore { addr, value, .. } => {
                    f(addr);
                    f(value);
                }
                MirStatement::PtrFree { addr } => f(addr),
                MirStatement::StoreGlobal { value, .. } => f(value),
                MirStatement::StorageLive(_) | MirStatement::StorageDead(_) | MirStatement::Nop => {}
            }
        }

        pub fn $tm(t: &$($m)* MirTerminator, f: &mut dyn FnMut(&$($m)* MirOperand)) {
            match t {
                MirTerminator::SwitchInt { discriminant, .. }
                | MirTerminator::SwitchEnum { discriminant, .. } => f(discriminant),
                MirTerminator::Return(op) => f(op),
                MirTerminator::Goto(_) | MirTerminator::Unreachable => {}
            }
        }
    };
}

operand_visitors!(rvalue_operands, statement_operands, terminator_operands);
operand_visitors!(
    rvalue_operands_mut,
    statement_operands_mut,
    terminator_operands_mut,
    mut
);

/// The local a statement assigns, if any.
pub fn assigned_local(st: &MirStatement) -> Option<MirLocal> {
    match st {
        MirStatement::Assign { dst, .. } => Some(*dst),
        _ => None,
    }
}
