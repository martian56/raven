//! Per module Cranelift state shared across all functions in a
//! `MirProgram`.
//!
//! The [`ModuleCx`] owns the Cranelift `ObjectModule`, the function
//! symbol table, the string literal interning table, and the
//! declarations for the runtime intrinsics the backend needs.

use std::collections::BTreeMap;
use std::collections::HashMap;

use cranelift_codegen::ir::{types, AbiParam, InstBuilder, Signature};
use cranelift_codegen::isa::CallConv;
use cranelift_codegen::Context;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{DataDescription, DataId, FuncId, Linkage, Module};
use cranelift_object::{ObjectModule, ObjectProduct};

use crate::mir::{MirFunction, MirProgram, MirType};

use super::function::FunctionLowering;
use super::intrinsics;
use super::CodegenError;

/// One registered struct or enum type: the descriptor id passed to
/// `raven_struct_register` and the GC pointer bitmask for its slots.
#[derive(Clone, Copy)]
pub struct StructDescriptor {
    pub type_id: u32,
    pub ptr_mask: u64,
}

/// Per module Cranelift state.
///
/// The struct deliberately keeps the `ObjectModule` private; every
/// declaration goes through one of the typed helpers so the symbol
/// table stays in sync with what Cranelift has been told about.
pub struct ModuleCx {
    module: ObjectModule,
    /// Cranelift function ids keyed by the MIR mangled name.
    functions: HashMap<String, FuncId>,
    /// Runtime intrinsic function ids keyed by their C symbol name.
    runtime: HashMap<&'static str, FuncId>,
    /// Interned string literal data ids keyed by the literal's bytes.
    strings: BTreeMap<Vec<u8>, DataId>,
    /// Counter for unique data symbol names.
    string_counter: u32,
    /// The C entry shim and the Raven `main` it wraps, recorded during
    /// declaration so the bodies can be emitted after every Raven
    /// function is in the symbol table. `None` when the program has no
    /// `main` (for example a unit test compiling a fragment).
    main_entry: Option<MainEntry>,
    /// Struct and enum type descriptors keyed by a stable type name.
    /// Each distinct heap aggregate type gets one descriptor id; the
    /// `main` shim registers every descriptor with the collector before
    /// running the program so a struct is always traceable.
    descriptors: HashMap<String, StructDescriptor>,
    /// Counter handing out the next struct descriptor id.
    next_type_id: u32,
    /// Emitted vtables keyed by `<concrete_type>$<trait>`. Each value is
    /// the read-only data symbol holding the method pointer slots in the
    /// trait's declaration order. Interned so one `(type, trait)` pair
    /// shares a single vtable across every coercion site.
    vtables: HashMap<String, DataId>,
    /// Counter handing out unique vtable data symbol names.
    vtable_counter: u32,
}

/// The exported `main` shim plus the Raven `main` it dispatches to.
struct MainEntry {
    /// The exported `int main(void)` symbol the C runtime starts.
    shim: FuncId,
    /// The internal symbol holding the user's Raven `main` body.
    raven_main: FuncId,
}

/// Symbol name for the user's Raven `main` body. The exported `main`
/// the C runtime calls is a thin shim that invokes this and returns a
/// deterministic `0` exit code, since the Raven entry point returns
/// unit and the C runtime would otherwise read an uninitialized
/// register as the process status.
const RAVEN_MAIN_SYMBOL: &str = "__raven_main";

impl ModuleCx {
    /// Build a fresh context wrapping `module`.
    pub fn new(module: ObjectModule) -> Self {
        Self {
            module,
            functions: HashMap::new(),
            runtime: HashMap::new(),
            strings: BTreeMap::new(),
            string_counter: 0,
            main_entry: None,
            descriptors: HashMap::new(),
            next_type_id: 0,
            vtables: HashMap::new(),
            vtable_counter: 0,
        }
    }

    /// Intern and emit the vtable for a `(concrete_type, trait)` pair.
    ///
    /// `key` is the stable identity `<concrete_type_mangle>$<trait>`.
    /// `method_symbols` is the list of method symbols (the concrete
    /// type's implementations) in the trait's declaration order, which is
    /// the vtable's slot order. The first call emits a read-only data
    /// object with one pointer-sized slot per method, each carrying a
    /// relocation to the method symbol; later calls return the same id.
    ///
    /// Method symbols must already be declared as functions (they are,
    /// because every reachable impl method is in the function table).
    pub fn intern_vtable(
        &mut self,
        key: &str,
        method_symbols: &[String],
    ) -> Result<DataId, CodegenError> {
        if let Some(id) = self.vtables.get(key) {
            return Ok(*id);
        }
        let ptr_bytes = self.pointer_type().bytes() as usize;
        let name = format!("__raven_vtable_{}", self.vtable_counter);
        self.vtable_counter += 1;
        let data_id = self
            .module
            .declare_data(&name, Linkage::Local, false, false)?;

        let mut desc = DataDescription::new();
        // A zeroed buffer of N pointer slots; each slot's bits are filled
        // by a function-address relocation written below.
        let total = ptr_bytes * method_symbols.len().max(1);
        desc.define(vec![0u8; total].into_boxed_slice());
        for (i, sym) in method_symbols.iter().enumerate() {
            let func_id = self.function_id(sym).ok_or_else(|| {
                CodegenError::Unsupported(format!(
                    "vtable slot {} references undefined method symbol `{}`",
                    i, sym
                ))
            })?;
            let func_ref = self.module.declare_func_in_data(func_id, &mut desc);
            desc.write_function_addr((i * ptr_bytes) as u32, func_ref);
        }
        self.module.define_data(data_id, &desc)?;
        self.vtables.insert(key.to_string(), data_id);
        Ok(data_id)
    }

    /// Return the descriptor id for an aggregate type, assigning a fresh
    /// id and recording its GC pointer mask the first time the type is
    /// seen. `name` is a stable per-type key (the mangled type name);
    /// `mask` is the same for every occurrence of a given type, so a
    /// later call with the same name keeps the first id.
    pub fn intern_descriptor(&mut self, name: &str, mask: u64) -> u32 {
        if let Some(d) = self.descriptors.get(name) {
            return d.type_id;
        }
        let type_id = self.next_type_id;
        self.next_type_id += 1;
        self.descriptors.insert(
            name.to_string(),
            StructDescriptor {
                type_id,
                ptr_mask: mask,
            },
        );
        type_id
    }

    /// Like `intern_descriptor`, but unions `mask` into an existing
    /// descriptor rather than keeping the first one. Used for enum types,
    /// whose variants are constructed independently: a value's traced
    /// pointer slots are the union of every variant's payload pointers,
    /// so the collector traces the active variant's pointers whichever
    /// it is. (A variant only ever populates its own payload slots; the
    /// inactive slots are zero and trace harmlessly.)
    pub fn merge_descriptor(&mut self, name: &str, mask: u64) -> u32 {
        if let Some(d) = self.descriptors.get_mut(name) {
            d.ptr_mask |= mask;
            return d.type_id;
        }
        let type_id = self.next_type_id;
        self.next_type_id += 1;
        self.descriptors.insert(
            name.to_string(),
            StructDescriptor {
                type_id,
                ptr_mask: mask,
            },
        );
        type_id
    }

    /// Borrow the underlying Cranelift module for low level operations.
    pub fn module(&mut self) -> &mut ObjectModule {
        &mut self.module
    }

    /// Look up the Cranelift function id for a previously declared MIR
    /// function by its mangled name.
    pub fn function_id(&self, mangled: &str) -> Option<FuncId> {
        self.functions.get(mangled).copied()
    }

    /// Look up the Cranelift function id for a runtime intrinsic by
    /// its C symbol name.
    pub fn runtime_id(&self, symbol: &str) -> Option<FuncId> {
        self.runtime.get(symbol).copied()
    }

    /// Width of an integer wide enough to hold a pointer on the host
    /// target.
    pub fn pointer_type(&self) -> cranelift_codegen::ir::Type {
        self.module.target_config().pointer_type()
    }

    /// Intern a byte slice as a static data symbol and return its id.
    ///
    /// Identical byte sequences share a single symbol so that repeated
    /// literals do not bloat the object file.
    pub fn intern_string(&mut self, bytes: &[u8]) -> Result<DataId, CodegenError> {
        if let Some(id) = self.strings.get(bytes) {
            return Ok(*id);
        }
        let name = format!("__raven_str_{}", self.string_counter);
        self.string_counter += 1;
        let id = self
            .module
            .declare_data(&name, Linkage::Local, false, false)?;
        let mut desc = DataDescription::new();
        desc.define(bytes.to_vec().into_boxed_slice());
        self.module.define_data(id, &desc)?;
        self.strings.insert(bytes.to_vec(), id);
        Ok(id)
    }

    /// Declare the runtime C ABI symbols the backend can call into.
    ///
    /// The heap-value lowering needs the string and integer print
    /// helpers, the struct value constructor and accessor, the GC root
    /// frame and struct descriptor registration entry points, and the
    /// closure constructor and accessors. They are all declared up front
    /// so any function body can reference them.
    pub fn declare_runtime_imports(&mut self) -> Result<(), CodegenError> {
        let ptr = self.pointer_type();
        let i32t = types::I32;
        let i64t = types::I64;

        // raven_println_str(ptr, len)
        let mut sig = self.make_sig(&[ptr, ptr], &[]);
        self.declare_runtime(intrinsics::RUNTIME_PRINTLN_STR, &sig)?;

        // raven_println_int(i64)
        sig = self.make_sig(&[i64t], &[]);
        self.declare_runtime(intrinsics::RUNTIME_PRINTLN_INT, &sig)?;

        // raven_struct_new(field_count: u32, type_id: u32) -> ptr
        sig = self.make_sig(&[i32t, i32t], &[ptr]);
        self.declare_runtime(intrinsics::RUNTIME_STRUCT_NEW, &sig)?;

        // raven_struct_fields(ptr) -> ptr
        sig = self.make_sig(&[ptr], &[ptr]);
        self.declare_runtime(intrinsics::RUNTIME_STRUCT_FIELDS, &sig)?;

        // raven_struct_register(type_id: u32, ptr_mask: u64)
        sig = self.make_sig(&[i32t, i64t], &[]);
        self.declare_runtime(intrinsics::RUNTIME_STRUCT_REGISTER, &sig)?;

        // raven_gc_enter_frame(roots: ptr, count: usize)
        sig = self.make_sig(&[ptr, ptr], &[]);
        self.declare_runtime(intrinsics::RUNTIME_GC_ENTER_FRAME, &sig)?;

        // raven_gc_leave_frame()
        sig = self.make_sig(&[], &[]);
        self.declare_runtime(intrinsics::RUNTIME_GC_LEAVE_FRAME, &sig)?;

        // raven_closure_new(fn_ptr, size: u32, align: u32, count: u32,
        //                   ptr_count: u32) -> ptr
        sig = self.make_sig(&[ptr, i32t, i32t, i32t, i32t], &[ptr]);
        self.declare_runtime(intrinsics::RUNTIME_CLOSURE_NEW, &sig)?;

        // raven_closure_fn_ptr(ptr) -> ptr
        sig = self.make_sig(&[ptr], &[ptr]);
        self.declare_runtime(intrinsics::RUNTIME_CLOSURE_FN_PTR, &sig)?;

        // raven_closure_captures(ptr) -> ptr
        sig = self.make_sig(&[ptr], &[ptr]);
        self.declare_runtime(intrinsics::RUNTIME_CLOSURE_CAPTURES, &sig)?;

        // String value support for interpolation and the generalized
        // print path.
        let f64t = types::F64;

        // raven_string_from_bytes(ptr, len) -> String ptr
        sig = self.make_sig(&[ptr, ptr], &[ptr]);
        self.declare_runtime(intrinsics::RUNTIME_STRING_FROM_BYTES, &sig)?;

        // raven_string_bytes(String ptr) -> byte ptr
        sig = self.make_sig(&[ptr], &[ptr]);
        self.declare_runtime(intrinsics::RUNTIME_STRING_BYTES, &sig)?;

        // raven_string_len(String ptr) -> u32
        sig = self.make_sig(&[ptr], &[i32t]);
        self.declare_runtime(intrinsics::RUNTIME_STRING_LEN, &sig)?;

        // raven_string_concat(String ptr, String ptr) -> String ptr
        sig = self.make_sig(&[ptr, ptr], &[ptr]);
        self.declare_runtime(intrinsics::RUNTIME_STRING_CONCAT, &sig)?;

        // raven_int_to_string(i64) -> String ptr
        sig = self.make_sig(&[i64t], &[ptr]);
        self.declare_runtime(intrinsics::RUNTIME_INT_TO_STRING, &sig)?;

        // raven_bool_to_string(i8) -> String ptr
        sig = self.make_sig(&[types::I8], &[ptr]);
        self.declare_runtime(intrinsics::RUNTIME_BOOL_TO_STRING, &sig)?;

        // raven_float_to_string(f64) -> String ptr
        sig = self.make_sig(&[f64t], &[ptr]);
        self.declare_runtime(intrinsics::RUNTIME_FLOAT_TO_STRING, &sig)?;

        // raven_char_to_string(u32) -> String ptr
        sig = self.make_sig(&[i32t], &[ptr]);
        self.declare_runtime(intrinsics::RUNTIME_CHAR_TO_STRING, &sig)?;

        Ok(())
    }

    /// Build a Cranelift signature from parameter and return types under
    /// the module's default calling convention.
    fn make_sig(
        &self,
        params: &[cranelift_codegen::ir::Type],
        returns: &[cranelift_codegen::ir::Type],
    ) -> Signature {
        let mut s = self.module.make_signature();
        for p in params {
            s.params.push(AbiParam::new(*p));
        }
        for r in returns {
            s.returns.push(AbiParam::new(*r));
        }
        s
    }

    /// Declare one imported runtime symbol and record its id.
    fn declare_runtime(
        &mut self,
        symbol: &'static str,
        sig: &Signature,
    ) -> Result<(), CodegenError> {
        let id = self.module.declare_function(symbol, Linkage::Import, sig)?;
        self.runtime.insert(symbol, id);
        Ok(())
    }

    /// Declare every MIR function ahead of body emission so that calls
    /// between functions can be resolved without a fix up pass.
    ///
    /// The Raven `main` is declared under an internal symbol and wrapped
    /// by an exported `int main(void)` shim. The shim is what the C
    /// runtime starts; it calls the Raven body and returns `0`, so the
    /// process exit code is deterministic rather than whatever the
    /// runtime reads out of a register after a unit returning function.
    pub fn declare_functions(&mut self, program: &MirProgram) -> Result<(), CodegenError> {
        for func in &program.functions {
            let sig = self.signature_for(func)?;
            let is_main = func.origin == "main";
            let linkage = Linkage::Local;
            let name = if is_main {
                RAVEN_MAIN_SYMBOL.to_string()
            } else {
                func.name.clone()
            };
            let id = self.module.declare_function(&name, linkage, &sig)?;
            self.functions.insert(func.name.clone(), id);
            if is_main {
                let shim = self.declare_main_shim()?;
                self.main_entry = Some(MainEntry {
                    shim,
                    raven_main: id,
                });
            }
        }
        Ok(())
    }

    /// Declare the exported `int main(void)` C entry shim.
    fn declare_main_shim(&mut self) -> Result<FuncId, CodegenError> {
        let mut sig = Signature::new(self.module.target_config().default_call_conv);
        sig.returns.push(AbiParam::new(types::I32));
        let id = self
            .module
            .declare_function("main", Linkage::Export, &sig)?;
        Ok(id)
    }

    /// Lower the body of every declared MIR function.
    pub fn define_functions(&mut self, program: &MirProgram) -> Result<(), CodegenError> {
        for func in &program.functions {
            self.define_one(func)?;
        }
        if self.main_entry.is_some() {
            self.define_main_shim()?;
        }
        Ok(())
    }

    /// Emit the body of the `int main(void)` shim: call the Raven
    /// `main`, discard its result, and return `0`.
    fn define_main_shim(&mut self) -> Result<(), CodegenError> {
        let MainEntry { shim, raven_main } = self
            .main_entry
            .as_ref()
            .expect("define_main_shim called without a declared main");
        let shim = *shim;
        let raven_main = *raven_main;

        let mut ctx = Context::new();
        ctx.func.signature = {
            let mut sig = Signature::new(self.module.target_config().default_call_conv);
            sig.returns.push(AbiParam::new(types::I32));
            sig
        };

        let callee = self.module.declare_func_in_func(raven_main, &mut ctx.func);
        // Collect descriptors and the registration symbol before the
        // builder borrows the function, so the closure below only needs
        // the resolved references.
        let descriptors: Vec<StructDescriptor> = self.descriptors.values().copied().collect();
        let register_id = self.runtime_id(intrinsics::RUNTIME_STRUCT_REGISTER);
        let register_ref =
            register_id.map(|id| self.module.declare_func_in_func(id, &mut ctx.func));
        let mut builder_ctx = FunctionBuilderContext::new();
        {
            let mut builder = FunctionBuilder::new(&mut ctx.func, &mut builder_ctx);
            let block = builder.create_block();
            builder.switch_to_block(block);
            builder.seal_block(block);
            // Register every struct and enum descriptor with the
            // collector before running the program, so any value the
            // program builds is traceable from its first allocation.
            if let Some(reg) = register_ref {
                for d in &descriptors {
                    let id = builder.ins().iconst(types::I32, d.type_id as i64);
                    let mask = builder.ins().iconst(types::I64, d.ptr_mask as i64);
                    builder.ins().call(reg, &[id, mask]);
                }
            }
            // The Raven `main` returns unit, so there is no result value
            // to forward; the call is emitted purely for its effects.
            builder.ins().call(callee, &[]);
            let zero = builder.ins().iconst(types::I32, 0);
            builder.ins().return_(&[zero]);
            builder.finalize();
        }

        self.module
            .define_function(shim, &mut ctx)
            .map_err(|e| CodegenError::Codegen(format!("define main shim: {}", e)))?;
        Ok(())
    }

    fn define_one(&mut self, func: &MirFunction) -> Result<(), CodegenError> {
        let func_id = self
            .functions
            .get(&func.name)
            .copied()
            .expect("function declared in declare_functions");
        let sig = self.signature_for(func)?;
        let mut ctx = Context::new();
        ctx.func.signature = sig;

        {
            let mut lowering = FunctionLowering::new(self, &mut ctx.func, func);
            lowering.lower()?;
        }

        self.module
            .define_function(func_id, &mut ctx)
            .map_err(|e| CodegenError::Codegen(format!("define {}: {}", func.name, e)))?;
        Ok(())
    }

    /// Consume the context and return the Cranelift `ObjectProduct`
    /// that knows how to serialize itself to bytes.
    pub fn finish(self) -> ObjectProduct {
        self.module.finish()
    }

    /// Build a Cranelift `Signature` from a MIR function's parameter
    /// and return types.
    pub fn signature_for(&self, func: &MirFunction) -> Result<Signature, CodegenError> {
        let mut sig = Signature::new(CallConv::SystemV);
        sig.call_conv = self.module.target_config().default_call_conv;
        for param_local in &func.params {
            let decl = func.local_decl(*param_local);
            if let Some(ty) = super::function::cranelift_ty(&decl.ty, self.pointer_type()) {
                sig.params.push(AbiParam::new(ty));
            }
        }
        if let Some(ty) = super::function::cranelift_ty(&func.ret_ty, self.pointer_type()) {
            sig.returns.push(AbiParam::new(ty));
        }
        let _ = MirType::Unit; // pull import into scope for the lint pass
        Ok(sig)
    }
}
