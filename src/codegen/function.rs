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
    Function, InstBuilder, MemFlags, Signature, StackSlot, StackSlotData, StackSlotKind,
    Type as CType, Value,
};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::Module;

use crate::mir::{
    MirBinOp, MirBlock, MirBlockId, MirConstant, MirFnRef, MirFunction, MirLocal, MirOperand,
    MirRvalue, MirStatement, MirTerminator, MirType, MirUnOp,
};

use super::context::ModuleCx;
use super::intrinsics;
use super::layout;
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
    slot: Option<StackSlot>,
    /// Cranelift type, `None` for `Unit` locals.
    ty: Option<CType>,
}

/// The GC root frame the function maintains for its lifetime. Holds the
/// stack slot of the contiguous root array and the count of GC locals it
/// covers. `None` when the function has no GC pointer locals.
#[derive(Clone, Copy)]
struct RootFrame {
    array: StackSlot,
    count: usize,
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

        // Allocate one stack slot per local with a machine type, and note
        // which locals hold GC pointers the collector must trace.
        let mut slots: Vec<LocalSlot> = Vec::with_capacity(self.func.locals.len());
        let mut gc_locals: Vec<MirLocal> = Vec::new();
        for (i, decl) in self.func.locals.iter().enumerate() {
            let ty = cranelift_ty(&decl.ty, ptr);
            let slot = ty.map(|t| {
                builder.create_sized_stack_slot(StackSlotData::new(
                    StackSlotKind::ExplicitSlot,
                    t.bytes(),
                ))
            });
            if layout::is_gc_pointer(&decl.ty) && slot.is_some() {
                gc_locals.push(MirLocal(i as u32));
            }
            slots.push(LocalSlot { slot, ty });
        }

        // Allocate the root array slot once when the function holds any
        // GC pointer locals. It is one pointer-sized slot per GC local.
        let root_frame = if gc_locals.is_empty() {
            None
        } else {
            let bytes = (gc_locals.len() as u32) * (ptr.bytes());
            let array = builder
                .create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, bytes));
            Some(RootFrame {
                array,
                count: gc_locals.len(),
            })
        };

        // Lower each block. The entry block first spills incoming
        // parameters, then sets up the GC root frame, then runs the MIR.
        let entry_idx = self.func.entry.0 as usize;
        for (idx, mir_block) in self.func.blocks.iter().enumerate() {
            builder.switch_to_block(blocks[idx]);
            if idx == entry_idx {
                Self::spill_params(&mut builder, self.func, &slots, entry);
                Self::enter_root_frame(
                    self.cx,
                    &mut builder,
                    self.func,
                    &slots,
                    &gc_locals,
                    root_frame,
                    ptr,
                );
            }
            lower_block(
                self.cx,
                &mut builder,
                mir_block,
                &slots,
                &blocks,
                root_frame,
            )?;
        }

        builder.seal_all_blocks();
        builder.finalize();
        Ok(())
    }

    /// Spill the entry block's incoming parameters into their stack
    /// slots so the MIR body reads them like any other local.
    fn spill_params(
        builder: &mut FunctionBuilder<'_>,
        func: &MirFunction,
        slots: &[LocalSlot],
        entry: cranelift_codegen::ir::Block,
    ) {
        let entry_params: Vec<Value> = builder.block_params(entry).to_vec();
        let mut iter = entry_params.into_iter();
        for (i, param_local) in func.params.iter().enumerate() {
            let slot_info = slots[param_local.0 as usize];
            if let (Some(slot), Some(_)) = (slot_info.slot, slot_info.ty) {
                let v = iter.next().unwrap_or_else(|| {
                    unreachable!(
                        "parameter count and block param count differ at index {}",
                        i
                    )
                });
                builder.ins().stack_store(v, slot, 0);
            }
        }
    }

    /// Set up the GC root frame for the function body.
    ///
    /// Every GC pointer local is first zeroed so a collection triggered
    /// before the body assigns it never reads an uninitialized slot.
    /// Parameter locals already hold their incoming pointer and are not
    /// re-zeroed. The slot address of each GC local is then written into
    /// the contiguous root array, and `raven_gc_enter_frame` registers
    /// the array. A matching `raven_gc_leave_frame` runs at every return.
    fn enter_root_frame(
        cx: &mut ModuleCx,
        builder: &mut FunctionBuilder<'_>,
        func: &MirFunction,
        slots: &[LocalSlot],
        gc_locals: &[MirLocal],
        root_frame: Option<RootFrame>,
        ptr: CType,
    ) {
        let Some(frame) = root_frame else {
            return;
        };
        let zero = builder.ins().iconst(ptr, 0);
        for (i, local) in gc_locals.iter().enumerate() {
            let info = slots[local.0 as usize];
            let slot = info.slot.expect("gc local has a stack slot");
            // Non-parameter GC locals start null so a collection that
            // fires before the body assigns them never follows a stale
            // pointer. Parameters already hold their spilled incoming
            // pointer and must not be clobbered.
            if !func.local_decl(*local).is_param {
                builder.ins().stack_store(zero, slot, 0);
            }
            // Record the slot's address in the contiguous root array; the
            // collector dereferences it to read the live pointer.
            let slot_addr = builder.ins().stack_addr(ptr, slot, 0);
            builder
                .ins()
                .stack_store(slot_addr, frame.array, (i as i32) * ptr.bytes() as i32);
        }
        let array_addr = builder.ins().stack_addr(ptr, frame.array, 0);
        let count = builder.ins().iconst(ptr, frame.count as i64);
        let enter = cx
            .runtime_id(intrinsics::RUNTIME_GC_ENTER_FRAME)
            .expect("gc enter frame declared at module init");
        let enter_ref = cx.module().declare_func_in_func(enter, builder.func);
        builder.ins().call(enter_ref, &[array_addr, count]);
    }
}

fn lower_block(
    cx: &mut ModuleCx,
    builder: &mut FunctionBuilder<'_>,
    mir_block: &MirBlock,
    slots: &[LocalSlot],
    blocks: &[cranelift_codegen::ir::Block],
    root_frame: Option<RootFrame>,
) -> Result<(), CodegenError> {
    for stmt in &mir_block.statements {
        lower_stmt(cx, builder, stmt, slots)?;
    }
    lower_terminator(
        cx,
        builder,
        &mir_block.terminator,
        slots,
        blocks,
        root_frame,
    )?;
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
        MirRvalue::StructCreate {
            ty,
            fields,
            field_tys,
        } => lower_struct_create(cx, builder, ty, fields, field_tys, slots),
        MirRvalue::EnumCreate {
            ty,
            variant,
            payload,
            payload_tys,
        } => lower_enum_create(cx, builder, ty, *variant, payload, payload_tys, slots),
        MirRvalue::FieldAccess { base, index } => {
            lower_field_access(cx, builder, base, *index, slots)
        }
        MirRvalue::ClosureCreate { fn_name, captures } => {
            lower_closure_create(cx, builder, fn_name, captures, slots)
        }
        MirRvalue::IndexAccess { .. } | MirRvalue::ArrayLit { .. } => {
            Err(CodegenError::Unsupported(format!(
                "rvalue not supported in MVP backend: {:?}",
                rvalue_kind(rvalue)
            )))
        }
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
    let (slot, want) = match (info.slot, info.ty) {
        (Some(s), Some(t)) => (s, t),
        _ => return,
    };
    let v = match value {
        Some(v) => v,
        None => return,
    };
    // Reconcile the produced value's type with the slot's declared type.
    // A field read hands back a pointer-width slot value; storing it into
    // a `Float` or narrow scalar local needs a bitcast or reduce so the
    // Cranelift store type checks.
    let v = narrow_from_slot(builder, v, want);
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

/// Widen a field operand to a pointer-sized value for storage in a
/// struct or enum slot. Scalars narrower than a pointer (`Bool`, `Char`,
/// and `Int` on a 32-bit host, which the MVP does not target) are zero
/// extended; floats are bit-cast through an integer so a slot can hold a
/// `Float` uniformly. A null operand (a `Unit` field, which the front
/// end never produces for a real field) becomes a null pointer.
fn widen_to_slot(builder: &mut FunctionBuilder<'_>, value: RValue, ptr: CType) -> Value {
    match value {
        Some(v) => {
            let ty = builder.func.dfg.value_type(v);
            if ty == ptr {
                v
            } else if ty == types::F64 {
                // Reinterpret the float's bits as an integer slot value.
                builder.ins().bitcast(ptr, MemFlags::new(), v)
            } else if ty.is_int() && ty.bytes() < ptr.bytes() {
                builder.ins().uextend(ptr, v)
            } else {
                v
            }
        }
        None => builder.ins().iconst(ptr, 0),
    }
}

/// Narrow a pointer-sized slot value back to a field's machine type when
/// the field is a scalar smaller than a pointer or a float.
fn narrow_from_slot(builder: &mut FunctionBuilder<'_>, raw: Value, want: CType) -> Value {
    let got = builder.func.dfg.value_type(raw);
    if got == want {
        return raw;
    }
    if want == types::F64 {
        return builder.ins().bitcast(types::F64, MemFlags::new(), raw);
    }
    if want.is_int() && got.is_int() && want.bytes() < got.bytes() {
        return builder.ins().ireduce(want, raw);
    }
    raw
}

/// Lower a struct value construction: allocate the body, then store each
/// field operand into its 8-byte slot. The struct's GC pointer mask is
/// registered with the module the first time the type is seen.
fn lower_struct_create(
    cx: &mut ModuleCx,
    builder: &mut FunctionBuilder<'_>,
    ty: &MirType,
    fields: &[MirOperand],
    field_tys: &[MirType],
    slots: &[LocalSlot],
) -> Result<RValue, CodegenError> {
    let ptr = cx.pointer_type();
    let mask = layout::struct_pointer_mask(field_tys);
    let type_id = cx.intern_descriptor(&ty.mangle(), mask);

    // Evaluate field operands before the allocation so their values are
    // in registers; the allocation may collect, but the operands here are
    // either constants or reads of already-rooted locals.
    let mut field_vals = Vec::with_capacity(fields.len());
    for f in fields {
        let v = lower_operand(cx, builder, f, slots)?;
        field_vals.push(widen_to_slot(builder, v, ptr));
    }

    let obj = call_struct_new(cx, builder, fields.len() as i64, type_id, ptr);
    let base = call_struct_fields(cx, builder, obj, ptr);
    for (i, v) in field_vals.into_iter().enumerate() {
        builder
            .ins()
            .store(MemFlags::new(), v, base, layout::field_offset(i));
    }
    Ok(Some(obj))
}

/// Lower an enum value construction: a struct value whose slot 0 holds
/// the variant discriminant and whose remaining slots hold the active
/// variant's payload.
#[allow(clippy::too_many_arguments)]
fn lower_enum_create(
    cx: &mut ModuleCx,
    builder: &mut FunctionBuilder<'_>,
    ty: &MirType,
    variant: usize,
    payload: &[MirOperand],
    payload_tys: &[MirType],
    slots: &[LocalSlot],
) -> Result<RValue, CodegenError> {
    let ptr = cx.pointer_type();
    let mask = layout::enum_pointer_mask(payload_tys);
    // The descriptor is per enum type; every variant of one enum shares
    // the same id and the union of payload pointer slots. Mangle the
    // type name so all variants intern the same descriptor, merging
    // their masks so any variant's pointer payload is traced.
    let key = ty.mangle();
    let type_id = cx.merge_descriptor(&key, mask);

    let mut payload_vals = Vec::with_capacity(payload.len());
    for p in payload {
        let v = lower_operand(cx, builder, p, slots)?;
        payload_vals.push(widen_to_slot(builder, v, ptr));
    }

    let field_count = 1 + payload.len() as i64;
    let obj = call_struct_new(cx, builder, field_count, type_id, ptr);
    let base = call_struct_fields(cx, builder, obj, ptr);
    // Slot 0: the discriminant.
    let disc = builder.ins().iconst(ptr, variant as i64);
    builder
        .ins()
        .store(MemFlags::new(), disc, base, layout::field_offset(0));
    // Slots 1..: the payload.
    for (i, v) in payload_vals.into_iter().enumerate() {
        builder
            .ins()
            .store(MemFlags::new(), v, base, layout::field_offset(i + 1));
    }
    Ok(Some(obj))
}

/// Lower a field read: load the base pointer, then load the slot at the
/// field's offset, narrowing it back to the field's machine type.
fn lower_field_access(
    cx: &mut ModuleCx,
    builder: &mut FunctionBuilder<'_>,
    base: &MirOperand,
    index: usize,
    slots: &[LocalSlot],
) -> Result<RValue, CodegenError> {
    let ptr = cx.pointer_type();
    let base_ptr = require_value(lower_operand(cx, builder, base, slots)?, "field base")?;
    let fields = call_struct_fields(cx, builder, base_ptr, ptr);
    // Load the slot as a pointer-width value; `store_local` narrows it to
    // the destination local's machine type (a `Float` or narrow scalar
    // field is reinterpreted there).
    let raw = builder
        .ins()
        .load(ptr, MemFlags::new(), fields, layout::field_offset(index));
    Ok(Some(raw))
}

/// Lower a closure construction. The MVP supports the non-capturing
/// shape the front end currently emits: a `Closure` object wrapping the
/// lifted body's function pointer with an empty capture buffer.
/// Capturing closures require front-end capture analysis and a lifted
/// body function, tracked separately.
fn lower_closure_create(
    cx: &mut ModuleCx,
    builder: &mut FunctionBuilder<'_>,
    fn_name: &str,
    captures: &[MirOperand],
    _slots: &[LocalSlot],
) -> Result<RValue, CodegenError> {
    if !captures.is_empty() {
        return Err(CodegenError::Unsupported(
            "capturing closures are not yet lowered; only non-capturing lambdas are supported"
                .into(),
        ));
    }
    let ptr = cx.pointer_type();
    // Resolve the lifted body's function pointer. The front end emits a
    // placeholder name until lambda body lifting lands; without a real
    // function to point at, a null function pointer is stored so the
    // object is still well formed and the program links.
    let fn_ptr = match cx.function_id(fn_name) {
        Some(id) => {
            let fref = cx.module().declare_func_in_func(id, builder.func);
            builder.ins().func_addr(ptr, fref)
        }
        None => builder.ins().iconst(ptr, 0),
    };
    let new_id = cx
        .runtime_id(intrinsics::RUNTIME_CLOSURE_NEW)
        .expect("closure new declared at module init");
    let new_ref = cx.module().declare_func_in_func(new_id, builder.func);
    let zero32 = builder.ins().iconst(types::I32, 0);
    let inst = builder
        .ins()
        .call(new_ref, &[fn_ptr, zero32, zero32, zero32, zero32]);
    Ok(builder.inst_results(inst).first().copied())
}

/// Emit a call to `raven_struct_new(field_count, type_id)`.
fn call_struct_new(
    cx: &mut ModuleCx,
    builder: &mut FunctionBuilder<'_>,
    field_count: i64,
    type_id: u32,
    _ptr: CType,
) -> Value {
    let new_id = cx
        .runtime_id(intrinsics::RUNTIME_STRUCT_NEW)
        .expect("struct new declared at module init");
    let new_ref = cx.module().declare_func_in_func(new_id, builder.func);
    let fc = builder.ins().iconst(types::I32, field_count);
    let tid = builder.ins().iconst(types::I32, type_id as i64);
    let inst = builder.ins().call(new_ref, &[fc, tid]);
    builder.inst_results(inst)[0]
}

/// Emit a call to `raven_struct_fields(obj)` returning the field base
/// pointer.
fn call_struct_fields(
    cx: &mut ModuleCx,
    builder: &mut FunctionBuilder<'_>,
    obj: Value,
    _ptr: CType,
) -> Value {
    let id = cx
        .runtime_id(intrinsics::RUNTIME_STRUCT_FIELDS)
        .expect("struct fields declared at module init");
    let fref = cx.module().declare_func_in_func(id, builder.func);
    let inst = builder.ins().call(fref, &[obj]);
    builder.inst_results(inst)[0]
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
    slots: &[LocalSlot],
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
        intrinsics::PRINT_INT => {
            if args.len() != 1 {
                return Err(CodegenError::Unsupported(format!(
                    "print_int intrinsic expects 1 arg, got {}",
                    args.len()
                )));
            }
            let v = require_value(
                lower_operand(cx, builder, &args[0], slots)?,
                "print_int argument",
            )?;
            let func_id = cx
                .runtime_id(intrinsics::RUNTIME_PRINTLN_INT)
                .expect("runtime imports declared at module init");
            let local_ref = cx.module().declare_func_in_func(func_id, builder.func);
            builder.ins().call(local_ref, &[v]);
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
    root_frame: Option<RootFrame>,
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
        MirTerminator::SwitchEnum {
            discriminant,
            targets,
            otherwise,
        } => lower_switch_enum(
            cx,
            builder,
            discriminant,
            targets,
            *otherwise,
            slots,
            blocks,
        ),
        MirTerminator::Return(op) => {
            // Evaluate the return value before leaving the root frame so
            // a collection during evaluation still sees the locals rooted.
            let v = lower_operand(cx, builder, op, slots)?;
            leave_root_frame(cx, builder, root_frame);
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

/// Emit the matching `raven_gc_leave_frame` for a function that entered
/// a root frame. A no-op when the function has no GC pointer locals.
fn leave_root_frame(
    cx: &mut ModuleCx,
    builder: &mut FunctionBuilder<'_>,
    root_frame: Option<RootFrame>,
) {
    if root_frame.is_none() {
        return;
    }
    let leave = cx
        .runtime_id(intrinsics::RUNTIME_GC_LEAVE_FRAME)
        .expect("gc leave frame declared at module init");
    let leave_ref = cx.module().declare_func_in_func(leave, builder.func);
    builder.ins().call(leave_ref, &[]);
}

/// Lower an enum dispatch: load the value's discriminant slot and branch
/// to the matching variant block, falling through to `otherwise`.
fn lower_switch_enum(
    cx: &mut ModuleCx,
    builder: &mut FunctionBuilder<'_>,
    discriminant: &MirOperand,
    targets: &[(usize, MirBlockId)],
    otherwise: Option<MirBlockId>,
    slots: &[LocalSlot],
    blocks: &[cranelift_codegen::ir::Block],
) -> Result<(), CodegenError> {
    let ptr = cx.pointer_type();
    let obj = require_value(
        lower_operand(cx, builder, discriminant, slots)?,
        "enum discriminant",
    )?;
    let fields = call_struct_fields(cx, builder, obj, ptr);
    // The discriminant is stored in slot 0 as a pointer-width integer.
    let disc = builder
        .ins()
        .load(ptr, MemFlags::new(), fields, layout::field_offset(0));

    // Split the targets into a cascade and a fall-through block. With an
    // explicit otherwise every target is compared and the otherwise block
    // is the default. Without one, the last target becomes the default so
    // the CFG always terminates (a well typed match is exhaustive).
    let (cascade, default_block) = match otherwise {
        Some(o) => (targets, blocks[o.0 as usize]),
        None => match targets.split_last() {
            Some((last, head)) => (head, blocks[last.1 .0 as usize]),
            None => {
                builder
                    .ins()
                    .trap(cranelift_codegen::ir::TrapCode::UnreachableCodeReached);
                return Ok(());
            }
        },
    };

    for (value, target) in cascade {
        let imm = builder.ins().iconst(ptr, *value as i64);
        let cmp = builder.ins().icmp(IntCC::Equal, disc, imm);
        let target_block = blocks[target.0 as usize];
        let continue_block = builder.create_block();
        builder
            .ins()
            .brif(cmp, target_block, &[], continue_block, &[]);
        builder.seal_block(continue_block);
        builder.switch_to_block(continue_block);
    }
    builder.ins().jump(default_block, &[]);
    Ok(())
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
