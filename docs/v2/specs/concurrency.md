# Concurrency Spec

## Goal

Give compiled Raven v2 programs goroutine-style concurrency: lightweight
green threads that the program spawns cheaply and that communicate over
typed channels. This document describes the first slice, which is a
cooperative scheduler on a single OS thread, and states precisely what is
deferred to later work.

## Scope of this slice

This slice ships:

* a cooperative scheduler that multiplexes many green threads
  (goroutines) onto one OS thread,
* the `spawn` surface form that starts a goroutine from a closure,
* unbuffered (rendezvous) and buffered channels with blocking `send` and
  `recv`,
* a `yield_now()` builtin for explicit cooperative yielding,
* a deadlock detector that panics when every goroutine is blocked,
* the garbage-collector change that scans the roots of every live
  goroutine, not only the one currently running.

Explicitly deferred (each a filed follow-up):

* multi-core parallelism (an M:N scheduler over several OS threads),
* a thread-safe and parallel collector (safepoints, per-thread
  allocation),
* `select` over multiple channels,
* non-blocking IO integration (a goroutine blocked in net/fs/http should
  yield instead of stalling the whole scheduler),
* sync primitives (mutex, waitgroup) and timers/sleep.

## Cooperative single-thread model

Exactly one goroutine runs at any instant. There is no preemption: a
goroutine runs until it reaches a cooperative yield point, at which it
suspends and the scheduler resumes another ready goroutine. The yield
points are:

* a channel `send` on a channel whose buffer is full (or an unbuffered
  channel with no waiting receiver),
* a channel `recv` on a channel whose buffer is empty (or an unbuffered
  channel with no waiting sender),
* an explicit `yield_now()`,
* a goroutine finishing its body (it switches away and never resumes).

Because only one goroutine ever runs at a time, no two goroutines touch
the heap or the collector concurrently. The collector therefore stays
single-threaded and lock-free in this slice. The single change the
collector needs is to find the roots of parked goroutines, described
below.

`main` is goroutine 0. It is created implicitly around the program entry
point; its stack is the ordinary OS thread stack, not a coroutine stack.
When `main` returns the program exits immediately, and any goroutines
still alive (ready or blocked) are abandoned without running to
completion, matching Go.

## Scheduler

The scheduler is a runtime global (single OS thread this slice, so a
plain `thread_local` with interior mutability is sound and lock-free). It
holds:

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
queue. Because the scheduler is cooperative and single-threaded, no lock
guards the channel state.

## GC integration: scanning every goroutine

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

Only channel `send`/`recv` and `yield_now()` are cooperative yield points
in this slice. A goroutine that blocks on a runtime IO call (a net read,
an fs read, an http request) blocks the whole scheduler, because those
calls are synchronous in the runtime. Making IO yield is a deferred
follow-up.

## Deadlock

If the scheduler is asked for the next goroutine to run and the ready
queue is empty while at least one goroutine is still alive and blocked,
no goroutine can ever make progress. The scheduler reports this as a
fatal runtime panic (`all goroutines are blocked: deadlock`) and exits,
the same failure Go reports.

## Deferred work

* M:N parallelism over multiple OS threads.
* Thread-safe and parallel collector (safepoints, per-thread allocation).
* `select` over multiple channels.
* Non-blocking IO integration.
* Sync primitives (mutex, waitgroup) and timers/sleep.
