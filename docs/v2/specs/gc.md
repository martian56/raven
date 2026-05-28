# Garbage Collector Spec

## Goal

Give compiled Raven v2 programs automatic memory management. The
collector reclaims every heap object the per-kind layouts in
`object-layout.md` allocate (`String`, `List`, `Map`, `Set`, `Closure`,
`Box`) once the running program can no longer reach it.

The design target is correctness and simplicity first: a stop-the-world,
single-threaded, tracing mark-and-sweep collector with precise roots
supplied by a shadow stack the code generator maintains. Throughput
optimisations (generational nurseries, incremental marking, Cranelift
stack maps) are out of scope and are listed at the end.

## Collector design

The collector is a two-phase tracing mark-and-sweep:

1. **Mark.** Starting from every root on the shadow stack, follow each
   object's internal pointers (per its `tag`) and set a mark bit on
   every object reached. Because marking traces the live object graph,
   it reclaims cycles that reference counting would leak: object A
   pointing at B pointing back at A is collected when neither is rooted.
2. **Sweep.** Walk the list of every allocated object. Any object whose
   mark bit is clear is unreachable: free its owned buffers, then free
   the object itself. Any object whose mark bit is set survives; clear
   its mark bit so the next cycle starts from a clean slate.

The collector is **stop the world**: a collection runs to completion
before the mutator (the compiled program) resumes. There is no
concurrent or incremental marking.

The collector is **single threaded**. v2.0 compiled programs are single
threaded, so the global collector state (the all-objects list, the byte
counters, the shadow stack) is plain global state guarded by that
assumption rather than a lock. Calling any runtime entry point from more
than one thread is undefined in v2.0.

### Mark bit

The mark bit lives in `ObjectHeader.gc_bits`, the field reserved for
exactly this purpose in `runtime.md`. Bit 0 (`GC_MARK_BIT = 0x1`) is the
mark; the remaining bits stay zero, reserved for a future colour scheme.
Using the existing header field keeps every object exactly the size the
layout spec already pins; the collector adds no per-object words.

## Shadow-stack root ABI

Codegen cannot leave live GC pointers only in CPU registers or unmanaged
stack slots, because the collector must find every root precisely. The
agreed mechanism is a **shadow stack**: a separate, runtime-owned stack
of pointers to the stack slots that currently hold live GC pointers.
Codegen pushes a function's GC-managed locals onto the shadow stack on
entry and pops them on every exit path.

The shadow stack stores **slot addresses** (`*mut *mut u8`), not object
pointers directly. Storing the address of the local lets the collector
read the current value at collection time, which matters because a slot
can be reassigned during the function body and a moving collector (a
later optimisation) could rewrite it in place. Reading through the slot
address always observes the live pointer.

### Entry points

The collector exposes a frame-based root API. Codegen builds a small
array of slot addresses in the function prologue and registers the whole
array in one call, then unregisters it on exit. A frame-based API is
cheaper for codegen to emit than per-slot push and pop: one call in,
one call out, regardless of how many locals the frame roots.

```c
// Register a frame's root array. `roots` points to `count` contiguous
// slots, each of which is the address of a stack local that holds a GC
// pointer (or null). The array must outlive the matching
// raven_gc_leave_frame call (it normally lives in the caller's frame).
void raven_gc_enter_frame(void **roots, size_t count);

// Unregister the most recently registered frame. Frames nest in strict
// last-in-first-out order, mirroring the call stack.
void raven_gc_leave_frame(void);
```

Rust signatures:

```rust
pub extern "C" fn raven_gc_enter_frame(roots: *mut *mut u8, count: usize);
pub extern "C" fn raven_gc_leave_frame();
```

A convenience per-slot API is also provided for hand-written tests and
for code paths that root a single temporary:

```rust
pub extern "C" fn raven_gc_push_root(slot: *mut *mut u8);
pub extern "C" fn raven_gc_pop_roots(n: usize);
```

`raven_gc_enter_frame` is exactly `count` successive pushes followed by
recording the frame boundary; `raven_gc_leave_frame` pops back to the
previous boundary. The two APIs share one underlying slot stack and may
be mixed, though codegen uses only the frame API.

### Calling convention codegen must follow

1. For each GC-managed local in the frame (any local whose static type
   is a heap kind: `String`, `List`, `Map`, `Set`, `Closure`, `Box`, or
   a user struct that lowers to one of those), reserve a stack slot.
2. In the prologue, build a contiguous array of the **addresses** of
   those slots. Initialise every rooted slot to null before the first
   call that can trigger a collection, so the collector never reads an
   uninitialised slot.
3. Call `raven_gc_enter_frame(roots, count)` once, after the slots are
   null-initialised and before any allocation.
4. On every path that leaves the function (normal return, early return,
   and the panic-free tail), call `raven_gc_leave_frame()` before
   returning.
5. A leaf function that allocates nothing and holds no GC locals across
   a call that can collect may skip the frame entirely.

### Worked example

A function with two GC locals (`a`, `b`) and one GC temporary (`mid`):

```raven
fn join(a: String, b: String) -> String {
    let mid: String = ", ";
    return concat(concat(a, mid), b);
}
```

Codegen emits, in pseudo-assembly:

```
join:
    slot_a = alloca ptr;  store a   -> slot_a
    slot_b = alloca ptr;  store b   -> slot_b
    slot_mid = alloca ptr; store null -> slot_mid   ; null before first collect
    roots = alloca [3 x ptr]
    roots[0] = &slot_a; roots[1] = &slot_b; roots[2] = &slot_mid
    call raven_gc_enter_frame(roots, 3)

    t0 = call raven_string_new(2)        ; may collect; a, b stay rooted
    store t0 -> slot_mid                 ; mid now rooted
    t1 = call raven_string_concat(load slot_a, load slot_mid)
    t2 = call raven_string_concat(t1, load slot_b)

    call raven_gc_leave_frame()
    return t2
```

Temporaries `t1` and `t2` are not separately rooted here because no
collection can happen between their creation and their consumption. When
a temporary does straddle a collection point, codegen spills it into a
rooted slot the same way `mid` is rooted.

## Object bookkeeping

The sweeper must visit every live object. The collector keeps a global
**all-objects list**: a `Vec<*mut ObjectHeader>` recording the base
pointer of every object handed out by `raven_gc_alloc`. The list is a
side table, not an intrusive header field, so the 16-byte `ObjectHeader`
keeps the exact shape pinned in `object-layout.md`; no object grows by a
`next` word.

Each allocation appends its base pointer. Sweep walks the vector,
retaining survivors (and clearing their mark bit) and dropping freed
entries; the retained order does not matter.

Two global counters back the trigger and the tests: `bytes_allocated`
tracks total live object bytes (bodies plus owned buffers) and drives
the collection threshold, and `live_objects` counts live objects so
tests can assert bounded liveness deterministically without measuring
flaky OS memory.

## Marking per layout

Marking dispatches on `ObjectHeader.tag`. For each root slot, the
collector reads the slot's current pointer; if non-null and not yet
marked, it marks the object and traces its internal pointers, following
the offsets pinned in `object-layout.md`. Tracing is iterative (an
explicit work stack), not recursive, so a deep or cyclic graph cannot
overflow the native stack.

| Tag | Pointers followed | Detail |
| --- | ----------------- | ------ |
| `TAG_STRING` | none into GC objects | `bytes` is a plain owned buffer, not a GC object. It is freed with the string in sweep, never traced. |
| `TAG_LIST` | `elements` at offset 24, when `elements_are_gc_ptrs != 0` | The buffer holds `len` pointer slots; each non-null slot is traced. When the flag is zero (scalar elements) the buffer is opaque bytes and is not traced. |
| `TAG_MAP` | `buckets` at offset 24; for each non-empty bucket `key` at entry offset 8 and `value` at entry offset 16 | `keys_are_gc_ptrs` and `values_are_gc_ptrs` flags gate whether key and value slots are traced. A slot is non-empty when `key != null`. |
| `TAG_SET` | `buckets` at offset 24; for each non-empty bucket `element` at entry offset 8 | `elements_are_gc_ptrs` gates tracing. A slot is non-empty when `element != null`. |
| `TAG_CLOSURE` | `captures` at offset 24 | The capture record holds `capture_ptr_count` leading pointer slots that the closure-lowering pass places first. Each non-null leading slot is traced; the remaining capture bytes are scalars and are not traced. |
| `TAG_BOX` | payload at offset 16, when `payload_is_gc_ptr != 0` | A box that wraps a heap value stores a single pointer at offset 16 and is traced; a box that wraps a scalar is opaque. |

### Why the layouts carry pointer-kind flags

The collector cannot guess whether an 8-byte slot is a pointer or an
integer: tracing an integer as a pointer is memory-unsafe. The
collection layouts therefore carry an explicit flag set at construction
time, supplied by codegen from the static element or capture type:

* `List` gains `elements_are_gc_ptrs: u8`.
* `Map` repurposes its reserved `_pad` word into
  `keys_are_gc_ptrs: u8` and `values_are_gc_ptrs: u8`.
* `Set` repurposes its reserved `_pad` word into
  `elements_are_gc_ptrs: u8`.
* `Closure` gains `capture_ptr_count: u32`, the number of leading
  pointer-sized capture slots that are GC pointers.
* `Box` gains `payload_is_gc_ptr: u8`.

These are the only layout changes the collector introduces. The exact
offsets after the change are pinned in `object-layout.md` and asserted
by the inline layout tests. Constructors gain the corresponding extra
parameters; the defaults (all flags zero, count zero) reproduce the
previous "no internal GC pointers" behaviour for callers that do not
yet supply them.

## Sweeping and buffer freeing

Sweep walks the all-objects list once. For each object:

* If the mark bit is clear, the object is dead. The sweeper frees the
  object's **owned buffers** first, then the object body:
  * `String`: free `bytes` (`cap` bytes, align 1).
  * `List`: free `elements` (`cap * element_size` bytes,
    `element_align`).
  * `Map` / `Set`: free `buckets` (`bucket_count * entry_size` bytes,
    entry alignment 8).
  * `Closure`: free `captures` (`capture_size` bytes, `capture_align`).
  * `Box`: no separate buffer; the payload is inline in the body.
  Then free the object body itself. The body size and alignment are
  derived from the tag (fixed per kind, except `Box`, whose body size is
  `16 + header.len`). The collector decrements `bytes_allocated` and
  `live_objects` for each freed object.
* If the mark bit is set, the object survives. Clear the mark bit and
  retain it in the list.

Owned buffers are **plain allocations** owned by their object, not
separate GC objects. They are never on the all-objects list, never
traced, and never independently collected; they live and die with the
object that owns them. This keeps tracing and bookkeeping simple: the
collector reasons only about object bodies, and every buffer has exactly
one owner.

## Collection trigger and threshold policy

Allocation goes through `raven_gc_alloc(size, align, tag)`:

1. If `bytes_allocated + size` would cross the current threshold, run a
   full collection first.
2. Allocate the object body through the raw allocator, zero it, register
   it in the all-objects list, bump the counters, and return it.

The threshold starts at `1 MiB`. After a collection it is reset to
`max(1 MiB, 2 * bytes_live_after_sweep)`, so a program with a large live
set collects less often while a program with a small live set keeps a
tight ceiling. This is the standard "allocation high-water mark"
heuristic; it bounds heap growth to a constant factor of the live set.

`raven_gc_collect()` forces a full collection regardless of the
threshold. It exists for deterministic testing and for any future
explicit-collection point; the compiled program never needs to call it
because allocation collects automatically.

```rust
pub extern "C" fn raven_gc_alloc(size: usize, align: usize, tag: u32) -> *mut u8;
pub extern "C" fn raven_gc_collect();
```

The constructors in `object/*.rs` route their object-body allocation
through `raven_gc_alloc` (passing the kind's tag) and continue to
allocate owned buffers through the raw `raven_alloc`. `raven_alloc`
itself stays a raw passthrough so non-object scratch allocations do not
register with the collector.

## Out of scope

* **Cranelift stack maps.** The shadow stack is the v2.0 root mechanism;
  stack maps are a later throughput optimisation that would not change
  the object layouts or the sweep logic.
* **Generational collection.** No nursery, remembered set, or write
  barrier. Every collection is a full heap trace.
* **Incremental or concurrent collection.** Marking and sweeping run to
  completion with the mutator stopped.
* **Moving or compacting collection.** Objects keep their addresses for
  their whole lifetime. The slot-address shadow stack is forward
  compatible with a future moving collector, but v2.0 never relocates an
  object.
* **Multiple threads.** Global collector state assumes a single mutator.
* **Codegen emission of root frames.** This document defines the ABI
  codegen targets; the prologue and epilogue emission is issue #67. The
  collector ships with a Rust-side test harness that plays codegen's
  role.

## Test coverage

Inline unit tests in the `gc` module and a cross-crate integration test
in `raven-runtime/tests/` cover:

* **Reachability.** Rooted objects, and objects reachable only
  transitively from a root, survive a forced collection; unrooted
  objects are freed.
* **Cycle reclamation.** Two objects that point at each other, neither
  rooted, are both collected: the property reference counting lacks and
  the reason for a tracing collector.
* **Per-layout tracing.** A graph built from a `List` of GC pointers, a
  `Map` with GC keys and values, a `Set` of GC elements, a `Closure`
  with GC captures, and a `Box` wrapping a GC pointer marks every
  reachable object.
* **Bounded liveness stress.** Allocating many small objects while
  rooting only a bounded working set keeps `live_objects` bounded and
  the final rooted state intact. The counter is asserted rather than OS
  memory, which is flaky in CI.
* **Frame nesting.** Nested enter and leave calls register and
  unregister roots in last-in-first-out order; a collection mid-nesting
  sees the union of all active frames' roots.
* **Threshold trigger.** Allocating past the threshold triggers an
  automatic collection without an explicit `raven_gc_collect` call.
