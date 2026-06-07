# Concurrency Spec

## Goal

Give compiled Raven v2 programs goroutine-style concurrency: lightweight
green threads that the program spawns cheaply and that communicate over
typed channels, running in parallel across CPU cores. This document
describes the channel and goroutine semantics; the scheduler and
collector implementation that makes them parallel is specified in
[`mn-scheduler.md`](mn-scheduler.md).

## Scope

Raven ships:

* an **M:N scheduler** that multiplexes many green threads (goroutines)
  onto a pool of worker OS threads (one per core), so goroutines run in
  parallel,
* the `spawn` surface form that starts a goroutine from a closure,
* unbuffered (rendezvous) and buffered channels with blocking `send` and
  `recv`,
* a `yield_now()` builtin,
* a deadlock detector that panics when every goroutine is blocked,
* a **shared-heap stop-the-world collector** that runs alongside parallel
  goroutines: each OS thread sweeps its own heap, a cross-thread registry
  surfaces every thread's roots, and compiled code reaches safepoints
  (allocations and loop back-edges) where a collection can park it.

Still deferred (each a filed follow-up):

* `select` over multiple channels,
* non-blocking IO integration (a goroutine blocked in net/fs/http should
  yield its worker instead of holding it),
* sync primitives (mutex, waitgroup) and timers/sleep.

## Execution model

Goroutines run **in parallel**: the worker pool resumes several at once,
each on its own OS thread, so a spawned goroutine makes progress
concurrently with the code that spawned it rather than only when that
code yields. A goroutine suspends at a blocking channel op, at
`yield_now()`, or when its body finishes; the scheduler then runs another
ready goroutine on that worker. Because goroutines touch the shared heap
concurrently, the collector is multi-threaded and coordinates a
stop-the-world pause at safepoints (see [`mn-scheduler.md`](mn-scheduler.md)
and the GC integration below); it is not lock-free as an earlier
single-thread slice was.

`main` is goroutine 0. It is created implicitly around the program entry
point; its stack is the ordinary OS thread stack, not a coroutine stack.
When `main` returns the program exits immediately, and any goroutines
still alive (ready or blocked) are abandoned without running to
completion, matching Go.

## Scheduler

The scheduler is a runtime global behind a lock, shared by the main
thread and the worker pool (see [`mn-scheduler.md`](mn-scheduler.md) for
the worker loop, the per-worker root handling, and the safepoint
coordination this section predates). It holds:

* `goroutines`: the list of all live goroutines (running, ready, and
  blocked), each owning its suspended coroutine handle and its saved GC
  root chain (see GC integration),
* `ready`: a FIFO queue of goroutine ids that are runnable,
* `current`: the id of the goroutine running right now.

The core operation is **switch**: the running goroutine suspends back to
the scheduler loop, the scheduler picks the next ready goroutine, and
resumes it. A goroutine suspends for one of three reasons, encoded as the
value yielded to the scheduler: it yielded voluntarily and is still
ready, it blocked on a channel and must not be re-queued until woken, or
it finished and must be retired.

The scheduler loop, entered when a goroutine blocks or yields, runs the
next ready goroutine. If `main` (goroutine 0) blocks and no goroutine is
ready, but some goroutine is blocked, that is a deadlock: every goroutine
is asleep with no way to wake. The scheduler panics with
`all goroutines are blocked: deadlock` (Go prints
`all goroutines are asleep - deadlock!`; the comma form avoids the dash
this codebase forbids). A program that simply spawns nothing and uses no
channel never enters the scheduler loop at all, so the non-concurrent
path is unchanged.

## Green-thread stacks and context switching

Each goroutine other than `main` owns its own stack. Switching between a
goroutine and the scheduler saves the suspending side's registers and
stack pointer and restores the other side's. This slice uses the
`corosensei` crate (v0.3), a maintained stackful-coroutine library that
supports windows-msvc, linux, and macOS. Each goroutine is a
`corosensei::Coroutine` with its own stack. The scheduler resumes a
coroutine; a yield or block calls the coroutine's `Yielder` to suspend
back into the scheduler.

A goroutine body is a Raven closure (the existing `Closure` object and
the `raven_closure_*` ABI). `spawn` evaluates its operand to a closure
value of type `fun() -> Unit` and hands the closure pointer to
`raven_go_spawn`. The runtime creates a coroutine that, on first resume,
calls the closure through its `fn_ptr` with its capture `env`, then marks
the goroutine finished and yields the finished signal so the scheduler
retires it.

`corosensei` runs each coroutine on a guard-paged stack it allocates, so
a goroutine has a real, growable-bounded native stack and can call
arbitrary compiled Raven code, including further `spawn`s and channel
operations.

## Channels

A channel is a runtime object kept in a registry keyed by an opaque
integer id, the same pattern `std/net` uses for sockets. The id is what
crosses the FFI; Raven wraps it in a `Channel<T>` struct. A channel holds
a bounded queue of pointer-width slots (`i64`), a capacity, and two wait
lists (sender ids waiting to send, receiver ids waiting to receive).

A value of type `T` is stored as one pointer-width slot. Raven v2 values
are each either an immediate (Int, Bool, Char, Float bits) or a single GC
pointer (String, List, struct, closure), so every `T` fits one 8-byte
slot exactly as list elements do. This slice's channel API carries `Int`
payloads end to end in its tests; a channel of a GC-pointer `T` is sound
because the registry's buffered slots are exposed to the collector as
roots (see GC integration), so a value in flight is never collected.

`channel<T>()` creates an unbuffered channel (capacity 0, a rendezvous).
`channel_buffered<T>(cap)` creates a buffered channel of capacity `cap`.

`send(self, v)`:

* buffered with room: push `v`, wake one waiting receiver, return.
* unbuffered with a waiting receiver, or buffered full: park the current
  goroutine on the channel's sender wait list and switch away; when a
  receiver later takes a value and wakes this sender, it resumes and
  completes.
* unbuffered with a waiting receiver ready: hand the value directly and
  wake the receiver.

`recv(self) -> T`:

* a value is buffered: pop it, wake one waiting sender, return it.
* empty: park the current goroutine on the receiver wait list and switch
  away; resume when a sender delivers a value.

The implementation keeps the rule simple: an operation that cannot
complete immediately parks the goroutine and switches to the scheduler;
the counterpart operation wakes it by moving its id back onto the ready
queue. The scheduler state, including channel state, is guarded by the
scheduler lock; a goroutine woken while a worker is still resuming it is
re-queued by that worker (see [`mn-scheduler.md`](mn-scheduler.md)).

## GC integration: scanning every goroutine

The principle here still holds: the mark phase scans every parked
goroutine's roots and every buffered channel value, not just the running
goroutine's. The mechanism below (a single thread-local chain swapped on
each switch) describes the original single-thread slice; under the worker
pool each thread keeps its own registered root context and a running
goroutine's roots live in its worker's context, while parked goroutines'
saved chains are surfaced by the same extra-roots hook. See
[`mn-scheduler.md`](mn-scheduler.md) for the current per-worker handling.

The shadow-stack root mechanism
(`raven_gc_enter_frame`/`raven_gc_leave_frame`, and the per-slot
`raven_gc_push_root`/`raven_gc_pop_roots`) records the live GC root slots
of the running code. With one OS thread but many goroutines, each
goroutine has its own native stack and therefore its own root chain.

The runtime keeps the root chain in the existing thread-local
`ROOTS`/`FRAMES` cells. When the scheduler switches goroutines it swaps
the live thread-local chain out into the suspending goroutine's saved
slot and swaps the resuming goroutine's saved chain into the
thread-local. So at any instant the thread-local holds exactly the
running goroutine's roots, and every parked goroutine's roots sit in its
saved slot on the scheduler.

The collector's mark phase scans:

* the live thread-local chain (the running goroutine's roots),
* every parked goroutine's saved chain,
* the buffered values still held in every channel (a value sent but not
  yet received is reachable only through the channel),
* the existing defer-frame roots.

This is the one mandatory collector change. A collector that scanned only
the thread-local chain would free objects a parked goroutine still holds,
corrupting memory the moment that goroutine resumed. The scheduler
exposes an iterator over all parked chains and over all channel buffers;
the mark phase visits them alongside the thread-local chain.

When no scheduler has ever started (a program with no `spawn`), there are
no parked goroutines and no channels, so the mark phase visits exactly
the thread-local chain and the defer frames, identical to before this
change. The non-concurrent path is a strict no-op.

## Language surface

`spawn` is a prefix keyword that takes a closure operand of type
`fun() -> Unit`:

```
spawn(fun() -> Unit {
    // goroutine body
})
```

It lowers, like `defer`, to a runtime call: the operand becomes a closure
value and the call is `raven_go_spawn(closure)`. `spawn` is a statement
(its result is Unit).

Channels come from the bundled `std/sync` module:

```
import std/sync

let ch = channel()            // unbuffered Channel
let buf = channel_buffered(4) // buffered, capacity 4
ch.send(7)
let v = ch.recv()
```

`channel`/`channel_buffered` build a `Channel` wrapping the runtime id;
`send`/`recv` are methods on it. `yield_now()` is a free function in
`std/sync` that lowers to `raven_go_yield`.

## What blocks a goroutine

Channel `send`/`recv`, `yield_now()`, the allocator, and loop back-edges
are the points a goroutine suspends or can be parked for a collection. A
goroutine that blocks on a synchronous runtime IO call (a net read, an fs
read, an http request) holds its worker OS thread for the duration of the
call (other goroutines keep running on the other workers); making IO
release the worker is a deferred follow-up.

## Deadlock

If the scheduler is asked for the next goroutine to run and the ready
queue is empty while at least one goroutine is still alive and blocked,
no goroutine can ever make progress. The scheduler reports this as a
fatal runtime panic (`all goroutines are blocked: deadlock`) and exits,
the same failure Go reports.

## Deferred work

* `select` over multiple channels.
* Non-blocking IO integration (a goroutine in a blocking IO call should
  release its worker).
* Sync primitives (mutex, waitgroup) and timers/sleep.
