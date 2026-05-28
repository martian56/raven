//! In-memory layout and constructor for Raven `Set<T>`.
//!
//! See `docs/v2/specs/object-layout.md` for the byte-exact field
//! offsets the back-end relies on. The bucket array is laid out
//! contiguously as `bucket_count` `SetEntry` slots.

use super::{ObjectHeader, OBJECT_ALIGN, TAG_SET};
use crate::{raven_alloc, raven_dealloc};
use std::mem::align_of;
use std::ptr;

/// Boxed `Set<T>`. The header carries `len` (entry count) and `cap`
/// (bucket count, mirroring `bucket_count`); `buckets` owns a buffer
/// of `bucket_count` `SetEntry` slots.
#[repr(C)]
pub struct Set {
    /// Standard 16-byte object header. `tag == TAG_SET`.
    pub header: ObjectHeader,
    /// Power of two, or zero for the freshly-constructed empty set.
    pub bucket_count: u32,
    /// Reserved padding; always zero.
    pub _pad: u32,
    /// Owned buffer of `bucket_count` `SetEntry` slots. Null when
    /// `bucket_count == 0`.
    pub buckets: *mut SetEntry,
}

/// One slot in a `Set`'s bucket array. `element == null` marks an
/// empty or tombstoned slot.
#[repr(C)]
pub struct SetEntry {
    /// Cached hash of the element.
    pub hash: u64,
    /// Pointer to the element payload. Null when the slot is empty or
    /// tombstoned.
    pub element: *mut u8,
}

/// Allocate a fresh `Set` with the given initial bucket count.
///
/// `bucket_count` is rounded up to the next power of two (zero stays
/// zero). The bucket buffer is zero-filled. `header.len = 0`,
/// `header.cap = bucket_count` after rounding.
///
/// Returns null on allocation failure.
#[no_mangle]
pub extern "C" fn raven_set_new(bucket_count: u32) -> *mut Set {
    let rounded = round_up_pow2(bucket_count);
    let set_ptr = raven_alloc(size_of_set(), align_of_set()) as *mut Set;
    if set_ptr.is_null() {
        return ptr::null_mut();
    }
    let buckets = if rounded == 0 {
        ptr::null_mut()
    } else {
        let bytes = (rounded as usize)
            .checked_mul(std::mem::size_of::<SetEntry>())
            .unwrap_or(0);
        if bytes == 0 {
            raven_dealloc(set_ptr as *mut u8, size_of_set(), align_of_set());
            return ptr::null_mut();
        }
        let p = raven_alloc(bytes, align_of::<SetEntry>());
        if p.is_null() {
            raven_dealloc(set_ptr as *mut u8, size_of_set(), align_of_set());
            return ptr::null_mut();
        }
        // SAFETY: the allocator just gave us `bytes` writable bytes.
        unsafe { ptr::write_bytes(p, 0, bytes) };
        p as *mut SetEntry
    };
    // SAFETY: set_ptr points to writable, correctly aligned storage.
    unsafe {
        ptr::write(
            set_ptr,
            Set {
                header: ObjectHeader::new(TAG_SET, 0, rounded),
                bucket_count: rounded,
                _pad: 0,
                buckets,
            },
        );
    }
    set_ptr
}

/// Return the bucket buffer pointer.
///
/// Returns null when `s` is null or has no buckets.
#[no_mangle]
pub extern "C" fn raven_set_buckets(s: *const Set) -> *mut SetEntry {
    if s.is_null() {
        return ptr::null_mut();
    }
    // SAFETY: caller passes a pointer obtained from a constructor.
    unsafe { (*s).buckets }
}

/// Return the bucket count.
///
/// Returns zero when `s` is null.
#[no_mangle]
pub extern "C" fn raven_set_bucket_count(s: *const Set) -> u32 {
    if s.is_null() {
        return 0;
    }
    // SAFETY: caller passes a pointer obtained from a constructor.
    unsafe { (*s).bucket_count }
}

/// Size of the in-memory `Set` object.
pub(crate) const fn size_of_set() -> usize {
    std::mem::size_of::<Set>()
}

/// Alignment of the in-memory `Set` object.
pub(crate) const fn align_of_set() -> usize {
    let a = align_of::<Set>();
    if a > OBJECT_ALIGN {
        a
    } else {
        OBJECT_ALIGN
    }
}

fn round_up_pow2(n: u32) -> u32 {
    if n == 0 {
        0
    } else if n.is_power_of_two() {
        n
    } else {
        n.checked_next_power_of_two().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{align_of, offset_of, size_of};

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn set_size_and_offsets_match_spec() {
        assert_eq!(size_of::<Set>(), 32);
        assert_eq!(offset_of!(Set, header), 0);
        assert_eq!(offset_of!(Set, bucket_count), 16);
        assert_eq!(offset_of!(Set, _pad), 20);
        assert_eq!(offset_of!(Set, buckets), 24);
        assert!(align_of::<Set>() >= 8);
    }

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn set_entry_size_and_offsets_match_spec() {
        assert_eq!(size_of::<SetEntry>(), 16);
        assert_eq!(offset_of!(SetEntry, hash), 0);
        assert_eq!(offset_of!(SetEntry, element), 8);
        assert_eq!(align_of::<SetEntry>(), 8);
    }

    #[test]
    fn new_zero_buckets_leaves_buffer_null() {
        let s = raven_set_new(0);
        assert!(!s.is_null());
        // SAFETY: s came from the constructor.
        unsafe {
            assert_eq!((*s).header.tag, TAG_SET);
            assert_eq!((*s).header.len, 0);
            assert_eq!((*s).header.cap, 0);
            assert_eq!((*s).bucket_count, 0);
            assert!((*s).buckets.is_null());
        }
        unsafe { drop_set_for_test(s) };
    }

    #[test]
    fn new_rounds_up_to_power_of_two() {
        let s = raven_set_new(9);
        assert!(!s.is_null());
        assert_eq!(raven_set_bucket_count(s), 16);
        unsafe { drop_set_for_test(s) };
    }

    #[test]
    fn new_zero_fills_bucket_buffer() {
        let s = raven_set_new(4);
        assert!(!s.is_null());
        let buckets = raven_set_buckets(s);
        assert!(!buckets.is_null());
        // SAFETY: 4 valid SetEntry slots.
        unsafe {
            for i in 0..4 {
                let e = &*buckets.add(i);
                assert_eq!(e.hash, 0);
                assert!(e.element.is_null());
            }
        }
        unsafe { drop_set_for_test(s) };
    }

    #[test]
    fn null_accessors_are_safe() {
        assert!(raven_set_buckets(std::ptr::null()).is_null());
        assert_eq!(raven_set_bucket_count(std::ptr::null()), 0);
    }

    /// Test-only deallocator.
    ///
    /// # Safety
    ///
    /// `s` must come from `raven_set_new` and not be freed yet.
    unsafe fn drop_set_for_test(s: *mut Set) {
        if s.is_null() {
            return;
        }
        // SAFETY: matches construction layout.
        let bucket_count = unsafe { (*s).bucket_count };
        let buckets = unsafe { (*s).buckets };
        if !buckets.is_null() && bucket_count > 0 {
            let bytes = (bucket_count as usize) * std::mem::size_of::<SetEntry>();
            raven_dealloc(buckets as *mut u8, bytes, align_of::<SetEntry>());
        }
        raven_dealloc(s as *mut u8, size_of_set(), align_of_set());
    }
}
