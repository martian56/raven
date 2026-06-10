//! M:N green-thread scheduler and channels for compiled Raven v2 programs.
//!
//! See `docs/v2/specs/mn-scheduler.md` for the model. Goroutines are
//! `corosensei` coroutines multiplexed onto a pool of worker OS threads (one
//! per available core), so they run in parallel; the main thread is goroutine 0
//! and runs on its own stack. A goroutine yields at a full/empty channel, at an
//! explicit `raven_go_yield`, and when its body finishes. The scheduler state
//! is global behind a lock; a worker brackets each goroutine it runs with the
//! collector's running-state calls so a stop-the-world parks it at a safepoint.
//! The mark phase scans the saved root chain of every parked goroutine (running
//! goroutines' roots live in their worker's registered context) and every
//! buffered channel value (see `crate::gc`).

use crate::gc::{
    for_each_slot_in, install_root_chain, raven_gc_enter_running, raven_gc_exit_running,
    set_extra_roots_hook, take_root_chain, RootSlot, SavedRoots,
};
use crate::object::Closure;
use corosensei::{Coroutine, CoroutineResult, Yielder};
use std::cell::Cell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, LazyLock, Mutex};

/// Why a goroutine suspended back to the scheduler. Yielded by the
/// coroutine body to the resume loop.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Suspend {
    /// Voluntary yield; the goroutine stays ready and is re-queued.
    Yielded,
    /// Blocked on a channel; the goroutine must not be re-queued until a
    /// counterpart wakes it.
    Blocked,
    /// The body finished; retire the goroutine.
    Finished,
}

/// The yielder a running goroutine uses to suspend back to the scheduler.
/// `corosensei` resumes a coroutine with `()` and the coroutine yields a
/// `Suspend`.
type GoYielder = Yielder<(), Suspend>;

/// A live goroutine: its suspended coroutine handle (None for the main
/// goroutine, which runs on the OS thread stack, and None once finished),
/// and its saved GC root chain while it is parked.
struct Goroutine {
    coro: Option<Coroutine<(), Suspend, ()>>,
    roots: SavedRoots,
}

/// A channel: a bounded queue of pointer-width value slots plus the wait
/// lists of goroutine ids blocked on it.
struct Channel {
    cap: usize,
    queue: VecDeque<i64>,
    send_waiters: VecDeque<usize>,
    recv_waiters: VecDeque<usize>,
}

/// A wait group: an outstanding-work counter plus the ids waiting for it to
/// reach zero. `add` adjusts the count; the adjustment that brings it to zero
/// wakes every waiter. Mirrors Go's `sync.WaitGroup`.
struct WaitGroup {
    count: i64,
    waiters: Vec<usize>,
}

/// The scheduler state, held behind a global lock (see [`SCHED`]) so the main
/// thread and the worker pool share one ready queue and channel set.
struct Scheduler {
    goroutines: HashMap<usize, Goroutine>,
    ready: VecDeque<usize>,
    /// Goroutines (and main, id 0) that have committed to blocking on a channel:
    /// registered on a wait list and waiting to be woken. A waker removes the id
    /// here; the deadlock check counts these, and the worker that was resuming a
    /// since-woken goroutine re-queues it when it finds the id gone from here.
    blocked: HashSet<usize>,
    /// Goroutines a worker has claimed and is resuming, set atomically with the
    /// pop from `ready`. Lets the deadlock check tell "about to run" from
    /// "blocked", and keeps a claimed goroutine visible so a quiescence check or
    /// the test reset cannot clear it out from under its worker.
    running: HashSet<usize>,
    next_id: usize,
    channels: HashMap<i64, Channel>,
    next_chan: i64,
    wait_groups: HashMap<i64, WaitGroup>,
    next_wg: i64,
    /// A `select` set: the channel ids a goroutine is selecting over, built up
    /// by `raven_select_add` and consumed by `raven_select_recv`.
    select_sets: HashMap<i64, Vec<i64>>,
    next_select: i64,
    /// Set once the first goroutine is spawned. Until then the program is
    /// strictly non-concurrent and the worker pool is never started.
    started: bool,
    /// Set to ask the worker pool to exit. Used only by the test harness to
    /// drain workers between isolated scheduler tests; production never sets it
    /// (the process exits when main returns).
    shutdown: bool,
}

/// The global scheduler, shared by the main thread and (once it lands) the
/// worker pool. Holding a `Coroutine` whose closure captures a raw `env`
/// pointer makes `Scheduler` non-`Send`, but that pointer is a GC heap object
/// safe to touch from any thread and each coroutine is resumed by one thread at
/// a time, so sending the scheduler across threads is sound.
struct SchedState(Scheduler);
// SAFETY: see the comment above; the raw pointers inside are thread-safe heap
// objects and the scheduler serializes access to each coroutine.
unsafe impl Send for SchedState {}

static SCHED: LazyLock<Mutex<SchedState>> =
    LazyLock::new(|| Mutex::new(SchedState(Scheduler::new())));

/// Signaled when a goroutine becomes ready, so an idle worker wakes to run it.
static WORK_CV: Condvar = Condvar::new();

/// Signaled when the main goroutine is unblocked. Main cannot suspend a
/// coroutine, so it parks the main OS thread on this condvar instead.
static MAIN_CV: Condvar = Condvar::new();

/// Whether the worker pool has been started (lazily, on the first spawn).
static POOL_STARTED: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// The yielder of the coroutine this OS thread is currently running, so
    /// `suspend_current` can reach it. Per-thread because each worker runs a
    /// different goroutine; null when the thread is between goroutines or
    /// running the main goroutine (which suspends via a condvar, not a yielder).
    static CURRENT_YIELDER: Cell<*const GoYielder> = const { Cell::new(std::ptr::null()) };

    /// The goroutine id this OS thread is currently running. The main thread is
    /// always 0; a worker holds the id it resumed (and 0 while between
    /// goroutines). Replaces the single `Scheduler::current` now that several
    /// threads run goroutines at once.
    static CURRENT_GOROUTINE: Cell<usize> = const { Cell::new(0) };

    /// The value the most recent `raven_select_recv` took from the ready
    /// channel, fetched by the immediately following `raven_select_value`. The
    /// goroutine does not yield between the two calls, so no other goroutine
    /// running on this worker can overwrite it in between.
    static SELECT_VALUE: Cell<i64> = const { Cell::new(0) };
}

/// Run `f` with exclusive access to the scheduler. The lock is never held
/// across a coroutine resume or suspend (that would deadlock); callers take it
/// only for queue and channel bookkeeping.
fn with_sched<R>(f: impl FnOnce(&mut Scheduler) -> R) -> R {
    let mut guard = SCHED.lock().unwrap();
    f(&mut guard.0)
}

impl Drop for Scheduler {
    fn drop(&mut self) {
        // A goroutine still suspended at teardown (the program exited while
        // it was parked, which Go also abandons) holds a coroutine whose
        // stack has live `extern "C"` Raven frames. Dropping it would force
        // corosensei to unwind that stack, and unwinding through `extern
        // "C"` frames aborts. Leak the suspended coroutines instead; the OS
        // reclaims their stacks at process exit.
        for (_, g) in self.goroutines.drain() {
            if let Some(coro) = g.coro {
                std::mem::forget(coro);
            }
        }
    }
}

impl Scheduler {
    fn new() -> Self {
        let mut goroutines = HashMap::new();
        // Goroutine 0 is main, running on the OS thread stack with no
        // coroutine handle.
        goroutines.insert(
            0,
            Goroutine {
                coro: None,
                roots: (Vec::new(), Vec::new()),
            },
        );
        Scheduler {
            goroutines,
            ready: VecDeque::new(),
            blocked: HashSet::new(),
            running: HashSet::new(),
            next_id: 1,
            channels: HashMap::new(),
            next_chan: 1,
            wait_groups: HashMap::new(),
            next_wg: 1,
            select_sets: HashMap::new(),
            next_select: 1,
            started: false,
            shutdown: false,
        }
    }
}

/// Visitor for the collector: surface every parked goroutine's root chain as
/// roots.
///
/// Buffered channel values are deliberately not scanned. A channel carries
/// plain `Int` payloads (the `std/sync` Channel API is `Int`-typed), so handing
/// the collector the address of a queue slot made it read the integer value and
/// dereference it as an object pointer, writing GC bits into arbitrary memory.
/// If channels ever carry GC pointers, the queue will need a per-channel flag
/// and the scan reinstated only for those.
fn extra_roots(visit: &mut dyn FnMut(RootSlot)) {
    with_sched(|sched| {
        for g in sched.goroutines.values() {
            // A running goroutine's saved chain is empty (taken into its worker's
            // registered context, scanned via the registry), so scanning it here
            // is a harmless no-op; a parked goroutine's saved chain is
            // authoritative. Either way, scan the saved chain.
            for_each_slot_in(&g.roots, visit);
        }
    });
}

/// Spawn a goroutine running the closure `closure` (a `fun() -> Unit`).
///
/// Creates a coroutine with its own stack that, on first resume, calls
/// the closure through its function pointer and capture env, then yields
/// `Finished`. The goroutine is queued ready; it does not run until the
/// current goroutine yields or blocks.
///
/// # Safety
///
/// `closure` must be a live `Closure` produced by `raven_closure_new`
/// whose lifted body is a `fun() -> Unit`.
#[no_mangle]
pub extern "C" fn raven_go_spawn(closure: *mut Closure) {
    if closure.is_null() {
        return;
    }
    let fn_ptr = unsafe { (*closure).fn_ptr };
    let env = unsafe { (*closure).captures };
    let fn_addr = fn_ptr as usize;
    let env_addr = env as usize;

    let coro = Coroutine::new(move |yielder: &GoYielder, _input: ()| {
        // Publish this goroutine's yielder so its channel ops and
        // `yield_now` can suspend back to the scheduler.
        CURRENT_YIELDER.with(|y| y.set(yielder as *const GoYielder));
        // The closure body is `extern "C" fn(env)`.
        // SAFETY: spawn's contract guarantees a `fun() -> Unit` lifted
        // body taking the capture env.
        let body: extern "C" fn(*mut u8) = unsafe { std::mem::transmute(fn_addr as *const u8) };
        body(env_addr as *mut u8);
        yielder.suspend(Suspend::Finished);
    });

    with_sched(|sched| {
        if !sched.started {
            sched.started = true;
            set_extra_roots_hook(extra_roots);
        }
        let id = sched.next_id;
        sched.next_id += 1;
        sched.goroutines.insert(
            id,
            Goroutine {
                coro: Some(coro),
                roots: (Vec::new(), Vec::new()),
            },
        );
        sched.ready.push_back(id);
    });
    // Start the worker pool on the first spawn, then wake a worker to run the
    // new goroutine (parallel with whatever the spawner does next).
    ensure_pool();
    WORK_CV.notify_one();
}

/// Number of worker OS threads: one per available core. Workers run goroutines
/// in parallel with the main thread; main is not a worker.
fn worker_count() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

/// Start the worker pool once, on the first spawn. Subsequent calls are a cheap
/// no-op. Workers run for the life of the process; the program exits when main
/// returns, which tears them down (matching the leak-on-exit of suspended
/// goroutines).
fn ensure_pool() {
    if POOL_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    for _ in 0..worker_count() {
        std::thread::spawn(worker_loop);
    }
}

/// Whether every live goroutine (including main) is blocked with nothing ready
/// or running: a deadlock, reported as Go does.
fn is_deadlocked(sched: &Scheduler) -> bool {
    sched.ready.is_empty()
        && sched.running.is_empty()
        && sched.blocked.len() == sched.goroutines.len()
}

/// A worker thread: pull a ready goroutine and run it to its next suspension,
/// forever. Parks on `WORK_CV` when nothing is ready.
fn worker_loop() {
    loop {
        let id = {
            let mut guard = SCHED.lock().unwrap();
            loop {
                if guard.0.shutdown {
                    return;
                }
                if let Some(id) = guard.0.ready.pop_front() {
                    // Claim it into `running` under the same lock as the pop, so
                    // it is never momentarily invisible (in neither `ready` nor
                    // `running`); otherwise a quiescence check or the test reset
                    // could clear the scheduler out from under this worker and
                    // its `expect("live goroutine")` would panic, leaving a
                    // phantom in-Raman thread that hangs every later collection.
                    guard.0.running.insert(id);
                    break id;
                }
                if is_deadlocked(&guard.0) {
                    drop(guard);
                    deadlock_panic();
                }
                guard = WORK_CV.wait(guard).unwrap();
            }
        };
        run_goroutine(id);
    }
}

/// Resume goroutine `id` to its next suspension on this worker thread.
///
/// The collector's running-state brackets the whole body, including the GC
/// root-chain hand-off, so a stop-the-world that another thread starts waits for
/// this worker to reach a safepoint inside `resume` (or to leave the running
/// state) rather than scanning a half-moved root chain. Concretely: `id` is
/// marked `running` and its saved chain installed into this worker's registered
/// context while in the running state, so a collection then sees the live roots
/// via the registry; on the way out the chain is saved and `id` cleared from
/// `running` before leaving the running state, so a collection then sees them
/// via `extra_roots`. The brief in-between windows are covered because the
/// collector waits for this in-Raman worker to park, and it only parks once the
/// chain is in place.
fn run_goroutine(id: usize) {
    CURRENT_GOROUTINE.with(|c| c.set(id));
    raven_gc_enter_running();
    // Take this goroutine's saved roots (leaving its saved chain empty) and
    // install them as this worker's live context. `id` was already claimed into
    // `running` by the worker loop. Taking the chain under the lock means a
    // concurrent `extra_roots` sees the chain either full (not yet taken, and it
    // marks it) or empty (taken; the live roots are in this worker's registered
    // context, or in transit while this in-Raman worker has not yet parked, so
    // the collector is still waiting for it).
    let saved = with_sched(|sched| {
        sched
            .goroutines
            .get_mut(&id)
            .map(|g| std::mem::take(&mut g.roots))
            .unwrap_or_default()
    });
    install_root_chain(saved);
    let mut coro = with_sched(|sched| {
        sched
            .goroutines
            .get_mut(&id)
            .expect("live goroutine")
            .coro
            .take()
            .expect("coroutine handle")
    });
    let result = coro.resume(());
    // Save this goroutine's live roots and clear `running` before leaving the
    // running state, so once the collector may proceed the saved chain is
    // authoritative again.
    let live = take_root_chain();
    with_sched(|sched| {
        if let Some(g) = sched.goroutines.get_mut(&id) {
            g.coro = Some(coro);
            g.roots = live;
        }
        sched.running.remove(&id);
        match result {
            CoroutineResult::Yield(Suspend::Yielded) => {
                sched.ready.push_back(id);
                WORK_CV.notify_one();
            }
            CoroutineResult::Yield(Suspend::Blocked) => {
                // The goroutine committed to blocking (it is in `blocked` and on
                // a channel wait list). But a waker may have removed it from
                // `blocked` while we were still resuming it (it could not be
                // re-queued then, because we held its coroutine). If so, re-queue
                // it now; otherwise leave it genuinely parked.
                if !sched.blocked.contains(&id) {
                    sched.ready.push_back(id);
                    WORK_CV.notify_one();
                }
            }
            CoroutineResult::Yield(Suspend::Finished) | CoroutineResult::Return(()) => {
                sched.goroutines.remove(&id);
            }
        }
    });
    raven_gc_exit_running();
    CURRENT_GOROUTINE.with(|c| c.set(0));
}

/// Suspend the running goroutine back to the scheduler with `reason`.
///
/// For a coroutine goroutine this calls its yielder. The main goroutine
/// (id 0) has no yielder; it instead drives the scheduler loop directly
/// and returns when it is runnable again, so this is only ever called
/// from a non-main goroutine's body.
fn suspend_current(reason: Suspend) {
    let yielder = CURRENT_YIELDER.with(|y| y.get());
    assert!(
        !yielder.is_null(),
        "suspend_current called with no running coroutine"
    );
    // SAFETY: `yielder` points to the live yielder of the running
    // coroutine, valid for the duration of the body.
    let yielder = unsafe { &*yielder };
    yielder.suspend(reason);
    // On resume, re-publish our yielder: the scheduler may have run other
    // goroutines that overwrote the shared slot.
    CURRENT_YIELDER.with(|y| y.set(yielder as *const GoYielder));
}

/// Cooperative yield point. A spawned goroutine yields its worker to another
/// ready goroutine. From main it is a hint only: goroutines already run in
/// parallel on the worker pool, so main need not step aside for them.
#[no_mangle]
pub extern "C" fn raven_go_yield() {
    if CURRENT_GOROUTINE.with(|c| c.get()) == 0 {
        std::thread::yield_now();
    } else {
        suspend_current(Suspend::Yielded);
    }
}

/// Park the calling goroutine, which has already, under one scheduler lock,
/// registered on a channel wait list and inserted itself into `blocked`. A
/// spawned goroutine suspends its coroutine so the worker runs something else;
/// main, which has no coroutine, parks the OS thread on `MAIN_CV`. Returns once
/// a counterpart has woken it (removed it from `blocked`).
fn park_current(me: usize) {
    if me == 0 {
        // Leave the running set while main is parked: it is not running compiled
        // Raven, so a worker's collection must not wait for it to reach a
        // safepoint (it never would, and the collection would deadlock). Main's
        // roots stay scannable through its registered context, which is stable
        // while it sits in the wait. A blocked goroutine's worker does the same
        // (it exit_running's after the suspend). Only if this thread is actually
        // in-Raven, though: a compiled `main` is, but the Rust test harness
        // thread that drives the scheduler is not, and must not exit a set it
        // never entered. Re-enter on wake, which parks if a collection is running.
        let was_running = crate::gc::thread_in_running();
        if was_running {
            raven_gc_exit_running();
        }
        {
            let mut guard = SCHED.lock().unwrap();
            // Committing main to block may complete a deadlock. A waker may also
            // have already cleared the block in the gap since registration, in
            // which case the predicate is false and main does not wait (no lost
            // wakeup).
            if guard.0.blocked.contains(&0) && is_deadlocked(&guard.0) {
                drop(guard);
                deadlock_panic();
            }
            while guard.0.blocked.contains(&0) {
                guard = MAIN_CV.wait(guard).unwrap();
            }
        }
        if was_running {
            raven_gc_enter_running();
        }
    } else {
        suspend_current(Suspend::Blocked);
    }
}

/// Wake goroutine `id` (called without the scheduler lock held): clear its block
/// and make it runnable. If it is still being resumed by a worker, leave the
/// re-queue to that worker (we must not touch its coroutine); otherwise re-queue
/// it and nudge a worker, or signal main's condvar. A no-op if `id` was already
/// woken, which keeps two counterparts from double-waking it.
fn wake(id: usize) {
    let mut guard = SCHED.lock().unwrap();
    let sched = &mut guard.0;
    if !sched.blocked.remove(&id) {
        return;
    }
    if id == 0 {
        MAIN_CV.notify_one();
    } else if sched.running.contains(&id) {
        // Still on a worker; that worker re-queues it when it sees `!blocked`.
    } else {
        if !sched.ready.contains(&id) {
            sched.ready.push_back(id);
        }
        WORK_CV.notify_one();
    }
}

/// Report an all-goroutines-blocked deadlock and exit, matching Go.
fn deadlock_panic() -> ! {
    eprintln!("raven panic: all goroutines are blocked: deadlock");
    std::process::exit(101);
}

/// A send or receive named a channel id with no registered channel (a freed or
/// invalid handle). Report and exit rather than busy-spinning on a receive or
/// silently dropping a send.
fn unknown_channel_panic(id: i64) -> ! {
    eprintln!(
        "raven panic: operation on unknown channel {id} (it may have been freed or never created)"
    );
    std::process::exit(101);
}

// ----- channels -----

/// Create a channel with capacity `cap` and return its id. `cap == 0` is
/// an unbuffered rendezvous channel.
fn make_channel(cap: usize) -> i64 {
    with_sched(|sched| {
        let id = sched.next_chan;
        sched.next_chan += 1;
        sched.channels.insert(
            id,
            Channel {
                cap,
                queue: VecDeque::new(),
                send_waiters: VecDeque::new(),
                recv_waiters: VecDeque::new(),
            },
        );
        id
    })
}

/// Create an unbuffered channel, returning its id.
#[no_mangle]
pub extern "C" fn raven_channel_new() -> i64 {
    make_channel(0)
}

/// Create a buffered channel of capacity `cap` (clamped to at least 0),
/// returning its id.
#[no_mangle]
pub extern "C" fn raven_channel_new_buffered(cap: i64) -> i64 {
    make_channel(cap.max(0) as usize)
}

/// The effective queue bound: a buffered channel holds up to `cap`
/// values; an unbuffered channel transfers one value at a time, modeled
/// as a one-slot hand-off so a sender blocks while the slot is occupied.
fn send_bound(ch: &Channel) -> usize {
    if ch.cap == 0 {
        1
    } else {
        ch.cap
    }
}

/// Whether the channel `id`'s queue can accept one more value right now.
fn can_send_now(sched: &Scheduler, id: i64) -> bool {
    match sched.channels.get(&id) {
        Some(ch) => ch.queue.len() < send_bound(ch),
        None => false,
    }
}

/// Send `value` on channel `id`, blocking until the channel can accept it.
///
/// A buffered channel blocks only when the buffer is full. An unbuffered
/// channel hands one value at a time: the sender blocks while a previously
/// sent value still waits to be received, so a sender never runs more than
/// one value ahead of its receiver.
#[no_mangle]
pub extern "C" fn raven_channel_send(id: i64, value: i64) {
    let me = CURRENT_GOROUTINE.with(|c| c.get());
    loop {
        // Check the slot and act in one lock so a receiver freeing a slot cannot
        // slip between the check and the register (a lost wakeup) or let two
        // senders both deposit past the bound.
        let waiter = with_sched(|sched| {
            if !sched.channels.contains_key(&id) {
                unknown_channel_panic(id);
            }
            if can_send_now(sched, id) {
                return match sched.channels.get_mut(&id) {
                    Some(ch) => {
                        ch.queue.push_back(value);
                        Some(ch.recv_waiters.pop_front())
                    }
                    None => Some(None),
                };
            }
            // Full: register on the send wait list and commit to blocking,
            // atomically, then park below.
            if let Some(ch) = sched.channels.get_mut(&id) {
                ch.send_waiters.push_back(me);
            }
            sched.blocked.insert(me);
            None
        });
        match waiter {
            Some(recv) => {
                if let Some(w) = recv {
                    wake(w);
                }
                return;
            }
            None => {
                park_current(me);
                with_sched(|sched| {
                    if let Some(ch) = sched.channels.get_mut(&id) {
                        ch.send_waiters.retain(|&w| w != me);
                    }
                });
            }
        }
    }
}

/// Receive a value from channel `id`, blocking until one is available.
#[no_mangle]
pub extern "C" fn raven_channel_recv(id: i64) -> i64 {
    let me = CURRENT_GOROUTINE.with(|c| c.get());
    loop {
        // Take a value, or register-and-commit, in one lock (same lost-wakeup
        // reasoning as send). Taking a value or finding none frees/needs a
        // sender, so wake one either way so it can deposit into the slot.
        enum Recv {
            Got(i64, Option<usize>),
            Block(Option<usize>),
        }
        let outcome = with_sched(|sched| {
            let Some(ch) = sched.channels.get_mut(&id) else {
                unknown_channel_panic(id);
            };
            if let Some(v) = ch.queue.pop_front() {
                return Recv::Got(v, ch.send_waiters.pop_front());
            }
            let sender = ch.send_waiters.pop_front();
            ch.recv_waiters.push_back(me);
            sched.blocked.insert(me);
            Recv::Block(sender)
        });
        match outcome {
            Recv::Got(v, sender) => {
                if let Some(snd) = sender {
                    wake(snd);
                }
                return v;
            }
            Recv::Block(sender) => {
                if let Some(snd) = sender {
                    wake(snd);
                }
                park_current(me);
                with_sched(|sched| {
                    if let Some(ch) = sched.channels.get_mut(&id) {
                        ch.recv_waiters.retain(|&w| w != me);
                    }
                });
            }
        }
    }
}

// ----- wait groups -----

/// Create a wait group with a zero counter, returning its id.
#[no_mangle]
pub extern "C" fn raven_waitgroup_new() -> i64 {
    with_sched(|sched| {
        let id = sched.next_wg;
        sched.next_wg += 1;
        sched.wait_groups.insert(
            id,
            WaitGroup {
                count: 0,
                waiters: Vec::new(),
            },
        );
        id
    })
}

/// Adjust wait group `id`'s counter by `delta` (negative for `done`). When the
/// counter reaches zero, every waiter is woken.
#[no_mangle]
pub extern "C" fn raven_waitgroup_add(id: i64, delta: i64) {
    let woken = with_sched(|sched| match sched.wait_groups.get_mut(&id) {
        Some(wg) => {
            wg.count += delta;
            if wg.count <= 0 {
                std::mem::take(&mut wg.waiters)
            } else {
                Vec::new()
            }
        }
        None => Vec::new(),
    });
    for w in woken {
        wake(w);
    }
}

/// Block until wait group `id`'s counter is zero, returning at once if it
/// already is. The waiter registers under the lock before parking (no lost
/// wakeup) and the deadlock check counts it like a channel waiter.
#[no_mangle]
pub extern "C" fn raven_waitgroup_wait(id: i64) {
    let me = CURRENT_GOROUTINE.with(|c| c.get());
    loop {
        let blocked = with_sched(|sched| {
            let should_block = match sched.wait_groups.get_mut(&id) {
                Some(wg) if wg.count > 0 => {
                    wg.waiters.push(me);
                    true
                }
                _ => false,
            };
            if should_block {
                sched.blocked.insert(me);
            }
            should_block
        });
        if !blocked {
            return;
        }
        park_current(me);
    }
}

/// Sleep the calling goroutine for `ms` milliseconds (a non-positive duration
/// returns at once). Leaves the collector's running set for the duration so a
/// collection does not wait on it. This blocks the OS thread it runs on, so a
/// sleeping goroutine holds its worker while it sleeps (like a blocking IO
/// call); releasing the worker via a timer is a possible refinement.
#[no_mangle]
pub extern "C" fn raven_sleep_millis(ms: i64) {
    if ms <= 0 {
        return;
    }
    let was = crate::gc::block_begin();
    std::thread::sleep(std::time::Duration::from_millis(ms as u64));
    crate::gc::block_end(was);
}

// ----- select -----

/// Begin a select set (the channels to wait on), returning its id. Add channels
/// with `raven_select_add`, then block on it with `raven_select_recv`.
#[no_mangle]
pub extern "C" fn raven_select_new() -> i64 {
    with_sched(|sched| {
        let id = sched.next_select;
        sched.next_select += 1;
        sched.select_sets.insert(id, Vec::new());
        id
    })
}

/// Add channel `chan_id` to select set `set_id` (its index in the set is the
/// order added).
#[no_mangle]
pub extern "C" fn raven_select_add(set_id: i64, chan_id: i64) {
    with_sched(|sched| {
        if let Some(set) = sched.select_sets.get_mut(&set_id) {
            set.push(chan_id);
        }
    });
}

/// Block until one of select set `set_id`'s channels has a value, then take it,
/// stash it for `raven_select_value`, and return the index of that channel in
/// the set. Returns -1 if the set is unknown or empty. Ties go to the
/// lowest index.
#[no_mangle]
pub extern "C" fn raven_select_recv(set_id: i64) -> i64 {
    let me = CURRENT_GOROUTINE.with(|c| c.get());
    loop {
        // (index, value, sender to wake) when ready; None means block.
        let ready: Option<(i64, i64, Option<usize>)> = with_sched(|sched| {
            let ids = sched.select_sets.get(&set_id)?.clone();
            if ids.is_empty() {
                return None;
            }
            // A select over a channel that does not exist would otherwise be
            // silently skipped and could block forever; report it instead.
            for &cid in &ids {
                if !sched.channels.contains_key(&cid) {
                    unknown_channel_panic(cid);
                }
            }
            // Clear any registration from a previous iteration so re-registering
            // below cannot duplicate this waiter on a channel's list.
            for &cid in &ids {
                if let Some(ch) = sched.channels.get_mut(&cid) {
                    ch.recv_waiters.retain(|&w| w != me);
                }
            }
            // Take from the first channel that has a value, waking one of its
            // blocked senders so it can refill the freed slot.
            for (i, &cid) in ids.iter().enumerate() {
                if let Some(ch) = sched.channels.get_mut(&cid) {
                    if let Some(v) = ch.queue.pop_front() {
                        let sender = ch.send_waiters.pop_front();
                        return Some((i as i64, v, sender));
                    }
                }
            }
            // None ready: register on every channel's recv list and commit to
            // blocking, then park below.
            for &cid in &ids {
                if let Some(ch) = sched.channels.get_mut(&cid) {
                    ch.recv_waiters.push_back(me);
                }
            }
            sched.blocked.insert(me);
            Some((-2, 0, None)) // sentinel: registered, must block
        });
        match ready {
            None => return -1,
            Some((-2, _, _)) => park_current(me),
            Some((index, value, sender)) => {
                if let Some(snd) = sender {
                    wake(snd);
                }
                SELECT_VALUE.with(|c| c.set(value));
                return index;
            }
        }
    }
}

/// The value taken by the most recent `raven_select_recv` on this thread.
#[no_mangle]
pub extern "C" fn raven_select_value() -> i64 {
    SELECT_VALUE.with(|c| c.get())
}

/// Discard select set `set_id`.
#[no_mangle]
pub extern "C" fn raven_select_free(set_id: i64) {
    with_sched(|sched| {
        sched.select_sets.remove(&set_id);
    });
}

/// Free a channel's registry entry. The id becomes invalid; a later operation
/// on it panics (see `unknown_channel_panic`). Free only when no goroutine is
/// still using the channel.
#[no_mangle]
pub extern "C" fn raven_channel_free(id: i64) {
    with_sched(|sched| {
        sched.channels.remove(&id);
    });
}

/// Free a wait group's registry entry.
#[no_mangle]
pub extern "C" fn raven_waitgroup_free(id: i64) {
    with_sched(|sched| {
        sched.wait_groups.remove(&id);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serializes the scheduler tests and resets the now-global scheduler
    /// between them. The scheduler is shared by all OS threads (it is global so
    /// the future worker pool can share it), so sibling tests running in
    /// parallel would corrupt each other's goroutines; the lock isolates them
    /// and the reset gives each a fresh scheduler. The body still runs on its
    /// own thread so its shadow stack and per-thread yielder start clean.
    static SCHED_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn isolated(body: impl FnOnce() + Send + 'static) {
        let guard = SCHED_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // The worker pool is global and persists across tests. Wait for the
        // previous test's goroutines to drain (nothing ready, nothing being
        // resumed) before resetting, or a worker mid-resume would find its
        // goroutine gone. Quiescence is reached because every parent-process
        // test completes its goroutines.
        loop {
            let quiet = with_sched(|s| s.ready.is_empty() && s.running.is_empty());
            if quiet {
                break;
            }
            std::thread::yield_now();
        }
        with_sched(|sched| *sched = Scheduler::new());
        std::thread::spawn(body).join().unwrap();
        drop(guard);
    }

    #[test]
    fn buffered_channel_send_recv_in_one_goroutine() {
        isolated(|| {
            let ch = raven_channel_new_buffered(4);
            raven_channel_send(ch, 10);
            raven_channel_send(ch, 20);
            assert_eq!(raven_channel_recv(ch), 10);
            assert_eq!(raven_channel_recv(ch), 20);
        });
    }

    // A goroutine body reads its capture env: the first slot is the
    // channel id, the second (when present) is a value to send.
    extern "C" fn producer_body(env: *mut u8) {
        let ch = unsafe { *(env as *const i64) };
        for i in 1..=5 {
            raven_channel_send(ch, i);
        }
    }

    /// Spawn a goroutine whose closure captures `slots` inline as the env.
    fn spawn_with_env(body: extern "C" fn(*mut u8), slots: &[i64]) {
        let size = (slots.len() * 8).max(8) as u32;
        let closure = crate::object::raven_closure_new(body as *const u8, size, 8, 0, 0);
        unsafe {
            let caps = crate::object::raven_closure_captures(closure) as *mut i64;
            for (i, &v) in slots.iter().enumerate() {
                caps.add(i).write(v);
            }
        }
        raven_go_spawn(closure);
    }

    #[test]
    fn producer_consumer_over_unbuffered_channel() {
        isolated(|| {
            let ch = raven_channel_new();
            spawn_with_env(producer_body, &[ch]);
            let mut sum = 0;
            for _ in 0..5 {
                sum += raven_channel_recv(ch);
            }
            assert_eq!(sum, 15);
        });
    }

    extern "C" fn fanin_body(env: *mut u8) {
        let pair = env as *const i64;
        let ch = unsafe { *pair };
        let val = unsafe { *pair.add(1) };
        raven_channel_send(ch, val);
    }

    #[test]
    fn fan_in_three_goroutines() {
        isolated(|| {
            let ch = raven_channel_new();
            for v in [3i64, 5, 7] {
                spawn_with_env(fanin_body, &[ch, v]);
            }
            let mut sum = 0;
            for _ in 0..3 {
                sum += raven_channel_recv(ch);
            }
            assert_eq!(sum, 15);
        });
    }

    // A goroutine that allocates a GC string, roots it on its own
    // shadow-stack frame, signals main, then blocks. While it is parked
    // the string is reachable only through the parked goroutine's saved
    // root chain. env slots: [ready_chan, go_chan].
    extern "C" fn gc_holder_body(env: *mut u8) {
        let p = env as *const i64;
        let ready = unsafe { *p };
        let go = unsafe { *p.add(1) };
        // Allocate a GC string and keep it rooted in a frame for the rest
        // of the body, exactly as compiled code would.
        let mut s = crate::object::raven_string_new(16);
        // The GC pointer lives in the `s` slot; the frame's root array holds
        // the *address* of that slot, and the collector dereferences it to
        // read the live pointer, exactly as compiled code emits.
        let mut roots: [*mut *mut u8; 1] = [&mut s as *mut _ as *mut *mut u8];
        crate::gc::raven_gc_enter_frame(roots.as_mut_ptr() as *mut *mut u8, 1);
        // Tell main we have allocated and are about to block. Send a plain
        // sentinel, not the pointer, so the string is reachable ONLY
        // through this parked goroutine's saved root chain (not through a
        // channel buffer), making this a clean test of parked-chain
        // scanning.
        raven_channel_send(ready, 1);
        // Block until main has run a collection and lets us continue.
        let _ = raven_channel_recv(go);
        // The string must still be live here: read its header tag.
        let tag = unsafe { (*s).header.tag };
        crate::gc::raven_gc_leave_frame();
        raven_channel_send(ready, tag as i64);
    }

    #[test]
    fn gc_scans_parked_goroutine_roots() {
        isolated(|| {
            let ready = raven_channel_new_buffered(2);
            let go = raven_channel_new_buffered(1);
            spawn_with_env(gc_holder_body, &[ready, go]);
            // Drive the goroutine until it has allocated and parked: it
            // sends a sentinel on `ready`, then blocks on `go`.
            assert_eq!(raven_channel_recv(ready), 1);
            // Force a collection while the goroutine is parked. The string it
            // allocated is reachable only through the parked goroutine's saved
            // root chain (surfaced by `extra_roots`). We assert survival through
            // the goroutine's own report below rather than a live-object count:
            // under the M:N model the goroutine allocates on its worker's
            // per-thread heap, not this (main) thread's, so a count here would
            // not see it.
            crate::gc::raven_gc_collect();
            // Let the goroutine resume; it reads its string's header (which
            // would be freed memory if the parked chain were not scanned)
            // and reports the tag. A correct scan reports TAG_STRING.
            raven_channel_send(go, 0);
            let reported = raven_channel_recv(ready);
            assert_eq!(reported as u32, crate::object::TAG_STRING);
        });
    }

    // Like gc_holder_body but it writes distinct marker bytes into its
    // string and reports them back so main can verify the buffer is intact,
    // not merely that the header tag survived. env: [ready_chan, go_chan].
    extern "C" fn gc_marker_body(env: *mut u8) {
        let p = env as *const i64;
        let ready = unsafe { *p };
        let go = unsafe { *p.add(1) };
        let mut s = crate::object::raven_string_new(8);
        // SAFETY: write 8 marker bytes 0..8 into the owned buffer.
        unsafe {
            let bytes = crate::object::raven_string_bytes(s) as *mut u8;
            for i in 0..8u8 {
                bytes.add(i as usize).write(i.wrapping_mul(3));
            }
            (*s).header.len = 8;
        }
        let mut roots: [*mut *mut u8; 1] = [&mut s as *mut _ as *mut *mut u8];
        crate::gc::raven_gc_enter_frame(roots.as_mut_ptr() as *mut *mut u8, 1);
        raven_channel_send(ready, 1);
        let _ = raven_channel_recv(go);
        // Sum the marker bytes through the surviving buffer. A freed or
        // corrupted buffer would not sum to the expected value.
        let mut sum = 0i64;
        for i in 0..8usize {
            sum += crate::object::raven_string_byte_at(s, i) as i64;
        }
        crate::gc::raven_gc_leave_frame();
        raven_channel_send(ready, sum);
    }

    #[test]
    fn parked_goroutine_string_survives_heavy_allocation() {
        isolated(|| {
            let ready = raven_channel_new_buffered(2);
            let go = raven_channel_new_buffered(1);
            spawn_with_env(gc_marker_body, &[ready, go]);
            assert_eq!(raven_channel_recv(ready), 1);
            // Allocate heavily and collect repeatedly while the goroutine is
            // parked. Its string is reachable only through the parked
            // goroutine's saved root chain.
            for i in 0..30_000usize {
                let _garbage = crate::object::raven_string_new(16);
                if i % 256 == 0 {
                    crate::gc::raven_gc_collect();
                }
            }
            crate::gc::raven_gc_collect();
            raven_channel_send(go, 0);
            // Expected sum of i*3 for i in 0..8 = 3 * (0+1+...+7) = 84.
            let expected: i64 = (0..8i64).map(|i| (i * 3) & 0xFF).sum();
            assert_eq!(raven_channel_recv(ready), expected);
        });
    }

    // Body that blocks forever on an empty channel, never woken.
    extern "C" fn blocker_body(env: *mut u8) {
        let ch = unsafe { *(env as *const i64) };
        let _ = raven_channel_recv(ch);
    }

    #[test]
    fn all_goroutines_blocked_is_a_deadlock() {
        // The deadlock path exits the process, so run it in a child: this
        // test re-execs the test binary with a trigger env var. The child
        // spawns a goroutine that blocks on an empty channel and then main
        // blocks on a second empty channel, so every goroutine is parked.
        if std::env::var("RAVEN_SCHED_DEADLOCK_CHILD").is_ok() {
            let go_ch = raven_channel_new();
            spawn_with_env(blocker_body, &[go_ch]);
            let main_ch = raven_channel_new();
            // Main blocks with no sender anywhere: deadlock, exit 101.
            let _ = raven_channel_recv(main_ch);
            // Unreachable on a correct deadlock detector.
            std::process::exit(0);
        }
        let exe = std::env::current_exe().expect("test exe path");
        let status = std::process::Command::new(exe)
            .args([
                "sched::tests::all_goroutines_blocked_is_a_deadlock",
                "--exact",
                "--nocapture",
            ])
            .env("RAVEN_SCHED_DEADLOCK_CHILD", "1")
            .output()
            .expect("spawn child");
        // The child must exit 101 (the deadlock panic code), not 0.
        assert_eq!(
            status.status.code(),
            Some(101),
            "expected deadlock exit 101, got {:?}; stderr: {}",
            status.status.code(),
            String::from_utf8_lossy(&status.stderr)
        );
        assert!(
            String::from_utf8_lossy(&status.stderr).contains("deadlock"),
            "expected a deadlock diagnostic on stderr"
        );
    }
}
