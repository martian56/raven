//! In-memory layout, constructor, and accessor for boxed primitives.
//!
//! A `Box` is the object that lets a primitive value live on the heap,
//! for example as a pointer slot inside a generic `List<T>`. Its
//! payload follows the header inline. See
//! `docs/v2/specs/object-layout.md` for the byte-exact layout.

use super::{ObjectHeader, OBJECT_ALIGN, TAG_BOX};
use crate::raven_alloc;
use std::ptr;

/// Offset, in bytes, from the start of a `Box` to its inline payload.
/// The payload begins immediately after the 16-byte header.
pub const BOX_PAYLOAD_OFFSET: usize = std::mem::size_of::<ObjectHeader>();

/// Boxed primitive object. The struct models only the fixed header;
/// the sized payload follows inline at `BOX_PAYLOAD_OFFSET`. `header.len`
/// is the payload byte size, `header.cap` is 1 (a box holds one value).
///
/// The payload is reached through `raven_box_payload`, not a struct
/// field, because its size is decided at allocation time.
#[repr(C)]
pub struct Box {
    /// Standard 16-byte object header. `tag == TAG_BOX`,
    /// `len == payload size`, `cap == 1`.
    pub header: ObjectHeader,
}

/// Allocate a fresh `Box` whose inline payload is `payload_size` bytes
/// aligned to `payload_align`. The payload is zero-filled.
///
/// The whole object (header plus payload) is one allocation aligned to
/// `max(OBJECT_ALIGN, payload_align)`. Returns null on allocation
/// failure or invalid layout.
#[no_mangle]
pub extern "C" fn raven_box_new(payload_size: u32, payload_align: u32) -> *mut Box {
    if payload_align != 0 && !payload_align.is_power_of_two() {
        return ptr::null_mut();
    }
    let align = box_align(payload_align);
    let total = box_total_size(payload_size);
    let ptr = raven_alloc(total, align) as *mut Box;
    if ptr.is_null() {
        return ptr::null_mut();
    }
    // SAFETY: `ptr` points to `total` writable bytes; zero the whole
    // object, then write the header.
    unsafe {
        ptr::write_bytes(ptr as *mut u8, 0, total);
        ptr::write(
            ptr,
            Box {
                header: ObjectHeader::new(TAG_BOX, payload_size, 1),
            },
        );
    }
    ptr
}

/// Return a pointer to the box's inline payload.
///
/// The payload is valid for `header.len` bytes. Returns null when `b`
/// is null.
#[no_mangle]
pub extern "C" fn raven_box_payload(b: *const Box) -> *mut u8 {
    if b.is_null() {
        return ptr::null_mut();
    }
    // SAFETY: the payload sits at a fixed offset after the header in
    // the same allocation.
    unsafe { (b as *mut u8).add(BOX_PAYLOAD_OFFSET) }
}

/// Total allocation size for a box with the given payload size.
pub(crate) const fn box_total_size(payload_size: u32) -> usize {
    BOX_PAYLOAD_OFFSET + payload_size as usize
}

/// Allocation alignment for a box with the given payload alignment.
pub(crate) const fn box_align(payload_align: u32) -> usize {
    let a = payload_align as usize;
    if a > OBJECT_ALIGN {
        a
    } else {
        OBJECT_ALIGN
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raven_dealloc;
    use std::mem::{offset_of, size_of};

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn box_header_size_and_payload_offset_match_spec() {
        assert_eq!(size_of::<Box>(), 16);
        assert_eq!(offset_of!(Box, header), 0);
        assert_eq!(BOX_PAYLOAD_OFFSET, 16);
    }

    #[test]
    fn new_sets_header_and_zero_fills_payload() {
        let b = raven_box_new(8, 8);
        assert!(!b.is_null());
        // SAFETY: b came from the constructor with an 8-byte payload.
        unsafe {
            assert_eq!((*b).header.tag, TAG_BOX);
            assert_eq!((*b).header.len, 8);
            assert_eq!((*b).header.cap, 1);
        }
        let payload = raven_box_payload(b);
        assert!(!payload.is_null());
        // SAFETY: payload points to 8 zeroed bytes.
        unsafe {
            for i in 0..8 {
                assert_eq!(payload.add(i).read(), 0);
            }
        }
        unsafe { drop_box_for_test(b) };
    }

    #[test]
    fn payload_roundtrips_a_value() {
        let b = raven_box_new(8, 8);
        let payload = raven_box_payload(b);
        // SAFETY: payload points to 8 writable, aligned bytes.
        unsafe {
            (payload as *mut u64).write(0x0102_0304_0506_0708);
            assert_eq!((payload as *const u64).read(), 0x0102_0304_0506_0708);
        }
        unsafe { drop_box_for_test(b) };
    }

    #[test]
    fn zero_sized_payload_is_valid() {
        let b = raven_box_new(0, 0);
        assert!(!b.is_null());
        // SAFETY: b came from the constructor.
        unsafe {
            assert_eq!((*b).header.len, 0);
            assert_eq!((*b).header.cap, 1);
        }
        // Payload pointer is one-past-the-header; valid but not
        // dereferenceable for reads.
        assert!(!raven_box_payload(b).is_null());
        unsafe { drop_box_for_test(b) };
    }

    #[test]
    fn invalid_align_returns_null() {
        assert!(raven_box_new(8, 3).is_null());
    }

    #[test]
    fn null_payload_is_safe() {
        assert!(raven_box_payload(std::ptr::null()).is_null());
    }

    /// Test-only deallocator.
    ///
    /// # Safety
    ///
    /// `b` must come from `raven_box_new` and not be freed yet.
    unsafe fn drop_box_for_test(b: *mut Box) {
        if b.is_null() {
            return;
        }
        // SAFETY: matches construction layout.
        let payload_size = unsafe { (*b).header.len };
        raven_dealloc(
            b as *mut u8,
            box_total_size(payload_size),
            box_align(OBJECT_ALIGN as u32),
        );
    }
}
