//! Per module Cranelift state shared across all functions in a
//! `MirProgram`.
//!
//! The [`ModuleCx`] owns the Cranelift `ObjectModule`, the function
//! symbol table, the string literal interning table, and the
//! declarations for the runtime intrinsics the backend needs.

use std::collections::BTreeMap;
use std::collections::HashMap;

use cranelift_codegen::ir::{AbiParam, Signature};
use cranelift_codegen::isa::CallConv;
use cranelift_codegen::Context;
use cranelift_module::{DataDescription, DataId, FuncId, Linkage, Module};
use cranelift_object::{ObjectModule, ObjectProduct};

use crate::mir::{MirFunction, MirProgram, MirType};

use super::function::FunctionLowering;
use super::intrinsics;
use super::CodegenError;

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
}

impl ModuleCx {
    /// Build a fresh context wrapping `module`.
    pub fn new(module: ObjectModule) -> Self {
        Self {
            module,
            functions: HashMap::new(),
            runtime: HashMap::new(),
            strings: BTreeMap::new(),
            string_counter: 0,
        }
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
    /// Only `raven_println_str` is declared by default; the rest are
    /// reserved for future intrinsic wiring and are pulled in lazily
    /// to keep the import table minimal.
    pub fn declare_runtime_imports(&mut self) -> Result<(), CodegenError> {
        let ptr = self.pointer_type();
        let sig_print = {
            let mut s = self.module.make_signature();
            s.params.push(AbiParam::new(ptr));
            s.params.push(AbiParam::new(ptr));
            s
        };
        let id = self.module.declare_function(
            intrinsics::RUNTIME_PRINTLN_STR,
            Linkage::Import,
            &sig_print,
        )?;
        self.runtime.insert(intrinsics::RUNTIME_PRINTLN_STR, id);
        Ok(())
    }

    /// Declare every MIR function ahead of body emission so that calls
    /// between functions can be resolved without a fix up pass.
    pub fn declare_functions(&mut self, program: &MirProgram) -> Result<(), CodegenError> {
        for func in &program.functions {
            let sig = self.signature_for(func)?;
            let linkage = if func.origin == "main" {
                Linkage::Export
            } else {
                Linkage::Local
            };
            let name = if func.origin == "main" {
                "main".to_string()
            } else {
                func.name.clone()
            };
            let id = self.module.declare_function(&name, linkage, &sig)?;
            self.functions.insert(func.name.clone(), id);
        }
        Ok(())
    }

    /// Lower the body of every declared MIR function.
    pub fn define_functions(&mut self, program: &MirProgram) -> Result<(), CodegenError> {
        for func in &program.functions {
            self.define_one(func)?;
        }
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
