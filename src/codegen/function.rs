//! Lowering of a single [`MirFunction`] into Cranelift IR.
//!
//! Each [`FunctionLowering`] owns a Cranelift `FunctionBuilder` plus
//! the mapping from MIR locals to stack slots and MIR block ids to
//! Cranelift blocks. The lowering is a single sweep: parameters spill
//! into slots first, then each block's statements emit and the
//! terminator closes the block.

use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::types;
use cranelift_codegen::ir::{
    Function, InstBuilder, Signature, StackSlotData, StackSlotKind, Type as CType, Value,
};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::Module;

use crate::mir::{
    MirBinOp, MirBlock, MirBlockId, MirConstant, MirFnRef, MirFunction, MirLocal, MirOperand,
    MirRvalue, MirStatement, MirTerminator, MirType, MirUnOp,
};

use super::context::ModuleCx;
use super::intrinsics;
use super::CodegenError;

/// Translate a `MirType` to a Cranelift `Type`. Returns `None` for
/// `Unit` (which has no machine representation) and for any deferred
/// aggregate or heap type.
pub fn cranelift_ty(ty: &MirType, ptr: CType) -> Option<CType> {
    match ty {
        MirType::Unit => None,
        MirType::Bool => Some(types::I8),
        MirType::Int => Some(types::I64),
        MirType::Float => Some(types::F64),
        MirType::Char => Some(types::I32),
        // Strings and aggregates flow through pointers in the future.
        // The MVP does not materialize them as Cranelift values; the
        // caller handles literals through the string table directly.
        MirType::Str => Some(ptr),
        MirType::Struct { .. }
        | MirType::Enum { .. }
        | MirType::Option(_)
        | MirType::Result(_, _)
        | MirType::List(_)
        | MirType::Function { .. } => Some(ptr),
    }
}

/// Per local lowering record.
#[derive(Clone, Copy)]
struct LocalSlot {
    /// Cranelift stack slot, `None` for `Unit` locals.
    slot: Option<cranelift_codegen::ir::StackSlot>,
    /// Cranelift type, `None` for `Unit` locals.
    ty: Option<CType>,
}

/// Lowering driver for a single function.
pub struct FunctionLowering<'cx, 'func> {
    cx: &'cx mut ModuleCx,
    func: &'func MirFunction,
    builder_ctx: FunctionBuilderContext,
    cranelift_func: &'func mut Function,
}

impl<'cx, 'func> FunctionLowering<'cx, 'func> {
    /// Construct a lowering for `mir` writing into `cranelift_func`.
    pub fn new(
        cx: &'cx mut ModuleCx,
        cranelift_func: &'func mut Function,
        mir: &'func MirFunction,
    ) -> Self {
        Self {
            cx,
            func: mir,
            builder_ctx: FunctionBuilderContext::new(),
            cranelift_func,
        }
    }

    /// Run the lowering. Mutates `self.cranelift_func` in place.
    pub fn lower(&mut self) -> Result<(), CodegenError> {
        let ptr = self.cx.pointer_type();
        // Borrow the cranelift function exclusively for builder use.
        let mut builder = FunctionBuilder::new(self.cranelift_func, &mut self.builder_ctx);

        // Allocate one Cranelift block per MIR block.
        let mut blocks = Vec::with_capacity(self.func.blocks.len());
        for _ in &self.func.blocks {
            blocks.push(builder.create_block());
        }
        let entry = blocks[self.func.entry.0 as usize];
        // The entry block gets the function's signature parameters.
        builder.append_block_params_for_function_params(entry);

        // Allocate one stack slot per local with a machine type.
        let mut slots: Vec<LocalSlot> = Vec::with_capacity(self.func.locals.len());
        for decl in &self.func.locals {
            let ty = cranelift_ty(&decl.ty, ptr);
            let slot = ty.map(|t| {
                builder.create_sized_stack_slot(StackSlotData::new(
                    StackSlotKind::ExplicitSlot,
                    t.bytes(),
                ))
            });
            slots.push(LocalSlot { slot, ty });
        }

        // Spill the incoming parameters into their slots.
        builder.switch_to_block(entry);
        let entry_params: Vec<Value> = builder.block_params(entry).to_vec();
        let mut entry_param_iter = entry_params.into_iter();
        for (i, param_local) in self.func.params.iter().enumerate() {
            let slot_info = slots[param_local.0 as usize];
            match (slot_info.slot, slot_info.ty) {
                (Some(slot), Some(_ty)) => {
                    let v = entry_param_iter.next().unwrap_or_else(|| {
                        unreachable!(
                            "parameter count and block param count differ at index {}",
                            i
                        )
                    });
                    builder.ins().stack_store(v, slot, 0);
                }
                _ => {
                    // Unit parameter: no machine value to consume.
                }
            }
        }

        // Lower each block.
        for (idx, mir_block) in self.func.blocks.iter().enumerate() {
            if idx != self.func.entry.0 as usize {
                builder.switch_to_block(blocks[idx]);
            } else {
                // entry was already switched above; ensure it is current
                builder.switch_to_block(blocks[idx]);
            }
            lower_block(self.cx, &mut builder, mir_block, &slots, &blocks)?;
        }

        builder.seal_all_blocks();
        builder.finalize();
        Ok(())
    }
}

fn lower_block(
    cx: &mut ModuleCx,
    builder: &mut FunctionBuilder<'_>,
    mir_block: &MirBlock,
    slots: &[LocalSlot],
    blocks: &[cranelift_codegen::ir::Block],
) -> Result<(), CodegenError> {
    for stmt in &mir_block.statements {
        lower_stmt(cx, builder, stmt, slots)?;
    }
    lower_terminator(cx, builder, &mir_block.terminator, slots, blocks)?;
    Ok(())
}

fn lower_stmt(
    cx: &mut ModuleCx,
    builder: &mut FunctionBuilder<'_>,
    stmt: &MirStatement,
    slots: &[LocalSlot],
) -> Result<(), CodegenError> {
    match stmt {
        MirStatement::Assign { dst, rvalue } => {
            let value = lower_rvalue(cx, builder, rvalue, slots)?;
            store_local(builder, slots, *dst, value);
            Ok(())
        }
        MirStatement::StorageLive(_) | MirStatement::StorageDead(_) | MirStatement::Nop => Ok(()),
    }
}

/// Result of lowering an rvalue: either a Cranelift `Value` for a
/// machine sized result, or `None` for a `Unit` valued operation.
type RValue = Option<Value>;

fn lower_rvalue(
    cx: &mut ModuleCx,
    builder: &mut FunctionBuilder<'_>,
    rvalue: &MirRvalue,
    slots: &[LocalSlot],
) -> Result<RValue, CodegenError> {
    match rvalue {
        MirRvalue::Use(op) => lower_operand(cx, builder, op, slots),
        MirRvalue::BinaryOp(op, lhs, rhs) => {
            let lhs_v = require_value(lower_operand(cx, builder, lhs, slots)?, "binop lhs")?;
            let rhs_v = require_value(lower_operand(cx, builder, rhs, slots)?, "binop rhs")?;
            Ok(Some(emit_binop(builder, *op, lhs_v, rhs_v)))
        }
        MirRvalue::UnaryOp(op, inner) => {
            let v = require_value(lower_operand(cx, builder, inner, slots)?, "unop operand")?;
            Ok(Some(emit_unop(builder, *op, v)))
        }
        MirRvalue::Call { callee, args } => lower_call(cx, builder, callee, args, slots),
        MirRvalue::Cast { operand, target } => {
            let v = require_value(lower_operand(cx, builder, operand, slots)?, "cast operand")?;
            Ok(Some(emit_cast(builder, v, cx.pointer_type(), target)))
        }
        MirRvalue::StructCreate { .. }
        | MirRvalue::EnumCreate { .. }
        | MirRvalue::FieldAccess { .. }
        | MirRvalue::IndexAccess { .. }
        | MirRvalue::ArrayLit { .. }
        | MirRvalue::ClosureCreate { .. } => Err(CodegenError::Unsupported(format!(
            "rvalue not supported in MVP backend: {:?}",
            rvalue_kind(rvalue)
        ))),
    }
}

fn rvalue_kind(r: &MirRvalue) -> &'static str {
    match r {
        MirRvalue::Use(_) => "Use",
        MirRvalue::BinaryOp(..) => "BinaryOp",
        MirRvalue::UnaryOp(..) => "UnaryOp",
        MirRvalue::Call { .. } => "Call",
        MirRvalue::StructCreate { .. } => "StructCreate",
        MirRvalue::EnumCreate { .. } => "EnumCreate",
        MirRvalue::FieldAccess { .. } => "FieldAccess",
        MirRvalue::IndexAccess { .. } => "IndexAccess",
        MirRvalue::ArrayLit { .. } => "ArrayLit",
        MirRvalue::Cast { .. } => "Cast",
        MirRvalue::ClosureCreate { .. } => "ClosureCreate",
    }
}

fn lower_operand(
    cx: &mut ModuleCx,
    builder: &mut FunctionBuilder<'_>,
    op: &MirOperand,
    slots: &[LocalSlot],
) -> Result<RValue, CodegenError> {
    match op {
        MirOperand::Copy(local) => Ok(load_local(builder, slots, *local)),
        MirOperand::Const(c) => lower_constant(cx, builder, c),
    }
}

fn lower_constant(
    cx: &mut ModuleCx,
    builder: &mut FunctionBuilder<'_>,
    c: &MirConstant,
) -> Result<RValue, CodegenError> {
    match c {
        MirConstant::Unit => Ok(None),
        MirConstant::Bool(b) => {
            let v = builder.ins().iconst(types::I8, if *b { 1 } else { 0 });
            Ok(Some(v))
        }
        MirConstant::Int(n) => Ok(Some(builder.ins().iconst(types::I64, *n))),
        MirConstant::Float(f) => Ok(Some(builder.ins().f64const(*f))),
        MirConstant::Char(c) => Ok(Some(builder.ins().iconst(types::I32, *c as i64))),
        MirConstant::Str(s) => {
            // String constants in the MVP only flow into the print
            // intrinsic. We materialize the pointer to the interned
            // bytes here so the caller can read the address and the
            // length companion separately.
            let id = cx.intern_string(s.as_bytes())?;
            let local_id = cx.module().declare_data_in_func(id, builder.func);
            let ptr = cx.pointer_type();
            Ok(Some(builder.ins().symbol_value(ptr, local_id)))
        }
    }
}

fn load_local(builder: &mut FunctionBuilder<'_>, slots: &[LocalSlot], local: MirLocal) -> RValue {
    let info = slots[local.0 as usize];
    let (slot, ty) = match (info.slot, info.ty) {
        (Some(s), Some(t)) => (s, t),
        _ => return None,
    };
    Some(builder.ins().stack_load(ty, slot, 0))
}

fn store_local(
    builder: &mut FunctionBuilder<'_>,
    slots: &[LocalSlot],
    local: MirLocal,
    value: RValue,
) {
    let info = slots[local.0 as usize];
    let slot = match info.slot {
        Some(s) => s,
        None => return,
    };
    let v = match value {
        Some(v) => v,
        None => return,
    };
    builder.ins().stack_store(v, slot, 0);
}

fn require_value(v: RValue, what: &'static str) -> Result<Value, CodegenError> {
    v.ok_or_else(|| CodegenError::Unsupported(format!("{} used a Unit value", what)))
}

fn emit_binop(builder: &mut FunctionBuilder<'_>, op: MirBinOp, lhs: Value, rhs: Value) -> Value {
    let ty = builder.func.dfg.value_type(lhs);
    let is_float = ty == types::F64;
    let is_bool = ty == types::I8;
    match op {
        MirBinOp::Add if is_float => builder.ins().fadd(lhs, rhs),
        MirBinOp::Sub if is_float => builder.ins().fsub(lhs, rhs),
        MirBinOp::Mul if is_float => builder.ins().fmul(lhs, rhs),
        MirBinOp::Div if is_float => builder.ins().fdiv(lhs, rhs),
        MirBinOp::Add => builder.ins().iadd(lhs, rhs),
        MirBinOp::Sub => builder.ins().isub(lhs, rhs),
        MirBinOp::Mul => builder.ins().imul(lhs, rhs),
        MirBinOp::Div => builder.ins().sdiv(lhs, rhs),
        MirBinOp::Mod => builder.ins().srem(lhs, rhs),
        MirBinOp::Eq => emit_compare(builder, op, lhs, rhs, is_float),
        MirBinOp::Ne => emit_compare(builder, op, lhs, rhs, is_float),
        MirBinOp::Lt => emit_compare(builder, op, lhs, rhs, is_float),
        MirBinOp::Le => emit_compare(builder, op, lhs, rhs, is_float),
        MirBinOp::Gt => emit_compare(builder, op, lhs, rhs, is_float),
        MirBinOp::Ge => emit_compare(builder, op, lhs, rhs, is_float),
        MirBinOp::And if is_bool => builder.ins().band(lhs, rhs),
        MirBinOp::Or if is_bool => builder.ins().bor(lhs, rhs),
        MirBinOp::And => builder.ins().band(lhs, rhs),
        MirBinOp::Or => builder.ins().bor(lhs, rhs),
        MirBinOp::BitAnd => builder.ins().band(lhs, rhs),
        MirBinOp::BitOr => builder.ins().bor(lhs, rhs),
        MirBinOp::BitXor => builder.ins().bxor(lhs, rhs),
        MirBinOp::Shl => builder.ins().ishl(lhs, rhs),
        MirBinOp::Shr => builder.ins().sshr(lhs, rhs),
    }
}

fn emit_compare(
    builder: &mut FunctionBuilder<'_>,
    op: MirBinOp,
    lhs: Value,
    rhs: Value,
    is_float: bool,
) -> Value {
    if is_float {
        let cc = match op {
            MirBinOp::Eq => FloatCC::Equal,
            MirBinOp::Ne => FloatCC::NotEqual,
            MirBinOp::Lt => FloatCC::LessThan,
            MirBinOp::Le => FloatCC::LessThanOrEqual,
            MirBinOp::Gt => FloatCC::GreaterThan,
            MirBinOp::Ge => FloatCC::GreaterThanOrEqual,
            _ => unreachable!("emit_compare only handles comparison binops"),
        };
        let cmp = builder.ins().fcmp(cc, lhs, rhs);
        // fcmp returns an i8 in Cranelift's IR.
        cmp
    } else {
        let cc = match op {
            MirBinOp::Eq => IntCC::Equal,
            MirBinOp::Ne => IntCC::NotEqual,
            MirBinOp::Lt => IntCC::SignedLessThan,
            MirBinOp::Le => IntCC::SignedLessThanOrEqual,
            MirBinOp::Gt => IntCC::SignedGreaterThan,
            MirBinOp::Ge => IntCC::SignedGreaterThanOrEqual,
            _ => unreachable!("emit_compare only handles comparison binops"),
        };
        builder.ins().icmp(cc, lhs, rhs)
    }
}

fn emit_unop(builder: &mut FunctionBuilder<'_>, op: MirUnOp, v: Value) -> Value {
    let ty = builder.func.dfg.value_type(v);
    let is_float = ty == types::F64;
    match op {
        MirUnOp::Neg if is_float => builder.ins().fneg(v),
        MirUnOp::Neg => builder.ins().ineg(v),
        MirUnOp::Not => {
            // `Bool` is i8; emit `v xor 1` to flip the low bit.
            let one = builder.ins().iconst(types::I8, 1);
            builder.ins().bxor(v, one)
        }
        MirUnOp::Ref => {
            // The address operator is not lowerable in the MVP.
            // Return the value unchanged so the function still compiles
            // for the type checker's benefit; any user reaching this
            // path will see incorrect runtime behavior, which is fine
            // because the type checker rejects taking the address of a
            // primitive in the MVP front end.
            v
        }
    }
}

fn emit_cast(builder: &mut FunctionBuilder<'_>, v: Value, _ptr: CType, target: &MirType) -> Value {
    let src_ty = builder.func.dfg.value_type(v);
    let dst_ty = match target {
        MirType::Int => types::I64,
        MirType::Float => types::F64,
        MirType::Bool => types::I8,
        MirType::Char => types::I32,
        _ => return v,
    };
    if src_ty == dst_ty {
        return v;
    }
    // Integer to integer narrowing or widening.
    if src_ty.is_int() && dst_ty.is_int() {
        if dst_ty.bytes() > src_ty.bytes() {
            return builder.ins().sextend(dst_ty, v);
        }
        if dst_ty.bytes() < src_ty.bytes() {
            return builder.ins().ireduce(dst_ty, v);
        }
        return v;
    }
    // Integer to float and vice versa.
    if src_ty.is_int() && dst_ty == types::F64 {
        return builder.ins().fcvt_from_sint(types::F64, v);
    }
    if src_ty == types::F64 && dst_ty.is_int() {
        return builder.ins().fcvt_to_sint_sat(dst_ty, v);
    }
    v
}

fn lower_call(
    cx: &mut ModuleCx,
    builder: &mut FunctionBuilder<'_>,
    callee: &MirFnRef,
    args: &[MirOperand],
    slots: &[LocalSlot],
) -> Result<RValue, CodegenError> {
    if intrinsics::is_intrinsic(&callee.mangled) {
        return lower_intrinsic(cx, builder, &callee.mangled, args, slots);
    }
    let func_id = cx.function_id(&callee.mangled).ok_or_else(|| {
        CodegenError::Unsupported(format!("unresolved callee: {}", callee.mangled))
    })?;
    let local_ref = cx.module().declare_func_in_func(func_id, builder.func);
    let mut arg_vals = Vec::with_capacity(args.len());
    for a in args {
        if let Some(v) = lower_operand(cx, builder, a, slots)? {
            arg_vals.push(v);
        }
    }
    let inst = builder.ins().call(local_ref, &arg_vals);
    let results = builder.inst_results(inst);
    Ok(results.first().copied())
}

fn lower_intrinsic(
    cx: &mut ModuleCx,
    builder: &mut FunctionBuilder<'_>,
    mangled: &str,
    args: &[MirOperand],
    _slots: &[LocalSlot],
) -> Result<RValue, CodegenError> {
    match mangled {
        intrinsics::PRINT => {
            if args.len() != 1 {
                return Err(CodegenError::Unsupported(format!(
                    "print intrinsic expects 1 arg, got {}",
                    args.len()
                )));
            }
            let (ptr_val, len_val) = lower_string_arg(cx, builder, &args[0])?;
            let func_id = cx
                .runtime_id(intrinsics::RUNTIME_PRINTLN_STR)
                .expect("runtime imports declared at module init");
            let local_ref = cx.module().declare_func_in_func(func_id, builder.func);
            builder.ins().call(local_ref, &[ptr_val, len_val]);
            Ok(None)
        }
        _ => Err(CodegenError::Unsupported(format!(
            "unknown intrinsic: {}",
            mangled
        ))),
    }
}

/// Produce a `(pointer, length)` pair for a string argument that
/// reaches an intrinsic.
///
/// Only literal strings are supported in the MVP. Non literal string
/// values would need the full object layout from issue #65.
fn lower_string_arg(
    cx: &mut ModuleCx,
    builder: &mut FunctionBuilder<'_>,
    op: &MirOperand,
) -> Result<(Value, Value), CodegenError> {
    match op {
        MirOperand::Const(MirConstant::Str(s)) => {
            let bytes = s.as_bytes();
            let id = cx.intern_string(bytes)?;
            let local_id = cx.module().declare_data_in_func(id, builder.func);
            let ptr = cx.pointer_type();
            let ptr_val = builder.ins().symbol_value(ptr, local_id);
            let len_val = builder.ins().iconst(ptr, bytes.len() as i64);
            Ok((ptr_val, len_val))
        }
        _ => Err(CodegenError::Unsupported(
            "print currently only accepts a string literal argument".into(),
        )),
    }
}

fn lower_terminator(
    cx: &mut ModuleCx,
    builder: &mut FunctionBuilder<'_>,
    term: &MirTerminator,
    slots: &[LocalSlot],
    blocks: &[cranelift_codegen::ir::Block],
) -> Result<(), CodegenError> {
    match term {
        MirTerminator::Goto(target) => {
            let b = blocks[target.0 as usize];
            builder.ins().jump(b, &[]);
            Ok(())
        }
        MirTerminator::SwitchInt {
            discriminant,
            targets,
            otherwise,
        } => lower_switch_int(
            cx,
            builder,
            discriminant,
            targets,
            *otherwise,
            slots,
            blocks,
        ),
        MirTerminator::SwitchEnum { .. } => {
            // Enum dispatch needs heap layouts; trap and move on.
            builder
                .ins()
                .trap(cranelift_codegen::ir::TrapCode::UnreachableCodeReached);
            Ok(())
        }
        MirTerminator::Return(op) => {
            let v = lower_operand(cx, builder, op, slots)?;
            match v {
                Some(value) => {
                    builder.ins().return_(&[value]);
                }
                None => {
                    builder.ins().return_(&[]);
                }
            }
            Ok(())
        }
        MirTerminator::Unreachable => {
            builder
                .ins()
                .trap(cranelift_codegen::ir::TrapCode::UnreachableCodeReached);
            Ok(())
        }
    }
}

fn lower_switch_int(
    cx: &mut ModuleCx,
    builder: &mut FunctionBuilder<'_>,
    discriminant: &MirOperand,
    targets: &[(i64, MirBlockId)],
    otherwise: MirBlockId,
    slots: &[LocalSlot],
    blocks: &[cranelift_codegen::ir::Block],
) -> Result<(), CodegenError> {
    let disc = require_value(
        lower_operand(cx, builder, discriminant, slots)?,
        "switch discriminant",
    )?;
    let disc_ty = builder.func.dfg.value_type(disc);
    // Widen i8 discriminants (typically Bool) to i64 so all comparisons
    // share a single integer type; this matches the conventional
    // expansion of an if into a switch_int with two integer targets.
    let disc_wide = if disc_ty == types::I64 {
        disc
    } else {
        builder.ins().uextend(types::I64, disc)
    };

    // Walk the targets and emit a cascade of `brif` against each value.
    // The final fall through goes to `otherwise`.
    for (value, target) in targets {
        let imm = builder.ins().iconst(types::I64, *value);
        let cmp = builder.ins().icmp(IntCC::Equal, disc_wide, imm);
        let target_block = blocks[target.0 as usize];
        let continue_block = builder.create_block();
        builder
            .ins()
            .brif(cmp, target_block, &[], continue_block, &[]);
        builder.seal_block(continue_block);
        builder.switch_to_block(continue_block);
    }

    let otherwise_block = blocks[otherwise.0 as usize];
    builder.ins().jump(otherwise_block, &[]);
    Ok(())
}

/// Build a Cranelift `Signature` matching the parameter and return
/// types of a MIR function. Used by both the declaration and
/// definition paths so the shapes never diverge.
pub fn build_signature(func: &MirFunction, ptr: CType, base: Signature) -> Signature {
    let mut sig = base;
    sig.params.clear();
    sig.returns.clear();
    for p in func.params.iter() {
        let decl = func.local_decl(*p);
        if let Some(t) = cranelift_ty(&decl.ty, ptr) {
            sig.params.push(cranelift_codegen::ir::AbiParam::new(t));
        }
    }
    if let Some(t) = cranelift_ty(&func.ret_ty, ptr) {
        sig.returns.push(cranelift_codegen::ir::AbiParam::new(t));
    }
    sig
}
