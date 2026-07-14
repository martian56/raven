# Concurrency: multi-core parallelism

> **Status:** Historical design record. The multi-core epic is implemented,
> but later refinements superseded some proposed details here. See
> [gc.md](gc.md) and [mn-scheduler.md](mn-scheduler.md) for the current
> per-thread heaps, stop-the-world coordination, and elastic worker pool.
> Sections labeled "current state" describe the pre-implementation baseline.

## Goal

Run goroutines across several OS threads so independent goroutines execute in
parallel on multiple cores, replacing the former single-OS-thread cooperative
model. This was the epic tracked by #212; this document is its design and slice
plan. It builds on the shipped cooperative scheduler (see
`docs/v2/specs/concurrency.md`) and is the prerequisite design for the filed
follow-ups: M:N scheduler (#237), thread-safe/parallel GC (#238), select
(#239), non-blocking IO (#240), and sync primitives/timers (#241).

The user-facing surface (`spawn`, `std/sync` channels, `yield_now`) does not
change. What changes is underneath: goroutines may now run truly concurrently,
so every shared runtime structure (the heap, channels, the scheduler queues)
becomes a concurrent structure, and the garbage collector must coordinate a
stop of all mutator threads.

## Current state (what this builds on)

- One OS thread runs an arbitrary number of goroutines, switching at the
  cooperative yield points (`send`/`recv`/`yield_now`).
- The GC is stop-the-world, single-threaded, mark-and-sweep. Its state (the
  heap object list, the shadow-stack root chain) lives in `thread_local!`
  cells, so the whole heap belongs to one OS thread. **Sharing GC objects
  across OS threads is undefined today.**
- The scheduler already swaps the thread-local root chain on a context switch
  and the mark phase already scans every parked goroutine's saved root chain
  plus every channel's buffered values. That root-enumeration machinery is the
  half of the GC that already generalizes; the heap ownership is the half that
  does not.

## The central problem: a shared heap and a coordinated GC

> This section describes the **shared-heap** model. Implementation attempts (see
> "What the implementation attempts proved" below) surfaced an alternative,
> **share-nothing**, and showed the model choice must be made first. Read this
> as the shared-heap branch, not a settled decision.

With multiple OS threads, two goroutines on different OS threads can hold the
same object (they pass it through a channel, or a captured closure env). So:

1. **The heap must be shared**, not thread-local. Allocation and the object
   list become concurrent structures.
2. **Collection must be coordinated.** A collector cannot mark and sweep a
   shared heap while other threads mutate it. Every mutator thread must reach a
   **safepoint** and stop before the collector runs, then resume after.

This is the hard, memory-safety-critical core of the epic. A wrong safepoint or
a missed root is a use-after-free that surfaces only under a race, the worst
class of bug to ship. The design therefore favors the simplest correct model
first (**stop-the-world with safepoints**) over a concurrent collector, and
de-risks the heap-ownership change before turning on parallelism.

### GC strategy: stop-the-world with safepoints

Not a concurrent collector. The collector still stops every mutator, marks, and
sweeps, exactly as today; the only new thing is *coordinating the stop across
OS threads*.

- A global `gc_requested` flag (an atomic). A thread that wants to collect (it
  hit the allocation floor) sets it and waits.
- Every mutator thread polls the flag at **safepoints**: on allocation, and on
  scheduler back-edges (every goroutine switch and every loop back-edge the
  back end emits a poll for). At a safepoint, if the flag is set, the thread
  parks itself at a known-safe point (no half-built object, roots all on the
  shadow stack) and increments a `parked` counter.
- When `parked == total mutator threads`, the requesting thread (the sole
  collector) runs mark/sweep over the shared heap, enumerating roots from
  **every** OS thread's shadow stack and every parked goroutine's saved chain
  and every channel buffer (the existing iterator, widened to all threads).
- The collector clears the flag and releases the parked threads.

This is the standard JVM/Go-style stop-the-world parallel-mutator collector. It
gives real multi-core parallelism for mutator work; only the GC pause is
serial. A concurrent or generational collector is a later optimization, out of
scope for this epic.

### Allocation

Today allocation is a thread-local bump into the object list. Shared, the
options are (in increasing complexity):

1. A single global allocator lock. Correct, simple, contended. The starting
   point.
2. Per-thread allocation buffers (TLABs): each OS thread bump-allocates from a
   thread-local chunk and only takes the global lock to refill. The standard
   way to remove allocation contention. A perf slice after correctness.

## Scheduler: M:N over an OS thread pool (#237)

- A baseline pool of OS worker threads (default: available parallelism), each
  running a scheduler loop that pulls a ready goroutine and runs it to its next
  yield point. The shipped runtime adds bounded temporary replacements around
  blocking syscalls and retires excess workers after the burst.
- A shared ready queue (start) or per-worker queues with work-stealing (perf
  follow-up). Per-OS-thread "current goroutine" state replaces the single
  global current.
- Context switch is unchanged per goroutine (save/restore stack + root chain);
  what is new is that a goroutine may resume on a *different* OS thread than it
  parked on, so nothing may assume a goroutine stays pinned to an OS thread
  (no OS-thread-local goroutine state survives a yield).
- Coordination with the GC: a worker about to block (empty queue) and a worker
  at a safepoint both participate in the stop protocol, so a collection can
  always make progress.

## Channels and sync (cross-thread)

Channels become cross-thread queues: `send`/`recv` need a lock and a wait list
of parked goroutines that the scheduler can wake on any worker. The existing
single-thread channel becomes a `Mutex`-guarded queue plus condition signaling
into the scheduler. `select` (#239) parks a goroutine on several wait lists at
once. Sync primitives (#241, mutex/waitgroup) and a yielding `sleep`/timers
build on the same park/wake machinery.

## Non-blocking IO (#240)

Today a goroutine blocked in a synchronous runtime IO call (net/fs/http) blocks
its OS thread. With a pool this is less catastrophic (other workers run), but a
worker is still lost for the call's duration. Proper integration is an event
loop / IO reactor that parks the goroutine and wakes it on completion. This is
the last slice and is independent of the GC core.

## What the implementation attempts proved

Three attempts de-risked the foundation and corrected the plan. Each ruled out
an approach that looked reasonable on paper, so they are worth recording.

1. **The heap, the cross-thread root registry, and exclusive collection access
   are one coupled unit, not separable slices.** Moving the heap to a global
   while roots stayed thread-local was tried and corrupted memory immediately
   (`STATUS_HEAP_CORRUPTION`): a collection on one thread scanned only its own
   roots and freed objects another thread held. The test harness alone is
   multi-threaded enough to trigger it.
2. **The hard primitives build and test cleanly in isolation.** The
   stop-the-world coordinator (#350) and the cross-thread root registry (#351)
   are merged, each stress-tested standalone (exclusivity, liveness under churn,
   cross-context enumeration). They are correct building blocks.
3. **Safepoint stop-the-world only works for cooperative mutators, so the GC
   cross-thread wiring cannot land before the M:N scheduler.** Wiring the
   coordinator + registry into the collector (PR #352, closed) deadlocked CI:
   `collect()` waits for every registered thread to reach a safepoint (a poll at
   allocation), but the Rust test harness's reused runner threads register on
   first GC touch and then run non-allocating code that never polls, so a
   collection waits forever. This is not a coordinator bug; a safepoint protocol
   is only meaningful when every mutator has poll points, which only **compiled
   Raven goroutine threads** do. The GC wiring can therefore only be validated
   against real goroutines on an OS-thread pool, so **it must land together with
   the scheduler, not before it.**

A useful artifact from attempt 3: the object header is a pinned 16-byte layout,
so the one-bit mark could not widen. Reinterpreting `gc_bits` as a per-collection
**mark epoch** (an object is marked when `gc_bits == current_epoch`) keeps a
stale mark left by one thread's collection of a shared object from confusing
another thread's later sweep, and removes the clear-marks pass. That commit is
preserved on the closed branch `feat/gc-cross-thread-collection`.

## Two models, and the fork to decide first

Both viable parallelism models have a hard part; neither is a quick win.

- **Shared heap (Go-style).** Goroutines on any worker share objects directly
  (channels and captured closures pass pointers). Needs the cross-thread,
  safepoint-coordinated collector above. Hard part: the memory-safety-critical
  GC, validatable only once cooperative goroutine threads exist.
- **Share-nothing (worker-isolated heaps).** Each worker keeps its own heap and
  cooperative scheduler, today's runtime replicated per worker, so the GC stays
  per-worker and unchanged. Hard part: transferring a value across workers, a
  spawn closure with its captures and any heap value sent on a cross-worker
  channel, must be deep-copied from one heap to another, and cross-worker
  channels need thread-safe queues plus cross-worker wakeups. For the current
  channel surface (which carries `Int`) the channel copy is trivial, but `spawn`
  already hands a closure to another worker, so the closure transfer is the
  first real cost.

Neither sidesteps concurrency's intrinsic difficulty: shared-heap pays it in
the collector, share-nothing pays it in cross-worker transfer. The choice is a
language-semantics decision, do channels and closures share or copy across
cores, and it shapes every later slice, so it is decided first.

## Slice plan (corrected)

The original plan put a GC foundation first and the scheduler third; attempt 3
showed that order is impossible (the GC wiring is untestable without the
scheduler). The corrected order pairs them.

1. **Decide the model** (shared-heap vs share-nothing). Everything else depends
   on it.
2. **M:N scheduler core and GC handling, together.** An OS-thread worker pool, a
   ready queue, per-worker current-goroutine state, and goroutines actually
   running on multiple OS threads, *with* the GC made correct for the chosen
   model (shared-heap: the merged coordinator + registry + epoch marking
   engaged, now validatable by real cooperative goroutine threads; share-nothing:
   per-worker heaps plus a cross-worker value-transfer path). The high-risk core.
   Verified end to end by a compiled Raven program that spawns goroutines which
   run in parallel, and a GC-under-parallel-load stress built from goroutines
   (not the Rust test harness).
3. **Cross-worker channels and sync primitives (#241).** Thread-safe channels,
   cross-worker wake, mutex/waitgroup, a yielding sleep/timers.
4. **Work-stealing / load balancing and TLAB allocation (perf).** Pure
   optimization once correctness holds.
5. **select (#239)** and **non-blocking IO (#240).**

The merged primitives (#350, #351) and the epoch insight feed slice 2 under the
shared-heap model; under share-nothing they are set aside and the per-worker GC
is reused unchanged.

## Finding: the safepoint model spans the compiler, and the system is one unit

Building toward slice 2 surfaced that a correct shared-object collector needs
more than a runtime coordinator, and that the parts cannot be validated apart.

**A collection may only run when every thread that could hold a live GC pointer
is at a point where that pointer is on its shadow stack.** Between such points,
compiled code legitimately keeps a GC pointer only in a register (codegen roots
across safepoints, not across every instruction). If a collection on one thread
frees an object another thread holds only in a register, that is a
use-after-free. This is exactly why real collectors stop the world at
**safepoints**: points the back end marks where all live GC pointers are
spilled to rooted slots.

Two consequences:

- **The compiler must emit safepoint polls**, at allocations (already a runtime
  call) and at loop back-edges (so a long non-allocating loop still reaches a
  safepoint and parks). A poll is a cheap load of a global flag plus a branch
  that calls a park routine only when a collection is pending. This is a
  codegen change, not just a runtime one.
- **A thread state distinguishes "in Raven" from "in native".** A thread inside
  a runtime call or blocked (or, in the runtime's own test harness, a Rust
  thread that never runs compiled Raven) is already at a complete, stable
  shadow stack and is scanned without waiting; only "in Raven" threads must
  reach a poll and park. This is what keeps idle/native threads from
  deadlocking a collection. The merged unsafe-region coordinator (#354) is the
  "in native, briefly mutating the shadow stack" half of this; the "in Raven,
  reach a safepoint" half is the codegen poll plus a park.

And the parts are interdependent: the multi-threaded collector is only
exercisable by **parallel compiled goroutines**, which need the M:N scheduler;
the scheduler's goroutines share objects, which need the collector; the
collector needs the codegen safepoint polls. None can be validated in isolation.
So slice 2 is genuinely **one interdependent build**, compiler safepoint polls +
runtime coordinator with thread state + shared heap + worker pool, brought up
together and validated as a whole by a parallel-goroutine program. The merged
primitives are correct sub-components, but the integration is a sustained,
dedicated effort, not a sequence of independently shippable PRs.

## Verification

The GC stress tests already in the suite (allocate under load, hold values
across collections) are the model; the epic adds parallel versions:

- Many goroutines on the pool each allocating in a tight loop, forcing
  collections under parallel mutation, then asserting every held value is
  intact (a wrong safepoint or missed root corrupts these).
- A channel ping-pong bouncing a shared object across workers many times.
- A determinism check: a parallel sum/map reduction produces the same result
  as the serial one.

Every slice keeps the existing single-threaded suite green (the no-`spawn` path
must stay a strict no-op). CI is Linux + Windows; both run the parallel
stresses.

## Out of scope

- A concurrent or generational collector (this epic is stop-the-world with
  safepoints; concurrency is a later optimization).
- Async/await as a language feature (goroutines are the concurrency model).
- Distributed or multi-process concurrency.
