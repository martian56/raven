//! Runtime support crate for compiled Raven v2 programs.
//!
//! Compiled v2 binaries link against this crate as a `staticlib`. The
//! exported `extern "C"` symbols below form the entire ABI surface the
//! back-end is allowed to call. See `docs/v2/specs/runtime.md` for the
//! full contract and what is intentionally deferred.

#![deny(unsafe_op_in_unsafe_fn)]
// The ABI surface is `extern "C"` and is called from machine code
// emitted by the back-end. The safety contract for each pointer
// argument is documented on the function itself; the symbols are not
// marked `unsafe` because the back-end emits direct call instructions
// and an `unsafe` qualifier would only matter for Rust callers.
#![allow(clippy::not_unsafe_ptr_arg_deref)]
#![allow(clippy::missing_safety_doc)]

pub mod gc;
pub mod object;

pub use gc::{
    raven_gc_alloc, raven_gc_bytes_allocated, raven_gc_collect, raven_gc_enter_frame,
    raven_gc_leave_frame, raven_gc_live_objects, raven_gc_pop_roots, raven_gc_push_root,
    raven_struct_register,
};
pub use object::{
    raven_bool_to_string, raven_box_new, raven_box_payload, raven_char_to_string,
    raven_closure_captures, raven_closure_fn_ptr, raven_closure_new, raven_float_to_string,
    raven_int_to_string, raven_list_elements, raven_list_len, raven_list_new, raven_list_push,
    raven_map_bucket_count, raven_map_buckets, raven_map_new, raven_set_bucket_count,
    raven_set_buckets, raven_set_new, raven_string_byte_at, raven_string_bytes,
    raven_string_concat, raven_string_eq, raven_string_from_byte, raven_string_from_bytes,
    raven_string_len, raven_string_new, raven_string_substring, raven_struct_fields,
    raven_struct_new, Box as RavenBox, Closure as RavenClosure, List as RavenList, Map as RavenMap,
    MapEntry, ObjectHeader, Set as RavenSet, SetEntry, String as RavenString, GC_MARK_BIT,
    OBJECT_ALIGN, TAG_BOX, TAG_CLOSURE, TAG_LIST, TAG_MAP, TAG_SET, TAG_STRING, TAG_STRUCT,
};

use std::alloc::{self, Layout};
use std::io::{self, BufRead, Write};
use std::process;
use std::slice;

/// Allocate `size` bytes aligned to `align`.
///
/// Returns null on allocation failure or invalid layout. The current
/// implementation is a thin `std::alloc::alloc` passthrough; the real
/// allocator lands with issue #64.
///
/// # Safety
///
/// The caller must pair every non-null return with exactly one
/// `raven_dealloc` using the same `size` and `align`.
#[no_mangle]
pub extern "C" fn raven_alloc(size: usize, align: usize) -> *mut u8 {
    let Ok(layout) = Layout::from_size_align(size, align) else {
        return std::ptr::null_mut();
    };
    if layout.size() == 0 {
        // A zero-sized allocation is well defined to return a non-null
        // dangling pointer. `std::alloc::alloc` is UB at size zero.
        return align as *mut u8;
    }
    // SAFETY: layout has a non-zero size, validated above.
    unsafe { alloc::alloc(layout) }
}

/// Free an allocation previously returned by `raven_alloc`.
///
/// A null pointer or zero-sized allocation is a no-op.
///
/// # Safety
///
/// `ptr` must come from a matching `raven_alloc(size, align)` call,
/// and must not have been freed already.
#[no_mangle]
pub extern "C" fn raven_dealloc(ptr: *mut u8, size: usize, align: usize) {
    if ptr.is_null() {
        return;
    }
    let Ok(layout) = Layout::from_size_align(size, align) else {
        return;
    };
    if layout.size() == 0 {
        return;
    }
    // SAFETY: per the contract, `ptr` matches a previous allocation
    // with this exact layout.
    unsafe { alloc::dealloc(ptr, layout) };
}

/// Report a fatal Raven panic and terminate the process.
///
/// Writes `raven panic: <msg>\n` to standard error and exits with
/// status 101 (the same code Rust uses for unwinding panics that hit
/// `abort`). Does not unwind.
///
/// # Safety
///
/// `msg_ptr` must point to `msg_len` initialized bytes of valid UTF-8.
/// `msg_len` may be zero, in which case `msg_ptr` is not dereferenced.
#[no_mangle]
pub extern "C" fn raven_panic(msg_ptr: *const u8, msg_len: usize) -> ! {
    let msg = if msg_len == 0 || msg_ptr.is_null() {
        ""
    } else {
        // SAFETY: caller guarantees the slice is initialized UTF-8.
        let bytes = unsafe { slice::from_raw_parts(msg_ptr, msg_len) };
        std::str::from_utf8(bytes).unwrap_or("<invalid utf-8>")
    };
    let stderr = io::stderr();
    let mut handle = stderr.lock();
    // Best-effort write; we are about to exit either way.
    let _ = writeln!(handle, "raven panic: {msg}");
    let _ = handle.flush();
    process::exit(101);
}

/// Write a UTF-8 byte slice to standard output without a trailing
/// newline.
///
/// # Safety
///
/// `ptr` must point to `len` initialized UTF-8 bytes, or `len` must be
/// zero.
#[no_mangle]
pub extern "C" fn raven_print_str(ptr: *const u8, len: usize) {
    if len == 0 || ptr.is_null() {
        return;
    }
    // SAFETY: caller guarantees the slice is initialized.
    let bytes = unsafe { slice::from_raw_parts(ptr, len) };
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    let _ = handle.write_all(bytes);
}

/// Write a UTF-8 byte slice to standard output followed by a single
/// `\n`.
///
/// # Safety
///
/// `ptr` must point to `len` initialized UTF-8 bytes, or `len` must be
/// zero.
#[no_mangle]
pub extern "C" fn raven_println_str(ptr: *const u8, len: usize) {
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    if len > 0 && !ptr.is_null() {
        // SAFETY: caller guarantees the slice is initialized.
        let bytes = unsafe { slice::from_raw_parts(ptr, len) };
        let _ = handle.write_all(bytes);
    }
    let _ = handle.write_all(b"\n");
}

/// Read one line from standard input and return it as a heap `String`.
///
/// The trailing line terminator is stripped: a final `\n` is dropped,
/// and a preceding `\r` (Windows `\r\n`) is dropped with it. At end of
/// input (no bytes read) an empty `String` is returned, so a caller can
/// always treat the result as a valid `String` pointer.
///
/// Returns null only when the underlying `String` allocation fails.
#[no_mangle]
pub extern "C" fn raven_read_line() -> *mut object::String {
    let mut line = std::string::String::new();
    let stdin = io::stdin();
    // A read error or clean EOF both leave `line` as the bytes gathered
    // so far (empty at a clean EOF); either way we hand back a String.
    let _ = stdin.lock().read_line(&mut line);
    // Strip the trailing newline and an optional preceding carriage
    // return so callers see the line content without the terminator.
    if line.ends_with('\n') {
        line.pop();
        if line.ends_with('\r') {
            line.pop();
        }
    }
    object::raven_string_from_bytes(line.as_ptr(), line.len())
}

/// Write a signed 64-bit integer to standard output in base ten,
/// followed by a single `\n`.
///
/// This is the integer companion of `raven_println_str`. The back-end
/// wires the built-in `print_int(Int)` free function to this symbol so a
/// program can observe a computed integer without a string conversion.
#[no_mangle]
pub extern "C" fn raven_println_int(value: i64) {
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    // Format into a small stack buffer to avoid a heap allocation.
    let mut buf = itoa_buf();
    let s = format_i64(value, &mut buf);
    let _ = handle.write_all(s.as_bytes());
    let _ = handle.write_all(b"\n");
}

/// Reinterpret a signed 64-bit integer as an `f64` by value conversion.
///
/// The v2 surface language has no Int-to-Float cast, so `std/random`
/// binds this symbol through `extern "C"` to build a `Float` in
/// `[0.0, 1.0)` from generated bits.
#[no_mangle]
pub extern "C" fn raven_int_to_float(value: i64) -> f64 {
    value as f64
}

/// Return a non-deterministic 64-bit seed for entropy seeding.
///
/// Mixes a high-resolution timestamp with the process id through a
/// splitmix64 finalizer so distinct calls differ. This is a seed source,
/// not a cryptographic random generator.
#[no_mangle]
pub extern "C" fn raven_random_entropy() -> i64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let mut z = nanos ^ (u64::from(process::id()) << 32);
    z = z.wrapping_add(0x9E3779B97F4A7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z = z ^ (z >> 31);
    z as i64
}

/// A stack buffer large enough for any base-ten `i64` plus a sign.
fn itoa_buf() -> [u8; 20] {
    [0u8; 20]
}

/// Format `value` into `buf` and return the written slice as a string.
/// Twenty bytes hold the widest `i64` (`-9223372036854775808`).
fn format_i64(value: i64, buf: &mut [u8; 20]) -> &str {
    // Work with the unsigned magnitude to handle i64::MIN safely.
    let negative = value < 0;
    let mut magnitude = value.unsigned_abs();
    let mut pos = buf.len();
    loop {
        pos -= 1;
        buf[pos] = b'0' + (magnitude % 10) as u8;
        magnitude /= 10;
        if magnitude == 0 {
            break;
        }
    }
    if negative {
        pos -= 1;
        buf[pos] = b'-';
    }
    // SAFETY: the bytes written are ASCII digits and an optional '-'.
    unsafe { std::str::from_utf8_unchecked(&buf[pos..]) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{align_of, size_of};

    #[test]
    fn format_i64_handles_edges() {
        let mut buf = itoa_buf();
        assert_eq!(format_i64(0, &mut buf), "0");
        let mut buf = itoa_buf();
        assert_eq!(format_i64(7, &mut buf), "7");
        let mut buf = itoa_buf();
        assert_eq!(format_i64(-7, &mut buf), "-7");
        let mut buf = itoa_buf();
        assert_eq!(format_i64(i64::MAX, &mut buf), "9223372036854775807");
        let mut buf = itoa_buf();
        assert_eq!(format_i64(i64::MIN, &mut buf), "-9223372036854775808");
    }

    #[test]
    fn object_header_is_sixteen_bytes() {
        assert_eq!(size_of::<ObjectHeader>(), 16);
    }

    #[test]
    fn object_header_alignment_divides_object_align() {
        assert!(OBJECT_ALIGN.is_power_of_two());
        assert_eq!(OBJECT_ALIGN % align_of::<ObjectHeader>(), 0);
    }

    #[test]
    fn object_header_new_zeroes_gc_bits() {
        let h = ObjectHeader::new(TAG_STRING, 5, 8);
        assert_eq!(h.tag, TAG_STRING);
        assert_eq!(h.gc_bits, 0);
        assert_eq!(h.len, 5);
        assert_eq!(h.cap, 8);
    }

    #[test]
    fn tag_constants_are_distinct_and_dense() {
        let tags = [
            TAG_STRING,
            TAG_LIST,
            TAG_MAP,
            TAG_SET,
            TAG_CLOSURE,
            TAG_BOX,
            TAG_STRUCT,
        ];
        for (i, t) in tags.iter().enumerate() {
            assert_eq!(*t as usize, i + 1, "tag {i} should be {}", i + 1);
        }
    }

    #[test]
    fn alloc_dealloc_roundtrip_succeeds() {
        let size = 64;
        let align = OBJECT_ALIGN;
        let ptr = raven_alloc(size, align);
        assert!(!ptr.is_null(), "raven_alloc returned null");
        // Touch the memory so a miscompile or layout bug would show up
        // under sanitizers.
        unsafe {
            std::ptr::write_bytes(ptr, 0xAB, size);
        }
        raven_dealloc(ptr, size, align);
    }

    #[test]
    fn alloc_with_invalid_layout_returns_null() {
        // align is not a power of two: invalid layout, must not abort.
        let ptr = raven_alloc(8, 3);
        assert!(ptr.is_null());
    }

    #[test]
    fn dealloc_null_is_noop() {
        raven_dealloc(std::ptr::null_mut(), 16, OBJECT_ALIGN);
    }

    #[test]
    fn print_str_accepts_empty_slice() {
        // Empty slices must not dereference the pointer.
        raven_print_str(std::ptr::null(), 0);
        raven_println_str(std::ptr::null(), 0);
    }
}
