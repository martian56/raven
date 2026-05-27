//! Recognized intrinsic mangled names.
//!
//! The MIR lowering emits ordinary `Call` rvalues for the built in
//! `print` free function; the codegen pattern matches on the mangled
//! name and rewrites the call into a runtime ABI call. Future stdlib
//! intrinsics extend this table.

/// MIR mangled name produced by the front end for a `print(s)` call.
pub const PRINT: &str = "print";

/// Runtime C symbol the `print` intrinsic dispatches to.
pub const RUNTIME_PRINTLN_STR: &str = "raven_println_str";

/// True when `mangled` is one of the recognized intrinsics.
pub fn is_intrinsic(mangled: &str) -> bool {
    matches!(mangled, PRINT)
}
