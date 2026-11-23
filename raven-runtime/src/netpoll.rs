//! Readiness-based socket I/O. Sockets are non-blocking and registered with one
//! process-wide `mio::Poll` that a dedicated poller thread drives. An operation
//! that would block parks the calling goroutine, freeing its worker thread, and
//! the poller wakes it when the socket becomes ready or its deadline passes. A
//! thread that is not running a goroutine (main, or a native caller) waits on a
//! condvar instead.
//!
//! Registration is edge-triggered, so each socket carries a readiness sequence
//! per direction. A caller reads the sequence before trying the syscall; if the
//! syscall would block, it parks only while the sequence is unchanged, so an
//! event that lands between the attempt and the park is never lost.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::io;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use mio::{Events, Interest, Poll, Registry, Token, Waker};

const WAKER: Token = Token(0);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Dir {
    Read,
    Write,
}

#[derive(Clone, Copy)]
enum Waiter {
    Goroutine(usize),
    Thread,
}

#[derive(Default)]
struct Side {
    seq: u64,
    waiter: Option<(Waiter, u64)>,
    // The current waiter's deadline, and the deadline of this side's one queued
    // heap timer. A timer that fires early re-arms for `deadline`, so a busy
    // socket keeps a single heap entry per direction instead of one per wait.
    deadline: Option<Instant>,
    armed: Option<Instant>,
}

#[derive(Default)]
struct State {
    read: Side,
    write: Side,
    closed: bool,
    next_gen: u64,
}

impl State {
    fn side(&mut self, dir: Dir) -> &mut Side {
        match dir {
            Dir::Read => &mut self.read,
            Dir::Write => &mut self.write,
        }
    }
}

/// The readiness state of one registered socket.
pub(crate) struct IoEntry {
    token: Token,
    state: Mutex<State>,
    cv: Condvar,
}

impl IoEntry {
    /// The readiness sequence for `dir`; pass it to `wait` after a syscall
    /// that would block.
    pub(crate) fn seq(&self, dir: Dir) -> u64 {
        let mut st = lock(&self.state);
        st.side(dir).seq
    }
}

struct Poller {
    registry: Registry,
    waker: Waker,
    entries: Mutex<HashMap<Token, Arc<IoEntry>>>,
    timers: Mutex<BinaryHeap<Reverse<(Instant, usize, u8)>>>,
    next_token: AtomicUsize,
    earliest: AtomicU64,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn poller() -> &'static Poller {
    static POLLER: OnceLock<&'static Poller> = OnceLock::new();
    POLLER.get_or_init(|| {
        let poll = Poll::new().expect("netpoll: cannot create the OS poller");
        let registry = poll
            .registry()
            .try_clone()
            .expect("netpoll: cannot clone the registry");
        let waker = Waker::new(poll.registry(), WAKER).expect("netpoll: cannot create the waker");
        let p: &'static Poller = Box::leak(Box::new(Poller {
            registry,
            waker,
            entries: Mutex::new(HashMap::new()),
            timers: Mutex::new(BinaryHeap::new()),
            next_token: AtomicUsize::new(1),
            earliest: AtomicU64::new(u64::MAX),
        }));
        std::thread::Builder::new()
            .name("raven-netpoll".into())
            .spawn(move || run(poll, p))
            .expect("netpoll: cannot start the poller thread");
        p
    })
}

/// Register `source` for read and write readiness.
pub(crate) fn register(source: &mut impl mio::event::Source) -> io::Result<Arc<IoEntry>> {
    let p = poller();
    let token = Token(p.next_token.fetch_add(1, Ordering::Relaxed));
    let entry = Arc::new(IoEntry {
        token,
        state: Mutex::new(State::default()),
        cv: Condvar::new(),
    });
    lock(&p.entries).insert(token, entry.clone());
    if let Err(e) = p
        .registry
        .register(source, token, Interest::READABLE | Interest::WRITABLE)
    {
        lock(&p.entries).remove(&token);
        return Err(e);
    }
    Ok(entry)
}

/// Remove `source` from the poller, for a socket that is handed to blocking
/// code. A socket that is simply closed needs only `close`.
pub(crate) fn deregister(source: &mut impl mio::event::Source, entry: &IoEntry) {
    let _ = poller().registry.deregister(source);
    close(entry);
}

/// Stop tracking `entry` and wake anything still waiting on it.
pub(crate) fn close(entry: &IoEntry) {
    lock(&poller().entries).remove(&entry.token);
    let waiters = {
        let mut st = lock(&entry.state);
        st.closed = true;
        st.read.seq += 1;
        st.write.seq += 1;
        [st.read.waiter.take(), st.write.waiter.take()]
    };
    entry.cv.notify_all();
    for (w, _) in waiters.into_iter().flatten() {
        if let Waiter::Goroutine(id) = w {
            crate::sched::wake_io(id);
        }
    }
}

/// Run `op` until it stops returning `WouldBlock`, waiting for `dir`
/// readiness in between. `deadline` bounds the whole operation.
pub(crate) fn retry<T>(
    entry: &IoEntry,
    dir: Dir,
    deadline: Option<Instant>,
    mut op: impl FnMut() -> io::Result<T>,
) -> io::Result<T> {
    loop {
        let seq = entry.seq(dir);
        match attempt(&mut op) {
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                if !wait(entry, dir, seq, deadline) {
                    return Err(io::Error::new(io::ErrorKind::TimedOut, "timed out"));
                }
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            other => return other,
        }
    }
}

/// One try of a socket operation, in its own frame: the caller may have parked
/// and resumed on another worker, and thread-local addresses (the GC heap, the
/// read buffer) must be computed on the thread that runs this attempt.
#[inline(never)]
fn attempt<T>(op: &mut impl FnMut() -> io::Result<T>) -> io::Result<T> {
    op()
}

/// Wait until `dir` readiness moves past `seq` (returns true) or `deadline`
/// passes first (returns false).
fn wait(entry: &IoEntry, dir: Dir, seq: u64, deadline: Option<Instant>) -> bool {
    loop {
        if let Some(d) = deadline {
            if Instant::now() >= d {
                return lock(&entry.state).side(dir).seq != seq;
            }
        }
        if crate::sched::on_goroutine() {
            let mut gen = 0;
            let parked = crate::sched::park_io(|me| {
                let mut st = lock(&entry.state);
                if st.side(dir).seq != seq || st.closed {
                    return false;
                }
                st.next_gen += 1;
                gen = st.next_gen;
                let side = st.side(dir);
                side.waiter = Some((Waiter::Goroutine(me), gen));
                side.deadline = deadline;
                if let Some(d) = deadline {
                    if side.armed.is_none_or(|t| t > d) {
                        side.armed = Some(d);
                        add_timer(d, entry.token, dir);
                    }
                }
                true
            });
            if !parked {
                return true;
            }
        } else {
            let was = crate::gc::block_begin();
            let mut st = lock(&entry.state);
            if st.side(dir).seq == seq && !st.closed {
                st.next_gen += 1;
                let gen = st.next_gen;
                st.side(dir).waiter = Some((Waiter::Thread, gen));
                let timeout = deadline.map(|d| d.saturating_duration_since(Instant::now()));
                st = match timeout {
                    Some(t) => {
                        entry
                            .cv
                            .wait_timeout(st, t)
                            .unwrap_or_else(|e| e.into_inner())
                            .0
                    }
                    None => entry.cv.wait(st).unwrap_or_else(|e| e.into_inner()),
                };
                if matches!(st.side(dir).waiter, Some((_, g)) if g == gen) {
                    st.side(dir).waiter = None;
                }
            }
            drop(st);
            crate::gc::block_end(was);
        }
        if lock(&entry.state).side(dir).seq != seq {
            return true;
        }
    }
}

fn add_timer(deadline: Instant, token: Token, dir: Dir) {
    let p = poller();
    lock(&p.timers).push(Reverse((deadline, token.0, dir as u8)));
    let nanos = nanos_since_base(deadline);
    if nanos < p.earliest.fetch_min(nanos, Ordering::AcqRel) {
        let _ = p.waker.wake();
    }
}

fn base() -> Instant {
    static BASE: OnceLock<Instant> = OnceLock::new();
    *BASE.get_or_init(Instant::now)
}

fn nanos_since_base(t: Instant) -> u64 {
    t.saturating_duration_since(base())
        .as_nanos()
        .min(u64::MAX as u128) as u64
}

fn run(mut poll: Poll, p: &'static Poller) {
    let mut events = Events::with_capacity(1024);
    let mut wake = Vec::with_capacity(1024);
    loop {
        let timeout = next_timeout(p);
        if let Err(e) = poll.poll(&mut events, timeout) {
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            eprintln!("raven netpoll: {e}");
            std::thread::sleep(Duration::from_millis(10));
            continue;
        }
        for ev in events.iter() {
            if ev.token() == WAKER {
                continue;
            }
            let Some(entry) = lock(&p.entries).get(&ev.token()).cloned() else {
                continue;
            };
            let broken = ev.is_error() || ev.is_read_closed() || ev.is_write_closed();
            let mut woken: [Option<(Waiter, u64)>; 2] = [None, None];
            {
                let mut st = lock(&entry.state);
                if ev.is_readable() || broken {
                    st.read.seq += 1;
                    woken[0] = st.read.waiter.take();
                }
                if ev.is_writable() || broken {
                    st.write.seq += 1;
                    woken[1] = st.write.waiter.take();
                }
            }
            notify(&entry, woken, &mut wake);
        }
        expire_timers(p, &mut wake);
        crate::sched::wake_io_batch(&wake);
        wake.clear();
    }
}

/// Wake `woken`: threads at once, goroutines collected into `wake` so the
/// whole poll batch takes the scheduler lock once.
fn notify(entry: &IoEntry, woken: [Option<(Waiter, u64)>; 2], wake: &mut Vec<usize>) {
    let mut thread = false;
    for (w, _) in woken.into_iter().flatten() {
        match w {
            Waiter::Goroutine(id) => wake.push(id),
            Waiter::Thread => thread = true,
        }
    }
    if thread {
        entry.cv.notify_all();
    }
}

fn next_timeout(p: &Poller) -> Option<Duration> {
    let timers = lock(&p.timers);
    match timers.peek() {
        Some(Reverse((d, ..))) => {
            p.earliest.store(nanos_since_base(*d), Ordering::Release);
            Some(d.saturating_duration_since(Instant::now()))
        }
        None => {
            p.earliest.store(u64::MAX, Ordering::Release);
            None
        }
    }
}

fn expire_timers(p: &Poller, wake: &mut Vec<usize>) {
    let now = Instant::now();
    loop {
        let due = {
            let mut timers = lock(&p.timers);
            match timers.peek() {
                Some(Reverse((d, ..))) if *d <= now => timers.pop().map(|Reverse(t)| t),
                _ => None,
            }
        };
        let Some((at, token, dir)) = due else {
            break;
        };
        let Some(entry) = lock(&p.entries).get(&Token(token)).cloned() else {
            continue;
        };
        let dir = if dir == Dir::Read as u8 {
            Dir::Read
        } else {
            Dir::Write
        };
        let woken = {
            let mut st = lock(&entry.state);
            let side = st.side(dir);
            if side.armed != Some(at) {
                None
            } else {
                side.armed = None;
                match (side.waiter, side.deadline) {
                    (Some(_), Some(d)) if d <= now => side.waiter.take(),
                    (Some(_), Some(d)) => {
                        side.armed = Some(d);
                        add_timer(d, Token(token), dir);
                        None
                    }
                    _ => None,
                }
            }
        };
        let pair = match dir {
            Dir::Read => [woken, None],
            Dir::Write => [None, woken],
        };
        notify(&entry, pair, wake);
    }
}
