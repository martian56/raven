# M:N scheduler implementation blueprint

The final piece of the Go-style multi-core parallelism epic (#212/#237). The GC
half is complete and merged (registry #351, coordinator #354+#357, epoch #356,
collector wiring #358, back-end safepoint markers #359); this document is the
precise design for the scheduler that runs goroutines on multiple OS threads on
top of it. It exists because the scheduler is the most concurrency-bug-prone
code in the project and must be built deliberately, not improvised.

## Where the current scheduler stands (`raven-runtime/src/sched.rs`)

- A `thread_local! SCHED: RefCell<Scheduler>`: one OS thread, lock-free.
- Goroutine 0 is `main`, running on the OS thread stack with no coroutine; it
  drives the scheduler loop directly when it blocks.
- Spawned goroutines are `corosensei::Coroutine` values with their own stacks.
- A goroutine suspends to the run loop via its `Yielder` (`Suspend::{Yielded,
  Blocked, Finished}`); the run loop resumes the next ready goroutine.
- Channels live in `SCHED` with `send_waiters`/`recv_waiters` lists.
- `switch_root_chain` keeps the GC's thread-local root chain holding exactly the
  running goroutine's roots; every parked goroutine's roots sit on its struct and
  are surfaced to the collector by the `extra_roots` hook.

## Target model

A fixed pool of worker OS threads (default `available_parallelism`) plus the
main thread. Goroutines (including ones that migrate) run on whichever worker
pulls them; `corosensei` coroutines are `Send`, so a coroutine may resume on a
different thread than it last ran on, as long as only one thread resumes a given
coroutine at a time (guaranteed by popping it from the ready queue exclusively).

The GC is already correct for this: each OS thread keeps its own heap and sweeps
only that heap, the registry makes a collection scan every thread's roots, and
epochs keep per-thread sweeps from tripping over cross-thread marks. A migrated
goroutine's objects simply spread across worker heaps and stay live through the
registry.

## Pieces to build

### 1. Global scheduler state

Move `SCHED` from a `thread_local` to a global `Mutex<Scheduler>` (via
`LazyLock` or a const `Mutex`). The `Scheduler` holds `Coroutine`s whose closures
capture a raw `env: *mut u8` (a GC heap pointer), so it is not auto-`Send`; wrap
it in a newtype with `unsafe impl Send`, justified because the raw pointer is a
heap object that is safe to touch from any thread. The lock is **never held
across a coroutine resume or suspend** (today's code already releases the borrow
before suspending; preserve that exactly, or it deadlocks).

### 2. Per-worker yielder

Remove `yielder` from `Scheduler` and make it a `thread_local! CURRENT_YIELDER:
Cell<*const GoYielder>`. Each worker resuming a coroutine publishes that
coroutine's yielder in its own thread-local; `suspend_current` reads the thread
local. This is also what makes `Scheduler` `Send` (no raw pointer in the shared
struct). Do steps 1+2 together as one behavior-preserving refactor (still one
worker: the main thread), validated by the existing goroutine golden examples.

### 3. Worker pool

Spawn the pool lazily on the first `raven_go_spawn`. Each worker loop:

```
loop {
    let id = { lock SCHED; pop ready, or wait on a "work available" condvar };
    if shutting_down { break }
    raven_gc_enter_running();           // in-Raven for the GC
    switch_root_chain(into this worker's context, id);
    let result = resume coroutine id;   // lock NOT held
    switch_root_chain(out, id);
    raven_gc_exit_running();
    lock SCHED; handle result (re-queue / park / retire); notify if work freed;
}
```

A worker bracketing each goroutine with `enter_running`/`exit_running` is what
lets the collector park it: the goroutine's safepoint polls (allocation, and the
loop-back-edge polls below) park the worker OS thread during a collection. An
idle worker waiting on the condvar is *not* in the running set, so it never
blocks a collection.

### 4. Main thread

`main` is goroutine 0 on the OS stack and cannot suspend a coroutine. It is
already `enter_running` (back-end program entry). When it blocks on a channel it
**parks on a condvar** (blocking the main OS thread) until a counterpart signals
it; a worker that frees main re-queues it and signals that condvar. When `main`
returns, the program exits (process exit tears down workers; suspended coroutines
are leaked exactly as `Scheduler::drop` does today). So a goroutine still runs
in parallel with `main` rather than only when `main` yields, which is the point.

### 5. Channels across workers

Keep the wait-list design but make send/recv correct under the global lock with
two distinct park/wake mechanisms:

- A goroutine blocks by adding itself to the wait list, releasing the lock, then
  `suspend_current(Blocked)`; the worker picks another goroutine. Crucially, it
  must add itself to the wait list **before** releasing the lock, so a sender
  that takes the lock next either rendezvous-transfers and re-queues it or leaves
  it parked to be woken later. There is no lost wakeup because the waiter is
  registered under the lock before it suspends.
- `main` blocks by parking on its condvar (it cannot suspend a coroutine).

A waker re-queues a blocked goroutine (and notifies a worker via the work
condvar) or signals main's condvar, depending on which the waiter is. The
unbuffered rendezvous still hands the value directly from sender to the woken
receiver.

### 6. Loop-back-edge safepoints (back end)

The deferred half of #359: the back end must emit a `raven_gc_safepoint()` call
at loop back-edges, so a goroutine in a long non-allocating loop still reaches a
safepoint and parks for a collection (allocation already polls). Without this a
compute-bound goroutine could stall a collection indefinitely. Emit it at the
back-edge of each MIR loop; it is a single atomic load when no collection is
pending, so the cost is small and it is behavior-preserving single-threaded.

### 7. Deadlock detection

Replace the single-thread "ready empty and current blocked" check with: all
goroutines blocked and no worker has runnable work *and* main is parked. Track a
running/parked count; when every goroutine is parked on a channel and the ready
queue is empty, report the deadlock as today.

## The subtle hazards to get right (where bugs will hide)

- **Lock across suspend/resume** -> deadlock. The lock is held only for queue and
  channel bookkeeping, never across a `coro.resume` or `yielder.suspend`.
- **Lost wakeups** on channel block -> a waiter must register under the lock
  before it releases and suspends; the work condvar must be notified under the
  lock after a re-queue.
- **A worker parking at a safepoint while holding the SCHED lock** -> the
  collector cannot scan. Never hold the lock across goroutine execution (where
  safepoints fire); channel ops release before suspend.
- **Cross-thread coroutine resume** is sound only because the ready queue hands a
  coroutine to exactly one worker at a time; never resume the same id from two
  workers.
- **`extra_roots` under stop-the-world**: the hook reads the global scheduler;
  during a collection all workers are parked, so the scheduler is quiescent, but
  the hook still takes the lock, so it must run on the collecting thread, which
  is not holding the lock (it is in `mark`).

## Validation (compiled Raven, not the Rust harness)

The Rust test harness threads never run compiled Raven and cannot poll
safepoints, so the parallel scheduler is validated by **compiled programs**:

- A parallel reduction (spawn N goroutines that each sum a slice, combine via a
  channel) producing the same result as the serial version, run repeatedly.
- A channel ping-pong bouncing a value across goroutines many times.
- A GC-under-parallel-load stress: many goroutines allocating in tight loops
  while a collection fires, asserting every held value survives (a missed root or
  a wrong safepoint corrupts this).
- The existing single-`spawn` golden examples stay green (the no-pool path when
  available parallelism is 1, or a single goroutine, must match today).

## Build order

1. Steps 1+2 (global scheduler + per-worker yielder), behavior-preserving,
   merge. **Done (PR #361).**
2. Steps 3+4+5+7 together (the pool, main-on-condvar, cross-worker channels,
   deadlock), with the loop-back-edge safepoints (step 6) folded in so they are
   not pure overhead in a single-threaded program. This is the irreducible
   concurrent core: it is all-or-nothing (a half-built pool runs nothing), so it
   lands as one reviewed, heavily-stressed change, validated end to end by the
   compiled programs above.

## Implementation sketch for the concurrent core (stage 2)

Worked out while building stage 1; capture so the core is built from a concrete
plan. The state below is what the pieces actually need beyond stage 1.

### New state

- `thread_local CURRENT_GOROUTINE: Cell<usize>` replaces the single
  `Scheduler::current`. Each worker (and the main thread) records the id it is
  running; main's is `0`. `current` was used by the deadlock check, the channel
  ops (who is blocking), and `extra_roots`; all become "this thread's current".
- In the locked `Scheduler`: a `running: HashSet<usize>` of goroutines a worker
  is currently resuming, and `main_blocked: bool`.
- Two condvars paired with the `SCHED` mutex: `WORK_CV` (a worker waits on it
  when the ready queue is empty) and `MAIN_CV` (main waits on it when blocked).
- A `shutdown: bool` and the worker `JoinHandle`s (or detach and let process
  exit reap them, matching today's leak-on-exit).

### Worker loop

```
loop {
    let id = { lock; loop { if shutdown { return }
                            if let Some(id) = ready.pop_front() { break id }
                            guard = WORK_CV.wait(guard) } };
    CURRENT_GOROUTINE.set(id);
    install_root_chain(take saved roots of id);   // into THIS worker's RootContext
    { lock; running.insert(id) }
    raven_gc_enter_running();
    let coro = { lock; goroutines[id].coro.take() };
    let result = coro.resume(());                 // lock NOT held; safepoints fire here
    { lock; goroutines[id].coro = Some(coro) }
    raven_gc_exit_running();
    let saved = take_root_chain();
    { lock; running.remove(&id); goroutines[id].roots = saved;
            match result { Yielded => { ready.push_back(id); WORK_CV.notify_one() }
                           Blocked => {}
                           Finished => { goroutines.remove(&id) } } }
    CURRENT_GOROUTINE.set(0);
}
```

`running` plus the per-worker `RootContext` is why `extra_roots` changes: it must
surface the saved roots of every goroutine **not** in `running` (a running
goroutine's live roots are in its worker's registered context, already scanned),
and it must run on the collecting thread, which holds neither the SCHED lock nor
another worker's context (all are parked at a safepoint during the stop).

### main and goroutine blocking

`raven_go_yield` from main becomes a no-op (or `std::thread::yield_now`): goroutines
already run in parallel on workers, so main need not drive them. A spawned
goroutine's yield still re-queues + `suspend(Yielded)`.

A channel op that must block, with `me = CURRENT_GOROUTINE`:
- register `me` on the wait list **under the lock**;
- if `me == 0` (main): `main_blocked = true`; `while still parked { guard =
  MAIN_CV.wait(guard) }`; on wake clear it. (Blocks the main OS thread.)
- else (goroutine): drop the lock, then `suspend_current(Blocked)`; the worker
  picks another goroutine. Registering before dropping the lock is what prevents
  the lost wakeup.

A waker holds the lock and: re-queues a blocked goroutine (`ready.push_back` +
`WORK_CV.notify_one`) **or**, if the waiter is main, sets it runnable and
`MAIN_CV.notify_one`. The unbuffered rendezvous hands the value straight to the
woken receiver.

### Lazy pool start and deadlock

Spawn the pool on the first `raven_go_spawn` (as `started` is set today). Deadlock
is: ready queue empty, no goroutine in `running`, and every live goroutine
(including main) blocked, report and exit as today, but detected by a worker (or
main on its condvar wake) rather than the single run loop.
