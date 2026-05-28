//! Recognized intrinsic mangled names.
//!
//! The MIR lowering emits ordinary `Call` rvalues for the built in
//! `print` free function; the codegen pattern matches on the mangled
//! name and rewrites the call into a runtime ABI call. Future stdlib
//! intrinsics extend this table.

/// MIR mangled name produced by the front end for a `print(s)` call.
pub const PRINT: &str = "print";

/// MIR mangled name produced by the front end for a `print_int(n)` call.
pub const PRINT_INT: &str = "print_int";

/// Runtime C symbol the `print` intrinsic dispatches to.
pub const RUNTIME_PRINTLN_STR: &str = "raven_println_str";

/// Runtime C symbol the `print_int` intrinsic dispatches to.
pub const RUNTIME_PRINTLN_INT: &str = "raven_println_int";

/// Runtime C symbol allocating a struct or enum value body.
pub const RUNTIME_STRUCT_NEW: &str = "raven_struct_new";

/// Runtime C symbol returning a pointer to a struct or enum value's
/// field slots.
pub const RUNTIME_STRUCT_FIELDS: &str = "raven_struct_fields";

/// Runtime C symbol registering a struct or enum type's GC pointer
/// descriptor.
pub const RUNTIME_STRUCT_REGISTER: &str = "raven_struct_register";

/// Runtime C symbol entering a GC root frame.
pub const RUNTIME_GC_ENTER_FRAME: &str = "raven_gc_enter_frame";

/// Runtime C symbol leaving the most recent GC root frame.
pub const RUNTIME_GC_LEAVE_FRAME: &str = "raven_gc_leave_frame";

/// Runtime C symbol allocating a closure object.
pub const RUNTIME_CLOSURE_NEW: &str = "raven_closure_new";

/// Runtime C symbol returning a closure's function pointer.
pub const RUNTIME_CLOSURE_FN_PTR: &str = "raven_closure_fn_ptr";

/// Runtime C symbol returning a closure's capture buffer.
pub const RUNTIME_CLOSURE_CAPTURES: &str = "raven_closure_captures";

/// True when `mangled` is one of the recognized intrinsics.
pub fn is_intrinsic(mangled: &str) -> bool {
    matches!(mangled, PRINT | PRINT_INT)
}
