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
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::Module;

use crate::mir::{
    ListMethodOp, MirBinOp, MirBlock, MirBlockId, MirConstant, MirFfiTy, MirFnRef, MirFunction,
    MirLocal, MirOperand, MirRvalue, MirStatement, MirTerminator, MirType, MirUnOp, ReprCLayout,
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
        | MirType::Function { .. }
        // A `dyn Trait` value is a single GC pointer to a boxed fat
        // pointer `{ data, vtable }`; see the heap layout note below.
        | MirType::Dyn { .. }
        // An `Any` is a single GC pointer to an `Any` box.
        | MirType::Any => Some(ptr),
        // C FFI primitives map to their C ABI machine types. `CInt` is a
        // 32-bit C `int`; `CLong`, `CSize`, `CStr`, and `CPtr<T>` are all
        // pointer-width on the 64-bit targets Raven supports. See
        // `docs/v2/specs/ffi.md`.
        MirType::Ffi(ffi) => Some(match ffi {
            MirFfiTy::CInt => types::I32,
            MirFfiTy::CLong => types::I64,
            MirFfiTy::CFloat => types::F32,
            MirFfiTy::CDouble => types::F64,
            MirFfiTy::CSize | MirFfiTy::CStr | MirFfiTy::CPtr(_) | MirFfiTy::CFnPtr => ptr,
        }),
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
                // Open the per-call defer frame after the GC frame so the
                // epilogue runs defers (which may touch GC locals) before
                // leaving the GC frame.
                if self.func.has_defer {
                    enter_defer_frame(self.cx, &mut builder);
                }
            }
            lower_block(
                self.cx,
                &mut builder,
                mir_block,
                &slots,
                &blocks,
                root_frame,
                self.func.has_defer,
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

#[allow(clippy::too_many_arguments)]
fn lower_block(
    cx: &mut ModuleCx,
    builder: &mut FunctionBuilder<'_>,
    mir_block: &MirBlock,
    slots: &[LocalSlot],
    blocks: &[cranelift_codegen::ir::Block],
    root_frame: Option<RootFrame>,
    has_defer: bool,
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
        has_defer,
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
        MirStatement::StoreField { base, index, value } => {
            lower_store_field(cx, builder, base, *index, value, slots)
        }
        MirStatement::StoreIndex { base, index, value } => {
            lower_store_index(cx, builder, base, index, value, slots)
        }
        MirStatement::PtrStore {
            addr,
            value,
            pointee,
        } => lower_ptr_store(cx, builder, addr, value, pointee, slots),
        MirStatement::PtrFree { addr } => lower_ptr_free(cx, builder, addr, slots),
        MirStatement::StoreGlobal { name, value } => {
            if let Some(v) = lower_operand(cx, builder, value, slots)? {
                let data_id = cx
                    .global_data(name)
                    .expect("global data slot declared at module init");
                let gv = cx.module().declare_data_in_func(data_id, builder.func);
                let ptr = cx.pointer_type();
                let addr = builder.ins().symbol_value(ptr, gv);
                builder.ins().store(MemFlags::new(), v, addr, 0);
            }
            Ok(())
        }
        MirStatement::StorageLive(_) | MirStatement::StorageDead(_) | MirStatement::Nop => Ok(()),
    }
}

/// Lower `base.field = value`.
///
/// Loads the object's field base pointer (the same base
/// [`lower_field_access`] reads from), widens the value to a
/// pointer-width slot, and stores it at the field's byte offset. The
/// store mirrors the field write `lower_struct_create` performs at
/// construction. `base` is an already-rooted GC pointer, so the written
/// value is reachable through it once stored; overwriting a slot simply
/// drops the old value's last reference through this object, and the
/// collector reclaims it on a later cycle. No new GC root is needed.
fn lower_store_field(
    cx: &mut ModuleCx,
    builder: &mut FunctionBuilder<'_>,
    base: &MirOperand,
    index: usize,
    value: &MirOperand,
    slots: &[LocalSlot],
) -> Result<(), CodegenError> {
    let ptr = cx.pointer_type();
    let base_ptr = require_value(lower_operand(cx, builder, base, slots)?, "field store base")?;
    let v = lower_operand(cx, builder, value, slots)?;
    let v = widen_to_slot(builder, v, ptr);
    let fields = call_struct_fields(cx, builder, base_ptr, ptr);
    builder
        .ins()
        .store(MemFlags::new(), v, fields, layout::slot_offset(index));
    Ok(())
}

/// Lower `__ptr_store<T>(p, value)`: coerce `value` to the pointee machine
/// width (a native `Int` narrows to `CInt`/`CFloat`, a `Float` to `CFloat`)
/// and store it at `p`.
fn lower_ptr_store(
    cx: &mut ModuleCx,
    builder: &mut FunctionBuilder<'_>,
    addr: &MirOperand,
    value: &MirOperand,
    pointee: &MirType,
    slots: &[LocalSlot],
) -> Result<(), CodegenError> {
    let ptr = cx.pointer_type();
    let a = require_value(
        lower_operand(cx, builder, addr, slots)?,
        "ptr_store address",
    )?;
    let v = require_value(lower_operand(cx, builder, value, slots)?, "ptr_store value")?;
    let v = coerce_to_param(builder, v, pointee, ptr);
    builder.ins().store(MemFlags::new(), v, a, 0);
    Ok(())
}

/// Lower `__ptr_free<T>(p)`: call `raven_ffi_free(p)`.
fn lower_ptr_free(
    cx: &mut ModuleCx,
    builder: &mut FunctionBuilder<'_>,
    addr: &MirOperand,
    slots: &[LocalSlot],
) -> Result<(), CodegenError> {
    let a = require_value(lower_operand(cx, builder, addr, slots)?, "ptr_free address")?;
    let func_id = cx
        .runtime_id(intrinsics::RUNTIME_FFI_FREE)
        .expect("ffi free runtime symbol declared at module init");
    let local_ref = cx.module().declare_func_in_func(func_id, builder.func);
    builder.ins().call(local_ref, &[a]);
    Ok(())
}

/// Lower `base[index] = value`.
///
/// Loads the list's element buffer base and length, bounds-checks
/// `index` (an out-of-range index calls `raven_panic`, matching
/// [`lower_index_access`]), then stores the widened value at
/// `base + index * ELEMENT_SLOT`. `base` is an already-rooted GC pointer
/// to the list, so a stored GC pointer element is reachable through it;
/// the overwritten element simply loses its last reference through this
/// list.
fn lower_store_index(
    cx: &mut ModuleCx,
    builder: &mut FunctionBuilder<'_>,
    base: &MirOperand,
    index: &MirOperand,
    value: &MirOperand,
    slots: &[LocalSlot],
) -> Result<(), CodegenError> {
    let ptr = cx.pointer_type();
    let list = require_value(lower_operand(cx, builder, base, slots)?, "index store base")?;
    let idx = require_value(
        lower_operand(cx, builder, index, slots)?,
        "index store value",
    )?;
    let v = lower_operand(cx, builder, value, slots)?;
    let v = widen_to_slot(builder, v, ptr);
    // The index is a native `Int` (i64); take it to pointer width for the
    // address arithmetic and the unsigned bounds compare.
    let idx = to_pointer_width(builder, idx, ptr);

    let len = call_list_len(cx, builder, list, ptr);
    emit_bounds_check(cx, builder, idx, len, "list index out of bounds");

    let elements = call_list_elements(cx, builder, list);
    let slot_size = builder.ins().iconst(ptr, ELEMENT_SLOT);
    let offset = builder.ins().imul(idx, slot_size);
    let addr = builder.ins().iadd(elements, offset);
    builder.ins().store(MemFlags::new(), v, addr, 0);
    Ok(())
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
        MirRvalue::GlobalLoad { name, ty } => {
            let data_id = cx
                .global_data(name)
                .expect("global data slot declared at module init");
            let gv = cx.module().declare_data_in_func(data_id, builder.func);
            let ptr = cx.pointer_type();
            let addr = builder.ins().symbol_value(ptr, gv);
            match cranelift_ty(ty, ptr) {
                Some(cty) => Ok(Some(builder.ins().load(cty, MemFlags::new(), addr, 0))),
                None => Ok(None),
            }
        }
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
        MirRvalue::ClosureCreate {
            fn_name,
            captures,
            capture_tys,
        } => lower_closure_create(cx, builder, fn_name, captures, capture_tys, slots),
        MirRvalue::EnvLoad { env, slot, ty } => lower_env_load(cx, builder, env, *slot, ty, slots),
        MirRvalue::ClosureCall {
            closure,
            args,
            param_tys,
            ret_ty,
        } => lower_closure_call(cx, builder, closure, args, param_tys, ret_ty, slots),
        MirRvalue::DynCoerce {
            value,
            concrete_ty,
            trait_name,
            methods,
        } => lower_dyn_coerce(cx, builder, value, concrete_ty, trait_name, methods, slots),
        MirRvalue::VirtualCall {
            receiver,
            slot,
            args,
            param_tys,
            ret_ty,
        } => lower_virtual_call(cx, builder, receiver, *slot, args, param_tys, ret_ty, slots),
        MirRvalue::ArrayLit { ty, elements } => lower_array_lit(cx, builder, ty, elements, slots),
        MirRvalue::IndexAccess { base, index } => {
            lower_index_access(cx, builder, base, index, slots)
        }
        MirRvalue::ListMethod {
            op,
            receiver,
            arg,
            elem_ty,
        } => lower_list_method(cx, builder, *op, receiver, arg.as_ref(), elem_ty, slots),
        MirRvalue::PtrLoad { addr, pointee } => lower_ptr_load(cx, builder, addr, pointee, slots),
        MirRvalue::PtrOffset {
            addr,
            count,
            pointee,
        } => lower_ptr_offset(cx, builder, addr, count, pointee, slots),
        MirRvalue::PtrIsNull { addr } => lower_ptr_is_null(cx, builder, addr, slots),
        MirRvalue::PtrNull => Ok(Some(builder.ins().iconst(cx.pointer_type(), 0))),
        MirRvalue::FnAddr { mangled } => Ok(Some(lower_fn_addr(cx, builder, mangled))),
        MirRvalue::PtrAlloc { count, pointee } => {
            lower_ptr_alloc(cx, builder, count, pointee, slots)
        }
        MirRvalue::AnyBox { value, value_ty } => lower_any_box(cx, builder, value, value_ty, slots),
        MirRvalue::AnyCast {
            any,
            target_ty,
            option_ty,
        } => lower_any_cast(cx, builder, any, target_ty, option_ty, slots),
        MirRvalue::AnyTypeName { any } => {
            lower_any_runtime_call(cx, builder, intrinsics::RUNTIME_ANY_TYPE_NAME, any, slots)
        }
        MirRvalue::AnyFieldNames { any } => {
            lower_any_runtime_call(cx, builder, intrinsics::RUNTIME_ANY_FIELD_NAMES, any, slots)
        }
        MirRvalue::AnyGetField {
            any,
            name,
            option_ty,
        } => lower_any_get_field(cx, builder, any, name, option_ty, slots),
        MirRvalue::AnySetField { any, name, value } => {
            lower_any_set_field(cx, builder, any, name, value, slots)
        }
    }
}

/// Lower `set_field(a, name, value)`: a void call to the runtime writer.
fn lower_any_set_field(
    cx: &mut ModuleCx,
    builder: &mut FunctionBuilder<'_>,
    any: &MirOperand,
    name: &MirOperand,
    value: &MirOperand,
    slots: &[LocalSlot],
) -> Result<RValue, CodegenError> {
    let a = require_value(lower_operand(cx, builder, any, slots)?, "set_field any")?;
    let n = require_value(lower_operand(cx, builder, name, slots)?, "set_field name")?;
    let v = require_value(lower_operand(cx, builder, value, slots)?, "set_field value")?;
    let func_id = cx
        .runtime_id(intrinsics::RUNTIME_ANY_SET_FIELD)
        .expect("any runtime symbol declared at module init");
    let fref = cx.module().declare_func_in_func(func_id, builder.func);
    builder.ins().call(fref, &[a, n, v]);
    Ok(None)
}

/// Cranelift machine type for a raw-pointer pointee, and its byte size.
/// `CStr`/`CSize` and any `CPtr` are pointer-width; the scalar widths come
/// from [`cranelift_ty`]. Returns `(ty, size_in_bytes)`.
fn pointee_machine_ty(pointee: &MirType, ptr: CType) -> (CType, i64) {
    let t = cranelift_ty(pointee, ptr).unwrap_or(ptr);
    (t, t.bytes() as i64)
}

/// Lower `__ptr_load<T>(p)`: a single load of the pointee machine type at
/// address `p`. An integer pointee narrower than `Int` (i64) is sign or
/// zero extended back to i64 so it flows as a native value; a `CInt`
/// result stays i32 (its own ABI width), matching how extern `CInt`
/// results are handled.
fn lower_ptr_load(
    cx: &mut ModuleCx,
    builder: &mut FunctionBuilder<'_>,
    addr: &MirOperand,
    pointee: &MirType,
    slots: &[LocalSlot],
) -> Result<RValue, CodegenError> {
    let ptr = cx.pointer_type();
    let a = require_value(lower_operand(cx, builder, addr, slots)?, "ptr_load address")?;
    let (ty, _) = pointee_machine_ty(pointee, ptr);
    let v = builder.ins().load(ty, MemFlags::new(), a, 0);
    Ok(Some(v))
}

/// Lower `__ptr_offset<T>(p, n)`: `p + n * sizeof(T)`. `n` is a native
/// `Int` taken to pointer width for the address arithmetic.
fn lower_ptr_offset(
    cx: &mut ModuleCx,
    builder: &mut FunctionBuilder<'_>,
    addr: &MirOperand,
    count: &MirOperand,
    pointee: &MirType,
    slots: &[LocalSlot],
) -> Result<RValue, CodegenError> {
    let ptr = cx.pointer_type();
    let a = require_value(
        lower_operand(cx, builder, addr, slots)?,
        "ptr_offset address",
    )?;
    let n = require_value(
        lower_operand(cx, builder, count, slots)?,
        "ptr_offset count",
    )?;
    let n = to_pointer_width(builder, n, ptr);
    let (_, size) = pointee_machine_ty(pointee, ptr);
    let stride = builder.ins().iconst(ptr, size);
    let bytes = builder.ins().imul(n, stride);
    Ok(Some(builder.ins().iadd(a, bytes)))
}

/// Lower `__ptr_is_null<T>(p)`: `p == 0`, producing an i8 Bool.
fn lower_ptr_is_null(
    cx: &mut ModuleCx,
    builder: &mut FunctionBuilder<'_>,
    addr: &MirOperand,
    slots: &[LocalSlot],
) -> Result<RValue, CodegenError> {
    let a = require_value(
        lower_operand(cx, builder, addr, slots)?,
        "ptr_is_null address",
    )?;
    // `icmp` yields an i8, the representation Raven uses for `Bool`.
    Ok(Some(builder.ins().icmp_imm(IntCC::Equal, a, 0)))
}

/// Lower `__ptr_alloc<T>(count)`: call `raven_ffi_alloc(count * sizeof(T))`.
fn lower_ptr_alloc(
    cx: &mut ModuleCx,
    builder: &mut FunctionBuilder<'_>,
    count: &MirOperand,
    pointee: &MirType,
    slots: &[LocalSlot],
) -> Result<RValue, CodegenError> {
    let ptr = cx.pointer_type();
    let n = require_value(lower_operand(cx, builder, count, slots)?, "ptr_alloc count")?;
    let n = to_pointer_width(builder, n, ptr);
    let (_, size) = pointee_machine_ty(pointee, ptr);
    let stride = builder.ins().iconst(ptr, size);
    let bytes = builder.ins().imul(n, stride);
    let func_id = cx
        .runtime_id(intrinsics::RUNTIME_FFI_ALLOC)
        .expect("ffi alloc runtime symbol declared at module init");
    let local_ref = cx.module().declare_func_in_func(func_id, builder.func);
    let inst = builder.ins().call(local_ref, &[bytes]);
    Ok(builder.inst_results(inst).first().copied())
}

/// Lower `to_any<T>(v)`: box `value` into a fresh `Any` tagged with `T`'s
/// runtime type id. A scalar payload is widened into the eight-byte slot; a
/// GC-pointer payload sets the box's traced flag so the collector keeps it
/// alive through the `Any`. See `docs/v2/specs/runtime-reflection.md`.
fn lower_any_box(
    cx: &mut ModuleCx,
    builder: &mut FunctionBuilder<'_>,
    value: &MirOperand,
    value_ty: &MirType,
    slots: &[LocalSlot],
) -> Result<RValue, CodegenError> {
    let ptr = cx.pointer_type();
    let v = lower_operand(cx, builder, value, slots)?;
    let payload = widen_to_slot(builder, v, ptr);
    // The payload word is passed as u64; widen the pointer-width slot to
    // i64 when the target is 32-bit, else it is already i64.
    let payload = if builder.func.dfg.value_type(payload) == types::I64 {
        payload
    } else {
        builder.ins().uextend(types::I64, payload)
    };
    // The struct (or scalar) descriptor was interned at its construction
    // site with the correct mask; re-interning by mangle returns that id
    // and keeps the prior mask. A type only ever boxed (never built here)
    // still gets a stable id with a zero mask, which is sound because such
    // a value is never the GC root of its own fields.
    let type_id = cx.intern_descriptor(&value_ty.mangle(), 0);
    let is_gc = if layout::is_gc_pointer(value_ty) {
        1
    } else {
        0
    };
    let tid = builder.ins().iconst(types::I32, type_id as i64);
    let gc = builder.ins().iconst(types::I32, is_gc);
    let func_id = cx
        .runtime_id(intrinsics::RUNTIME_ANY_NEW)
        .expect("any new declared at module init");
    let fref = cx.module().declare_func_in_func(func_id, builder.func);
    let inst = builder.ins().call(fref, &[payload, tid, gc]);
    Ok(builder.inst_results(inst).first().copied())
}

/// Lower a one-argument `Any` runtime call (`type_name_of`,
/// `field_names_of`): pass the `Any` pointer and return the heap result.
fn lower_any_runtime_call(
    cx: &mut ModuleCx,
    builder: &mut FunctionBuilder<'_>,
    symbol: &'static str,
    any: &MirOperand,
    slots: &[LocalSlot],
) -> Result<RValue, CodegenError> {
    let a = require_value(lower_operand(cx, builder, any, slots)?, "any operand")?;
    let func_id = cx
        .runtime_id(symbol)
        .expect("any runtime symbol declared at module init");
    let fref = cx.module().declare_func_in_func(func_id, builder.func);
    let inst = builder.ins().call(fref, &[a]);
    Ok(builder.inst_results(inst).first().copied())
}

/// Lower `cast<T>(a)`: a checked downcast to `Option<T>`. Compares the
/// boxed runtime type id against `T`'s id and builds one `Option<T>` enum
/// whose discriminant and payload are chosen by the comparison: `Some` with
/// the payload reinterpreted as `T` on a match, `None` (with a null
/// payload) otherwise. The branch-free `select` keeps the rvalue a single
/// value. See `docs/v2/specs/runtime-reflection.md`.
fn lower_any_cast(
    cx: &mut ModuleCx,
    builder: &mut FunctionBuilder<'_>,
    any: &MirOperand,
    target_ty: &MirType,
    option_ty: &MirType,
    slots: &[LocalSlot],
) -> Result<RValue, CodegenError> {
    let ptr = cx.pointer_type();
    let a = require_value(lower_operand(cx, builder, any, slots)?, "cast any")?;
    let target_id = cx.intern_descriptor(&target_ty.mangle(), 0);

    // Read the boxed runtime type id and the payload word.
    let tid_fn = cx
        .runtime_id(intrinsics::RUNTIME_ANY_TYPE_ID)
        .expect("any type id declared at module init");
    let tid_ref = cx.module().declare_func_in_func(tid_fn, builder.func);
    let tid_inst = builder.ins().call(tid_ref, &[a]);
    let boxed_id = builder.inst_results(tid_inst)[0];

    let pay_fn = cx
        .runtime_id(intrinsics::RUNTIME_ANY_PAYLOAD)
        .expect("any payload declared at module init");
    let pay_ref = cx.module().declare_func_in_func(pay_fn, builder.func);
    let pay_inst = builder.ins().call(pay_ref, &[a]);
    let payload = builder.inst_results(pay_inst)[0]; // i64

    let want = builder.ins().iconst(types::I32, target_id as i64);
    let matches = builder.ins().icmp(IntCC::Equal, boxed_id, want);

    // Build the Option<T> enum: slot 0 discriminant, slot 1 payload.
    let mask = layout::enum_pointer_mask(std::slice::from_ref(target_ty));
    let opt_type_id = cx.merge_descriptor(&option_ty.mangle(), mask);
    let obj = call_struct_new(cx, builder, 2, opt_type_id, ptr);
    let base = call_struct_fields(cx, builder, obj, ptr);

    // discriminant: Some == 0 on match, None == 1 otherwise.
    let some_disc = builder.ins().iconst(ptr, 0);
    let none_disc = builder.ins().iconst(ptr, 1);
    let disc = builder.ins().select(matches, some_disc, none_disc);
    builder
        .ins()
        .store(MemFlags::new(), disc, base, layout::slot_offset(0));

    // payload: the boxed payload word narrowed to the slot on match, else 0.
    let payload_slot = if ptr == types::I64 {
        payload
    } else {
        builder.ins().ireduce(ptr, payload)
    };
    let zero = builder.ins().iconst(ptr, 0);
    let stored = builder.ins().select(matches, payload_slot, zero);
    builder
        .ins()
        .store(MemFlags::new(), stored, base, layout::slot_offset(1));
    Ok(Some(obj))
}

/// Lower `get_field(a, name)`: call the runtime to read the named field as
/// an `Any` (or null), then wrap it into `Option<Any>` (`Some(any)` when
/// non-null, `None` when null). See `docs/v2/specs/runtime-reflection.md`.
fn lower_any_get_field(
    cx: &mut ModuleCx,
    builder: &mut FunctionBuilder<'_>,
    any: &MirOperand,
    name: &MirOperand,
    option_ty: &MirType,
    slots: &[LocalSlot],
) -> Result<RValue, CodegenError> {
    let ptr = cx.pointer_type();
    let a = require_value(lower_operand(cx, builder, any, slots)?, "get_field any")?;
    let n = require_value(lower_operand(cx, builder, name, slots)?, "get_field name")?;
    let func_id = cx
        .runtime_id(intrinsics::RUNTIME_ANY_GET_FIELD)
        .expect("any get field declared at module init");
    let fref = cx.module().declare_func_in_func(func_id, builder.func);
    let inst = builder.ins().call(fref, &[a, n]);
    let field_any = builder.inst_results(inst)[0]; // Any ptr or null

    // Root the freshly built field Any across the Option allocation below:
    // `call_struct_new` can trigger a collection, and the only reference to
    // this new Any is the unrooted register here, so it would be freed
    // before it is stored into the Option. A null (the None case) roots
    // harmlessly.
    let field_root = push_temp_root(cx, builder, field_any, ptr);

    // Wrap into Option<Any>: the payload slot holds a GC pointer.
    let mask = layout::enum_pointer_mask(std::slice::from_ref(&MirType::Any));
    let opt_type_id = cx.merge_descriptor(&option_ty.mangle(), mask);
    let obj = call_struct_new(cx, builder, 2, opt_type_id, ptr);
    let field_any = pop_temp_root(cx, builder, field_root, ptr);
    let base = call_struct_fields(cx, builder, obj, ptr);

    let is_some = builder.ins().icmp_imm(IntCC::NotEqual, field_any, 0);
    let some_disc = builder.ins().iconst(ptr, 0);
    let none_disc = builder.ins().iconst(ptr, 1);
    let disc = builder.ins().select(is_some, some_disc, none_disc);
    builder
        .ins()
        .store(MemFlags::new(), disc, base, layout::slot_offset(0));
    builder
        .ins()
        .store(MemFlags::new(), field_any, base, layout::slot_offset(1));
    Ok(Some(obj))
}

/// Lower `FnAddr`: the address of a top-level function as a C function
/// pointer. The function is compiled under the platform default (C)
/// calling convention, so the address is callable directly by C. A
/// missing symbol is a lowering bug; keep the program well formed with a
/// null pointer rather than aborting the build.
fn lower_fn_addr(cx: &mut ModuleCx, builder: &mut FunctionBuilder<'_>, mangled: &str) -> Value {
    let ptr = cx.pointer_type();
    match cx.function_id(mangled) {
        Some(id) => {
            let fref = cx.module().declare_func_in_func(id, builder.func);
            builder.ins().func_addr(ptr, fref)
        }
        None => builder.ins().iconst(ptr, 0),
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
            // A string constant used as a value (assigned to a local,
            // passed to concat, interpolated, ...) is promoted to a heap
            // `String` so every `Str`-typed value is a real GC object the
            // collector can trace and the runtime string functions can
            // consume. The direct `print("literal")` fast path bypasses
            // this by pattern matching the const operand before it
            // reaches here, so a bare literal print stays allocation
            // free.
            let bytes = s.as_bytes();
            let id = cx.intern_string(bytes)?;
            let local_id = cx.module().declare_data_in_func(id, builder.func);
            let ptr = cx.pointer_type();
            let bytes_ptr = builder.ins().symbol_value(ptr, local_id);
            let len_val = builder.ins().iconst(ptr, bytes.len() as i64);
            let func_id = cx
                .runtime_id(intrinsics::RUNTIME_STRING_FROM_BYTES)
                .expect("string-from-bytes runtime symbol declared at module init");
            let fref = cx.module().declare_func_in_func(func_id, builder.func);
            let inst = builder.ins().call(fref, &[bytes_ptr, len_val]);
            Ok(Some(builder.inst_results(inst)[0]))
        }
        MirConstant::CStr(s) => {
            // A C string literal lowers to the address of a static,
            // read-only, null-terminated byte buffer: a `*const c_char`.
            // No heap allocation and no runtime call; the pointer is
            // handed straight to the C function.
            let id = cx.intern_cstring(s.as_bytes())?;
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
    // A `CFloat` (f32) result widens to a Raven `Float` (f64).
    if src_ty == types::F32 && dst_ty == types::F64 {
        return builder.ins().fpromote(types::F64, v);
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
    // Merge rather than keep-first so a prior interning of this type with a
    // zero mask (for example a `to_any<T>` box site reached first) cannot
    // drop the struct's real pointer-field bits. Unioning the same mask is
    // idempotent.
    let type_id = cx.merge_descriptor(&ty.mangle(), mask);

    // Allocate the struct first and root it for the duration of field
    // evaluation. A field operand can itself allocate (a String-literal or
    // interpolated field promotes to a heap String), which can trigger a
    // collection; the partially built struct and the fields already stored
    // must stay reachable across it. Each field value is unrooted only
    // between its production and its immediate store into the rooted struct,
    // with no allocation in between.
    let obj = call_struct_new(cx, builder, fields.len() as i64, type_id, ptr);
    let root = push_temp_root(cx, builder, obj, ptr);
    for (i, f) in fields.iter().enumerate() {
        let v = lower_operand(cx, builder, f, slots)?;
        let v = widen_to_slot(builder, v, ptr);
        let obj = builder.ins().stack_load(ptr, root, 0);
        let base = call_struct_fields(cx, builder, obj, ptr);
        builder
            .ins()
            .store(MemFlags::new(), v, base, layout::slot_offset(i));
    }
    let obj = pop_temp_root(cx, builder, root, ptr);
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

    // Allocate the enum value first and root it while the payload is
    // evaluated, for the same reason struct creation does: a payload
    // operand can allocate and trigger a collection, and the partially
    // built value must stay reachable across it.
    let field_count = 1 + payload.len() as i64;
    let obj = call_struct_new(cx, builder, field_count, type_id, ptr);
    let root = push_temp_root(cx, builder, obj, ptr);
    // Slot 0: the discriminant.
    let base = call_struct_fields(cx, builder, obj, ptr);
    let disc = builder.ins().iconst(ptr, variant as i64);
    builder
        .ins()
        .store(MemFlags::new(), disc, base, layout::slot_offset(0));
    // Slots 1..: the payload.
    for (i, p) in payload.iter().enumerate() {
        let v = lower_operand(cx, builder, p, slots)?;
        let v = widen_to_slot(builder, v, ptr);
        let obj = builder.ins().stack_load(ptr, root, 0);
        let base = call_struct_fields(cx, builder, obj, ptr);
        builder
            .ins()
            .store(MemFlags::new(), v, base, layout::slot_offset(i + 1));
    }
    let obj = pop_temp_root(cx, builder, root, ptr);
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
        .load(ptr, MemFlags::new(), fields, layout::slot_offset(index));
    Ok(Some(raw))
}

/// Spill a freshly allocated heap object into a one-pointer stack slot and
/// register that slot as a single GC root, keeping the object (and any GC
/// pointers later stored into it) reachable across further allocations that
/// run while the object is still being filled. Returns the slot so the
/// caller can reload the rooted pointer; pair with `pop_temp_root`.
fn push_temp_root(
    cx: &mut ModuleCx,
    builder: &mut FunctionBuilder<'_>,
    obj: Value,
    ptr: CType,
) -> StackSlot {
    let slot = builder
        .create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, ptr.bytes()));
    builder.ins().stack_store(obj, slot, 0);
    let slot_addr = builder.ins().stack_addr(ptr, slot, 0);
    let push_id = cx
        .runtime_id(intrinsics::RUNTIME_GC_PUSH_ROOT)
        .expect("gc push root declared at module init");
    let push_ref = cx.module().declare_func_in_func(push_id, builder.func);
    builder.ins().call(push_ref, &[slot_addr]);
    slot
}

/// Pop the single root `push_temp_root` registered and reload the rooted
/// pointer from its slot (the authoritative value the collector observed).
fn pop_temp_root(
    cx: &mut ModuleCx,
    builder: &mut FunctionBuilder<'_>,
    slot: StackSlot,
    ptr: CType,
) -> Value {
    let pop_id = cx
        .runtime_id(intrinsics::RUNTIME_GC_POP_ROOTS)
        .expect("gc pop roots declared at module init");
    let pop_ref = cx.module().declare_func_in_func(pop_id, builder.func);
    let one = builder.ins().iconst(ptr, 1);
    builder.ins().call(pop_ref, &[one]);
    builder.ins().stack_load(ptr, slot, 0)
}

/// Spill `obj` to a one-pointer stack slot and register it as a single GC
/// root, without returning the slot. Used to keep a freshly built heap
/// value alive across further allocations when the caller pops a batch of
/// roots at once (with [`pop_n_roots`]) and does not need to reload the
/// pointer (the value is non-moving, so the register stays valid).
fn root_temp(cx: &mut ModuleCx, builder: &mut FunctionBuilder<'_>, obj: Value, ptr: CType) {
    let slot = builder
        .create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, ptr.bytes()));
    builder.ins().stack_store(obj, slot, 0);
    let slot_addr = builder.ins().stack_addr(ptr, slot, 0);
    let push_id = cx
        .runtime_id(intrinsics::RUNTIME_GC_PUSH_ROOT)
        .expect("gc push root declared at module init");
    let push_ref = cx.module().declare_func_in_func(push_id, builder.func);
    builder.ins().call(push_ref, &[slot_addr]);
}

/// Pop `n` single roots registered with [`root_temp`]. A zero count emits
/// nothing.
fn pop_n_roots(cx: &mut ModuleCx, builder: &mut FunctionBuilder<'_>, n: usize, ptr: CType) {
    if n == 0 {
        return;
    }
    let pop_id = cx
        .runtime_id(intrinsics::RUNTIME_GC_POP_ROOTS)
        .expect("gc pop roots declared at module init");
    let pop_ref = cx.module().declare_func_in_func(pop_id, builder.func);
    let count = builder.ins().iconst(ptr, n as i64);
    builder.ins().call(pop_ref, &[count]);
}

/// True when lowering this operand allocates a fresh heap value that the
/// register holds as the only reference. A `Const::Str` promotes to a heap
/// `String` at its use site (see [`lower_constant`]); a later allocation in
/// the same rvalue would free it unless it is rooted across that
/// allocation. Every other operand is a scalar constant or a copy of an
/// already rooted local, so it needs no extra root.
fn operand_allocates_heap(op: &MirOperand) -> bool {
    matches!(op, MirOperand::Const(MirConstant::Str(_)))
}

/// Width in bytes of one `List` element slot. Every element, scalar or GC
/// pointer, occupies one pointer-width slot, the same uniform width
/// struct and enum fields use. Scalars narrower than a pointer are
/// widened on store and narrowed on load; a GC pointer is already this
/// wide. Keeping a single slot width means indexing is a plain
/// `base + i * ELEMENT_SLOT` and the collector's element tracing walks
/// pointer-sized slots. See `docs/v2/specs/object-layout.md`.
const ELEMENT_SLOT: i64 = 8;

/// Lower a `List<T>` literal `[a, b, c]`.
///
/// Allocates a `List` sized for the element count, then appends each
/// evaluated element through `raven_list_push`. The element slot is a
/// uniform eight bytes (`element_size == element_align == ELEMENT_SLOT`);
/// `elements_are_gc_ptrs` is set from the static element type so the
/// collector traces pointer elements and treats scalar buffers as opaque
/// bytes. Each element value is widened to a pointer-width slot, spilled
/// to a one-slot scratch buffer, and the buffer's address is handed to
/// `raven_list_push`, which copies the eight bytes into the list.
fn lower_array_lit(
    cx: &mut ModuleCx,
    builder: &mut FunctionBuilder<'_>,
    ty: &MirType,
    elements: &[MirOperand],
    slots: &[LocalSlot],
) -> Result<RValue, CodegenError> {
    let ptr = cx.pointer_type();
    let elem_ty = match ty {
        MirType::List(inner) => inner.as_ref(),
        // The front end only ever types an array literal as `List<T>`; a
        // mismatch is a lowering bug, so fall back to a scalar element so
        // the build stays well formed.
        _ => &MirType::Int,
    };
    let gc_ptrs = layout::is_gc_pointer(elem_ty);

    // Allocate the empty list first and root it through a single shadow
    // stack slot for the whole build. Evaluating a later element can
    // allocate (a String-literal element promotes to a heap String, and an
    // interpolated element allocates), which can trigger a collection; the
    // list and the elements already pushed into it must stay reachable
    // across that collection. Rooting the list keeps its pushed elements
    // alive transitively. Each element value is unrooted only between its
    // own production and its immediate push, with no allocation in between,
    // so it is never live across a collection on its own.
    let list = call_list_new(cx, builder, elements.len() as i64, gc_ptrs);
    let root = push_temp_root(cx, builder, list, ptr);

    // One reusable scratch slot the size of a single element; each push
    // writes the next value into it and passes its address.
    let scratch = builder.create_sized_stack_slot(StackSlotData::new(
        StackSlotKind::ExplicitSlot,
        ELEMENT_SLOT as u32,
    ));
    let scratch_addr = builder.ins().stack_addr(ptr, scratch, 0);
    let push_id = cx
        .runtime_id(intrinsics::RUNTIME_LIST_PUSH)
        .expect("list push declared at module init");
    for e in elements {
        let v = lower_operand(cx, builder, e, slots)?;
        let v = widen_to_slot(builder, v, ptr);
        builder.ins().stack_store(v, scratch, 0);
        // Reload the rooted list so the push targets the authoritative
        // pointer the collector observes through the slot.
        let list = builder.ins().stack_load(ptr, root, 0);
        let push_ref = cx.module().declare_func_in_func(push_id, builder.func);
        builder.ins().call(push_ref, &[list, scratch_addr]);
    }

    let list = pop_temp_root(cx, builder, root, ptr);
    Ok(Some(list))
}

/// Lower `xs[i]`.
///
/// Loads the element buffer base and the length from the list, bounds
/// checks `i` (an out-of-range index calls `raven_panic`), then loads the
/// eight-byte element slot at `base + i * ELEMENT_SLOT`. The result is a
/// pointer-width value; `store_local` narrows it to the destination
/// local's machine type (a `Float` or narrow scalar element is
/// reinterpreted there).
fn lower_index_access(
    cx: &mut ModuleCx,
    builder: &mut FunctionBuilder<'_>,
    base: &MirOperand,
    index: &MirOperand,
    slots: &[LocalSlot],
) -> Result<RValue, CodegenError> {
    let ptr = cx.pointer_type();
    let list = require_value(lower_operand(cx, builder, base, slots)?, "index base")?;
    let idx = require_value(lower_operand(cx, builder, index, slots)?, "index value")?;
    // The index is a native `Int` (i64); take it to pointer width for the
    // address arithmetic and the unsigned bounds compare.
    let idx = to_pointer_width(builder, idx, ptr);

    let len = call_list_len(cx, builder, list, ptr);
    emit_bounds_check(cx, builder, idx, len, "list index out of bounds");

    let elements = call_list_elements(cx, builder, list);
    let slot_size = builder.ins().iconst(ptr, ELEMENT_SLOT);
    let offset = builder.ins().imul(idx, slot_size);
    let addr = builder.ins().iadd(elements, offset);
    let raw = builder.ins().load(ptr, MemFlags::new(), addr, 0);
    Ok(Some(raw))
}

/// Lower a built-in `List<T>` method to its runtime call.
///
/// `len`/`is_empty` read the count; `push` appends through the shared
/// heap object (mutating it in place, so every alias observes the new
/// element); `pop`/`get` copy an element into a scratch slot and panic
/// when the list is empty or the index is out of range, matching the
/// element-returning method signatures the type checker assigns.
fn lower_list_method(
    cx: &mut ModuleCx,
    builder: &mut FunctionBuilder<'_>,
    op: ListMethodOp,
    receiver: &MirOperand,
    arg: Option<&MirOperand>,
    _elem_ty: &MirType,
    slots: &[LocalSlot],
) -> Result<RValue, CodegenError> {
    let ptr = cx.pointer_type();
    let list = require_value(
        lower_operand(cx, builder, receiver, slots)?,
        "list receiver",
    )?;
    match op {
        ListMethodOp::Len => {
            // `raven_list_len` returns a u32; `call_list_len` already
            // widens it to pointer width, and `to_int64` reconciles it
            // with the native `Int` (i64) the destination local expects.
            let raw = call_list_len(cx, builder, list, ptr);
            Ok(Some(to_int64(builder, raw)))
        }
        ListMethodOp::IsEmpty => {
            let len = call_list_len(cx, builder, list, ptr);
            let zero = builder.ins().iconst(ptr, 0);
            // The result is a `Bool` (i8) in the value model.
            let cmp = builder.ins().icmp(IntCC::Equal, len, zero);
            Ok(Some(cmp))
        }
        ListMethodOp::Push => {
            let value = require_value(
                lower_operand(cx, builder, arg.expect("push has one argument"), slots)?,
                "list push value",
            )?;
            let value = widen_to_slot(builder, Some(value), ptr);
            let scratch = builder.create_sized_stack_slot(StackSlotData::new(
                StackSlotKind::ExplicitSlot,
                ELEMENT_SLOT as u32,
            ));
            builder.ins().stack_store(value, scratch, 0);
            let scratch_addr = builder.ins().stack_addr(ptr, scratch, 0);
            let push_id = cx
                .runtime_id(intrinsics::RUNTIME_LIST_PUSH)
                .expect("list push declared at module init");
            let push_ref = cx.module().declare_func_in_func(push_id, builder.func);
            builder.ins().call(push_ref, &[list, scratch_addr]);
            Ok(None)
        }
        ListMethodOp::Pop => {
            let scratch = builder.create_sized_stack_slot(StackSlotData::new(
                StackSlotKind::ExplicitSlot,
                ELEMENT_SLOT as u32,
            ));
            let scratch_addr = builder.ins().stack_addr(ptr, scratch, 0);
            let pop_id = cx
                .runtime_id(intrinsics::RUNTIME_LIST_POP)
                .expect("list pop declared at module init");
            let pop_ref = cx.module().declare_func_in_func(pop_id, builder.func);
            let inst = builder.ins().call(pop_ref, &[list, scratch_addr]);
            let ok = builder.inst_results(inst)[0];
            emit_status_check(cx, builder, ok, "pop from empty list");
            let raw = builder.ins().stack_load(ptr, scratch, 0);
            Ok(Some(raw))
        }
        ListMethodOp::Get => {
            let idx = require_value(
                lower_operand(cx, builder, arg.expect("get has one argument"), slots)?,
                "list get index",
            )?;
            // `raven_list_get` takes a u32 index; reduce the native Int.
            let idx32 = narrow_to_u32(builder, idx);
            let scratch = builder.create_sized_stack_slot(StackSlotData::new(
                StackSlotKind::ExplicitSlot,
                ELEMENT_SLOT as u32,
            ));
            let scratch_addr = builder.ins().stack_addr(ptr, scratch, 0);
            let get_id = cx
                .runtime_id(intrinsics::RUNTIME_LIST_GET)
                .expect("list get declared at module init");
            let get_ref = cx.module().declare_func_in_func(get_id, builder.func);
            let inst = builder.ins().call(get_ref, &[list, idx32, scratch_addr]);
            let ok = builder.inst_results(inst)[0];
            emit_status_check(cx, builder, ok, "list index out of bounds");
            let raw = builder.ins().stack_load(ptr, scratch, 0);
            Ok(Some(raw))
        }
    }
}

/// Emit `raven_list_new(ELEMENT_SLOT, ELEMENT_SLOT, cap, gc_ptrs) ->
/// List` and return the list pointer.
fn call_list_new(
    cx: &mut ModuleCx,
    builder: &mut FunctionBuilder<'_>,
    cap: i64,
    gc_ptrs: bool,
) -> Value {
    let new_id = cx
        .runtime_id(intrinsics::RUNTIME_LIST_NEW)
        .expect("list new declared at module init");
    let new_ref = cx.module().declare_func_in_func(new_id, builder.func);
    let size = builder.ins().iconst(types::I32, ELEMENT_SLOT);
    let align = builder.ins().iconst(types::I32, ELEMENT_SLOT);
    let cap = builder.ins().iconst(types::I32, cap);
    let flag = builder
        .ins()
        .iconst(types::I32, if gc_ptrs { 1 } else { 0 });
    let inst = builder.ins().call(new_ref, &[size, align, cap, flag]);
    builder.inst_results(inst)[0]
}

/// Emit `raven_list_len(list) -> u32` and return the count zero-extended
/// to pointer width for address arithmetic and comparisons.
fn call_list_len(
    cx: &mut ModuleCx,
    builder: &mut FunctionBuilder<'_>,
    list: Value,
    ptr: CType,
) -> Value {
    let len_id = cx
        .runtime_id(intrinsics::RUNTIME_LIST_LEN)
        .expect("list len declared at module init");
    let len_ref = cx.module().declare_func_in_func(len_id, builder.func);
    let inst = builder.ins().call(len_ref, &[list]);
    let len_u32 = builder.inst_results(inst)[0];
    builder.ins().uextend(ptr, len_u32)
}

/// Emit `raven_list_elements(list) -> ptr` and return the buffer base.
fn call_list_elements(cx: &mut ModuleCx, builder: &mut FunctionBuilder<'_>, list: Value) -> Value {
    let id = cx
        .runtime_id(intrinsics::RUNTIME_LIST_ELEMENTS)
        .expect("list elements declared at module init");
    let fref = cx.module().declare_func_in_func(id, builder.func);
    let inst = builder.ins().call(fref, &[list]);
    builder.inst_results(inst)[0]
}

/// Branch on an unsigned `index < len` check, calling `raven_panic` with
/// `message` on the out-of-bounds path and continuing otherwise.
fn emit_bounds_check(
    cx: &mut ModuleCx,
    builder: &mut FunctionBuilder<'_>,
    index: Value,
    len: Value,
    message: &str,
) {
    let in_bounds = builder.ins().icmp(IntCC::UnsignedLessThan, index, len);
    emit_status_check(cx, builder, in_bounds, message);
}

/// Branch on a nonzero `ok` flag, calling `raven_panic` with `message`
/// when it is zero and continuing on the success path. Used both by the
/// index bounds check and by the `pop`/`get` runtime status results.
fn emit_status_check(
    cx: &mut ModuleCx,
    builder: &mut FunctionBuilder<'_>,
    ok: Value,
    message: &str,
) {
    let panic_block = builder.create_block();
    let continue_block = builder.create_block();
    builder
        .ins()
        .brif(ok, continue_block, &[], panic_block, &[]);

    // The panic path: write the message bytes and terminate. The block is
    // sealed immediately since it has the single predecessor above.
    builder.switch_to_block(panic_block);
    builder.seal_block(panic_block);
    emit_panic(cx, builder, message);
    builder
        .ins()
        .trap(cranelift_codegen::ir::TrapCode::UnreachableCodeReached);

    builder.switch_to_block(continue_block);
    builder.seal_block(continue_block);
}

/// Emit a `raven_panic(msg_ptr, msg_len)` call for a static message.
fn emit_panic(cx: &mut ModuleCx, builder: &mut FunctionBuilder<'_>, message: &str) {
    let ptr = cx.pointer_type();
    let bytes = message.as_bytes();
    let id = cx
        .intern_string(bytes)
        .expect("panic message interns as static bytes");
    let local_id = cx.module().declare_data_in_func(id, builder.func);
    let msg_ptr = builder.ins().symbol_value(ptr, local_id);
    let msg_len = builder.ins().iconst(ptr, bytes.len() as i64);
    let panic_id = cx
        .runtime_id(intrinsics::RUNTIME_PANIC)
        .expect("panic declared at module init");
    let panic_ref = cx.module().declare_func_in_func(panic_id, builder.func);
    builder.ins().call(panic_ref, &[msg_ptr, msg_len]);
}

/// Widen or pass through an integer value to i64 (a native `Int`).
fn to_int64(builder: &mut FunctionBuilder<'_>, v: Value) -> Value {
    let got = builder.func.dfg.value_type(v);
    if got == types::I64 {
        v
    } else if got.is_int() && got.bytes() < types::I64.bytes() {
        builder.ins().uextend(types::I64, v)
    } else {
        v
    }
}

/// Reduce a native `Int` (i64) value to a u32 for a runtime ABI that
/// takes a `u32` index. A value already i32-wide passes through.
fn narrow_to_u32(builder: &mut FunctionBuilder<'_>, v: Value) -> Value {
    let got = builder.func.dfg.value_type(v);
    if got == types::I32 {
        v
    } else if got.is_int() && got.bytes() > types::I32.bytes() {
        builder.ins().ireduce(types::I32, v)
    } else if got.is_int() {
        builder.ins().uextend(types::I32, v)
    } else {
        v
    }
}

/// Width in bytes of one capture slot in the closure env. Every capture,
/// scalar or GC pointer, occupies one pointer-width slot, so the env is a
/// uniform array of pointer-width words. The lifted body and the indirect
/// call agree on this layout.
const CAPTURE_SLOT: i32 = 8;

/// Lower a closure construction.
///
/// Allocates a `Closure` object sized for the captured environment,
/// stores the lifted body's function pointer, and copies each captured
/// value into its env slot. Captures are by value: the value at
/// closure-creation time is copied into the env. For a GC-managed value
/// the copied value is the same pointer, so the captured object aliases
/// the original. Capture analysis orders GC-pointer captures first, so
/// the leading `capture_ptr_count` slots are the traced GC pointers the
/// collector follows through the closure descriptor.
fn lower_closure_create(
    cx: &mut ModuleCx,
    builder: &mut FunctionBuilder<'_>,
    fn_name: &str,
    captures: &[MirOperand],
    capture_tys: &[MirType],
    slots: &[LocalSlot],
) -> Result<RValue, CodegenError> {
    let ptr = cx.pointer_type();
    // Resolve the lifted body's function pointer. A missing function is a
    // lowering bug, but keep the program well formed with a null pointer
    // rather than aborting the whole build.
    let fn_ptr = match cx.function_id(fn_name) {
        Some(id) => {
            let fref = cx.module().declare_func_in_func(id, builder.func);
            builder.ins().func_addr(ptr, fref)
        }
        None => builder.ins().iconst(ptr, 0),
    };

    // Evaluate every capture operand before the allocation so their
    // values are in registers; each operand is a copy of an already
    // rooted local or a constant.
    let mut capture_vals = Vec::with_capacity(captures.len());
    for c in captures {
        let v = lower_operand(cx, builder, c, slots)?;
        capture_vals.push(widen_to_slot(builder, v, ptr));
    }

    let count = captures.len() as i64;
    let capture_size = (captures.len() as i32) * CAPTURE_SLOT;
    let ptr_count = capture_tys
        .iter()
        .filter(|t| layout::is_gc_pointer(t))
        .count() as i64;
    let align: i64 = if captures.is_empty() { 0 } else { 8 };

    let new_id = cx
        .runtime_id(intrinsics::RUNTIME_CLOSURE_NEW)
        .expect("closure new declared at module init");
    let new_ref = cx.module().declare_func_in_func(new_id, builder.func);
    let size_v = builder.ins().iconst(types::I32, capture_size as i64);
    let align_v = builder.ins().iconst(types::I32, align);
    let count_v = builder.ins().iconst(types::I32, count);
    let ptr_count_v = builder.ins().iconst(types::I32, ptr_count);
    let inst = builder
        .ins()
        .call(new_ref, &[fn_ptr, size_v, align_v, count_v, ptr_count_v]);
    let closure = builder.inst_results(inst)[0];

    // Copy each capture value into its env slot. The env base is the
    // closure's owned capture buffer.
    if !captures.is_empty() {
        let captures_id = cx
            .runtime_id(intrinsics::RUNTIME_CLOSURE_CAPTURES)
            .expect("closure captures declared at module init");
        let captures_ref = cx.module().declare_func_in_func(captures_id, builder.func);
        let env_inst = builder.ins().call(captures_ref, &[closure]);
        let env_base = builder.inst_results(env_inst)[0];
        for (i, v) in capture_vals.into_iter().enumerate() {
            builder
                .ins()
                .store(MemFlags::new(), v, env_base, (i as i32) * CAPTURE_SLOT);
        }
    }

    Ok(Some(closure))
}

/// Lower an env load: read a capture from the lifted body's env pointer.
/// The env is the function's leading parameter (a raw pointer-width
/// value); slot `slot` lives at byte offset `slot * CAPTURE_SLOT`. The
/// word is loaded pointer-width and narrowed back to the capture's
/// machine type (a `Float` or narrow scalar capture is reinterpreted by
/// `store_local`).
fn lower_env_load(
    cx: &mut ModuleCx,
    builder: &mut FunctionBuilder<'_>,
    env: &MirOperand,
    slot: usize,
    _ty: &MirType,
    slots: &[LocalSlot],
) -> Result<RValue, CodegenError> {
    let ptr = cx.pointer_type();
    let env_base = require_value(lower_operand(cx, builder, env, slots)?, "env load base")?;
    let raw = builder
        .ins()
        .load(ptr, MemFlags::new(), env_base, (slot as i32) * CAPTURE_SLOT);
    Ok(Some(raw))
}

/// Lower a closure-value call: dispatch indirectly through a `Closure`
/// object. Loads the function pointer and the capture env from the
/// closure, then emits an indirect call passing the env as the leading
/// argument followed by the user arguments. The lifted body's signature
/// is `(env_ptr, <user params...>) -> ret`.
#[allow(clippy::too_many_arguments)]
fn lower_closure_call(
    cx: &mut ModuleCx,
    builder: &mut FunctionBuilder<'_>,
    closure: &MirOperand,
    args: &[MirOperand],
    param_tys: &[MirType],
    ret_ty: &MirType,
    slots: &[LocalSlot],
) -> Result<RValue, CodegenError> {
    let ptr = cx.pointer_type();
    let closure_ptr = require_value(
        lower_operand(cx, builder, closure, slots)?,
        "closure call receiver",
    )?;

    // Load the function pointer from the closure.
    let fn_ptr_id = cx
        .runtime_id(intrinsics::RUNTIME_CLOSURE_FN_PTR)
        .expect("closure fn ptr declared at module init");
    let fn_ptr_ref = cx.module().declare_func_in_func(fn_ptr_id, builder.func);
    let fn_inst = builder.ins().call(fn_ptr_ref, &[closure_ptr]);
    let fn_ptr = builder.inst_results(fn_inst)[0];

    // Load the capture env base from the closure (null when no captures).
    let captures_id = cx
        .runtime_id(intrinsics::RUNTIME_CLOSURE_CAPTURES)
        .expect("closure captures declared at module init");
    let captures_ref = cx.module().declare_func_in_func(captures_id, builder.func);
    let env_inst = builder.ins().call(captures_ref, &[closure_ptr]);
    let env_base = builder.inst_results(env_inst)[0];

    // Build the indirect signature: the env pointer plus each user
    // parameter, returning the closure's return type.
    let mut sig = Signature::new(cx.module().target_config().default_call_conv);
    sig.params.push(cranelift_codegen::ir::AbiParam::new(ptr));
    for pt in param_tys {
        if let Some(t) = cranelift_ty(pt, ptr) {
            sig.params.push(cranelift_codegen::ir::AbiParam::new(t));
        }
    }
    if let Some(t) = cranelift_ty(ret_ty, ptr) {
        sig.returns.push(cranelift_codegen::ir::AbiParam::new(t));
    }
    let sig_ref = builder.import_signature(sig);

    let mut call_args = vec![env_base];
    for a in args {
        if let Some(v) = lower_operand(cx, builder, a, slots)? {
            call_args.push(v);
        }
    }
    let inst = builder.ins().call_indirect(sig_ref, fn_ptr, &call_args);
    Ok(builder.inst_results(inst).first().copied())
}

/// Lower a `dyn Trait` unsizing coercion.
///
/// A trait object is a single GC pointer to a boxed two-slot fat pointer
/// `{ data, vtable }`. This allocates the box through the struct value
/// constructor (so the GC traces it like any aggregate), stores the
/// concrete value in slot 0 (a traced pointer), and the address of the
/// `(concrete_type, trait)` vtable in slot 1 (a static pointer). The
/// box's descriptor marks only slot 0 as a GC pointer, so the collector
/// follows the data word and leaves the static vtable word alone.
#[allow(clippy::too_many_arguments)]
fn lower_dyn_coerce(
    cx: &mut ModuleCx,
    builder: &mut FunctionBuilder<'_>,
    value: &MirOperand,
    concrete_ty: &MirType,
    trait_name: &str,
    methods: &[String],
    slots: &[LocalSlot],
) -> Result<RValue, CodegenError> {
    let ptr = cx.pointer_type();

    // Evaluate the concrete data value first; it is a GC pointer (every
    // struct or enum the front end coerces is a heap value).
    let data = require_value(
        lower_operand(cx, builder, value, slots)?,
        "dyn coerce value",
    )?;
    let data = widen_to_slot(builder, Some(data), ptr);

    // Build the vtable for this (concrete_type, trait) pair. The method
    // symbols are the concrete type's implementations, in trait order.
    let vtable_key = format!("{}${}", concrete_ty.mangle(), trait_name);
    let method_symbols: Vec<String> = methods
        .iter()
        .map(|m| concrete_ty.method_symbol(m))
        .collect();
    let vtable_id = cx.intern_vtable(&vtable_key, &method_symbols)?;
    let vtable_ref = cx.module().declare_data_in_func(vtable_id, builder.func);
    let vtable_ptr = builder.ins().symbol_value(ptr, vtable_ref);

    // The box is a two-slot struct value: slot 0 = data (GC pointer),
    // slot 1 = vtable (static). The descriptor marks only slot 0, keyed
    // by the trait object's mangled name so every coercion to the same
    // `dyn Trait` shares the descriptor.
    let box_key = format!("dyn_{}", trait_name);
    let type_id = cx.intern_descriptor(&box_key, 0b01);
    let obj = call_struct_new(cx, builder, 2, type_id, ptr);
    let base = call_struct_fields(cx, builder, obj, ptr);
    builder
        .ins()
        .store(MemFlags::new(), data, base, layout::slot_offset(0));
    builder
        .ins()
        .store(MemFlags::new(), vtable_ptr, base, layout::slot_offset(1));
    Ok(Some(obj))
}

/// Lower a virtual call through a `dyn Trait` receiver.
///
/// Loads the data and vtable words from the receiver's fat pointer box,
/// loads the method pointer at `slot` from the vtable, and emits an
/// indirect call with the data word as the receiver plus the arguments.
#[allow(clippy::too_many_arguments)]
fn lower_virtual_call(
    cx: &mut ModuleCx,
    builder: &mut FunctionBuilder<'_>,
    receiver: &MirOperand,
    slot: usize,
    args: &[MirOperand],
    param_tys: &[MirType],
    ret_ty: &MirType,
    slots: &[LocalSlot],
) -> Result<RValue, CodegenError> {
    let ptr = cx.pointer_type();
    let box_ptr = require_value(
        lower_operand(cx, builder, receiver, slots)?,
        "virtual call receiver",
    )?;
    let base = call_struct_fields(cx, builder, box_ptr, ptr);
    // Slot 0: the data pointer (the erased receiver). Slot 1: the vtable.
    let data = builder
        .ins()
        .load(ptr, MemFlags::new(), base, layout::slot_offset(0));
    let vtable = builder
        .ins()
        .load(ptr, MemFlags::new(), base, layout::slot_offset(1));
    // Load the method pointer from the vtable's slot.
    let method_ptr = builder.ins().load(
        ptr,
        MemFlags::new(),
        vtable,
        (slot as i32) * (ptr.bytes() as i32),
    );

    // Build the indirect call signature: the receiver (a pointer) plus
    // each non-receiver parameter, returning the method's return type.
    let mut sig = Signature::new(cx.module().target_config().default_call_conv);
    sig.params.push(cranelift_codegen::ir::AbiParam::new(ptr));
    for pt in param_tys {
        if let Some(t) = cranelift_ty(pt, ptr) {
            sig.params.push(cranelift_codegen::ir::AbiParam::new(t));
        }
    }
    if let Some(t) = cranelift_ty(ret_ty, ptr) {
        sig.returns.push(cranelift_codegen::ir::AbiParam::new(t));
    }
    let sig_ref = builder.import_signature(sig);

    let mut call_args = vec![data];
    for a in args {
        if let Some(v) = lower_operand(cx, builder, a, slots)? {
            call_args.push(v);
        }
    }
    let inst = builder.ins().call_indirect(sig_ref, method_ptr, &call_args);
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
    // String runtime intrinsics (concat, per-type to-string, and the
    // `==`/`!=` byte-equality compare). Route each to its runtime symbol;
    // every argument lowers as an ordinary operand (a String pointer or
    // a scalar) and the call returns a single result.
    if let Some(symbol) = intrinsics::string_runtime_symbol(&callee.mangled) {
        let ptr = cx.pointer_type();
        let mut arg_vals = Vec::with_capacity(args.len());
        // Root each freshly promoted String-literal argument across the
        // evaluation of the remaining arguments and the call. A later
        // argument (or the runtime callee, which allocates internally, for
        // example `raven_string_concat`) can trigger a collection that would
        // otherwise free an unrooted earlier argument before the call reads
        // it.
        let mut roots = 0usize;
        for a in args {
            if let Some(v) = lower_operand(cx, builder, a, slots)? {
                if operand_allocates_heap(a) {
                    root_temp(cx, builder, v, ptr);
                    roots += 1;
                }
                arg_vals.push(v);
            }
        }
        let func_id = cx
            .runtime_id(symbol)
            .expect("string runtime symbol declared at module init");
        let local_ref = cx.module().declare_func_in_func(func_id, builder.func);
        let inst = builder.ins().call(local_ref, &arg_vals);
        pop_n_roots(cx, builder, roots, ptr);
        return Ok(builder.inst_results(inst).first().copied());
    }
    let func_id = cx.function_id(&callee.mangled).ok_or_else(|| {
        CodegenError::Unsupported(format!("unresolved callee: {}", callee.mangled))
    })?;
    // When the callee is a foreign C function, coerce each argument to
    // its declared C ABI machine width. A native `Int` passed to a `CInt`
    // parameter is an i64 value that must be reduced to i32 to match the
    // imported signature; an `Int` to a `CLong`/`CSize` is already i64.
    // A `@repr(C)` struct argument is a heap pointer that must instead be
    // packed into the single register the platform ABI passes it in.
    let is_extern = cx.extern_params(&callee.mangled).is_some();
    let extern_param_tys: Option<Vec<MirType>> = cx
        .extern_params(&callee.mangled)
        .or_else(|| cx.fn_params(&callee.mangled))
        .map(|s| s.to_vec());
    let extern_ret: Option<MirType> = cx.extern_ret(&callee.mangled).cloned();
    let local_ref = cx.module().declare_func_in_func(func_id, builder.func);
    let ptr = cx.pointer_type();
    let mut arg_vals = Vec::with_capacity(args.len() + 1);

    // A by-value struct return that does not fit in registers comes back
    // through a hidden pointer (sret) on Windows x64: the caller allocates
    // the result slot and passes its address as the first argument. System V
    // and AArch64 return such a struct in registers, so no hidden argument.
    let ret_struct_plan = match &extern_ret {
        Some(rt) if is_extern && is_repr_c_struct(cx, rt) => Some(repr_c_plan(cx, rt)),
        _ => None,
    };
    let mut sret_addr: Option<Value> = None;
    if matches!(ret_struct_plan, Some(RegPlan::ByRef)) {
        let size = cx
            .repr_c_layout(&extern_ret.as_ref().unwrap().mangle())
            .unwrap()
            .size;
        let (_, addr) = struct_image_slot(cx, builder, size);
        sret_addr = Some(addr);
        arg_vals.push(addr);
    }

    // Root each freshly promoted String-literal argument across the
    // evaluation of the remaining arguments, so a collection a later
    // argument triggers cannot free an earlier one before the call reads it.
    // A by-value struct argument is read into registers or a memory image
    // here and is not a heap value the callee retains, so it needs no root.
    // The roots are popped right after the call site.
    let mut roots = 0usize;
    for (i, a) in args.iter().enumerate() {
        let Some(v) = lower_operand(cx, builder, a, slots)? else {
            continue;
        };
        let param = extern_param_tys
            .as_ref()
            .and_then(|tys| tys.get(i))
            .cloned();
        match param {
            Some(pt) if is_extern && is_repr_c_struct(cx, &pt) => {
                let addr = struct_to_image(cx, builder, v, &pt);
                match repr_c_plan(cx, &pt) {
                    RegPlan::ByRef => arg_vals.push(addr),
                    RegPlan::Regs(slots) => {
                        for s in slots {
                            arg_vals.push(builder.ins().load(
                                s.ty,
                                MemFlags::new(),
                                addr,
                                s.offset as i32,
                            ));
                        }
                    }
                }
            }
            Some(pt) => {
                let coerced = coerce_to_param(builder, v, &pt, ptr);
                if operand_allocates_heap(a) {
                    root_temp(cx, builder, coerced, ptr);
                    roots += 1;
                }
                arg_vals.push(coerced);
            }
            None => {
                if operand_allocates_heap(a) {
                    root_temp(cx, builder, v, ptr);
                    roots += 1;
                }
                arg_vals.push(v);
            }
        }
    }
    let inst = builder.ins().call(local_ref, &arg_vals);
    // Capture results before popping roots, which emits further instructions.
    let results: Vec<Value> = builder.inst_results(inst).to_vec();
    pop_n_roots(cx, builder, roots, ptr);

    // Rebuild a by-value struct return into a Raven heap object from the
    // register(s) or the sret slot it arrived in.
    if let (Some(plan), Some(ret_ty)) = (ret_struct_plan, &extern_ret) {
        let obj = match plan {
            RegPlan::ByRef => image_to_struct(cx, builder, sret_addr.unwrap(), ret_ty),
            RegPlan::Regs(slots) => {
                let size = cx.repr_c_layout(&ret_ty.mangle()).unwrap().size;
                let (slot, addr) = struct_image_slot(cx, builder, size);
                for (s, r) in slots.iter().zip(results.iter()) {
                    builder.ins().stack_store(*r, slot, s.offset as i32);
                }
                image_to_struct(cx, builder, addr, ret_ty)
            }
        };
        return Ok(Some(obj));
    }
    Ok(results.first().copied())
}

/// True when `ty` is a `@repr(C)` struct with a recorded C layout, the
/// shape that crosses the FFI by value in a single register.
fn is_repr_c_struct(cx: &ModuleCx, ty: &MirType) -> bool {
    matches!(ty, MirType::Struct { .. }) && cx.repr_c_layout(&ty.mangle()).is_some()
}

/// The Cranelift integer type that holds exactly `size` bytes (1, 2, 4, or
/// 8). Used to load and store one C field of a struct image.
fn int_type_for_size(size: u32) -> CType {
    match size {
        1 => types::I8,
        2 => types::I16,
        4 => types::I32,
        _ => types::I64,
    }
}

/// One register a by-value struct occupies: the Cranelift type moved through
/// it (i64 for an integer eightbyte, f64/f32 for a float register) and the
/// byte offset into the struct's C image it loads from and stores to.
#[derive(Clone, Copy)]
pub(crate) struct RegSlot {
    pub ty: CType,
    pub offset: u32,
}

/// How a by-value `@repr(C)` struct crosses the C ABI: spread across these
/// registers, or by reference (a pointer to a caller-made copy).
pub(crate) enum RegPlan {
    Regs(Vec<RegSlot>),
    ByRef,
}

fn is_float_ffi_ty(f: &MirFfiTy) -> bool {
    matches!(f, MirFfiTy::CFloat | MirFfiTy::CDouble)
}

fn ffi_byte_size(f: &MirFfiTy) -> u32 {
    match f {
        MirFfiTy::CInt | MirFfiTy::CFloat => 4,
        _ => 8,
    }
}

/// Integer eightbyte registers covering `size` bytes (one i64 per eightbyte).
fn integer_eightbytes(size: u32) -> Vec<RegSlot> {
    let mut out = Vec::new();
    let mut off = 0;
    while off < size {
        out.push(RegSlot {
            ty: types::I64,
            offset: off,
        });
        off += 8;
    }
    out
}

/// System V AMD64 eightbyte classification: an eightbyte is SSE (an f64
/// register) when every field overlapping it is a float, otherwise INTEGER
/// (an i64 register holding the eightbyte's bytes).
fn sysv_eightbytes(layout: &ReprCLayout) -> Vec<RegSlot> {
    let mut out = Vec::new();
    let mut off = 0;
    while off < layout.size {
        let end = off + 8;
        let mut any = false;
        let mut all_float = true;
        for f in &layout.fields {
            if f.offset < end && f.offset + ffi_byte_size(&f.ffi) > off {
                any = true;
                if !is_float_ffi_ty(&f.ffi) {
                    all_float = false;
                }
            }
        }
        let ty = if any && all_float {
            types::F64
        } else {
            types::I64
        };
        out.push(RegSlot { ty, offset: off });
        off += 8;
    }
    out
}

/// AArch64 homogeneous floating aggregate: a struct of 1..=4 fields all of
/// the same float type, each passed in its own SIMD register. `None` for any
/// other shape (which uses general registers / eightbytes).
fn aarch64_hfa(layout: &ReprCLayout) -> Option<Vec<RegSlot>> {
    if layout.fields.is_empty() || layout.fields.len() > 4 {
        return None;
    }
    let all_f32 = layout
        .fields
        .iter()
        .all(|f| matches!(f.ffi, MirFfiTy::CFloat));
    let all_f64 = layout
        .fields
        .iter()
        .all(|f| matches!(f.ffi, MirFfiTy::CDouble));
    let ty = if all_f32 {
        types::F32
    } else if all_f64 {
        types::F64
    } else {
        return None;
    };
    Some(
        layout
            .fields
            .iter()
            .map(|f| RegSlot {
                ty,
                offset: f.offset,
            })
            .collect(),
    )
}

/// The register plan for a by-value `@repr(C)` struct under the target ABI.
/// See `docs/v2/specs/ffi-extensions.md`.
pub(crate) fn repr_c_register_plan(layout: &ReprCLayout, conv: CallConv) -> RegPlan {
    match conv {
        // Windows x64: sizes 1/2/4/8 in one integer register (a float-field
        // struct passes its bits in an integer register), otherwise by
        // reference. No SSE classification.
        CallConv::WindowsFastcall => {
            if matches!(layout.size, 1 | 2 | 4 | 8) {
                RegPlan::Regs(vec![RegSlot {
                    ty: types::I64,
                    offset: 0,
                }])
            } else {
                RegPlan::ByRef
            }
        }
        // AArch64: a homogeneous float aggregate goes in SIMD registers, any
        // other <=16 byte struct in general registers, larger by reference.
        CallConv::AppleAarch64 => {
            if layout.size > 16 {
                RegPlan::ByRef
            } else if let Some(slots) = aarch64_hfa(layout) {
                RegPlan::Regs(slots)
            } else {
                RegPlan::Regs(integer_eightbytes(layout.size))
            }
        }
        // System V AMD64: per-eightbyte INTEGER/SSE classification up to 16
        // bytes, larger structs in memory (by reference).
        _ => {
            if layout.size > 16 {
                RegPlan::ByRef
            } else {
                RegPlan::Regs(sysv_eightbytes(layout))
            }
        }
    }
}

/// The register plan for a `@repr(C)` struct MIR type under the target ABI.
fn repr_c_plan(cx: &ModuleCx, ty: &MirType) -> RegPlan {
    let layout = cx
        .repr_c_layout(&ty.mangle())
        .cloned()
        .expect("is_repr_c_struct checked the layout is present");
    repr_c_register_plan(&layout, cx.default_call_conv())
}

/// Slot size in bytes for a struct's C image: rounded up to 16 and at least
/// 8, so a two-eightbyte load never reads past the slot.
fn image_slot_bytes(size: u32) -> u32 {
    ((size + 7) & !7).max(8)
}

/// Copy a `@repr(C)` struct's heap fields into a fresh stack slot at their C
/// offsets, producing the struct's exact C memory image, and return the slot
/// address. Used to pass a multi-register or by-reference struct argument.
fn struct_to_image(
    cx: &mut ModuleCx,
    builder: &mut FunctionBuilder<'_>,
    obj: Value,
    ty: &MirType,
) -> Value {
    let ptr = cx.pointer_type();
    let layout = cx
        .repr_c_layout(&ty.mangle())
        .cloned()
        .expect("is_repr_c_struct checked the layout is present");
    let bytes = image_slot_bytes(layout.size);
    let slot =
        builder.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, bytes));
    let base = call_struct_fields(cx, builder, obj, ptr);
    for (i, f) in layout.fields.iter().enumerate() {
        let raw = builder
            .ins()
            .load(types::I64, MemFlags::new(), base, layout::slot_offset(i));
        match f.ffi {
            // A float field is stored in the heap as f64 bits. The C image
            // wants f32 for `CFloat` (narrow with fdemote) and f64 for
            // `CDouble` (the same 8 bytes).
            MirFfiTy::CFloat => {
                let f64v = builder.ins().bitcast(types::F64, MemFlags::new(), raw);
                let f32v = builder.ins().fdemote(types::F32, f64v);
                builder.ins().stack_store(f32v, slot, f.offset as i32);
            }
            MirFfiTy::CDouble => {
                builder.ins().stack_store(raw, slot, f.offset as i32);
            }
            _ => {
                let (fsize, _) = f
                    .ffi
                    .c_scalar_layout()
                    .expect("repr(C) field has a scalar layout");
                let v = if fsize >= 8 {
                    raw
                } else {
                    builder.ins().ireduce(int_type_for_size(fsize), raw)
                };
                builder.ins().stack_store(v, slot, f.offset as i32);
            }
        }
    }
    builder.ins().stack_addr(ptr, slot, 0)
}

/// Rebuild a Raven heap struct from a `@repr(C)` struct's C memory image at
/// `addr`. Each field is read at its C offset and width and widened into the
/// pointer-width heap slot. The inverse of `struct_to_image`.
fn image_to_struct(
    cx: &mut ModuleCx,
    builder: &mut FunctionBuilder<'_>,
    addr: Value,
    ty: &MirType,
) -> Value {
    let ptr = cx.pointer_type();
    let layout = cx
        .repr_c_layout(&ty.mangle())
        .cloned()
        .expect("is_repr_c_struct checked the layout is present");
    let type_id = cx.intern_descriptor(&ty.mangle(), 0);
    let field_count = layout.fields.len() as i64;
    let obj = call_struct_new(cx, builder, field_count, type_id, ptr);
    let base = call_struct_fields(cx, builder, obj, ptr);
    for (i, f) in layout.fields.iter().enumerate() {
        // Read the field from the C image at its offset and produce the f64
        // bits a heap slot holds: a `CFloat` is read as f32 and widened with
        // fpromote, a `CDouble` is its 8 bytes, an integer is zero-extended.
        let bits = match f.ffi {
            MirFfiTy::CFloat => {
                let f32v = builder
                    .ins()
                    .load(types::F32, MemFlags::new(), addr, f.offset as i32);
                let f64v = builder.ins().fpromote(types::F64, f32v);
                builder.ins().bitcast(types::I64, MemFlags::new(), f64v)
            }
            MirFfiTy::CDouble => {
                builder
                    .ins()
                    .load(types::I64, MemFlags::new(), addr, f.offset as i32)
            }
            _ => {
                let (fsize, _) = f
                    .ffi
                    .c_scalar_layout()
                    .expect("repr(C) field has a scalar layout");
                let raw = builder.ins().load(
                    int_type_for_size(fsize),
                    MemFlags::new(),
                    addr,
                    f.offset as i32,
                );
                if fsize >= 8 {
                    raw
                } else {
                    builder.ins().uextend(types::I64, raw)
                }
            }
        };
        builder
            .ins()
            .store(MemFlags::new(), bits, base, layout::slot_offset(i));
    }
    obj
}

/// Allocate a fresh stack slot for a struct's C image and return
/// `(slot, address)`. Used to receive an `sret` return.
fn struct_image_slot(
    cx: &ModuleCx,
    builder: &mut FunctionBuilder<'_>,
    size: u32,
) -> (StackSlot, Value) {
    let ptr = cx.pointer_type();
    let slot = builder.create_sized_stack_slot(StackSlotData::new(
        StackSlotKind::ExplicitSlot,
        image_slot_bytes(size),
    ));
    let addr = builder.ins().stack_addr(ptr, slot, 0);
    (slot, addr)
}

/// Reconcile an argument value's machine type with the type the callee's
/// parameter expects. Used for foreign C calls, where a native `Int`
/// (i64) may need reducing to a narrower C integer (`CInt` is i32) or a
/// scalar may need widening to pointer width. Equal widths pass through.
fn coerce_to_param(
    builder: &mut FunctionBuilder<'_>,
    v: Value,
    param: &MirType,
    ptr: CType,
) -> Value {
    let Some(want) = cranelift_ty(param, ptr) else {
        return v;
    };
    let got = builder.func.dfg.value_type(v);
    if got == want {
        return v;
    }
    // A Raven `Float` (f64) passed to a `CFloat` parameter narrows to f32.
    if got == types::F64 && want == types::F32 {
        return builder.ins().fdemote(types::F32, v);
    }
    if !got.is_int() || !want.is_int() {
        return v;
    }
    if want.bytes() < got.bytes() {
        builder.ins().ireduce(want, v)
    } else {
        builder.ins().sextend(want, v)
    }
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
            let (ptr_val, len_val) = lower_string_arg(cx, builder, &args[0], slots)?;
            let func_id = cx
                .runtime_id(intrinsics::RUNTIME_PRINTLN_STR)
                .expect("runtime imports declared at module init");
            let local_ref = cx.module().declare_func_in_func(func_id, builder.func);
            builder.ins().call(local_ref, &[ptr_val, len_val]);
            Ok(None)
        }
        intrinsics::IO_PRINT_STR | intrinsics::IO_PRINTLN_STR => {
            if args.len() != 1 {
                return Err(CodegenError::Unsupported(format!(
                    "{} intrinsic expects 1 arg, got {}",
                    mangled,
                    args.len()
                )));
            }
            let (ptr_val, len_val) = lower_string_arg(cx, builder, &args[0], slots)?;
            let symbol = if mangled == intrinsics::IO_PRINTLN_STR {
                intrinsics::RUNTIME_PRINTLN_STR
            } else {
                intrinsics::RUNTIME_PRINT_STR
            };
            let func_id = cx
                .runtime_id(symbol)
                .expect("runtime imports declared at module init");
            let local_ref = cx.module().declare_func_in_func(func_id, builder.func);
            builder.ins().call(local_ref, &[ptr_val, len_val]);
            Ok(None)
        }
        intrinsics::PANIC_FN => {
            if args.len() != 1 {
                return Err(CodegenError::Unsupported(format!(
                    "__panic intrinsic expects 1 arg, got {}",
                    args.len()
                )));
            }
            let (ptr_val, len_val) = lower_string_arg(cx, builder, &args[0], slots)?;
            let func_id = cx
                .runtime_id(intrinsics::RUNTIME_PANIC)
                .expect("panic declared at module init");
            let local_ref = cx.module().declare_func_in_func(func_id, builder.func);
            // raven_panic terminates the process; the call is treated as a
            // normal returning void call so lowering of any trailing code in
            // the block stays well formed (that code is dead at runtime).
            builder.ins().call(local_ref, &[ptr_val, len_val]);
            Ok(None)
        }
        intrinsics::IO_READ_LINE => {
            if !args.is_empty() {
                return Err(CodegenError::Unsupported(format!(
                    "__io_read_line intrinsic expects 0 args, got {}",
                    args.len()
                )));
            }
            let func_id = cx
                .runtime_id(intrinsics::RUNTIME_READ_LINE)
                .expect("runtime imports declared at module init");
            let local_ref = cx.module().declare_func_in_func(func_id, builder.func);
            let inst = builder.ins().call(local_ref, &[]);
            Ok(builder.inst_results(inst).first().copied())
        }
        intrinsics::STR_LEN => {
            // `raven_string_len` returns a u32; the bundled source treats
            // the result as a native `Int` (i64), so zero-extend it.
            if args.len() != 1 {
                return Err(CodegenError::Unsupported(format!(
                    "__str_len intrinsic expects 1 arg, got {}",
                    args.len()
                )));
            }
            let s = require_value(
                lower_operand(cx, builder, &args[0], slots)?,
                "__str_len argument",
            )?;
            let func_id = cx
                .runtime_id(intrinsics::RUNTIME_STRING_LEN)
                .expect("string-len runtime symbol declared at module init");
            let local_ref = cx.module().declare_func_in_func(func_id, builder.func);
            let inst = builder.ins().call(local_ref, &[s]);
            let len_u32 = builder.inst_results(inst)[0];
            Ok(Some(builder.ins().uextend(types::I64, len_u32)))
        }
        intrinsics::STR_BYTE_AT => {
            // `raven_string_byte_at(String ptr, index: usize) -> i32`.
            // The index is a native `Int` (i64); reduce it to pointer
            // width for the ABI, and sign-extend the i32 result back to a
            // native `Int` so the -1 sentinel survives.
            if args.len() != 2 {
                return Err(CodegenError::Unsupported(format!(
                    "__str_byte_at intrinsic expects 2 args, got {}",
                    args.len()
                )));
            }
            let s = require_value(
                lower_operand(cx, builder, &args[0], slots)?,
                "__str_byte_at string argument",
            )?;
            let idx = require_value(
                lower_operand(cx, builder, &args[1], slots)?,
                "__str_byte_at index argument",
            )?;
            let idx = to_pointer_width(builder, idx, cx.pointer_type());
            let func_id = cx
                .runtime_id(intrinsics::RUNTIME_STRING_BYTE_AT)
                .expect("string-byte-at runtime symbol declared at module init");
            let local_ref = cx.module().declare_func_in_func(func_id, builder.func);
            let inst = builder.ins().call(local_ref, &[s, idx]);
            let byte_i32 = builder.inst_results(inst)[0];
            Ok(Some(builder.ins().sextend(types::I64, byte_i32)))
        }
        intrinsics::STR_SUBSTRING => {
            // `raven_string_substring(String ptr, start, end) -> String`.
            // Both indices are native `Int` reduced to pointer width.
            if args.len() != 3 {
                return Err(CodegenError::Unsupported(format!(
                    "__str_substring intrinsic expects 3 args, got {}",
                    args.len()
                )));
            }
            let ptr = cx.pointer_type();
            let s = require_value(
                lower_operand(cx, builder, &args[0], slots)?,
                "__str_substring string argument",
            )?;
            // Root a String-literal source across the allocating
            // `raven_string_substring` so it is not freed before the call.
            let mut roots = 0usize;
            if operand_allocates_heap(&args[0]) {
                root_temp(cx, builder, s, ptr);
                roots += 1;
            }
            let start = require_value(
                lower_operand(cx, builder, &args[1], slots)?,
                "__str_substring start argument",
            )?;
            let end = require_value(
                lower_operand(cx, builder, &args[2], slots)?,
                "__str_substring end argument",
            )?;
            let start = to_pointer_width(builder, start, ptr);
            let end = to_pointer_width(builder, end, ptr);
            let func_id = cx
                .runtime_id(intrinsics::RUNTIME_STRING_SUBSTRING)
                .expect("string-substring runtime symbol declared at module init");
            let local_ref = cx.module().declare_func_in_func(func_id, builder.func);
            let inst = builder.ins().call(local_ref, &[s, start, end]);
            pop_n_roots(cx, builder, roots, ptr);
            Ok(builder.inst_results(inst).first().copied())
        }
        intrinsics::STR_FROM_BYTE => {
            // `raven_string_from_byte(byte: i32) -> String`. The argument
            // is a native `Int`; reduce it to i32 for the ABI.
            if args.len() != 1 {
                return Err(CodegenError::Unsupported(format!(
                    "__str_from_byte intrinsic expects 1 arg, got {}",
                    args.len()
                )));
            }
            let b = require_value(
                lower_operand(cx, builder, &args[0], slots)?,
                "__str_from_byte argument",
            )?;
            let b_i32 = builder.ins().ireduce(types::I32, b);
            let func_id = cx
                .runtime_id(intrinsics::RUNTIME_STRING_FROM_BYTE)
                .expect("string-from-byte runtime symbol declared at module init");
            let local_ref = cx.module().declare_func_in_func(func_id, builder.func);
            let inst = builder.ins().call(local_ref, &[b_i32]);
            Ok(builder.inst_results(inst).first().copied())
        }
        intrinsics::STR_CONCAT_FN => {
            // `raven_string_concat(String ptr, String ptr) -> String`.
            if args.len() != 2 {
                return Err(CodegenError::Unsupported(format!(
                    "__str_concat intrinsic expects 2 args, got {}",
                    args.len()
                )));
            }
            let ptr = cx.pointer_type();
            let a = require_value(
                lower_operand(cx, builder, &args[0], slots)?,
                "__str_concat first argument",
            )?;
            // Root the first operand across the second's evaluation and the
            // concat call: a String-literal first operand promotes to a heap
            // String, and the second operand's promotion or the allocating
            // `raven_string_concat` would otherwise free it before the call.
            let mut roots = 0usize;
            if operand_allocates_heap(&args[0]) {
                root_temp(cx, builder, a, ptr);
                roots += 1;
            }
            let b = require_value(
                lower_operand(cx, builder, &args[1], slots)?,
                "__str_concat second argument",
            )?;
            let func_id = cx
                .runtime_id(intrinsics::RUNTIME_STRING_CONCAT)
                .expect("string-concat runtime symbol declared at module init");
            let local_ref = cx.module().declare_func_in_func(func_id, builder.func);
            let inst = builder.ins().call(local_ref, &[a, b]);
            pop_n_roots(cx, builder, roots, ptr);
            Ok(builder.inst_results(inst).first().copied())
        }
        intrinsics::DEFER_PUSH_FN => {
            // Park a deferred thunk closure on the current defer frame.
            if args.len() != 1 {
                return Err(CodegenError::Unsupported(format!(
                    "__defer_push intrinsic expects 1 arg, got {}",
                    args.len()
                )));
            }
            let closure = require_value(
                lower_operand(cx, builder, &args[0], slots)?,
                "__defer_push closure",
            )?;
            let func_id = cx
                .runtime_id(intrinsics::RUNTIME_DEFER_PUSH)
                .expect("defer push declared at module init");
            let local_ref = cx.module().declare_func_in_func(func_id, builder.func);
            builder.ins().call(local_ref, &[closure]);
            Ok(None)
        }
        intrinsics::GO_SPAWN_FN => {
            // Start a goroutine running the closure operand.
            if args.len() != 1 {
                return Err(CodegenError::Unsupported(format!(
                    "__go_spawn intrinsic expects 1 arg, got {}",
                    args.len()
                )));
            }
            let closure = require_value(
                lower_operand(cx, builder, &args[0], slots)?,
                "__go_spawn closure",
            )?;
            let func_id = cx
                .runtime_id(intrinsics::RUNTIME_GO_SPAWN)
                .expect("go spawn declared at module init");
            let local_ref = cx.module().declare_func_in_func(func_id, builder.func);
            builder.ins().call(local_ref, &[closure]);
            Ok(None)
        }
        _ => Err(CodegenError::Unsupported(format!(
            "unknown intrinsic: {}",
            mangled
        ))),
    }
}

/// Reduce or extend a native `Int` (i64) value to the platform pointer
/// width so it can be passed where the runtime ABI expects a `usize`.
/// On a 64-bit target the value passes through unchanged.
fn to_pointer_width(builder: &mut FunctionBuilder<'_>, v: Value, ptr: CType) -> Value {
    let got = builder.func.dfg.value_type(v);
    if got == ptr {
        v
    } else if ptr.bytes() < got.bytes() {
        builder.ins().ireduce(ptr, v)
    } else {
        builder.ins().sextend(ptr, v)
    }
}

/// Produce a `(pointer, length)` pair for a string argument that
/// reaches the `print` intrinsic.
///
/// A bare string literal takes the static fast path: the interned bytes
/// and their compile-time length are passed straight to the runtime, so
/// `print("literal")` performs no allocation. Any other string value is
/// a heap `String` pointer (an interpolation result, a `let`-bound
/// string, a returned string, ...); the bytes pointer and byte length
/// are read from the String object through the runtime accessors so
/// `print(someStringValue)` works uniformly.
fn lower_string_arg(
    cx: &mut ModuleCx,
    builder: &mut FunctionBuilder<'_>,
    op: &MirOperand,
    slots: &[LocalSlot],
) -> Result<(Value, Value), CodegenError> {
    let ptr = cx.pointer_type();
    match op {
        MirOperand::Const(MirConstant::Str(s)) => {
            let bytes = s.as_bytes();
            let id = cx.intern_string(bytes)?;
            let local_id = cx.module().declare_data_in_func(id, builder.func);
            let ptr_val = builder.ins().symbol_value(ptr, local_id);
            let len_val = builder.ins().iconst(ptr, bytes.len() as i64);
            Ok((ptr_val, len_val))
        }
        _ => {
            // A heap String value. Load its byte buffer pointer and byte
            // length from the object through the runtime accessors.
            let string_ptr = require_value(
                lower_operand(cx, builder, op, slots)?,
                "print string argument",
            )?;
            let bytes_id = cx
                .runtime_id(intrinsics::RUNTIME_STRING_BYTES)
                .expect("string-bytes runtime symbol declared at module init");
            let bytes_ref = cx.module().declare_func_in_func(bytes_id, builder.func);
            let bytes_inst = builder.ins().call(bytes_ref, &[string_ptr]);
            let ptr_val = builder.inst_results(bytes_inst)[0];

            let len_id = cx
                .runtime_id(intrinsics::RUNTIME_STRING_LEN)
                .expect("string-len runtime symbol declared at module init");
            let len_ref = cx.module().declare_func_in_func(len_id, builder.func);
            let len_inst = builder.ins().call(len_ref, &[string_ptr]);
            // raven_string_len returns a u32; widen it to pointer width
            // for the `raven_println_str(ptr, len)` ABI.
            let len_u32 = builder.inst_results(len_inst)[0];
            let len_val = builder.ins().uextend(ptr, len_u32);
            Ok((ptr_val, len_val))
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn lower_terminator(
    cx: &mut ModuleCx,
    builder: &mut FunctionBuilder<'_>,
    term: &MirTerminator,
    slots: &[LocalSlot],
    blocks: &[cranelift_codegen::ir::Block],
    root_frame: Option<RootFrame>,
    has_defer: bool,
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
            // Evaluate the return value before running defers or leaving
            // the root frame so a collection during evaluation still sees
            // the locals rooted, and the deferred thunks observe the
            // already-computed result (they are Unit-typed and cannot
            // change it). Defers run before leaving the GC frame, so they
            // may still touch rooted GC locals.
            let v = lower_operand(cx, builder, op, slots)?;
            if has_defer {
                run_defer_frame(cx, builder);
            }
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

/// Open the per-call defer frame at function entry.
fn enter_defer_frame(cx: &mut ModuleCx, builder: &mut FunctionBuilder<'_>) {
    let enter = cx
        .runtime_id(intrinsics::RUNTIME_DEFER_ENTER_FRAME)
        .expect("defer enter frame declared at module init");
    let enter_ref = cx.module().declare_func_in_func(enter, builder.func);
    builder.ins().call(enter_ref, &[]);
}

/// Run and pop the per-call defer frame at a return path, invoking the
/// parked thunks in LIFO order.
fn run_defer_frame(cx: &mut ModuleCx, builder: &mut FunctionBuilder<'_>) {
    let run = cx
        .runtime_id(intrinsics::RUNTIME_DEFER_RUN_FRAME)
        .expect("defer run frame declared at module init");
    let run_ref = cx.module().declare_func_in_func(run, builder.func);
    builder.ins().call(run_ref, &[]);
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
        .load(ptr, MemFlags::new(), fields, layout::slot_offset(0));

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
