//! Cooperative single-OS-thread green-thread scheduler and channels for
//! compiled Raven v2 programs.
//!
//! See `docs/v2/specs/concurrency.md` for the model. One goroutine runs
//! at a time; goroutines yield at channel send/recv on a full/empty
//! channel, at an explicit `raven_go_yield`, and when their body
//! finishes. Because there is no parallelism the collector stays
//! single-threaded; the one collector change is that the mark phase scans
//! the saved root chain of every parked goroutine and every buffered
//! channel value (see `crate::gc`).

use crate::gc::{
    for_each_slot_in, install_root_chain, set_extra_roots_hook, take_root_chain, RootSlot,
    SavedRoots,
};
use crate::object::Closure;
use corosensei::{Coroutine, CoroutineResult, Yielder};
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};

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
    finished: bool,
}

/// A channel: a bounded queue of pointer-width value slots plus the wait
/// lists of goroutine ids blocked on it.
struct Channel {
    cap: usize,
    queue: VecDeque<i64>,
    send_waiters: VecDeque<usize>,
    recv_waiters: VecDeque<usize>,
}

/// The scheduler state. Single OS thread this slice, so a thread-local
/// with interior mutability is sound and lock-free.
struct Scheduler {
    goroutines: HashMap<usize, Goroutine>,
    ready: VecDeque<usize>,
    /// Goroutines parked on a channel wait list. A blocked goroutine is
    /// not runnable until a counterpart wakes it (moving it to `ready`).
    /// Tracked explicitly so the deadlock check can tell a blocked current
    /// goroutine (notably main, whose id stays `current` while it drives
    /// the scheduler loop) from a runnable one.
    blocked: std::collections::HashSet<usize>,
    current: usize,
    next_id: usize,
    channels: HashMap<i64, Channel>,
    next_chan: i64,
    /// Set once the first goroutine is spawned. Until then the program is
    /// strictly non-concurrent and the scheduler is never entered.
    started: bool,
    /// The yielder of the goroutine currently running, raw because it is
    /// borrowed from inside the coroutine body. Null when the main
    /// goroutine (id 0) runs, which suspends differently (see `block`).
    yielder: *const GoYielder,
}

thread_local! {
    static SCHED: RefCell<Scheduler> = RefCell::new(Scheduler::new());
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
                finished: false,
            },
        );
        Scheduler {
            goroutines,
            ready: VecDeque::new(),
            blocked: std::collections::HashSet::new(),
            current: 0,
            next_id: 1,
            channels: HashMap::new(),
            next_chan: 1,
            started: false,
            yielder: std::ptr::null(),
        }
    }
}

/// Visitor for the collector: surface every parked goroutine's root chain
/// and every buffered channel value as roots. The buffered slots live in
/// the channel queues, so we hand the collector the address of each slot.
fn extra_roots(visit: &mut dyn FnMut(RootSlot)) {
    SCHED.with(|s| {
        let sched = s.borrow();
        for (&id, g) in sched.goroutines.iter() {
            // The running goroutine's roots live in the thread-local
            // chain, already scanned by the mark phase; skip it here to
            // avoid double counting.
            if id == sched.current {
                continue;
            }
            for_each_slot_in(&g.roots, visit);
        }
        for chan in sched.channels.values() {
            for slot in chan.queue.iter() {
                visit(slot as *const i64 as RootSlot);
            }
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
        SCHED.with(|s| s.borrow_mut().yielder = yielder as *const GoYielder);
        // The closure body is `extern "C" fn(env)`.
        // SAFETY: spawn's contract guarantees a `fun() -> Unit` lifted
        // body taking the capture env.
        let body: extern "C" fn(*mut u8) = unsafe { std::mem::transmute(fn_addr as *const u8) };
        body(env_addr as *mut u8);
        yielder.suspend(Suspend::Finished);
    });

    SCHED.with(|s| {
        let mut sched = s.borrow_mut();
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
                finished: false,
            },
        );
        sched.ready.push_back(id);
    });
}

/// Run ready goroutines until the current goroutine is runnable again.
///
/// Called from `yield_now`, from a channel op that must block, and is the
/// core of the scheduler. The caller has already arranged for `current`
/// to be re-queued (a voluntary yield) or parked on a channel (a block).
/// This drives the next ready goroutine to its next suspension, repeating
/// until `current` is back at the front of the ready queue or there is
/// nothing left to run.
///
/// Switching is symmetric: the running goroutine suspends back here via
/// its yielder (handled by the caller through `suspend_current`), and
/// this function resumes the next goroutine. The main goroutine (id 0)
/// never owns a coroutine, so when it must block this function runs other
/// goroutines until id 0 is ready again, then returns to let main resume
/// on the OS stack.
fn run_scheduler_until_current_ready() {
    loop {
        let next = SCHED.with(|s| {
            let mut sched = s.borrow_mut();
            // If the current goroutine is ready at the front, resume it by
            // returning to the caller.
            if sched.ready.front() == Some(&sched.current) {
                sched.ready.pop_front();
                return None;
            }
            sched.ready.pop_front()
        });

        let Some(id) = next else {
            // Nothing ready. If the current goroutine is the one we are
            // waiting on and it is parked, every goroutine is blocked.
            let deadlock = SCHED.with(|s| {
                let sched = s.borrow();
                sched.ready.is_empty() && !is_runnable(&sched, sched.current)
            });
            if deadlock {
                deadlock_panic();
            }
            // The current goroutine became runnable (it was re-queued and
            // then popped as the front match above). Return to it.
            return;
        };

        if id == 0 {
            // Main goroutine is ready again. Hand control back so it
            // resumes on the OS thread stack.
            SCHED.with(|s| s.borrow_mut().ready.push_front(0));
            // The front is now 0; loop once more to pop and return.
            continue;
        }

        // Resume goroutine `id` until it next suspends. The coroutine
        // handle is moved out across the resume so the coroutine body can
        // freely borrow the scheduler (a borrow held across resume would
        // alias the thread-local cell).
        let prev = SCHED.with(|s| s.borrow().current);
        switch_root_chain(prev, id);
        let mut coro = SCHED.with(|s| {
            s.borrow_mut()
                .goroutines
                .get_mut(&id)
                .expect("live goroutine")
                .coro
                .take()
                .expect("coroutine handle")
        });
        let result = coro.resume(());
        SCHED.with(|s| {
            if let Some(g) = s.borrow_mut().goroutines.get_mut(&id) {
                g.coro = Some(coro);
            }
        });
        switch_root_chain(id, prev);

        match result {
            CoroutineResult::Yield(Suspend::Yielded) => {
                SCHED.with(|s| s.borrow_mut().ready.push_back(id));
            }
            CoroutineResult::Yield(Suspend::Blocked) => {
                // The goroutine parked itself on a channel; do not requeue.
            }
            CoroutineResult::Yield(Suspend::Finished) | CoroutineResult::Return(()) => {
                retire(id);
            }
        }
    }
}

/// Whether goroutine `id` is currently runnable: queued ready, or it is
/// the current goroutine and not parked on a channel.
fn is_runnable(sched: &Scheduler, id: usize) -> bool {
    sched.ready.contains(&id) || (sched.current == id && !sched.blocked.contains(&id))
}

/// Switch the live GC root chain from goroutine `from` to goroutine `to`
/// and set `to` as current. Stashes `from`'s live thread-local chain on
/// its goroutine struct, then installs `to`'s saved chain as the live
/// thread-local chain. The invariant is that the thread-local cells always
/// hold exactly the current goroutine's roots; every other goroutine's
/// roots sit in its `roots` slot.
fn switch_root_chain(from: usize, to: usize) {
    SCHED.with(|s| {
        let mut sched = s.borrow_mut();
        let live = take_root_chain();
        if let Some(g) = sched.goroutines.get_mut(&from) {
            g.roots = live;
        }
        let restored = sched
            .goroutines
            .get_mut(&to)
            .map(|g| std::mem::take(&mut g.roots))
            .unwrap_or_default();
        install_root_chain(restored);
        sched.current = to;
    });
}

/// Retire a finished goroutine: mark it finished and drop its coroutine.
fn retire(id: usize) {
    SCHED.with(|s| {
        let mut sched = s.borrow_mut();
        if let Some(g) = sched.goroutines.get_mut(&id) {
            g.finished = true;
            g.coro = None;
            g.roots = (Vec::new(), Vec::new());
        }
        sched.goroutines.remove(&id);
    });
}

/// Suspend the running goroutine back to the scheduler with `reason`.
///
/// For a coroutine goroutine this calls its yielder. The main goroutine
/// (id 0) has no yielder; it instead drives the scheduler loop directly
/// and returns when it is runnable again, so this is only ever called
/// from a non-main goroutine's body.
fn suspend_current(reason: Suspend) {
    let yielder = SCHED.with(|s| s.borrow().yielder);
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
    SCHED.with(|s| s.borrow_mut().yielder = yielder as *const GoYielder);
}

/// Cooperative yield point. The running goroutine yields control; the
/// scheduler runs other ready goroutines before resuming it.
///
/// If called from the main goroutine it drives the scheduler loop
/// directly. If called from a spawned goroutine it suspends back to the
/// scheduler, which re-queues it.
#[no_mangle]
pub extern "C" fn raven_go_yield() {
    let (is_main, started) = SCHED.with(|s| {
        let sched = s.borrow();
        (sched.current == 0, sched.started)
    });
    if !started {
        return;
    }
    if is_main {
        // Re-queue main and run others until it is ready again.
        SCHED.with(|s| {
            let mut sched = s.borrow_mut();
            let cur = sched.current;
            sched.ready.push_back(cur);
        });
        run_scheduler_until_current_ready();
    } else {
        suspend_current(Suspend::Yielded);
    }
}

/// Park the current goroutine (it is already on a channel wait list) and
/// switch to the scheduler. On return the goroutine has been woken.
fn block_current() {
    let (is_main, me) = SCHED.with(|s| {
        let mut sched = s.borrow_mut();
        let me = sched.current;
        sched.blocked.insert(me);
        (me == 0, me)
    });
    if is_main {
        run_scheduler_until_current_ready();
    } else {
        suspend_current(Suspend::Blocked);
    }
    // Woken: no longer blocked.
    SCHED.with(|s| {
        s.borrow_mut().blocked.remove(&me);
    });
}

/// Move goroutine `id` back onto the ready queue (waking it). Clears its
/// blocked flag so the deadlock check sees it as runnable again.
fn wake(id: usize) {
    SCHED.with(|s| {
        let mut sched = s.borrow_mut();
        sched.blocked.remove(&id);
        if !sched.ready.contains(&id) {
            sched.ready.push_back(id);
        }
    });
}

/// Report an all-goroutines-blocked deadlock and exit, matching Go.
fn deadlock_panic() -> ! {
    eprintln!("raven panic: all goroutines are blocked: deadlock");
    std::process::exit(101);
}

// ----- channels -----

/// Create a channel with capacity `cap` and return its id. `cap == 0` is
/// an unbuffered rendezvous channel.
fn make_channel(cap: usize) -> i64 {
    SCHED.with(|s| {
        let mut sched = s.borrow_mut();
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
    loop {
        let ready = SCHED.with(|s| can_send_now(&s.borrow(), id));
        if ready {
            break;
        }
        // Park on the send wait list and block until a receiver frees a
        // slot and wakes us.
        let me = SCHED.with(|s| {
            let mut sched = s.borrow_mut();
            let me = sched.current;
            if let Some(ch) = sched.channels.get_mut(&id) {
                ch.send_waiters.push_back(me);
            }
            me
        });
        block_current();
        SCHED.with(|s| {
            let mut sched = s.borrow_mut();
            if let Some(ch) = sched.channels.get_mut(&id) {
                ch.send_waiters.retain(|&w| w != me);
            }
        });
    }

    // Deposit the value and wake a waiting receiver.
    let waiter = SCHED.with(|s| {
        let mut sched = s.borrow_mut();
        if let Some(ch) = sched.channels.get_mut(&id) {
            ch.queue.push_back(value);
            ch.recv_waiters.pop_front()
        } else {
            None
        }
    });
    if let Some(w) = waiter {
        wake(w);
    }
}

/// Receive a value from channel `id`, blocking until one is available.
#[no_mangle]
pub extern "C" fn raven_channel_recv(id: i64) -> i64 {
    loop {
        // Take a buffered value if one is present and wake a blocked
        // sender, since taking a value frees a slot.
        let taken = SCHED.with(|s| {
            let mut sched = s.borrow_mut();
            let ch = sched.channels.get_mut(&id)?;
            let v = ch.queue.pop_front()?;
            let sender = ch.send_waiters.pop_front();
            Some((v, sender))
        });
        if let Some((v, sender)) = taken {
            if let Some(snd) = sender {
                wake(snd);
            }
            return v;
        }

        // Empty: wake a blocked sender so it can deposit, then park on the
        // recv wait list and block until a sender delivers.
        let me = SCHED.with(|s| {
            let mut sched = s.borrow_mut();
            let me = sched.current;
            let sender = sched
                .channels
                .get_mut(&id)
                .and_then(|ch| ch.send_waiters.pop_front());
            if let Some(ch) = sched.channels.get_mut(&id) {
                ch.recv_waiters.push_back(me);
            }
            (me, sender)
        });
        if let Some(snd) = me.1 {
            wake(snd);
        }
        let me = me.0;
        block_current();
        SCHED.with(|s| {
            let mut sched = s.borrow_mut();
            if let Some(ch) = sched.channels.get_mut(&id) {
                ch.recv_waiters.retain(|&w| w != me);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn isolated(body: impl FnOnce() + Send + 'static) {
        std::thread::spawn(body).join().unwrap();
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
        let s = crate::object::raven_string_new(16);
        // The frame's roots array holds the GC pointer slots directly;
        // `raven_gc_enter_frame` registers the address of each slot.
        let mut roots: [*mut u8; 1] = [s as *mut u8];
        crate::gc::raven_gc_enter_frame(roots.as_mut_ptr(), 1);
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
            // Force a collection while the goroutine is parked. The string
            // it allocated is reachable only through the parked
            // goroutine's saved root chain.
            crate::gc::raven_gc_collect();
            assert!(crate::gc::raven_gc_live_objects() >= 1);
            // Let the goroutine resume; it reads its string's header (which
            // would be freed memory if the parked chain were not scanned)
            // and reports the tag. A correct scan reports TAG_STRING.
            raven_channel_send(go, 0);
            let reported = raven_channel_recv(ready);
            assert_eq!(reported as u32, crate::object::TAG_STRING);
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
