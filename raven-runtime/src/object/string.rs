//! In-memory layout, constructors, and accessors for Raven `String`.
//!
//! See `docs/v2/specs/object-layout.md` for the byte-exact field
//! offsets the back-end relies on.

use super::{ObjectHeader, OBJECT_ALIGN, TAG_STRING};
use crate::{raven_alloc, raven_dealloc};
use std::mem::align_of;
use std::ptr;

/// UTF-8 string object. The header carries `len` (byte count) and
/// `cap` (allocated byte capacity); the `bytes` pointer owns the
/// payload buffer.
#[repr(C)]
pub struct String {
    /// Standard 16-byte object header. `tag == TAG_STRING`.
    pub header: ObjectHeader,
    /// Owned buffer of `header.cap` bytes. The first `header.len`
    /// bytes are valid UTF-8. Null when `header.cap == 0`.
    pub bytes: *mut u8,
}

/// Allocate a fresh `String` with the requested byte capacity.
///
/// The header is initialised with `tag = TAG_STRING`, `len = 0`,
/// `cap = cap`. The byte buffer is zero-filled.
///
/// Returns null when the allocation fails.
#[no_mangle]
pub extern "C" fn raven_string_new(cap: u32) -> *mut String {
    let header_ptr = raven_alloc(size_of_string(), align_of_string()) as *mut String;
    if header_ptr.is_null() {
        return ptr::null_mut();
    }
    let bytes_ptr = if cap == 0 {
        ptr::null_mut()
    } else {
        let p = raven_alloc(cap as usize, 1);
        if p.is_null() {
            raven_dealloc(header_ptr as *mut u8, size_of_string(), align_of_string());
            return ptr::null_mut();
        }
        // SAFETY: the allocator just gave us `cap` writable bytes.
        unsafe { ptr::write_bytes(p, 0, cap as usize) };
        p
    };
    // SAFETY: header_ptr points to writable, correctly aligned storage.
    unsafe {
        ptr::write(
            header_ptr,
            String {
                header: ObjectHeader::new(TAG_STRING, 0, cap),
                bytes: bytes_ptr,
            },
        );
    }
    header_ptr
}

/// Return the byte length of the string, i.e. `header.len`.
///
/// Returns zero when `s` is null.
#[no_mangle]
pub extern "C" fn raven_string_len(s: *const String) -> u32 {
    if s.is_null() {
        return 0;
    }
    // SAFETY: caller passes a pointer obtained from a constructor.
    unsafe { (*s).header.len }
}

/// Return a pointer to the string's UTF-8 byte buffer.
///
/// The buffer is valid for `raven_string_len(s)` bytes. Returns null
/// when `s` is null.
#[no_mangle]
pub extern "C" fn raven_string_bytes(s: *const String) -> *const u8 {
    if s.is_null() {
        return ptr::null();
    }
    // SAFETY: caller passes a pointer obtained from a constructor.
    unsafe { (*s).bytes }
}

/// Concatenate two strings into a freshly allocated string.
///
/// The result has `len = a.len + b.len` and a capacity equal to that
/// length. Either input may be null, in which case it is treated as
/// the empty string.
#[no_mangle]
pub extern "C" fn raven_string_concat(a: *const String, b: *const String) -> *mut String {
    let a_len = raven_string_len(a) as usize;
    let b_len = raven_string_len(b) as usize;
    let total = a_len + b_len;
    let total_u32 = match u32::try_from(total) {
        Ok(v) => v,
        Err(_) => return ptr::null_mut(),
    };
    let out = raven_string_new(total_u32);
    if out.is_null() {
        return ptr::null_mut();
    }
    // SAFETY: out was just constructed with `total_u32` capacity. We
    // copy `a_len` bytes from a's buffer then `b_len` bytes from b's
    // buffer, both bounded by the caller-supplied lengths.
    unsafe {
        let dst = (*out).bytes;
        if a_len > 0 {
            ptr::copy_nonoverlapping(raven_string_bytes(a), dst, a_len);
        }
        if b_len > 0 {
            ptr::copy_nonoverlapping(raven_string_bytes(b), dst.add(a_len), b_len);
        }
        (*out).header.len = total_u32;
    }
    out
}

/// Size, in bytes, of the in-memory `String` object on the host
/// target. Used by constructors and by the GC's free routine in a
/// follow-up.
pub(crate) const fn size_of_string() -> usize {
    std::mem::size_of::<String>()
}

/// Alignment, in bytes, of the in-memory `String` object.
pub(crate) const fn align_of_string() -> usize {
    let a = align_of::<String>();
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

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn string_size_and_offsets_match_spec() {
        assert_eq!(size_of::<String>(), 24);
        assert_eq!(offset_of!(String, header), 0);
        assert_eq!(offset_of!(String, bytes), 16);
        assert!(align_of::<String>() >= 8);
    }

    #[test]
    fn new_zero_capacity_yields_empty_string() {
        let s = raven_string_new(0);
        assert!(!s.is_null());
        // SAFETY: s came from the constructor.
        unsafe {
            assert_eq!((*s).header.tag, TAG_STRING);
            assert_eq!((*s).header.len, 0);
            assert_eq!((*s).header.cap, 0);
            assert!((*s).bytes.is_null());
        }
        // Manual cleanup until the GC arrives.
        unsafe { drop_string_for_test(s) };
    }

    #[test]
    fn new_with_capacity_zero_fills_buffer() {
        let cap = 16u32;
        let s = raven_string_new(cap);
        assert!(!s.is_null());
        // SAFETY: s came from the constructor with `cap` bytes.
        unsafe {
            assert_eq!((*s).header.cap, cap);
            for i in 0..cap as usize {
                assert_eq!((*s).bytes.add(i).read(), 0);
            }
        }
        unsafe { drop_string_for_test(s) };
    }

    #[test]
    fn len_and_bytes_accessors_match_header() {
        let s = raven_string_new(8);
        assert!(!s.is_null());
        // SAFETY: write into the just-allocated buffer.
        unsafe {
            ptr::copy_nonoverlapping(b"hi".as_ptr(), (*s).bytes, 2);
            (*s).header.len = 2;
        }
        assert_eq!(raven_string_len(s), 2);
        let bytes = raven_string_bytes(s);
        assert!(!bytes.is_null());
        // SAFETY: bytes points to two valid bytes "hi".
        let slice = unsafe { std::slice::from_raw_parts(bytes, 2) };
        assert_eq!(slice, b"hi");
        unsafe { drop_string_for_test(s) };
    }

    #[test]
    fn concat_joins_inputs() {
        let a = raven_string_new(5);
        let b = raven_string_new(5);
        assert!(!a.is_null() && !b.is_null());
        // SAFETY: a and b have capacity 5 each.
        unsafe {
            ptr::copy_nonoverlapping(b"hel".as_ptr(), (*a).bytes, 3);
            (*a).header.len = 3;
            ptr::copy_nonoverlapping(b"lo".as_ptr(), (*b).bytes, 2);
            (*b).header.len = 2;
        }
        let joined = raven_string_concat(a, b);
        assert!(!joined.is_null());
        assert_eq!(raven_string_len(joined), 5);
        // SAFETY: joined has len 5.
        let slice = unsafe { std::slice::from_raw_parts(raven_string_bytes(joined), 5) };
        assert_eq!(slice, b"hello");
        unsafe {
            drop_string_for_test(joined);
            drop_string_for_test(a);
            drop_string_for_test(b);
        }
    }

    #[test]
    fn concat_with_null_treats_input_as_empty() {
        let a = raven_string_new(2);
        // SAFETY: a has capacity 2.
        unsafe {
            ptr::copy_nonoverlapping(b"hi".as_ptr(), (*a).bytes, 2);
            (*a).header.len = 2;
        }
        let joined = raven_string_concat(a, std::ptr::null());
        assert!(!joined.is_null());
        assert_eq!(raven_string_len(joined), 2);
        let slice = unsafe { std::slice::from_raw_parts(raven_string_bytes(joined), 2) };
        assert_eq!(slice, b"hi");
        unsafe {
            drop_string_for_test(joined);
            drop_string_for_test(a);
        }
    }

    #[test]
    fn null_accessors_are_safe() {
        assert_eq!(raven_string_len(std::ptr::null()), 0);
        assert!(raven_string_bytes(std::ptr::null()).is_null());
    }

    /// Test-only deallocator. The real free path lands with the GC
    /// (issue #64); for unit tests we want valgrind-clean teardown.
    ///
    /// # Safety
    ///
    /// `s` must come from `raven_string_new` and not be freed yet.
    unsafe fn drop_string_for_test(s: *mut String) {
        if s.is_null() {
            return;
        }
        // SAFETY: matches the construction layout.
        let cap = unsafe { (*s).header.cap };
        let bytes = unsafe { (*s).bytes };
        if !bytes.is_null() && cap > 0 {
            raven_dealloc(bytes, cap as usize, 1);
        }
        raven_dealloc(s as *mut u8, size_of_string(), align_of_string());
    }
}
