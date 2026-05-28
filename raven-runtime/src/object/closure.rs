//! In-memory layout, constructor, and accessors for Raven closures.
//!
//! See `docs/v2/specs/object-layout.md` for the byte-exact field
//! offsets the back-end relies on. Captures are stored in a separately
//! allocated, owned buffer pointed to by `captures`, so every closure
//! object is the same fixed size regardless of capture count.

use super::{ObjectHeader, OBJECT_ALIGN, TAG_CLOSURE};
use crate::{raven_alloc, raven_dealloc};
use std::mem::align_of;
use std::ptr;

/// Closure object. The header's `len` is the capture count; `cap` is
/// unused and zero. `fn_ptr` is the lifted body, `captures` owns the
/// capture record of `capture_size` bytes.
#[repr(C)]
pub struct Closure {
    /// Standard 16-byte object header. `tag == TAG_CLOSURE`,
    /// `len == capture count`.
    pub header: ObjectHeader,
    /// Raw code pointer of the lifted closure body.
    pub fn_ptr: *const u8,
    /// Owned buffer of `capture_size` bytes. Null when
    /// `capture_size == 0`.
    pub captures: *mut u8,
    /// Size in bytes of the capture record.
    pub capture_size: u32,
    /// Alignment in bytes of the capture record.
    pub capture_align: u32,
}

/// Allocate a fresh `Closure` with the given function pointer and
/// capture record shape. The capture buffer is zero-filled.
///
/// `capture_count` is recorded in `header.len`; `capture_size` and
/// `capture_align` describe the owned capture buffer. Returns null on
/// allocation failure or invalid layout.
#[no_mangle]
pub extern "C" fn raven_closure_new(
    fn_ptr: *const u8,
    capture_size: u32,
    capture_align: u32,
    capture_count: u32,
) -> *mut Closure {
    if capture_align != 0 && !capture_align.is_power_of_two() {
        return ptr::null_mut();
    }
    let closure_ptr = raven_alloc(size_of_closure(), align_of_closure()) as *mut Closure;
    if closure_ptr.is_null() {
        return ptr::null_mut();
    }
    let captures = if capture_size == 0 {
        ptr::null_mut()
    } else {
        let align = if capture_align == 0 {
            1
        } else {
            capture_align as usize
        };
        let p = raven_alloc(capture_size as usize, align);
        if p.is_null() {
            raven_dealloc(
                closure_ptr as *mut u8,
                size_of_closure(),
                align_of_closure(),
            );
            return ptr::null_mut();
        }
        // SAFETY: the allocator just gave us `capture_size` bytes.
        unsafe { ptr::write_bytes(p, 0, capture_size as usize) };
        p
    };
    // SAFETY: closure_ptr points to writable, correctly aligned storage.
    unsafe {
        ptr::write(
            closure_ptr,
            Closure {
                header: ObjectHeader::new(TAG_CLOSURE, capture_count, 0),
                fn_ptr,
                captures,
                capture_size,
                capture_align,
            },
        );
    }
    closure_ptr
}

/// Return the closure's function pointer.
///
/// Returns null when `c` is null.
#[no_mangle]
pub extern "C" fn raven_closure_fn_ptr(c: *const Closure) -> *const u8 {
    if c.is_null() {
        return ptr::null();
    }
    // SAFETY: caller passes a pointer obtained from a constructor.
    unsafe { (*c).fn_ptr }
}

/// Return a pointer to the closure's capture buffer.
///
/// Returns null when `c` is null or has no captures.
#[no_mangle]
pub extern "C" fn raven_closure_captures(c: *const Closure) -> *mut u8 {
    if c.is_null() {
        return ptr::null_mut();
    }
    // SAFETY: caller passes a pointer obtained from a constructor.
    unsafe { (*c).captures }
}

/// Size of the in-memory `Closure` object.
pub(crate) const fn size_of_closure() -> usize {
    std::mem::size_of::<Closure>()
}

/// Alignment of the in-memory `Closure` object.
pub(crate) const fn align_of_closure() -> usize {
    let a = align_of::<Closure>();
    if a > OBJECT_ALIGN {
        a
    } else {
        OBJECT_ALIGN
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{align_of, offset_of, size_of};

    extern "C" fn dummy_body() {}

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn closure_size_and_offsets_match_spec() {
        assert_eq!(size_of::<Closure>(), 40);
        assert_eq!(offset_of!(Closure, header), 0);
        assert_eq!(offset_of!(Closure, fn_ptr), 16);
        assert_eq!(offset_of!(Closure, captures), 24);
        assert_eq!(offset_of!(Closure, capture_size), 32);
        assert_eq!(offset_of!(Closure, capture_align), 36);
        assert!(align_of::<Closure>() >= 8);
    }

    #[test]
    fn new_no_captures_leaves_buffer_null() {
        let fp = dummy_body as *const u8;
        let c = raven_closure_new(fp, 0, 0, 0);
        assert!(!c.is_null());
        // SAFETY: c came from the constructor.
        unsafe {
            assert_eq!((*c).header.tag, TAG_CLOSURE);
            assert_eq!((*c).header.len, 0);
            assert_eq!((*c).capture_size, 0);
            assert!((*c).captures.is_null());
            assert_eq!((*c).fn_ptr, fp);
        }
        unsafe { drop_closure_for_test(c) };
    }

    #[test]
    fn new_with_captures_zero_fills_buffer() {
        let fp = dummy_body as *const u8;
        let c = raven_closure_new(fp, 24, 8, 3);
        assert!(!c.is_null());
        // SAFETY: c has a 24-byte capture buffer and len 3.
        unsafe {
            assert_eq!((*c).header.len, 3);
            assert_eq!((*c).capture_size, 24);
            assert_eq!((*c).capture_align, 8);
            let buf = (*c).captures;
            assert!(!buf.is_null());
            for i in 0..24 {
                assert_eq!(buf.add(i).read(), 0);
            }
        }
        unsafe { drop_closure_for_test(c) };
    }

    #[test]
    fn accessors_match_fields() {
        let fp = dummy_body as *const u8;
        let c = raven_closure_new(fp, 16, 8, 2);
        assert_eq!(raven_closure_fn_ptr(c), fp);
        let captures = raven_closure_captures(c);
        assert!(!captures.is_null());
        // Write a value through the captures pointer and read it back.
        // SAFETY: captures points to 16 writable bytes.
        unsafe {
            (captures as *mut u64).write(0xABCD);
            assert_eq!((captures as *const u64).read(), 0xABCD);
        }
        unsafe { drop_closure_for_test(c) };
    }

    #[test]
    fn null_accessors_are_safe() {
        assert!(raven_closure_fn_ptr(std::ptr::null()).is_null());
        assert!(raven_closure_captures(std::ptr::null()).is_null());
    }

    /// Test-only deallocator.
    ///
    /// # Safety
    ///
    /// `c` must come from `raven_closure_new` and not be freed yet.
    unsafe fn drop_closure_for_test(c: *mut Closure) {
        if c.is_null() {
            return;
        }
        // SAFETY: matches construction layout.
        let capture_size = unsafe { (*c).capture_size };
        let capture_align = unsafe { (*c).capture_align };
        let captures = unsafe { (*c).captures };
        if !captures.is_null() && capture_size > 0 {
            let align = if capture_align == 0 {
                1
            } else {
                capture_align as usize
            };
            raven_dealloc(captures, capture_size as usize, align);
        }
        raven_dealloc(c as *mut u8, size_of_closure(), align_of_closure());
    }
}
