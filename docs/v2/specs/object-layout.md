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
    u8          *elements;   // owned buffer of `cap * element_size` bytes
};
```

Field offsets on 64-bit:

| offset | size | field         |
| -----: | ---: | ------------- |
| 0      | 16   | header        |
| 16     | 4    | element_size  |
| 20     | 4    | element_align |
| 24     | 8    | elements      |
| total  | 32   |               |

* Alignment: 8.
* `header.tag = TAG_LIST`.
* `header.len` is the element count, `header.cap` the element capacity.
* `element_size` is the size in bytes of one element. Scalars are inline
  (e.g. `List<Int>` stores 8-byte slots). Heap objects are stored as
  pointer-sized slots, so `element_size == 8` and the slot holds a
  pointer to the inner object.
* `element_align` is the alignment of one element, the same value the
  back-end would pass to `raven_alloc` for an element.
* `elements` points to an owned buffer of `cap * element_size` bytes.
  When `cap == 0`, `elements` is null.

The GC tracing rule keys on whether the element is a pointer type. The
back-end records this in a per-list descriptor in a follow-up; for now
the tracer treats `element_size == 8` and `element_align == 8` lists as
potentially pointer-bearing.

## Map

```c
struct Map {
    ObjectHeader header;     // tag = TAG_MAP
    u32          bucket_count;
    u32          _pad;       // reserved, zeroed
    MapEntry    *buckets;
};

struct MapEntry {
    u64   hash;
    void *key;     // null if empty or tombstone
    void *value;
};
```

Map field offsets on 64-bit:

| offset | size | field         |
| -----: | ---: | ------------- |
| 0      | 16   | header        |
| 16     | 4    | bucket_count  |
| 20     | 4    | _pad          |
| 24     | 8    | buckets       |
| total  | 32   |               |

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
    u32          _pad;
    SetEntry    *buckets;
};

struct SetEntry {
    u64   hash;
    void *element;
};
```

Set field offsets mirror Map:

| offset | size | field         |
| -----: | ---: | ------------- |
| 0      | 16   | header        |
| 16     | 4    | bucket_count  |
| 20     | 4    | _pad          |
| 24     | 8    | buckets       |
| total  | 32   |               |

`SetEntry` offsets:

| offset | size | field         |
| -----: | ---: | ------------- |
| 0      | 8    | hash          |
| 8      | 8    | element       |
| total  | 16   |               |

Same rules as `Map`: `header.tag = TAG_SET`, `header.cap` mirrors
`bucket_count`, empty or tombstoned slots have `element == null`,
FNV-1a 64 is the hash function.

## Closure

```c
struct Closure {
    ObjectHeader header;     // tag = TAG_CLOSURE, len = capture count
    void        *fn_ptr;     // pointer to the lifted body
    void        *captures;   // owned buffer of size `capture_size`
    u32          capture_size;
    u32          capture_align;
};
```

Field offsets on 64-bit:

| offset | size | field         |
| -----: | ---: | ------------- |
| 0      | 16   | header        |
| 16     | 8    | fn_ptr        |
| 24     | 8    | captures      |
| 32     | 4    | capture_size  |
| 36     | 4    | capture_align |
| total  | 40   |               |

* Alignment: 8.
* `header.tag = TAG_CLOSURE`. `header.len` is the capture count (the
  number of distinct captured bindings, not the byte size).
* `header.cap` is unused and zeroed.
* `fn_ptr` is the raw code pointer of the lifted closure body. The body
  has the calling convention `(captures: *mut u8, args...) -> ret`.
* `captures` is a pointer to an owned buffer of `capture_size` bytes.
  Layout inside the capture record is decided by the closure-lowering
  pass and is opaque to the runtime. The GC walks it through a
  descriptor produced by codegen in a follow-up.
* `captures` is null when `capture_size == 0`.

The pointer-indirect form (rather than inline-after-header) keeps every
`Closure` object the same size, so the GC and stdlib do not need to read
the capture footer to advance over a closure. The cost is one extra
allocation per closure with non-empty captures.

## Box

```c
struct Box {
    ObjectHeader header;     // tag = TAG_BOX, len = payload byte size
    u8           payload[];  // sized payload follows the header
};
```

Field offsets on 64-bit:

| offset | size       | field   |
| -----: | ---------: | ------- |
| 0      | 16         | header  |
| 16     | payload_sz | payload |

* Alignment: `max(OBJECT_ALIGN, payload_align)`.
* `header.tag = TAG_BOX`. `header.len` is the payload size in bytes.
* `header.cap` is 1 (a `Box<T>` always holds exactly one `T`).
* The payload lives inline at offset 16. For payload alignments greater
  than 8 the constructor pads the request to the alignment, but at
  current Raven scalar widths (8 bytes for `Int`, `Float`, pointers, less
  for `Bool`) no padding is needed.

`Box` exists so generic collections over primitives can be uniformly
typed: a `List<Int>` may store `Int` inline (when codegen specialises it)
or as a `Box<Int>` pointer slot (when monomorphisation falls back to a
generic body, e.g. a method on `List<T>` taking a `T` by value).

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
| `raven_list_new` | `fn(element_size: u32, element_align: u32, cap: u32) -> *mut List` | Allocates a `List` with the given per-element shape and slot capacity. `header.len = 0`. |
| `raven_list_len` | `fn(l: *const List) -> u32` | Returns `header.len`. |
| `raven_list_elements` | `fn(l: *const List) -> *mut u8` | Returns the `elements` field. |
| `raven_list_push` | `fn(l: *mut List, payload: *const u8)` | Copies `element_size` bytes from `payload` into the next slot, growing the buffer if needed. |
| `raven_map_new` | `fn(bucket_count: u32) -> *mut Map` | Allocates a `Map` with the given initial bucket count, rounded up to a power of two. Bucket buffer is zero-filled. |
| `raven_map_buckets` | `fn(m: *const Map) -> *mut MapEntry` | Returns the `buckets` field. |
| `raven_map_bucket_count` | `fn(m: *const Map) -> u32` | Returns the `bucket_count` field. |
| `raven_set_new` | `fn(bucket_count: u32) -> *mut Set` | Set counterpart of `raven_map_new`. |
| `raven_set_buckets` | `fn(s: *const Set) -> *mut SetEntry` | Returns the `buckets` field. |
| `raven_set_bucket_count` | `fn(s: *const Set) -> u32` | Returns the `bucket_count` field. |
| `raven_closure_new` | `fn(fn_ptr: *const u8, capture_size: u32, capture_align: u32, capture_count: u32) -> *mut Closure` | Allocates a `Closure` with the given function pointer and capture record. Capture buffer is zero-filled and owned by the closure. |
| `raven_closure_fn_ptr` | `fn(c: *const Closure) -> *const u8` | Returns the function pointer. |
| `raven_closure_captures` | `fn(c: *const Closure) -> *mut u8` | Returns the captures buffer. |
| `raven_box_new` | `fn(payload_size: u32, payload_align: u32) -> *mut Box` | Allocates a `Box` whose payload is `payload_size` bytes aligned to `payload_align`. Payload is zero-filled. |
| `raven_box_payload` | `fn(b: *const Box) -> *mut u8` | Returns a pointer to the inline payload at offset 16. |

The deallocator counterpart for each kind is a follow-up issue; today the
process leaks every heap object on exit, which is fine for the v2
scaffold because no test program holds objects long enough to matter.

## Tracing summary

The GC (issue #64) will dispatch on `header.tag` and walk the layout's
internal pointers. The pointers each layout owns are:

| Tag | Internal pointers | Notes |
| --- | ----------------- | ----- |
| `TAG_STRING` | `bytes` at offset 16 | Owned `u8` buffer of size `cap`. No further tracing needed. |
| `TAG_LIST` | `elements` at offset 24 | The buffer holds `cap * element_size` bytes. If `element_size == 8` and `element_align == 8` it may hold heap pointers that the tracer recurses into. |
| `TAG_MAP` | `buckets` at offset 24, then for each non-empty bucket `key` at entry offset 8 and `value` at entry offset 16 | Both `key` and `value` are heap pointers (or boxes for primitives). |
| `TAG_SET` | `buckets` at offset 24, then for each non-empty bucket `element` at entry offset 8 | |
| `TAG_CLOSURE` | `captures` at offset 24 | The capture record is opaque to the tracer; a per-closure descriptor produced by codegen will enumerate its pointers in a follow-up. |
| `TAG_BOX` | none of its own; the payload at offset 16 is treated by the descriptor attached to the box's static type | |

The actual tracing implementation lands in issue #64. This document
exists so that issue can be written against a fixed set of offsets.

## Out of scope

* Real GC. `gc_bits` stays zero. Allocations leak at process exit. Issue
  #64 lands the collector.
* Codegen integration. Issue #67 wires field loads and stores through the
  offsets pinned here.
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
* Integration tests in `raven-runtime/tests/object_layouts.rs`:
  * Construct and read back every layout through the C ABI surface, so
    the staticlib's exported symbols stay reachable.
  * Cross-check that `header.tag` and `header.cap` agree with what the
    constructor was told.

Size assertions are gated on `target_pointer_width = "64"` so 32-bit
hosts still compile cleanly until their layouts are pinned in a future
revision.
