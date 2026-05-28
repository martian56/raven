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

/// Internal stdlib I/O intrinsics. The bundled `std/io` source calls
/// these to reach the runtime's byte-level I/O symbols; the leading
/// `__io_` marks them internal (a user uses `std/io`'s exported
/// functions instead). See `docs/v2/specs/stdlib.md`.
///
/// `__io_print_str(s: String)` writes the bytes of `s` with no newline.
pub const IO_PRINT_STR: &str = "__io_print_str";
/// `__io_println_str(s: String)` writes the bytes of `s` plus a newline.
pub const IO_PRINTLN_STR: &str = "__io_println_str";
/// `__io_read_line() -> String` reads one line from stdin (no newline).
pub const IO_READ_LINE: &str = "__io_read_line";

/// Runtime C symbol the `print` intrinsic dispatches to.
pub const RUNTIME_PRINTLN_STR: &str = "raven_println_str";

/// Runtime C symbol writing bytes to stdout with no trailing newline.
pub const RUNTIME_PRINT_STR: &str = "raven_print_str";

/// Runtime C symbol reading one line from stdin into a heap `String`.
pub const RUNTIME_READ_LINE: &str = "raven_read_line";

/// Runtime C symbol the `print_int` intrinsic dispatches to.
pub const RUNTIME_PRINTLN_INT: &str = "raven_println_int";

/// Runtime C symbol building a heap `String` from a byte slice. Used to
/// promote a string literal into a heap `String` value.
pub const RUNTIME_STRING_FROM_BYTES: &str = "raven_string_from_bytes";

/// Runtime C symbol returning a heap `String`'s byte buffer pointer.
pub const RUNTIME_STRING_BYTES: &str = "raven_string_bytes";

/// Runtime C symbol returning a heap `String`'s byte length.
pub const RUNTIME_STRING_LEN: &str = "raven_string_len";

/// Runtime C symbols backing the interpolation desugaring intrinsics.
/// Each MIR mangled name on the left dispatches to the runtime symbol on
/// the right; see [`crate::mir::intrinsics`].
pub const RUNTIME_STRING_CONCAT: &str = "raven_string_concat";
pub const RUNTIME_INT_TO_STRING: &str = "raven_int_to_string";
pub const RUNTIME_BOOL_TO_STRING: &str = "raven_bool_to_string";
pub const RUNTIME_FLOAT_TO_STRING: &str = "raven_float_to_string";
pub const RUNTIME_CHAR_TO_STRING: &str = "raven_char_to_string";

/// Map a MIR interpolation intrinsic mangled name to the runtime C
/// symbol it lowers to, or `None` when `mangled` is not one of them.
pub fn interpolation_runtime_symbol(mangled: &str) -> Option<&'static str> {
    use crate::mir::intrinsics as mir_intr;
    Some(match mangled {
        mir_intr::STR_CONCAT => RUNTIME_STRING_CONCAT,
        mir_intr::INT_TO_STRING => RUNTIME_INT_TO_STRING,
        mir_intr::BOOL_TO_STRING => RUNTIME_BOOL_TO_STRING,
        mir_intr::FLOAT_TO_STRING => RUNTIME_FLOAT_TO_STRING,
        mir_intr::CHAR_TO_STRING => RUNTIME_CHAR_TO_STRING,
        _ => return None,
    })
}

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
    matches!(
        mangled,
        PRINT | PRINT_INT | IO_PRINT_STR | IO_PRINTLN_STR | IO_READ_LINE
    )
}
