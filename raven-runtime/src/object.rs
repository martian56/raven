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

pub mod hash;
pub mod list;
pub mod map;
pub mod set;
pub mod string;

pub use list::{raven_list_elements, raven_list_len, raven_list_new, raven_list_push, List};
pub use map::{raven_map_bucket_count, raven_map_buckets, raven_map_new, Map, MapEntry};
pub use set::{raven_set_bucket_count, raven_set_buckets, raven_set_new, Set, SetEntry};
pub use string::{
    raven_string_bytes, raven_string_concat, raven_string_len, raven_string_new, String,
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
}
