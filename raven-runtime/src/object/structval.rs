//! In-memory layout and constructor for Raven struct values.
//!
//! A struct value is a heap object: the standard 16-byte `ObjectHeader`
//! followed by the struct's fields, each occupying one 8-byte slot in
//! declaration order. Storing every field in a uniform 8-byte slot keeps
//! the back-end layout trivial: a primitive (`Int`, `Float`) fits a slot
//! exactly, a smaller scalar (`Bool`, `Char`) is widened into one, and a
//! heap value is a single pointer.
//!
//! The collector cannot infer a struct's pointer fields from the tag
//! alone, because two structs with the same tag have different field
//! shapes. The back-end therefore registers a per-type descriptor (a
//! bitmask of which slots hold GC pointers) keyed by a small integer
//! type id, and stores that id in `header.cap`. See
//! `docs/v2/specs/object-layout.md` and `docs/v2/specs/gc.md`.

use super::{ObjectHeader, OBJECT_ALIGN, TAG_STRUCT};
use crate::gc::raven_gc_alloc;
use std::ptr;

/// Offset, in bytes, from the start of a struct value to its first
/// field slot. The fields begin immediately after the 16-byte header.
pub const STRUCT_FIELDS_OFFSET: usize = std::mem::size_of::<ObjectHeader>();

/// Width, in bytes, of one struct field slot. Every field, whether a
/// scalar or a GC pointer, occupies one slot.
pub const STRUCT_FIELD_SLOT: usize = 8;

/// Allocate a fresh struct value with `field_count` zero-filled field
/// slots, tagged with the per-type descriptor `type_id`.
///
/// The whole object (header plus field slots) is one allocation aligned
/// to `OBJECT_ALIGN`. The body is zero-filled, so a collection triggered
/// before the back-end stores the fields never follows a stale pointer.
/// Returns null on allocation failure.
///
/// `type_id` must already have been registered with
/// `raven_struct_register` so the collector can trace the value.
#[no_mangle]
pub extern "C" fn raven_struct_new(field_count: u32, type_id: u32) -> *mut ObjectHeader {
    let total = struct_total_size(field_count);
    let ptr = raven_gc_alloc(total, OBJECT_ALIGN, TAG_STRUCT) as *mut ObjectHeader;
    if ptr.is_null() {
        return ptr::null_mut();
    }
    // SAFETY: `ptr` points to `total` zeroed bytes from the collector;
    // write the header, leaving the field slots zeroed.
    unsafe {
        ptr::write(ptr, ObjectHeader::new(TAG_STRUCT, field_count, type_id));
    }
    ptr
}

/// Return a pointer to the struct's field slots.
///
/// The field area is valid for `header.len` eight-byte slots. Returns
/// null when `s` is null.
#[no_mangle]
pub extern "C" fn raven_struct_fields(s: *const ObjectHeader) -> *mut u8 {
    if s.is_null() {
        return ptr::null_mut();
    }
    // SAFETY: the field slots sit at a fixed offset after the header in
    // the same allocation.
    unsafe { (s as *mut u8).add(STRUCT_FIELDS_OFFSET) }
}

/// Total allocation size for a struct value with `field_count` fields.
pub(crate) const fn struct_total_size(field_count: u32) -> usize {
    STRUCT_FIELDS_OFFSET + field_count as usize * STRUCT_FIELD_SLOT
}

/// Allocation alignment for a struct value.
pub(crate) const fn struct_align() -> usize {
    OBJECT_ALIGN
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gc::raven_struct_register;
    use std::mem::size_of;

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn struct_offsets_match_spec() {
        assert_eq!(STRUCT_FIELDS_OFFSET, 16);
        assert_eq!(STRUCT_FIELD_SLOT, 8);
        assert_eq!(struct_total_size(0), 16);
        assert_eq!(struct_total_size(2), 32);
        assert_eq!(struct_align(), OBJECT_ALIGN);
    }

    #[test]
    fn new_sets_header_and_zero_fills_fields() {
        std::thread::spawn(|| {
            // type 0 has no pointer fields.
            raven_struct_register(0, 0);
            let s = raven_struct_new(2, 0);
            assert!(!s.is_null());
            // SAFETY: s came from the constructor with two field slots.
            unsafe {
                assert_eq!((*s).tag, TAG_STRUCT);
                assert_eq!((*s).len, 2);
                assert_eq!((*s).cap, 0);
            }
            let fields = raven_struct_fields(s) as *const u64;
            // SAFETY: two zeroed slots follow the header.
            unsafe {
                assert_eq!(fields.add(0).read(), 0);
                assert_eq!(fields.add(1).read(), 0);
            }
            assert_eq!(size_of::<ObjectHeader>(), STRUCT_FIELDS_OFFSET);
        })
        .join()
        .unwrap();
    }

    #[test]
    fn fields_roundtrip_values() {
        std::thread::spawn(|| {
            raven_struct_register(0, 0);
            let s = raven_struct_new(2, 0);
            let fields = raven_struct_fields(s) as *mut u64;
            // SAFETY: two writable, aligned slots.
            unsafe {
                fields.add(0).write(3);
                fields.add(1).write(4);
                assert_eq!(fields.add(0).read(), 3);
                assert_eq!(fields.add(1).read(), 4);
            }
        })
        .join()
        .unwrap();
    }

    #[test]
    fn null_fields_is_safe() {
        assert!(raven_struct_fields(std::ptr::null()).is_null());
    }
}
