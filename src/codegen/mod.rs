//! Cranelift backend for the Raven v2 compiler.
//!
//! The codegen module consumes a fully monomorphized [`MirProgram`]
//! and produces a native relocatable object file. The object is then
//! linked with the `raven-runtime` staticlib by the driver to produce
//! an executable.
//!
//! Scope for the MVP (issue #63): primitives, binary and unary
//! operators on `Int` / `Float` / `Bool`, branches, switches, static
//! function calls, returns, and the `print` intrinsic. Heap allocated
//! values, trait objects, closures, and `defer` are tracked by issues
//! #65, #66, #67, and #68.
//!
//! See `docs/v2/specs/codegen.md` for the full design.

pub mod context;
pub mod function;
pub mod intrinsics;
pub mod layout;
pub mod linker;

#[cfg(test)]
mod tests;

use std::fmt;
use std::sync::Arc;

use cranelift_codegen::isa::TargetIsa;
use cranelift_codegen::settings::{self, Configurable};
use cranelift_module::ModuleError;
use cranelift_object::{ObjectBuilder, ObjectModule};

use crate::mir::MirProgram;

use context::ModuleCx;

/// Errors that can surface during codegen.
///
/// Most diagnostics are programmer errors (a deferred construct sneaks
/// past the supported subset) rather than user errors, since the type
/// checker, resolver, and MIR builder have already validated the input
/// program.
#[derive(Debug)]
pub enum CodegenError {
    /// The MIR construct is not yet supported by the backend.
    Unsupported(String),
    /// The Cranelift module API rejected a declaration or definition.
    Module(ModuleError),
    /// Cranelift's instruction selection failed.
    Codegen(String),
    /// The host target could not be resolved.
    Target(String),
}

impl fmt::Display for CodegenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CodegenError::Unsupported(s) => write!(f, "codegen: unsupported MIR construct: {}", s),
            CodegenError::Module(e) => write!(f, "codegen: module error: {}", e),
            CodegenError::Codegen(s) => write!(f, "codegen: instruction selection failed: {}", s),
            CodegenError::Target(s) => write!(f, "codegen: target setup failed: {}", s),
        }
    }
}

impl std::error::Error for CodegenError {}

impl From<ModuleError> for CodegenError {
    fn from(e: ModuleError) -> Self {
        CodegenError::Module(e)
    }
}

/// Build a Cranelift target ISA for the host machine.
///
/// Uses the system triple and Cranelift's default optimization flags.
/// Returns a `CodegenError::Target` if Cranelift cannot construct an
/// ISA for the host (effectively impossible on a supported target).
pub fn host_isa() -> Result<Arc<dyn TargetIsa>, CodegenError> {
    let mut flag_builder = settings::builder();
    // Position-independent code is required by most modern linkers and
    // does not cost anything on x86_64; we set it unconditionally.
    flag_builder
        .set("is_pic", "true")
        .map_err(|e| CodegenError::Target(e.to_string()))?;
    let flags = settings::Flags::new(flag_builder);
    let isa_builder = cranelift_native::builder()
        .map_err(|s| CodegenError::Target(format!("native ISA: {}", s)))?;
    isa_builder
        .finish(flags)
        .map_err(|e| CodegenError::Target(e.to_string()))
}

/// Compile a [`MirProgram`] into a relocatable object file.
///
/// Returns the raw object bytes, ready to be written to disk and
/// linked. Use [`compile_to_object`] when the caller already has an
/// ISA; this convenience routine builds the host ISA first.
pub fn compile_program(program: &MirProgram) -> Result<Vec<u8>, CodegenError> {
    let isa = host_isa()?;
    compile_to_object(program, isa)
}

/// Compile a [`MirProgram`] into a relocatable object file using the
/// provided target ISA. Exposed as a thin wrapper so tests can drive
/// the codegen without invoking the host detection.
pub fn compile_to_object(
    program: &MirProgram,
    isa: Arc<dyn TargetIsa>,
) -> Result<Vec<u8>, CodegenError> {
    let builder = ObjectBuilder::new(
        isa,
        "raven-program".as_bytes().to_vec(),
        cranelift_module::default_libcall_names(),
    )
    .map_err(|e| CodegenError::Target(e.to_string()))?;
    let module = ObjectModule::new(builder);

    let mut cx = ModuleCx::new(module);
    cx.declare_runtime_imports()?;
    cx.declare_functions(program)?;
    cx.define_functions(program)?;

    let product = cx.finish();
    product
        .emit()
        .map_err(|e| CodegenError::Codegen(e.to_string()))
}
