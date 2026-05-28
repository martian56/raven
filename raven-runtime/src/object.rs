//! Canonical object header layout shared by every heap-allocated
//! Raven value.
//!
//! The shape pinned here is what the future garbage collector
//! (issue #64) walks, and what the per-kind object layouts
//! (issue #65) prepend to their payload. Keeping it stable across
//! every object kind lets the GC trace without knowing the kind in
//! advance, dispatching on the `tag` field only when it needs the
//! payload shape.
//!
//! See `docs/v2/specs/runtime.md` for the contract and
//! `docs/v2/specs/object-layout.md` for per-kind payload layouts.

pub mod boxed;
pub mod closure;
pub mod hash;
pub mod list;
pub mod map;
pub mod set;
pub mod string;
pub mod structval;

pub use boxed::{raven_box_new, raven_box_payload, Box, BOX_PAYLOAD_OFFSET};
pub use closure::{raven_closure_captures, raven_closure_fn_ptr, raven_closure_new, Closure};
pub use list::{raven_list_elements, raven_list_len, raven_list_new, raven_list_push, List};
pub use map::{raven_map_bucket_count, raven_map_buckets, raven_map_new, Map, MapEntry};
pub use set::{raven_set_bucket_count, raven_set_buckets, raven_set_new, Set, SetEntry};
pub use string::{
    raven_bool_to_string, raven_char_to_string, raven_float_to_string, raven_int_to_string,
    raven_string_byte_at, raven_string_bytes, raven_string_concat, raven_string_from_byte,
    raven_string_from_bytes, raven_string_len, raven_string_new, raven_string_substring, String,
};
pub use structval::{
    raven_struct_fields, raven_struct_new, STRUCT_FIELDS_OFFSET, STRUCT_FIELD_SLOT,
};

/// Alignment, in bytes, used for every heap object the runtime
/// allocates. The header itself is 4-byte aligned, but objects are
/// over-aligned to 8 so payload fields immediately after the header
/// satisfy the alignment of every primitive Raven value (`Int`,
/// `Float`, raw pointers).
pub const OBJECT_ALIGN: usize = 8;

/// UTF-8 string. `len` is the byte length, `cap` is the allocated
/// byte capacity.
pub const TAG_STRING: u32 = 0x01;

/// Boxed `List<T>`. `len` is the element count, `cap` is the allocated
/// element capacity.
pub const TAG_LIST: u32 = 0x02;

/// Boxed `Map<K, V>`. `len` is the entry count, `cap` is the bucket
/// count.
pub const TAG_MAP: u32 = 0x03;

/// Boxed `Set<T>`. `len` is the entry count, `cap` is the bucket
/// count.
pub const TAG_SET: u32 = 0x04;

/// Closure object: a function pointer followed by an inline capture
/// tail.
pub const TAG_CLOSURE: u32 = 0x05;

/// Generic heap box. Reserved for trait-object payloads (issue #66).
pub const TAG_BOX: u32 = 0x06;

/// User-defined struct value. `len` is the field count; `cap` carries
/// the per-type descriptor id the collector looks up to find which field
/// slots hold GC pointers. The fields follow the header, one 8-byte slot
/// each, in declaration order.
pub const TAG_STRUCT: u32 = 0x07;

/// Bit 0 of `ObjectHeader.gc_bits`: the mark bit the tracing collector
/// sets during the mark phase and clears during sweep. The remaining
/// `gc_bits` stay zero and are reserved for a future colour scheme. See
/// `docs/v2/specs/gc.md`.
pub const GC_MARK_BIT: u32 = 0x1;

/// Canonical 16-byte object header.
///
/// Every heap allocation the GC traces starts with one of these,
/// followed by a kind-specific payload. The layout is `#[repr(C)]`
/// and pinned: the compiler back-end and the future GC both depend
/// on the field order and offsets.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjectHeader {
    /// Discriminator picking one of the `TAG_*` constants.
    pub tag: u32,
    /// Mark / colour bits reserved for the future tracing collector.
    /// Always zero in the current scaffold.
    pub gc_bits: u32,
    /// Logical length in the unit appropriate for the tag (bytes for
    /// strings, elements for lists, entries for maps and sets).
    pub len: u32,
    /// Allocated capacity in the same unit as `len`, or zero for
    /// kinds where capacity is not applicable.
    pub cap: u32,
}

impl ObjectHeader {
    /// Construct a header with the given tag and zeroed GC bits.
    pub const fn new(tag: u32, len: u32, cap: u32) -> Self {
        Self {
            tag,
            gc_bits: 0,
            len,
            cap,
        }
    }

    /// Return true when the mark bit is set.
    #[inline]
    pub const fn is_marked(&self) -> bool {
        self.gc_bits & GC_MARK_BIT != 0
    }

    /// Set the mark bit.
    #[inline]
    pub fn set_mark(&mut self) {
        self.gc_bits |= GC_MARK_BIT;
    }

    /// Clear the mark bit.
    #[inline]
    pub fn clear_mark(&mut self) {
        self.gc_bits &= !GC_MARK_BIT;
    }
}

/// Return the allocation size and alignment of an object body given its
/// header. The body is the fixed-size struct that begins with the
/// header; owned buffers it points to are sized and freed separately.
///
/// For every kind except `Box` the body size is a constant; a `Box`
/// carries its inline payload, so its body is
/// `BOX_PAYLOAD_OFFSET + header.len` bytes.
///
/// # Safety
///
/// `header` must point to a live object produced by one of the
/// constructors, so its `tag` and `len` are valid.
pub(crate) unsafe fn object_body_layout(header: *const ObjectHeader) -> (usize, usize) {
    // SAFETY: caller guarantees `header` is a live object header.
    let tag = unsafe { (*header).tag };
    match tag {
        TAG_STRING => (string::size_of_string(), string::align_of_string()),
        TAG_LIST => (list::size_of_list(), list::align_of_list()),
        TAG_MAP => (map::size_of_map(), map::align_of_map()),
        TAG_SET => (set::size_of_set(), set::align_of_set()),
        TAG_CLOSURE => (closure::size_of_closure(), closure::align_of_closure()),
        TAG_BOX => {
            // SAFETY: a live Box header carries its payload size in `len`.
            let payload_size = unsafe { (*header).len };
            (
                boxed::box_total_size(payload_size),
                boxed::box_align(OBJECT_ALIGN as u32),
            )
        }
        TAG_STRUCT => {
            // SAFETY: a live struct header carries its field count in `len`.
            let field_count = unsafe { (*header).len };
            (
                structval::struct_total_size(field_count),
                structval::struct_align(),
            )
        }
        // An unknown tag should never reach the collector. Treat it as a
        // bare header so freeing at least releases the body.
        _ => (
            std::mem::size_of::<ObjectHeader>(),
            std::mem::align_of::<ObjectHeader>().max(OBJECT_ALIGN),
        ),
    }
}

/// Free the owned buffers a heap object points to (string bytes, list
/// elements, map and set buckets, closure captures). The object body
/// itself is freed by the collector after this call. `Box` and unknown
/// tags own no separate buffer and are a no-op.
///
/// # Safety
///
/// `header` must point to a live object produced by one of the
/// constructors.
pub(crate) unsafe fn free_object_buffers(header: *mut ObjectHeader) {
    // SAFETY: caller guarantees `header` is a live object header.
    let tag = unsafe { (*header).tag };
    match tag {
        // SAFETY: tag matches the layout cast in each arm.
        TAG_STRING => unsafe { string::free_buffers(header as *mut String) },
        TAG_LIST => unsafe { list::free_buffers(header as *mut List) },
        TAG_MAP => unsafe { map::free_buffers(header as *mut Map) },
        TAG_SET => unsafe { set::free_buffers(header as *mut Set) },
        TAG_CLOSURE => unsafe { closure::free_buffers(header as *mut Closure) },
        _ => {}
    }
}
