# Garbage Collector Spec

## Goal

Give compiled Raven v2 programs automatic memory management. The
collector reclaims every heap object the per-kind layouts in
`object-layout.md` allocate (`String`, `List`, `Map`, `Set`, `Closure`,
`Box`) once the running program can no longer reach it.

The design target is correctness and simplicity first: a stop-the-world,
tracing mark-and-sweep collector with precise roots supplied by shadow stacks
the code generator maintains. Raven goroutines may run in parallel across an
OS-thread pool; the collector coordinates those mutators at safepoints.
Throughput
optimisations (generational nurseries, incremental marking, Cranelift
stack maps) are out of scope and are listed at the end.

## Collector design

The collector is a two-phase tracing mark-and-sweep:

1. **Mark.** Starting from every registered thread and parked-goroutine root,
   follow each object's internal pointers (per its `tag`) and stamp every
   object reached with the current collection epoch. Because marking traces
   the live object graph,
   it reclaims cycles that reference counting would leak: object A
   pointing at B pointing back at A is collected when neither is rooted.
2. **Sweep.** Walk the initiating thread's allocation list. Any object whose
   stamp differs from the current epoch is unreachable: free its owned
   buffers, then free the object itself. An object stamped with the current
   epoch survives. The next collection uses a fresh epoch, so no clear pass is
   needed.

The collector is **stop the world**: a collection runs to completion
before the mutator (the compiled program) resumes. There is no
concurrent or incremental marking.

The mark-and-sweep cycle itself is **single threaded**, but Raven mutators are
not. Each worker thread owns its allocation list, byte counters, and current
shadow stack. Every live thread registers that stack in a process-wide root
registry, while parked goroutine roots and buffered channel values are exposed
through the scheduler. A collector stops all in-Raven threads at allocation or
loop-back-edge safepoints, scans the complete root set, and sweeps only the
initiating thread's allocation list. Globally unique mark epochs distinguish
the current cycle from marks written by another thread's collection.

### Mark epoch

The current mark epoch lives in `ObjectHeader.gc_bits`, the field reserved for
collector state in `runtime.md`. Epochs are process-wide, monotonically
allocated non-zero `u32` values, so two worker collections cannot confuse a
stale mark for their own. Using the existing header field keeps every object
exactly the size the layout spec pins; the collector adds no per-object words.

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

A heap-valued mutable module-level global (a `let` at file scope) lives in
a fixed data slot rather than a stack frame. The entry shim pushes each
such slot's address as a single permanent root (`raven_gc_push_root`)
before running the global initializers, so a value stored into a global
stays reachable for the whole program. The slots start zeroed, so a global
not yet initialized reads as null, which the collector skips.

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

### Deferred-thunk roots

A `defer expr` parks a thunk closure on a per-call defer frame and runs
it at the function's return (see `docs/v2/specs/defer.md`). A parked
thunk is a heap closure object, and a collection can fire between the
`raven_defer_push` that registers it and the `raven_defer_run_frame` that
runs it. The collector therefore treats every closure pointer in every
open defer frame as a root: the mark phase visits the shadow stack and
then walks the defer frames, marking each parked thunk so it and the
values it captures survive until it runs. The defer frame's lifetime is
tied to the call frame, so these roots are released as soon as the frame
runs.

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

```rust
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

The sweeper must visit every object allocated by its worker. Each thread keeps
an **all-objects list**: a `Vec<*mut ObjectHeader>` recording the base pointer
of every object it receives from `raven_gc_alloc`. The list is a side table,
not an intrusive header field, so the 16-byte `ObjectHeader` keeps the exact
shape pinned in `object-layout.md`; no object grows by a `next` word.

Each allocation appends its base pointer. Sweep walks the current thread's
vector, retaining objects stamped with its collection epoch and dropping freed
entries; the retained order does not matter.

Two per-thread counters back the trigger and the tests: `bytes_allocated`
tracks live object-body bytes (the bytes handed out by
`raven_gc_alloc`) and drives the collection threshold, and
`live_objects` counts live objects so tests can assert bounded liveness
deterministically without measuring flaky OS memory. Owned buffers are
freed with their object but are not separately metered; counting bodies
is enough to bound the live object set and pace collection.

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
| `TAG_STRUCT` | field slots at offset 16, gated by the per-type descriptor | The collector looks up a GC pointer bitmask by `header.cap` (the type id) and traces each of the `header.len` slots whose bit is set. An unregistered id is treated as having no pointers. Enum values reserve slot 0 for the discriminant. |

### Struct descriptors

Unlike the collection layouts, a struct cannot carry its pointer-kind
flags inline, because two structs sharing `TAG_STRUCT` have different
field shapes and the mask would bloat every instance. Instead the back
end registers a per-type descriptor with the collector:

```c
// type_id: the small integer id the back end assigns one per
//   monomorphic struct (or enum) type, stored in the value's header.cap.
// ptr_mask: bit i set means field slot i holds a traced GC pointer.
void raven_struct_register(uint32_t type_id, uint64_t ptr_mask);
```

```rust
pub extern "C" fn raven_struct_register(type_id: u32, ptr_mask: u64);
```

The back end emits one `raven_struct_register` call per type in the
program entry shim, before running `main`, so every struct or enum value
is traceable from its first allocation. Registering the same id twice is
harmless (the back end always supplies the same or a wider mask for a
given id; enum types union their variants' masks). The descriptors live in a
process-wide read-mostly map. Registration completes before goroutines start,
and every worker collection sees the same masks.

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

Sweep walks the initiating thread's all-objects list once. For each object:

* If the object's stamp differs from the current epoch, it is dead. The
  sweeper frees the object's **owned buffers** first, then the object body:
  * `String`: free `bytes` (`cap` bytes, align 1).
  * `List`: free `elements` (`cap * element_size` bytes,
    `element_align`).
  * `Map` / `Set`: free `buckets` (`bucket_count * entry_size` bytes,
    entry alignment 8).
  * `Closure`: free `captures` (`capture_size` bytes, `capture_align`).
  * `Box`: no separate buffer; the payload is inline in the body.
  Then free the object body itself. The body size and alignment are
  derived from the tag (fixed per kind, except `Box`, whose body size is
  `BOX_PAYLOAD_OFFSET + header.len`). The collector decrements
  `bytes_allocated` by the body size and `live_objects` by one for each
  freed object.
* If the object carries the current epoch, it survives and remains in the
  list. Its stamp is left in place; a later collection compares against a new
  epoch.

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

The threshold starts at the collection floor (default `1 MiB`). After a
collection it is reset to `max(floor, 2 * bytes_live_after_sweep)`, so a
program with a large live set collects less often while a program with a
small live set keeps a tight ceiling. This is the standard "allocation
high-water mark" heuristic; it bounds heap growth to a constant factor of
the live set.

### Threshold override for testing

The collection floor reads the `RAVEN_GC_THRESHOLD` environment variable
once at startup. When it is set to a positive byte count the floor (and so
the starting and post-collection threshold) is lowered or raised to that
value; an unset, empty, zero, or unparseable value leaves the `1 MiB`
default unchanged. Lowering it to a few hundred bytes makes a collection
fire after only a handful of allocations, so a test or a stress program
exercises the frame-based root paths on nearly every allocation without
allocating gigabytes. The override changes only collection pacing, never
correctness: a program prints the same output at any threshold. It is a
test and diagnostics hook; production runs leave it unset.

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
* **Parallel marking or sweeping.** Raven mutators run across multiple OS
  threads, but one thread performs each collection cycle after stopping the
  world.

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
* **Per-kind churn stress.** For each object kind (struct with a String
  field, nested struct, list of structs, hash `Map` and `Set`, closure
  captures, an `Any`/`Box` payload, and a parked deferred closure), a
  rooted instance is held across thousands of real allocations that force
  repeated collections, then its transitive contents are read back and
  asserted intact. A parked goroutine's heap value is held across heavy
  main-goroutine allocation and verified after it unblocks, exercising the
  scheduler's parked-root scan under pressure.

### End-to-end stress suite

The unit tests play codegen's role by hand. To check the real frame
emission, `examples/v2/gc_stress.rv` holds a live root of every heap object
kind across heavy allocation and reads each back, producing deterministic
output. The `gc_stress_survives_repeated_collections` smoke test compiles
and links it, then runs it many times under a low `RAVEN_GC_THRESHOLD` so a
collection fires every few allocations. Because a freed-live-object bug is
nondeterministic, repeating the run is what makes the test reliable: a
single pass can pass by luck.

This suite first surfaced, and the back end then fixed, a class of rooting
gaps where a heap temporary an rvalue builds is left unrooted across a
later allocation in the same expression:

* **Aggregate construction.** A struct, enum, or list literal allocates
  its body, then evaluates and stores fields or elements that may
  themselves allocate (a String-literal or interpolated field). The body
  is allocated first and rooted through a single shadow-stack slot
  (`raven_gc_push_root`/`raven_gc_pop_roots`) for the duration of the fill,
  so a partially built aggregate and the values already stored survive a
  collection.
* **Interpolation parts.** A string interpolation folds its parts with
  `raven_string_concat`, which allocates internally. Every part, literal
  text included, is bound to a rooted temp so it is not freed mid-concat.
* **Reflection wrapping.** `get_field` boxes a field into a fresh `Any`,
  then allocates the `Option<Any>` wrapper. The new `Any` is rooted across
  that wrapper allocation so it is not freed before it is stored.
* **Call arguments.** A call with more than one argument evaluates each
  argument in turn. A String-literal argument promotes to a fresh heap
  String at the call site (see [`lower_constant`]); a later argument's
  promotion, or an allocating runtime callee (for example
  `raven_string_concat`), would otherwise free an earlier String argument
  before the call reads it. Each freshly promoted String argument is rooted
  across the remaining argument evaluation and the call, then popped. This
  covers direct calls, string-runtime calls (`concat`, `==`), the
  `__str_concat` and `__str_substring` method intrinsics (whose runtime
  callees allocate), and the desugared `.set`/`.add`/`.insert` calls that
  build hash collections from String literals.

### Allocation-site rooting audit

Every codegen path that allocates was audited against the invariant that any
heap value live across an allocating call is reachable from the shadow stack
(a rooted local, the function root frame, or a `raven_gc_push_root` temp).
The status of each site:

| Allocation site | Builds a heap value held across a later allocation? | Rooting |
| --- | --- | --- |
| `MirRvalue::Use` of a `Const::Str` | No (single alloc, result stored to a rooted local) | Frame slot on store |
| Struct / enum / list literal | Yes (body, then allocating fields/elements) | Body rooted via temp slot for the fill |
| String interpolation | Yes (parts folded through allocating concat) | Every part bound to a rooted temp |
| `AnyGetField` (`get_field`) | Yes (boxed `Any`, then `Option` wrapper) | `Any` rooted across the wrapper alloc |
| Call with multiple args | Yes (String-literal arg across later arg / callee alloc) | Each promoted String arg rooted across the call |
| `__str_concat` / `__str_substring` intrinsics | Yes (String-literal source across the allocating callee) | Source rooted across the call |
| `AnyBox` (`to_any`) | No (single alloc; payload is a rooted local or scalar) | None needed |
| `AnyCast` (`cast`) | No (reads payload word, then one `Option` alloc; no prior heap temp) | None needed |
| `AnyTypeName` / `AnyFieldNames` | No (single alloc from a rooted `Any` operand) | None needed |
| `ClosureCreate` | No (captures are `Copy` of rooted locals; evaluated before the one alloc) | None needed |
| `DynCoerce` | No (single fat-pointer alloc; value is a rooted local) | None needed |
| `Cast` (numeric) | No (no allocation) | N/A |
| `marshal_reg_to_struct` (FFI return) | No (single struct alloc; fields are scalar words) | None needed |
| `print` / `panic` string arg | No (literal takes the static byte path; heap String is a rooted local) | None needed |

The randomized generator in `tests/gc_rooting_stress.rs` exercises the rooted
sites under `RAVEN_GC_THRESHOLD=1` across many program shapes; reverting any
of the fixes above makes it fail deterministically.
