# Object Layout Spec

## Goal

Pin the in-memory shape of every heap-allocated Raven v2 value. The shared
`ObjectHeader` from issue #63 is already fixed; this document specifies what
follows the header for each tag, the exact field offsets, the constructor
APIs the back-end calls, and how the future GC will walk each layout.

The layouts are documented in two parallel forms: a `#[repr(C)]` Rust
struct in `raven-runtime/src/object/<kind>.rs` that the in-tree tests
assert against, and a C-style diagram with explicit byte offsets that
codegen treats as a binary contract.

All sizes and offsets are for 64-bit targets. Per-layout `cfg` gates keep
size assertions correct on 32-bit hosts (the runtime is not yet supported
there beyond compiling).

## Shared header

Recap from `runtime.md`:

```
offset  size  field
  0      4    tag
  4      4    gc_bits
  8      4    len
 12      4    cap
```

Total 16 bytes, alignment 4, but every object is allocated at
`OBJECT_ALIGN = 8` so the payload that follows is 8-byte aligned. Every
subsequent layout therefore starts its first payload field at offset 16
without padding.

## String

```c
struct String {
    ObjectHeader header;     // tag = TAG_STRING
    u8          *bytes;      // owned buffer of `cap` bytes
};
```

Field offsets on 64-bit:

| offset | size | field         |
| -----: | ---: | ------------- |
| 0      | 16   | header        |
| 16     | 8    | bytes         |
| total  | 24   |               |

* Alignment: 8.
* `header.tag = TAG_STRING`.
* `header.len` is the UTF-8 byte length, not the codepoint count.
* `header.cap` is the allocated byte capacity of `bytes`.
* `bytes` points to an owned buffer of `cap` bytes. The first `len` bytes
  are valid UTF-8. The remaining `cap - len` bytes are undefined.
* When `cap == 0`, `bytes` is null.

## List

```c
struct List {
    ObjectHeader header;     // tag = TAG_LIST
    u32          element_size;
    u32          element_align;
    u32          elements_are_gc_ptrs; // nonzero when slots are GC pointers
    u32          _pad;                 // reserved, zeroed
    u8          *elements;   // owned buffer of `cap * element_size` bytes
};
```

Field offsets on 64-bit:

| offset | size | field                |
| -----: | ---: | -------------------- |
| 0      | 16   | header               |
| 16     | 4    | element_size         |
| 20     | 4    | element_align        |
| 24     | 4    | elements_are_gc_ptrs |
| 28     | 4    | _pad                 |
| 32     | 8    | elements             |
| total  | 40   |                      |

* Alignment: 8.
* `header.tag = TAG_LIST`.
* `header.len` is the element count, `header.cap` the element capacity.
* `element_size` is the size in bytes of one element. Scalars are inline
  (e.g. `List<Int>` stores 8-byte slots). Heap objects are stored as
  pointer-sized slots, so `element_size == 8` and the slot holds a
  pointer to the inner object.
* `element_align` is the alignment of one element, the same value the
  back-end would pass to `raven_alloc` for an element.
* `elements_are_gc_ptrs` is nonzero when each element slot is a GC
  pointer the collector traces, zero for scalar elements. Codegen sets
  it from the static element type. The GC reads it instead of guessing
  from `element_size`, because an 8-byte `Int` slot is not a pointer.
* `elements` points to an owned buffer of `cap * element_size` bytes.
  When `cap == 0`, `elements` is null.

## Map

```c
struct Map {
    ObjectHeader header;     // tag = TAG_MAP
    u32          bucket_count;
    u8           keys_are_gc_ptrs;   // nonzero when keys are GC pointers
    u8           values_are_gc_ptrs; // nonzero when values are GC pointers
    u16          _pad;               // reserved, zeroed
    MapEntry    *buckets;
};

struct MapEntry {
    u64   hash;
    void *key;     // null if empty or tombstone
    void *value;
};
```

Map field offsets on 64-bit:

| offset | size | field              |
| -----: | ---: | ------------------ |
| 0      | 16   | header             |
| 16     | 4    | bucket_count       |
| 20     | 1    | keys_are_gc_ptrs   |
| 21     | 1    | values_are_gc_ptrs |
| 22     | 2    | _pad               |
| 24     | 8    | buckets            |
| total  | 32   |                    |

`MapEntry` offsets:

| offset | size | field         |
| -----: | ---: | ------------- |
| 0      | 8    | hash          |
| 8      | 8    | key           |
| 16     | 8    | value         |
| total  | 24   |               |

* Alignment of `Map`: 8. Alignment of `MapEntry`: 8.
* `header.tag = TAG_MAP`. `header.len` is the live entry count.
* `header.cap` mirrors `bucket_count`. Keeping `cap` in the header lets
  the GC walk the buckets without dispatching through `tag`.
* `bucket_count` is a power of two, or zero for the freshly-constructed
  empty map. The constructor accepts any non-negative request and rounds
  up to the next power of two.
* `keys_are_gc_ptrs` and `values_are_gc_ptrs` are nonzero when bucket
  keys or values are GC pointers the collector traces. Codegen sets them
  from the static key and value types. They reuse the word that was
  reserved padding before the collector landed.
* `buckets` points to `bucket_count` contiguous `MapEntry` slots. An
  empty or tombstoned slot has `key == null`. Tombstones are distinguished
  from truly-empty slots by `hash == TOMBSTONE_HASH` (a reserved bit
  pattern; see the constructor module). Tombstone bookkeeping is not yet
  used by the v2 stdlib, but the slot reservation keeps the entry shape
  stable when rehash and delete land in a stdlib issue.
* `key` and `value` are pointers. When the logical key or value is a
  primitive that does not fit in a pointer, the back-end boxes it through
  `raven_box_new` before insertion.

The hash function for the v2 scaffold is FNV-1a 64. It is deterministic
(no per-process seed) and depends on no external crate. The choice is
documented here because the hash is part of the persisted-bucket layout:
re-hashing on resize must agree with the original insert. A future
issue may swap to a seeded hasher; the swap is a behaviour change, not
an ABI change, because `hash` is internal to `MapEntry`.

## Set

```c
struct Set {
    ObjectHeader header;     // tag = TAG_SET
    u32          bucket_count;
    u8           elements_are_gc_ptrs; // nonzero when elements are GC pointers
    u8           _pad[3];              // reserved, zeroed
    SetEntry    *buckets;
};

struct SetEntry {
    u64   hash;
    void *element;
};
```

Set field offsets mirror Map:

| offset | size | field                |
| -----: | ---: | -------------------- |
| 0      | 16   | header               |
| 16     | 4    | bucket_count         |
| 20     | 1    | elements_are_gc_ptrs |
| 21     | 3    | _pad                 |
| 24     | 8    | buckets              |
| total  | 32   |                      |

`SetEntry` offsets:

| offset | size | field         |
| -----: | ---: | ------------- |
| 0      | 8    | hash          |
| 8      | 8    | element       |
| total  | 16   |               |

Same rules as `Map`: `header.tag = TAG_SET`, `header.cap` mirrors
`bucket_count`, empty or tombstoned slots have `element == null`,
FNV-1a 64 is the hash function. `elements_are_gc_ptrs` is nonzero when
elements are GC pointers the collector traces.

## Closure

```c
struct Closure {
    ObjectHeader header;     // tag = TAG_CLOSURE, len = capture count
    void        *fn_ptr;     // pointer to the lifted body
    void        *captures;   // owned buffer of size `capture_size`
    u32          capture_size;
    u32          capture_align;
    u32          capture_ptr_count; // leading GC-pointer capture slots
    u32          _pad;              // reserved, zeroed
};
```

Field offsets on 64-bit:

| offset | size | field             |
| -----: | ---: | ----------------- |
| 0      | 16   | header            |
| 16     | 8    | fn_ptr            |
| 24     | 8    | captures          |
| 32     | 4    | capture_size      |
| 36     | 4    | capture_align     |
| 40     | 4    | capture_ptr_count |
| 44     | 4    | _pad              |
| total  | 48   |                   |

* Alignment: 8.
* `header.tag = TAG_CLOSURE`. `header.len` is the capture count (the
  number of distinct captured bindings, not the byte size).
* `header.cap` is unused and zeroed.
* `fn_ptr` is the raw code pointer of the lifted closure body. The body
  has the calling convention `(captures: *mut u8, args...) -> ret`.
* `captures` is a pointer to an owned buffer of `capture_size` bytes.
  Layout inside the capture record is decided by the closure-lowering
  pass and is opaque to the runtime apart from the convention below.
* `capture_ptr_count` is the number of leading pointer-sized capture
  slots that are GC pointers. The closure-lowering pass places the GC
  pointer captures first in the record, so the collector traces the
  first `capture_ptr_count` pointer-sized slots and leaves the rest. A
  closure with no GC captures sets it to zero.
* `captures` is null when `capture_size == 0`.

The pointer-indirect form (rather than inline-after-header) keeps every
`Closure` object the same size, so the GC and stdlib do not need to read
the capture footer to advance over a closure. The cost is one extra
allocation per closure with non-empty captures.

## Box

```c
struct Box {
    ObjectHeader header;     // tag = TAG_BOX, len = payload byte size
    u32          payload_is_gc_ptr; // nonzero when payload is a GC pointer
    u32          _pad;              // reserved, zeroed
    u8           payload[];  // sized payload follows the flag word
};
```

Field offsets on 64-bit:

| offset | size       | field             |
| -----: | ---------: | ----------------- |
| 0      | 16         | header            |
| 16     | 4          | payload_is_gc_ptr |
| 20     | 4          | _pad              |
| 24     | payload_sz | payload           |

* Alignment: `max(OBJECT_ALIGN, payload_align)`.
* `header.tag = TAG_BOX`. `header.len` is the payload size in bytes.
* `header.cap` is 1 (a `Box<T>` always holds exactly one `T`).
* `payload_is_gc_ptr` is nonzero when the inline payload is a single GC
  pointer the collector traces, zero for a scalar payload. Codegen sets
  it from the static payload type.
* The payload lives inline at offset 24 (`BOX_PAYLOAD_OFFSET`), after the
  header and flag word, so it stays 8-byte aligned. For payload
  alignments greater than 8 the constructor pads the request to the
  alignment, but at current Raven scalar widths (8 bytes for `Int`,
  `Float`, pointers, less for `Bool`) no padding is needed.

`Box` exists so generic collections over primitives can be uniformly
typed: a `List<Int>` may store `Int` inline (when codegen specialises it)
or as a `Box<Int>` pointer slot (when monomorphisation falls back to a
generic body, e.g. a method on `List<T>` taking a `T` by value).

## Struct (and enum) value

A user-defined struct value, and an enum value, share one shape: the
header followed by uniform 8-byte field slots in declaration order.

```c
struct StructValue {
    ObjectHeader header;     // tag = TAG_STRUCT, len = field count, cap = type id
    u64          fields[];   // one 8-byte slot per field, declaration order
};
```

Field offsets on 64-bit:

| offset      | size | field            |
| ----------: | ---: | ---------------- |
| 0           | 16   | header           |
| 16          | 8    | field 0          |
| 24          | 8    | field 1          |
| 16 + 8 * i  | 8    | field i          |

* Alignment: `OBJECT_ALIGN` (8). The total body size is
  `16 + 8 * field_count`, already 8-byte aligned.
* `header.tag = TAG_STRUCT`. `header.len` is the field count.
* `header.cap` carries the per-type descriptor id. Two structs with the
  same tag have different field shapes, so the collector cannot infer the
  pointer fields from the tag. The back end assigns each monomorphic
  struct (and enum) type a small integer id and registers a GC pointer
  bitmask for it through `raven_struct_register` before any value is
  built; the collector looks the mask up by `header.cap`.
* Each field occupies one 8-byte slot regardless of its type: an `Int`
  or `Float` fits exactly, a `Bool` or `Char` is widened into the slot,
  and a heap value is a single pointer. A slot holds a GC pointer exactly
  when its bit is set in the type's registered mask.

An enum value uses the same shape with slot 0 reserved for the variant
discriminant (a pointer-width integer) and slots `1..` holding the active
variant's payload. The registered mask therefore shifts each payload
pointer to slot `i + 1`, and is the union of every variant's payload
pointers (an inactive variant's slots are zero and trace harmlessly).

## Constructor APIs

Every constructor is `#[no_mangle] pub extern "C"`. Every constructor
returns a typed pointer (e.g. `*mut String`) that codegen treats as
opaque and only ever passes back through accessors.

| Symbol | Signature | Notes |
| ------ | --------- | ----- |
| `raven_string_new` | `fn(cap: u32) -> *mut String` | Allocates a `String` with the given byte capacity. Bytes buffer is allocated separately and zero-filled. `header.len = 0`. |
| `raven_string_len` | `fn(s: *const String) -> u32` | Returns `header.len`. |
| `raven_string_bytes` | `fn(s: *const String) -> *const u8` | Returns the `bytes` field. |
| `raven_string_concat` | `fn(a: *const String, b: *const String) -> *mut String` | Returns a new heap string equal to `a` followed by `b`. |
| `raven_list_new` | `fn(element_size: u32, element_align: u32, cap: u32, elements_are_gc_ptrs: u32) -> *mut List` | Allocates a `List` with the given per-element shape, slot capacity, and GC-pointer flag. `header.len = 0`. |
| `raven_list_len` | `fn(l: *const List) -> u32` | Returns `header.len`. |
| `raven_list_elements` | `fn(l: *const List) -> *mut u8` | Returns the `elements` field. |
| `raven_list_push` | `fn(l: *mut List, payload: *const u8)` | Copies `element_size` bytes from `payload` into the next slot, growing the buffer if needed. |
| `raven_map_new` | `fn(bucket_count: u32, keys_are_gc_ptrs: u8, values_are_gc_ptrs: u8) -> *mut Map` | Allocates a `Map` with the given initial bucket count, rounded up to a power of two, and the key and value GC-pointer flags. Bucket buffer is zero-filled. |
| `raven_map_buckets` | `fn(m: *const Map) -> *mut MapEntry` | Returns the `buckets` field. |
| `raven_map_bucket_count` | `fn(m: *const Map) -> u32` | Returns the `bucket_count` field. |
| `raven_set_new` | `fn(bucket_count: u32, elements_are_gc_ptrs: u8) -> *mut Set` | Set counterpart of `raven_map_new`, with one element GC-pointer flag. |
| `raven_set_buckets` | `fn(s: *const Set) -> *mut SetEntry` | Returns the `buckets` field. |
| `raven_set_bucket_count` | `fn(s: *const Set) -> u32` | Returns the `bucket_count` field. |
| `raven_closure_new` | `fn(fn_ptr: *const u8, capture_size: u32, capture_align: u32, capture_count: u32, capture_ptr_count: u32) -> *mut Closure` | Allocates a `Closure` with the given function pointer, capture record, and count of leading GC-pointer capture slots. Capture buffer is zero-filled and owned by the closure. |
| `raven_closure_fn_ptr` | `fn(c: *const Closure) -> *const u8` | Returns the function pointer. |
| `raven_closure_captures` | `fn(c: *const Closure) -> *mut u8` | Returns the captures buffer. |
| `raven_box_new` | `fn(payload_size: u32, payload_align: u32, payload_is_gc_ptr: u32) -> *mut Box` | Allocates a `Box` whose payload is `payload_size` bytes aligned to `payload_align`, with the payload GC-pointer flag. Payload is zero-filled. |
| `raven_box_payload` | `fn(b: *const Box) -> *mut u8` | Returns a pointer to the inline payload at `BOX_PAYLOAD_OFFSET` (offset 24). |
| `raven_struct_new` | `fn(field_count: u32, type_id: u32) -> *mut ObjectHeader` | Allocates a struct (or enum) value with `field_count` zero-filled 8-byte field slots, tagged `TAG_STRUCT`, recording `type_id` in `header.cap`. |
| `raven_struct_fields` | `fn(s: *const ObjectHeader) -> *mut u8` | Returns a pointer to the first field slot, at `STRUCT_FIELDS_OFFSET` (offset 16). |
| `raven_struct_register` | `fn(type_id: u32, ptr_mask: u64)` | Registers a struct or enum type's GC pointer descriptor: bit `i` set means field slot `i` holds a traced GC pointer. Called from the program entry shim before any value is built. Lives in the collector; see `docs/v2/specs/gc.md`. |

Each constructor routes its object-body allocation through
`raven_gc_alloc` so the collector tracks every object, and allocates its
owned buffer (string bytes, list elements, map and set buckets, closure
captures) through the raw `raven_alloc`. The collector frees both during
sweep; see `docs/v2/specs/gc.md`.

## Tracing summary

The collector (issue #64) dispatches on `header.tag` and walks the
layout's internal GC pointers, gated by the per-layout pointer-kind
flags. The pointers each layout owns are:

| Tag | Internal pointers | Notes |
| --- | ----------------- | ----- |
| `TAG_STRING` | none into GC objects | `bytes` at offset 16 is a plain owned buffer of size `cap`, freed with the string in sweep. Never traced. |
| `TAG_LIST` | `elements` at offset 32 | When `elements_are_gc_ptrs != 0`, each of the first `len` pointer slots is traced; otherwise the buffer is opaque scalar bytes. |
| `TAG_MAP` | `buckets` at offset 24, then for each non-empty bucket `key` at entry offset 8 and `value` at entry offset 16 | `keys_are_gc_ptrs` and `values_are_gc_ptrs` gate tracing of keys and values. A bucket is non-empty when `key != null`. |
| `TAG_SET` | `buckets` at offset 24, then for each non-empty bucket `element` at entry offset 8 | `elements_are_gc_ptrs` gates tracing. A bucket is non-empty when `element != null`. |
| `TAG_CLOSURE` | `captures` at offset 24 | The first `capture_ptr_count` pointer-sized capture slots are traced; the rest of the record is scalar. |
| `TAG_BOX` | payload at offset 24 | When `payload_is_gc_ptr != 0` the single payload pointer is traced; otherwise the payload is a scalar. |
| `TAG_STRUCT` | field slots at offset 16 | The collector looks up the per-type bitmask by `header.cap` and traces each of the `header.len` slots whose bit is set. An unregistered id traces nothing. Enum values store the discriminant in slot 0 (never a pointer) and payload in later slots. |

The marking, sweeping, and threshold logic live in `docs/v2/specs/gc.md`
and `raven-runtime/src/gc.rs`.

## Out of scope

* Trait object fat pointers. The `BOX` tag is the storage shape; the
  vtable layout is issue #66.
* Full stdlib over these layouts (string methods, list iteration, map
  iteration). Issues #71 to #80 ship those.
* Resize policy beyond the constructor. `raven_list_push` doubles `cap`
  when full; map and set resize live with the stdlib issues.

## Test coverage

* Inline unit tests in `raven-runtime/src/object/*.rs`:
  * `size_of::<String>() == 24`, `size_of::<List>() == 32`,
    `size_of::<Map>() == 32`, `size_of::<Set>() == 32`,
    `size_of::<Closure>() == 40` on 64-bit targets.
  * `offset_of!` for every field matches the diagram above.
  * Constructors zero the body and set `header.tag` correctly.
  * `raven_string_concat` produces a string whose bytes equal the
    concatenation of its inputs.
  * `raven_list_push` grows the buffer and preserves earlier elements.
  * `STRUCT_FIELDS_OFFSET == 16`, `STRUCT_FIELD_SLOT == 8`, and a struct
    value zero-fills its slots and tags itself `TAG_STRUCT`.
* Integration tests in `raven-runtime/tests/object_layouts.rs`:
  * Construct and read back every layout through the C ABI surface, so
    the staticlib's exported symbols stay reachable.
  * Cross-check that `header.tag` and `header.cap` agree with what the
    constructor was told.

Size assertions are gated on `target_pointer_width = "64"` so 32-bit
hosts still compile cleanly until their layouts are pinned in a future
revision.
