//! In-memory layout, constructor, and accessors for Raven `List<T>`.
//!
//! See `docs/v2/specs/object-layout.md` for the byte-exact field
//! offsets the back-end relies on.

use super::{ObjectHeader, OBJECT_ALIGN, TAG_LIST};
use crate::gc::raven_gc_alloc;
use crate::{raven_alloc, raven_dealloc};
use std::mem::align_of;
use std::ptr;

/// Boxed `List<T>`. The header carries `len` (element count) and
/// `cap` (slot capacity); `element_size`/`element_align` describe one
/// slot; `elements` owns a buffer of `cap * element_size` bytes.
#[repr(C)]
pub struct List {
    /// Standard 16-byte object header. `tag == TAG_LIST`.
    pub header: ObjectHeader,
    /// Size in bytes of one element slot.
    pub element_size: u32,
    /// Alignment in bytes of one element slot.
    pub element_align: u32,
    /// Nonzero when each element slot is a GC pointer the collector must
    /// trace. Zero for scalar elements (the buffer is opaque bytes).
    /// Codegen sets it from the static element type.
    pub elements_are_gc_ptrs: u32,
    /// Reserved padding; keeps `elements` 8-byte aligned. Always zero.
    pub _pad: u32,
    /// Owned buffer of `header.cap * element_size` bytes. Null when
    /// `header.cap == 0`.
    pub elements: *mut u8,
}

/// Allocate a fresh `List` with the given per-element shape and slot
/// capacity. Header is `len = 0`, `cap = cap`. Elements buffer is
/// zero-filled.
///
/// `elements_are_gc_ptrs` is nonzero when each slot holds a GC pointer
/// the collector traces; zero for scalar elements.
///
/// Returns null on allocation failure or invalid layout.
#[no_mangle]
pub extern "C" fn raven_list_new(
    element_size: u32,
    element_align: u32,
    cap: u32,
    elements_are_gc_ptrs: u32,
) -> *mut List {
    if element_align != 0 && !element_align.is_power_of_two() {
        return ptr::null_mut();
    }
    // Allocate the owned buffer first so a body-allocation failure does
    // not leave a half-registered object in the collector.
    let buffer = match alloc_elements(element_size, element_align, cap) {
        Some(p) => p,
        None => return ptr::null_mut(),
    };
    let list_ptr = raven_gc_alloc(size_of_list(), align_of_list(), TAG_LIST) as *mut List;
    if list_ptr.is_null() {
        free_element_buffer(buffer, element_size, element_align, cap);
        return ptr::null_mut();
    }
    // SAFETY: list_ptr points to writable, correctly aligned storage.
    unsafe {
        ptr::write(
            list_ptr,
            List {
                header: ObjectHeader::new(TAG_LIST, 0, cap),
                element_size,
                element_align,
                elements_are_gc_ptrs,
                _pad: 0,
                elements: buffer,
            },
        );
    }
    list_ptr
}

/// Return the current element count, i.e. `header.len`.
///
/// Returns zero when `l` is null.
#[no_mangle]
pub extern "C" fn raven_list_len(l: *const List) -> u32 {
    if l.is_null() {
        return 0;
    }
    // SAFETY: caller passes a pointer obtained from a constructor.
    unsafe { (*l).header.len }
}

/// Return a pointer to the element buffer.
///
/// The buffer holds `raven_list_len(l) * element_size` initialised
/// bytes. Returns null when `l` is null.
#[no_mangle]
pub extern "C" fn raven_list_elements(l: *const List) -> *mut u8 {
    if l.is_null() {
        return ptr::null_mut();
    }
    // SAFETY: caller passes a pointer obtained from a constructor.
    unsafe { (*l).elements }
}

/// Append one element to the list, growing the buffer if needed.
///
/// Copies `element_size` bytes from `payload` into the slot at index
/// `header.len`, then increments `header.len`. The growth policy
/// doubles `cap` each time (starting from 4 when the list is empty).
///
/// No-op when `l` is null or `payload` is null.
#[no_mangle]
pub extern "C" fn raven_list_push(l: *mut List, payload: *const u8) {
    if l.is_null() || payload.is_null() {
        return;
    }
    // SAFETY: caller passes a pointer obtained from a constructor.
    let (len, cap, elem_size, elem_align) = unsafe {
        (
            (*l).header.len,
            (*l).header.cap,
            (*l).element_size,
            (*l).element_align,
        )
    };
    if elem_size == 0 {
        // Zero-sized elements still count for `len` but never copy.
        // SAFETY: bumping a counter on a live header.
        unsafe { (*l).header.len = len.saturating_add(1) };
        return;
    }
    if len == cap {
        let new_cap = if cap == 0 { 4 } else { cap.saturating_mul(2) };
        if !grow_buffer(l, elem_size, elem_align, cap, new_cap) {
            return;
        }
    }
    // SAFETY: after the potential grow the slot at `len` is valid for
    // `elem_size` writable bytes.
    unsafe {
        let dst = (*l).elements.add((len as usize) * (elem_size as usize));
        ptr::copy_nonoverlapping(payload, dst, elem_size as usize);
        (*l).header.len = len + 1;
    }
}

/// Size of the in-memory `List` object.
pub(crate) const fn size_of_list() -> usize {
    std::mem::size_of::<List>()
}

/// Alignment of the in-memory `List` object.
pub(crate) const fn align_of_list() -> usize {
    let a = align_of::<List>();
    if a > OBJECT_ALIGN {
        a
    } else {
        OBJECT_ALIGN
    }
}

/// Free a `List`'s owned element buffer. The collector frees the object
/// body separately after this call.
///
/// # Safety
///
/// `l` must point to a live `List` produced by `raven_list_new`.
pub(crate) unsafe fn free_buffers(l: *mut List) {
    // SAFETY: caller guarantees `l` is a live List.
    let cap = unsafe { (*l).header.cap };
    let elem_size = unsafe { (*l).element_size };
    let elem_align = unsafe { (*l).element_align };
    let buffer = unsafe { (*l).elements };
    if !buffer.is_null() && cap > 0 && elem_size > 0 {
        let bytes = (cap as usize) * (elem_size as usize);
        let align = if elem_align == 0 {
            1
        } else {
            elem_align as usize
        };
        raven_dealloc(buffer, bytes, align);
    }
}

fn alloc_elements(element_size: u32, element_align: u32, cap: u32) -> Option<*mut u8> {
    if cap == 0 || element_size == 0 {
        return Some(ptr::null_mut());
    }
    let bytes = (element_size as usize).checked_mul(cap as usize)?;
    let align = if element_align == 0 {
        1
    } else {
        element_align as usize
    };
    let p = raven_alloc(bytes, align);
    if p.is_null() {
        return None;
    }
    // SAFETY: the allocator just gave us `bytes` writable bytes.
    unsafe { ptr::write_bytes(p, 0, bytes) };
    Some(p)
}

/// Release an element buffer allocated by `alloc_elements`. Used to
/// unwind a partly-built list when the body allocation fails.
fn free_element_buffer(buffer: *mut u8, element_size: u32, element_align: u32, cap: u32) {
    if buffer.is_null() || cap == 0 || element_size == 0 {
        return;
    }
    let bytes = (element_size as usize) * (cap as usize);
    let align = if element_align == 0 {
        1
    } else {
        element_align as usize
    };
    raven_dealloc(buffer, bytes, align);
}

fn grow_buffer(l: *mut List, elem_size: u32, elem_align: u32, old_cap: u32, new_cap: u32) -> bool {
    let new_buffer = match alloc_elements(elem_size, elem_align, new_cap) {
        Some(p) => p,
        None => return false,
    };
    // SAFETY: l is a valid List pointer from a constructor.
    unsafe {
        let old_buffer = (*l).elements;
        if !old_buffer.is_null() && old_cap > 0 {
            let copy_bytes = (old_cap as usize) * (elem_size as usize);
            if !new_buffer.is_null() {
                ptr::copy_nonoverlapping(old_buffer, new_buffer, copy_bytes);
            }
            let align = if elem_align == 0 {
                1
            } else {
                elem_align as usize
            };
            raven_dealloc(old_buffer, (old_cap as usize) * (elem_size as usize), align);
        }
        (*l).elements = new_buffer;
        (*l).header.cap = new_cap;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{align_of, offset_of, size_of};

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn list_size_and_offsets_match_spec() {
        assert_eq!(size_of::<List>(), 40);
        assert_eq!(offset_of!(List, header), 0);
        assert_eq!(offset_of!(List, element_size), 16);
        assert_eq!(offset_of!(List, element_align), 20);
        assert_eq!(offset_of!(List, elements_are_gc_ptrs), 24);
        assert_eq!(offset_of!(List, elements), 32);
        assert!(align_of::<List>() >= 8);
    }

    #[test]
    fn new_zero_capacity_yields_empty_list() {
        let l = raven_list_new(8, 8, 0, 0);
        assert!(!l.is_null());
        // SAFETY: l came from the constructor.
        unsafe {
            assert_eq!((*l).header.tag, TAG_LIST);
            assert_eq!((*l).header.len, 0);
            assert_eq!((*l).header.cap, 0);
            assert_eq!((*l).element_size, 8);
            assert_eq!((*l).element_align, 8);
            assert_eq!((*l).elements_are_gc_ptrs, 0);
            assert_eq!((*l)._pad, 0);
            assert!((*l).elements.is_null());
        }
        unsafe { drop_list_for_test(l) };
    }

    #[test]
    fn new_records_gc_ptr_flag() {
        let l = raven_list_new(8, 8, 4, 1);
        assert!(!l.is_null());
        // SAFETY: l came from the constructor.
        unsafe {
            assert_eq!((*l).elements_are_gc_ptrs, 1);
        }
        unsafe { drop_list_for_test(l) };
    }

    #[test]
    fn new_with_capacity_zero_fills_buffer() {
        let l = raven_list_new(4, 4, 6, 0);
        assert!(!l.is_null());
        // SAFETY: l has a 24-byte buffer.
        unsafe {
            let bytes = 4 * 6;
            for i in 0..bytes {
                assert_eq!((*l).elements.add(i).read(), 0);
            }
        }
        unsafe { drop_list_for_test(l) };
    }

    #[test]
    fn push_appends_and_grows() {
        // Start with cap 0 so we exercise the grow path.
        let l = raven_list_new(8, 8, 0, 0);
        let values: [u64; 10] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        for v in &values {
            // SAFETY: each push reads 8 bytes from a stack-local u64.
            raven_list_push(l, v as *const u64 as *const u8);
        }
        assert_eq!(raven_list_len(l), 10);
        // SAFETY: l has 10 valid u64 slots.
        unsafe {
            let slots = (*l).elements as *const u64;
            for (i, v) in values.iter().enumerate() {
                assert_eq!(*slots.add(i), *v);
            }
            assert!((*l).header.cap >= 10);
        }
        unsafe { drop_list_for_test(l) };
    }

    #[test]
    fn push_preserves_earlier_elements_across_grow() {
        let l = raven_list_new(8, 8, 2, 0);
        let first = 0xDEADBEEFu64;
        let second = 0xCAFEBABEu64;
        raven_list_push(l, &first as *const u64 as *const u8);
        raven_list_push(l, &second as *const u64 as *const u8);
        let third = 0x12345678u64;
        // This push forces a grow.
        raven_list_push(l, &third as *const u64 as *const u8);
        assert_eq!(raven_list_len(l), 3);
        // SAFETY: l has at least three valid u64 slots.
        unsafe {
            let slots = (*l).elements as *const u64;
            assert_eq!(*slots.add(0), first);
            assert_eq!(*slots.add(1), second);
            assert_eq!(*slots.add(2), third);
        }
        unsafe { drop_list_for_test(l) };
    }

    #[test]
    fn null_accessors_are_safe() {
        assert_eq!(raven_list_len(std::ptr::null()), 0);
        assert!(raven_list_elements(std::ptr::null()).is_null());
        // Null inputs must not crash.
        raven_list_push(std::ptr::null_mut(), std::ptr::null());
    }

    /// Test-only deallocator: unregister the object from the collector
    /// and free its buffer and body.
    ///
    /// # Safety
    ///
    /// `l` must come from `raven_list_new` and not be freed yet.
    unsafe fn drop_list_for_test(l: *mut List) {
        // SAFETY: matches the construction layout.
        unsafe { crate::gc::free_for_test(l as *mut ObjectHeader) };
    }
}
