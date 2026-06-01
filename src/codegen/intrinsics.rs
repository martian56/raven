//! Recognized intrinsic mangled names.
//!
//! The MIR lowering emits ordinary `Call` rvalues for the built in
//! `print` free function; the codegen pattern matches on the mangled
//! name and rewrites the call into a runtime ABI call. Future stdlib
//! intrinsics extend this table.

/// MIR mangled name produced by the front end for a `print(s)` call.
pub const PRINT: &str = "print";

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

/// Internal panic intrinsic. The bundled `std/test` source calls
/// `__panic(msg: String)` to abort the process with a message on a failed
/// assertion; it lowers to the runtime's `raven_panic` symbol. The
/// leading `__` marks it internal (a user calls `std/test`'s assertions).
pub const PANIC_FN: &str = "__panic";

/// Internal stdlib string intrinsics. The bundled `std/string` source
/// calls these byte-level primitives to build the higher-level utilities
/// (case mapping, search, trim, ...) in pure Raven. The leading `__str_`
/// marks them internal; a user calls the exported `std/string` functions
/// instead. See `docs/v2/specs/std-string.md`.
///
/// `__str_len(s: String) -> Int` returns the byte length of `s`.
pub const STR_LEN: &str = "__str_len";
/// `__str_byte_at(s: String, i: Int) -> Int` returns the byte at index
/// `i` as a value in `0..=255`, or `-1` when `i` is out of range.
pub const STR_BYTE_AT: &str = "__str_byte_at";
/// `__str_substring(s: String, start: Int, end: Int) -> String` returns
/// the half-open byte range `[start, end)` of `s` (bounds clamped).
pub const STR_SUBSTRING: &str = "__str_substring";
/// `__str_from_byte(b: Int) -> String` builds a one-byte `String` from
/// the low eight bits of `b`.
pub const STR_FROM_BYTE: &str = "__str_from_byte";
/// `__str_concat(a: String, b: String) -> String` concatenates two
/// strings into a fresh `String`.
pub const STR_CONCAT_FN: &str = "__str_concat";

/// Internal defer intrinsic. MIR lowering of a `defer expr` builds a
/// thunk closure capturing what `expr` needs, then emits
/// `__defer_push(thunk)`, which lowers to the runtime's
/// `raven_defer_push`. The function epilogue runs the parked thunks at
/// each return. See `docs/v2/specs/defer.md`.
pub const DEFER_PUSH_FN: &str = "__defer_push";

/// Internal spawn intrinsic. MIR lowering of a `spawn expr` evaluates
/// `expr` to a `fun() -> Unit` closure and emits `__go_spawn(closure)`,
/// which lowers to the runtime's `raven_go_spawn`. The runtime starts a
/// goroutine running the closure. See `docs/v2/specs/concurrency.md`.
pub const GO_SPAWN_FN: &str = "__go_spawn";

/// Runtime C symbol the `print` intrinsic dispatches to.
pub const RUNTIME_PRINTLN_STR: &str = "raven_println_str";

/// Runtime C symbol writing bytes to stdout with no trailing newline.
pub const RUNTIME_PRINT_STR: &str = "raven_print_str";

/// Runtime C symbol reading one line from stdin into a heap `String`.
pub const RUNTIME_READ_LINE: &str = "raven_read_line";

/// Runtime C symbol building a heap `String` from a byte slice. Used to
/// promote a string literal into a heap `String` value.
pub const RUNTIME_STRING_FROM_BYTES: &str = "raven_string_from_bytes";

/// Runtime C symbol returning a heap `String`'s byte buffer pointer.
pub const RUNTIME_STRING_BYTES: &str = "raven_string_bytes";

/// Runtime C symbol returning a heap `String`'s byte length.
pub const RUNTIME_STRING_LEN: &str = "raven_string_len";

/// Runtime C symbol returning the byte at an index (or -1 out of range).
pub const RUNTIME_STRING_BYTE_AT: &str = "raven_string_byte_at";

/// Runtime C symbol returning a clamped half-open byte sub-range.
pub const RUNTIME_STRING_SUBSTRING: &str = "raven_string_substring";

/// Runtime C symbol building a one-byte `String` from a byte value.
pub const RUNTIME_STRING_FROM_BYTE: &str = "raven_string_from_byte";

/// Runtime C symbols backing the interpolation desugaring intrinsics.
/// Each MIR mangled name on the left dispatches to the runtime symbol on
/// the right; see [`crate::mir::intrinsics`].
pub const RUNTIME_STRING_CONCAT: &str = "raven_string_concat";
pub const RUNTIME_INT_TO_STRING: &str = "raven_int_to_string";
pub const RUNTIME_BOOL_TO_STRING: &str = "raven_bool_to_string";
pub const RUNTIME_FLOAT_TO_STRING: &str = "raven_float_to_string";
pub const RUNTIME_CHAR_TO_STRING: &str = "raven_char_to_string";

/// Runtime C symbol comparing two `String` values by content. Backs the
/// `==`/`!=` operators on `String`.
pub const RUNTIME_STRING_EQ: &str = "raven_string_eq";

/// Map a MIR string-runtime intrinsic mangled name to the runtime C
/// symbol it lowers to, or `None` when `mangled` is not one of them.
/// These intrinsics share one call shape: each operand lowers to an
/// ordinary value and the call returns a single result.
pub fn string_runtime_symbol(mangled: &str) -> Option<&'static str> {
    use crate::mir::intrinsics as mir_intr;
    Some(match mangled {
        mir_intr::STR_CONCAT => RUNTIME_STRING_CONCAT,
        mir_intr::STR_EQ => RUNTIME_STRING_EQ,
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

/// Runtime C symbol registering a single root slot. Used to keep a heap
/// temporary that an rvalue builds across further allocations reachable
/// before it is stored into its rooted destination local.
pub const RUNTIME_GC_PUSH_ROOT: &str = "raven_gc_push_root";

/// Runtime C symbol popping the last `n` single root slots.
pub const RUNTIME_GC_POP_ROOTS: &str = "raven_gc_pop_roots";

/// Runtime C symbol opening a per-call defer frame.
pub const RUNTIME_DEFER_ENTER_FRAME: &str = "raven_defer_enter_frame";

/// Runtime C symbol running and popping the current defer frame, called
/// at every function return path.
pub const RUNTIME_DEFER_RUN_FRAME: &str = "raven_defer_run_frame";

/// Runtime C symbol parking a deferred thunk closure on the current
/// defer frame.
pub const RUNTIME_DEFER_PUSH: &str = "raven_defer_push";

/// Runtime C symbol spawning a goroutine from a `fun() -> Unit` closure.
pub const RUNTIME_GO_SPAWN: &str = "raven_go_spawn";

/// Runtime C symbols backing `List<T>` literals, indexing, and methods.
///
/// A list value is a single GC pointer to a heap `List` object. Codegen
/// stores every element in a uniform eight-byte slot (`element_size ==
/// element_align == 8`), the same slot width struct and enum fields use,
/// and sets `elements_are_gc_ptrs` from the static element type so the
/// collector traces pointer elements and leaves scalar buffers opaque.
/// See `docs/v2/specs/codegen.md` and `docs/v2/specs/object-layout.md`.
///
/// `raven_list_new(element_size, element_align, cap, gc_ptrs) -> List`.
pub const RUNTIME_LIST_NEW: &str = "raven_list_new";
/// `raven_list_len(List) -> u32` returns the element count.
pub const RUNTIME_LIST_LEN: &str = "raven_list_len";
/// `raven_list_elements(List) -> ptr` returns the element buffer base.
pub const RUNTIME_LIST_ELEMENTS: &str = "raven_list_elements";
/// `raven_list_push(List, payload_ptr)` appends one eight-byte slot.
pub const RUNTIME_LIST_PUSH: &str = "raven_list_push";
/// `raven_list_pop(List, out_ptr) -> u32` removes the last element into
/// `out_ptr`, returning `1` on success and `0` when the list is empty.
pub const RUNTIME_LIST_POP: &str = "raven_list_pop";
/// `raven_list_get(List, index, out_ptr) -> u32` reads the element at
/// `index` into `out_ptr`, returning `1` on success and `0` when the
/// index is out of range.
pub const RUNTIME_LIST_GET: &str = "raven_list_get";

/// Runtime C symbol reporting a fatal panic and terminating the process.
/// Used by the out-of-bounds index check.
pub const RUNTIME_PANIC: &str = "raven_panic";

/// Runtime C symbol allocating a closure object.
pub const RUNTIME_CLOSURE_NEW: &str = "raven_closure_new";

/// Runtime C symbol returning a closure's function pointer.
pub const RUNTIME_CLOSURE_FN_PTR: &str = "raven_closure_fn_ptr";

/// Runtime C symbol returning a closure's capture buffer.
pub const RUNTIME_CLOSURE_CAPTURES: &str = "raven_closure_captures";

/// Runtime symbol: `raven_ffi_alloc(bytes: usize) -> ptr`. Backs the raw
/// FFI `__ptr_alloc` builtin.
pub const RUNTIME_FFI_ALLOC: &str = "raven_ffi_alloc";
/// Runtime symbol: `raven_ffi_free(p: ptr)`. Backs the `__ptr_free` builtin.
pub const RUNTIME_FFI_FREE: &str = "raven_ffi_free";

/// Runtime reflection symbols. See `docs/v2/specs/runtime-reflection.md`.
///
/// `raven_type_register(type_id, name, is_struct, field_count, field_names,
/// field_type_ids, field_is_gc_ptr)` registers one type's metadata.
pub const RUNTIME_TYPE_REGISTER: &str = "raven_type_register";
/// `raven_any_new(value, type_id, is_gc_ptr) -> Any` boxes a value.
pub const RUNTIME_ANY_NEW: &str = "raven_any_new";
/// `raven_any_type_id(Any) -> u32` reads the boxed runtime type id.
pub const RUNTIME_ANY_TYPE_ID: &str = "raven_any_type_id";
/// `raven_any_payload(Any) -> u64` reads the boxed payload word.
pub const RUNTIME_ANY_PAYLOAD: &str = "raven_any_payload";
/// `raven_any_type_name(Any) -> String` renders the runtime type name.
pub const RUNTIME_ANY_TYPE_NAME: &str = "raven_any_type_name";
/// `raven_any_field_names(Any) -> List<String>` lists struct field names.
pub const RUNTIME_ANY_FIELD_NAMES: &str = "raven_any_field_names";
/// `raven_any_get_field(Any, name) -> Any` reads a struct field by name.
pub const RUNTIME_ANY_GET_FIELD: &str = "raven_any_get_field";

/// True when `mangled` is one of the recognized intrinsics.
pub fn is_intrinsic(mangled: &str) -> bool {
    matches!(
        mangled,
        PRINT
            | IO_PRINT_STR
            | IO_PRINTLN_STR
            | IO_READ_LINE
            | PANIC_FN
            | STR_LEN
            | STR_BYTE_AT
            | STR_SUBSTRING
            | STR_FROM_BYTE
            | STR_CONCAT_FN
            | DEFER_PUSH_FN
            | GO_SPAWN_FN
    )
}
