# Concurrency: multi-core parallelism

## Goal

Run goroutines across several OS threads so independent goroutines execute in
parallel on multiple cores, replacing the current single-OS-thread cooperative
model. This is the epic tracked by #212; this document is its design and slice
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

- A fixed pool of OS worker threads (default: available parallelism), each
  running a scheduler loop that pulls a ready goroutine and runs it to its next
  yield point.
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

## Slice plan

Ordered so each slice is independently shippable and verifiable, and the
risky GC core is approached incrementally rather than in one rewrite.

1. **Shared heap + cross-thread root registry + serializing GC lock (one
   slice).** A first attempt split this into "move the heap to a global" alone,
   but that is not a safe unit: the moment more than one OS thread touches the
   runtime, a collection on thread B scans only B's thread-local roots and
   frees objects thread A still holds, a use-after-free. The runtime's own test
   harness is multi-threaded (cargo runs tests in parallel, and the GC tests
   spawn threads), so a global heap with thread-local roots corrupts the heap
   immediately, before any parallelism feature exists. **The heap, the root
   enumeration, and exclusive collection access are therefore coupled and must
   land together:**
   - the heap and object list move from `thread_local!` to a global,
     lock-protected structure;
   - every OS thread registers its shadow-stack root chain in a global
     registry, and a collection scans every registered chain (plus the parked
     goroutines and channel buffers the scheduler already exposes), not just the
     current thread's;
   - allocation and collection take a global GC lock so a collection has
     exclusive access to a quiescent heap (no concurrent mutation). For
     single-OS-thread production this lock is uncontended; for the concurrent
     test harness it is what makes the shared heap correct.

   Execution is still single-scheduler-thread; this slice only makes the
   collector correct under *any* threading. It is the foundation, and bigger
   than first scoped, but it is the minimal safe unit. Verified by the existing
   suite (including the multi-threaded GC tests) staying green.
2. **Safepoint protocol.** Replace the coarse global GC lock with a
   `gc_requested`/`parked` safepoint poll at allocation and scheduler
   back-edges, so threads cooperatively reach a stop instead of blocking on the
   lock. Still one scheduler thread, so "all threads parked" is trivially true;
   lands and tests the mechanism with no real concurrency.
3. **M:N scheduler (#237) + the safepoint GC engaged.** The OS thread pool, the
   shared ready queue, per-worker current-goroutine state, and the stop
   protocol now coordinating real threads. This is the parallelism slice and
   the one that needs the heaviest testing (parallel allocation stress, a
   channel ping-pong across workers, a GC-under-parallel-load stress).
4. **Cross-thread channels + sync primitives (#241).** Lock the channel,
   wake across workers, add mutex/waitgroup and a yielding sleep/timers.
5. **TLAB allocation + work-stealing (perf).** Remove the allocation-lock and
   ready-queue contention. Pure optimization; correctness unchanged.
6. **select (#239)** and **non-blocking IO (#240).** Build on the park/wake
   machinery; independent of the GC core.

Slice 1 is the foundation (and, per the finding above, larger than a heap
refactor alone); slice 3 is the high-risk core and gates everything after it.

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
