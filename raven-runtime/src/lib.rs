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

pub mod object;

pub use object::{
    ObjectHeader, OBJECT_ALIGN, TAG_BOX, TAG_CLOSURE, TAG_LIST, TAG_MAP, TAG_SET, TAG_STRING,
};

use std::alloc::{self, Layout};
use std::io::{self, Write};
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{align_of, size_of};

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
        let tags = [TAG_STRING, TAG_LIST, TAG_MAP, TAG_SET, TAG_CLOSURE, TAG_BOX];
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
