//! In-memory layout, constructor, and accessor for boxed primitives.
//!
//! A `Box` is the object that lets a primitive value live on the heap,
//! for example as a pointer slot inside a generic `List<T>`. Its
//! payload follows the header inline. See
//! `docs/v2/specs/object-layout.md` for the byte-exact layout.

use super::{ObjectHeader, OBJECT_ALIGN, TAG_BOX};
use crate::gc::raven_gc_alloc;
use std::ptr;

/// Offset, in bytes, from the start of a `Box` to its inline payload.
/// The payload begins after the 16-byte header and the 8-byte flag word
/// (`payload_is_gc_ptr` plus reserved padding), at offset 24, so the
/// payload stays 8-byte aligned.
pub const BOX_PAYLOAD_OFFSET: usize = std::mem::size_of::<Box>();

/// Boxed primitive or pointer object. The struct models the fixed
/// header and flag word; the sized payload follows inline at
/// `BOX_PAYLOAD_OFFSET`. `header.len` is the payload byte size,
/// `header.cap` is 1 (a box holds one value).
///
/// The payload is reached through `raven_box_payload`, not a struct
/// field, because its size is decided at allocation time.
#[repr(C)]
pub struct Box {
    /// Standard 16-byte object header. `tag == TAG_BOX`,
    /// `len == payload size`, `cap == 1`.
    pub header: ObjectHeader,
    /// Nonzero when the inline payload is a single GC pointer the
    /// collector traces; zero for a scalar payload.
    pub payload_is_gc_ptr: u32,
    /// Reserved padding; keeps the inline payload 8-byte aligned.
    /// Always zero.
    pub _pad: u32,
}

/// Allocate a fresh `Box` whose inline payload is `payload_size` bytes
/// aligned to `payload_align`. The payload is zero-filled.
///
/// `payload_is_gc_ptr` is nonzero when the payload is a single GC
/// pointer the collector traces. The whole object (header, flag word,
/// and payload) is one allocation aligned to
/// `max(OBJECT_ALIGN, payload_align)`. Returns null on allocation
/// failure or invalid layout.
#[no_mangle]
pub extern "C" fn raven_box_new(
    payload_size: u32,
    payload_align: u32,
    payload_is_gc_ptr: u32,
) -> *mut Box {
    if payload_align != 0 && !payload_align.is_power_of_two() {
        return ptr::null_mut();
    }
    let align = box_align(payload_align);
    let total = box_total_size(payload_size);
    let ptr = raven_gc_alloc(total, align, TAG_BOX) as *mut Box;
    if ptr.is_null() {
        return ptr::null_mut();
    }
    // SAFETY: `ptr` points to `total` zeroed bytes from the collector;
    // write the header and flag word, leaving the payload zeroed.
    unsafe {
        ptr::write(
            ptr,
            Box {
                header: ObjectHeader::new(TAG_BOX, payload_size, 1),
                payload_is_gc_ptr,
                _pad: 0,
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
    use std::mem::{offset_of, size_of};

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn box_header_size_and_payload_offset_match_spec() {
        assert_eq!(size_of::<Box>(), 24);
        assert_eq!(offset_of!(Box, header), 0);
        assert_eq!(offset_of!(Box, payload_is_gc_ptr), 16);
        assert_eq!(BOX_PAYLOAD_OFFSET, 24);
    }

    #[test]
    fn new_sets_header_and_zero_fills_payload() {
        let b = raven_box_new(8, 8, 0);
        assert!(!b.is_null());
        // SAFETY: b came from the constructor with an 8-byte payload.
        unsafe {
            assert_eq!((*b).header.tag, TAG_BOX);
            assert_eq!((*b).header.len, 8);
            assert_eq!((*b).header.cap, 1);
            assert_eq!((*b).payload_is_gc_ptr, 0);
            assert_eq!((*b)._pad, 0);
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
    fn new_records_gc_ptr_flag() {
        let b = raven_box_new(8, 8, 1);
        assert!(!b.is_null());
        // SAFETY: b came from the constructor.
        unsafe {
            assert_eq!((*b).payload_is_gc_ptr, 1);
        }
        unsafe { drop_box_for_test(b) };
    }

    #[test]
    fn payload_roundtrips_a_value() {
        let b = raven_box_new(8, 8, 0);
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
        let b = raven_box_new(0, 0, 0);
        assert!(!b.is_null());
        // SAFETY: b came from the constructor.
        unsafe {
            assert_eq!((*b).header.len, 0);
            assert_eq!((*b).header.cap, 1);
        }
        // Payload pointer is one-past-the-body; valid but not
        // dereferenceable for reads.
        assert!(!raven_box_payload(b).is_null());
        unsafe { drop_box_for_test(b) };
    }

    #[test]
    fn invalid_align_returns_null() {
        assert!(raven_box_new(8, 3, 0).is_null());
    }

    #[test]
    fn null_payload_is_safe() {
        assert!(raven_box_payload(std::ptr::null()).is_null());
    }

    /// Test-only deallocator: unregister the object from the collector
    /// and free its body. A box owns no separate buffer.
    ///
    /// # Safety
    ///
    /// `b` must come from `raven_box_new` and not be freed yet.
    unsafe fn drop_box_for_test(b: *mut Box) {
        // SAFETY: matches construction layout.
        unsafe { crate::gc::free_for_test(b as *mut ObjectHeader) };
    }
}
