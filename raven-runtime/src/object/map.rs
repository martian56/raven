//! In-memory layout and constructor for Raven `Map<K, V>`.
//!
//! See `docs/v2/specs/object-layout.md` for the byte-exact field
//! offsets the back-end relies on. The bucket array is laid out
//! contiguously as `bucket_count` `MapEntry` slots.

use super::{ObjectHeader, OBJECT_ALIGN, TAG_MAP};
use crate::{raven_alloc, raven_dealloc};
use std::mem::align_of;
use std::ptr;

/// Boxed `Map<K, V>`. The header carries `len` (entry count) and
/// `cap` (bucket count, mirroring `bucket_count`); `buckets` owns a
/// buffer of `bucket_count` `MapEntry` slots.
#[repr(C)]
pub struct Map {
    /// Standard 16-byte object header. `tag == TAG_MAP`.
    pub header: ObjectHeader,
    /// Power of two, or zero for the freshly-constructed empty map.
    pub bucket_count: u32,
    /// Reserved padding; always zero.
    pub _pad: u32,
    /// Owned buffer of `bucket_count` `MapEntry` slots. Null when
    /// `bucket_count == 0`.
    pub buckets: *mut MapEntry,
}

/// One slot in a `Map`'s bucket array. `key == null` marks an empty
/// or tombstoned slot.
#[repr(C)]
pub struct MapEntry {
    /// Cached hash of the key. Used to skip key comparisons on lookup
    /// and to seed the new bucket index on resize.
    pub hash: u64,
    /// Pointer to the key payload. Null when the slot is empty or
    /// tombstoned.
    pub key: *mut u8,
    /// Pointer to the value payload.
    pub value: *mut u8,
}

/// Allocate a fresh `Map` with the given initial bucket count.
///
/// `bucket_count` is rounded up to the next power of two (zero stays
/// zero). The bucket buffer is zero-filled, leaving every slot in the
/// "empty" state. `header.len = 0`, `header.cap = bucket_count` after
/// rounding.
///
/// Returns null on allocation failure.
#[no_mangle]
pub extern "C" fn raven_map_new(bucket_count: u32) -> *mut Map {
    let rounded = round_up_pow2(bucket_count);
    let map_ptr = raven_alloc(size_of_map(), align_of_map()) as *mut Map;
    if map_ptr.is_null() {
        return ptr::null_mut();
    }
    let buckets = if rounded == 0 {
        ptr::null_mut()
    } else {
        let bytes = (rounded as usize)
            .checked_mul(std::mem::size_of::<MapEntry>())
            .unwrap_or(0);
        if bytes == 0 {
            raven_dealloc(map_ptr as *mut u8, size_of_map(), align_of_map());
            return ptr::null_mut();
        }
        let p = raven_alloc(bytes, align_of::<MapEntry>());
        if p.is_null() {
            raven_dealloc(map_ptr as *mut u8, size_of_map(), align_of_map());
            return ptr::null_mut();
        }
        // SAFETY: the allocator just gave us `bytes` writable bytes.
        unsafe { ptr::write_bytes(p, 0, bytes) };
        p as *mut MapEntry
    };
    // SAFETY: map_ptr points to writable, correctly aligned storage.
    unsafe {
        ptr::write(
            map_ptr,
            Map {
                header: ObjectHeader::new(TAG_MAP, 0, rounded),
                bucket_count: rounded,
                _pad: 0,
                buckets,
            },
        );
    }
    map_ptr
}

/// Return the bucket buffer pointer.
///
/// Returns null when `m` is null or has no buckets.
#[no_mangle]
pub extern "C" fn raven_map_buckets(m: *const Map) -> *mut MapEntry {
    if m.is_null() {
        return ptr::null_mut();
    }
    // SAFETY: caller passes a pointer obtained from a constructor.
    unsafe { (*m).buckets }
}

/// Return the bucket count.
///
/// Returns zero when `m` is null.
#[no_mangle]
pub extern "C" fn raven_map_bucket_count(m: *const Map) -> u32 {
    if m.is_null() {
        return 0;
    }
    // SAFETY: caller passes a pointer obtained from a constructor.
    unsafe { (*m).bucket_count }
}

/// Size of the in-memory `Map` object.
pub(crate) const fn size_of_map() -> usize {
    std::mem::size_of::<Map>()
}

/// Alignment of the in-memory `Map` object.
pub(crate) const fn align_of_map() -> usize {
    let a = align_of::<Map>();
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
    fn map_size_and_offsets_match_spec() {
        assert_eq!(size_of::<Map>(), 32);
        assert_eq!(offset_of!(Map, header), 0);
        assert_eq!(offset_of!(Map, bucket_count), 16);
        assert_eq!(offset_of!(Map, _pad), 20);
        assert_eq!(offset_of!(Map, buckets), 24);
        assert!(align_of::<Map>() >= 8);
    }

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn map_entry_size_and_offsets_match_spec() {
        assert_eq!(size_of::<MapEntry>(), 24);
        assert_eq!(offset_of!(MapEntry, hash), 0);
        assert_eq!(offset_of!(MapEntry, key), 8);
        assert_eq!(offset_of!(MapEntry, value), 16);
        assert_eq!(align_of::<MapEntry>(), 8);
    }

    #[test]
    fn new_zero_buckets_leaves_buffer_null() {
        let m = raven_map_new(0);
        assert!(!m.is_null());
        // SAFETY: m came from the constructor.
        unsafe {
            assert_eq!((*m).header.tag, TAG_MAP);
            assert_eq!((*m).header.len, 0);
            assert_eq!((*m).header.cap, 0);
            assert_eq!((*m).bucket_count, 0);
            assert_eq!((*m)._pad, 0);
            assert!((*m).buckets.is_null());
        }
        unsafe { drop_map_for_test(m) };
    }

    #[test]
    fn new_rounds_up_to_power_of_two() {
        let m = raven_map_new(5);
        assert!(!m.is_null());
        assert_eq!(raven_map_bucket_count(m), 8);
        // SAFETY: m came from the constructor.
        unsafe {
            assert_eq!((*m).header.cap, 8);
        }
        unsafe { drop_map_for_test(m) };
    }

    #[test]
    fn new_zero_fills_bucket_buffer() {
        let m = raven_map_new(4);
        assert!(!m.is_null());
        let buckets = raven_map_buckets(m);
        assert!(!buckets.is_null());
        // SAFETY: 4 valid MapEntry slots.
        unsafe {
            for i in 0..4 {
                let e = &*buckets.add(i);
                assert_eq!(e.hash, 0);
                assert!(e.key.is_null());
                assert!(e.value.is_null());
            }
        }
        unsafe { drop_map_for_test(m) };
    }

    #[test]
    fn null_accessors_are_safe() {
        assert!(raven_map_buckets(std::ptr::null()).is_null());
        assert_eq!(raven_map_bucket_count(std::ptr::null()), 0);
    }

    /// Test-only deallocator.
    ///
    /// # Safety
    ///
    /// `m` must come from `raven_map_new` and not be freed yet.
    unsafe fn drop_map_for_test(m: *mut Map) {
        if m.is_null() {
            return;
        }
        // SAFETY: matches construction layout.
        let bucket_count = unsafe { (*m).bucket_count };
        let buckets = unsafe { (*m).buckets };
        if !buckets.is_null() && bucket_count > 0 {
            let bytes = (bucket_count as usize) * std::mem::size_of::<MapEntry>();
            raven_dealloc(buckets as *mut u8, bytes, align_of::<MapEntry>());
        }
        raven_dealloc(m as *mut u8, size_of_map(), align_of_map());
    }
}
