//! Stop-the-world coordination for the multi-threaded collector.
//!
//! Before the collector marks and sweeps the shared heap it must reach a state
//! where no other thread is mutating the heap or a shadow stack, so it sees a
//! quiescent heap and a consistent set of roots. This module is that
//! coordination, built and tested on its own before it guards real memory (see
//! `docs/v2/specs/concurrency-parallelism.md`).
//!
//! # The model: short unsafe regions, not a poll-and-park barrier
//!
//! A thread is **safe by default**. It enters a short **unsafe region** only
//! while it mutates the heap (allocation) or its shadow stack
//! (`enter_frame`/`push_root`/...), bracketing each with
//! [`enter_unsafe`](StopTheWorld::enter_unsafe) /
//! [`exit_unsafe`](StopTheWorld::exit_unsafe). A collection waits only for the
//! *in-flight* unsafe regions to drain, which is always quick because the
//! regions are bounded (push a slot, allocate one object). A thread that is
//! idle, blocked, or running non-mutating code is already safe and never blocks
//! a collection.
//!
//! This is why the model replaces an earlier "wait for every registered thread
//! to poll and park" barrier: that barrier deadlocked whenever a registered
//! thread stopped polling (a worker between goroutines, or, in the runtime's
//! own test harness, a reused thread running a non-allocating test). Here such a
//! thread contributes nothing to the wait.
//!
//! # The handshake
//!
//! Entering an unsafe region and a collector starting a stop race. The
//! resolution is a sequentially-consistent store/load pair: the entering thread
//! bumps `unsafe_count` then reads `stop_requested`; the collector sets
//! `stop_requested` then reads `unsafe_count`. Under `SeqCst` at least one side
//! observes the other, so a thread that proceeds into an unsafe region is
//! always counted by the collector, and a thread that is not counted has either
//! not entered or has parked. No thread mutates while the world is stopped.
//!
//! The common path is lock-free: `enter_unsafe` is one atomic add plus one
//! atomic load, `exit_unsafe` one atomic subtract. The mutex and condvar are
//! touched only to park (a stop is pending) or to wake (a region drained while a
//! collector waits, or the world resumes).

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};

/// Coordinates a stop-the-world pause across mutator threads.
///
/// Mutators bracket heap/shadow-stack mutations with
/// [`enter_unsafe`](Self::enter_unsafe) / [`exit_unsafe`](Self::exit_unsafe). A
/// thread that needs to collect calls [`stop_the_world`](Self::stop_the_world)
/// from *outside* an unsafe region, marks and sweeps while the world is held,
/// then [`resume_the_world`](Self::resume_the_world).
pub struct StopTheWorld {
    /// Set while a collection is pending or in progress. A thread that observes
    /// it on entry to an unsafe region backs out and parks instead.
    stop_requested: AtomicBool,
    /// Threads currently inside an unsafe region. A collection waits for this to
    /// reach zero. Lock-free so the common bracket is cheap.
    unsafe_count: AtomicUsize,
    /// Threads currently *running compiled Raven* (the "in-Raven" state), which
    /// may hold live GC pointers in registers between safepoints. A collection
    /// waits for every such thread to reach a safepoint and park. A thread in a
    /// blocking call, or the Rust test harness (never running compiled Raven),
    /// is not counted and so never blocks a collection.
    running: AtomicUsize,
    /// In-Raven threads currently parked at a safepoint. When `parked` equals
    /// `running`, every in-Raven thread is stopped at a point with a complete
    /// shadow stack.
    parked: AtomicUsize,
    inner: Mutex<Inner>,
    /// Signaled when a region drains to zero or a thread parks with a stop
    /// pending (so a waiting collector re-checks) and when the world resumes
    /// (so parked threads and waiting collectors wake).
    cv: Condvar,
}

struct Inner {
    /// True while one collector holds the world stopped; serializes collectors.
    stopping: bool,
    /// Bumped on each resume so a parked thread cannot miss a wakeup that lands
    /// between its decision to park and the wait.
    epoch: u64,
}

impl StopTheWorld {
    pub const fn new() -> Self {
        StopTheWorld {
            stop_requested: AtomicBool::new(false),
            unsafe_count: AtomicUsize::new(0),
            running: AtomicUsize::new(0),
            parked: AtomicUsize::new(0),
            inner: Mutex::new(Inner {
                stopping: false,
                epoch: 0,
            }),
            cv: Condvar::new(),
        }
    }

    /// Mark the calling thread as now running compiled Raven (in-Raven). The
    /// scheduler calls this when it dispatches a goroutine, and the program
    /// entry calls it for the main thread. While in-Raven a thread must reach a
    /// [`safepoint`](Self::safepoint) for a collection to proceed. If a
    /// collection is already in progress the thread parks here first, so no new
    /// in-Raven thread starts running while the world is stopped.
    pub fn enter_running(&self) {
        loop {
            self.running.fetch_add(1, Ordering::SeqCst);
            if !self.stop_requested.load(Ordering::SeqCst) {
                return;
            }
            self.running.fetch_sub(1, Ordering::SeqCst);
            let mut inner = self.inner.lock().unwrap();
            // Dropping out may complete the collector's wait; nudge it.
            self.cv.notify_all();
            let epoch = inner.epoch;
            while self.stop_requested.load(Ordering::SeqCst) && inner.epoch == epoch {
                inner = self.cv.wait(inner).unwrap();
            }
        }
    }

    /// Mark the calling thread as no longer running compiled Raven (it is
    /// blocking, yielding, or about to collect). A collection no longer waits
    /// for it. If a stop is pending, dropping out of the running set may satisfy
    /// the collector's `parked == running` condition, so nudge it.
    pub fn exit_running(&self) {
        self.running.fetch_sub(1, Ordering::SeqCst);
        if self.stop_requested.load(Ordering::SeqCst) {
            let _guard = self.inner.lock().unwrap();
            self.cv.notify_all();
        }
    }

    /// A safepoint poll for an in-Raven thread: the back end emits a call to
    /// this at allocations and loop back-edges, points where every live GC
    /// pointer is on the shadow stack. Returns immediately when no collection is
    /// pending (one atomic load); otherwise the thread parks here until the
    /// world resumes, so the collector sees it stopped with a complete shadow
    /// stack.
    pub fn safepoint(&self) {
        // Loop so that if a fresh collection is already pending when this thread
        // wakes from a park, it re-parks instead of returning to the caller. A
        // thread that woke from collection N and ran even briefly before
        // re-parking for N+1 could be miscounted as parked for N+1 (and the
        // caller could run while the world is meant to be stopped). The thread
        // returns only when no stop is pending.
        while self.stop_requested.load(Ordering::SeqCst) {
            let mut inner = self.inner.lock().unwrap();
            // A resume may have landed between the load above and the lock.
            if !self.stop_requested.load(Ordering::SeqCst) {
                break;
            }
            self.parked.fetch_add(1, Ordering::SeqCst);
            // The collector may now observe `parked == running`.
            self.cv.notify_all();
            let epoch = inner.epoch;
            while self.stop_requested.load(Ordering::SeqCst) && inner.epoch == epoch {
                inner = self.cv.wait(inner).unwrap();
            }
            self.parked.fetch_sub(1, Ordering::SeqCst);
        }
    }

    /// Enter an unsafe region (about to mutate the heap or a shadow stack).
    /// Returns once it is safe to mutate. If a collection is pending the thread
    /// parks here until the world resumes, then retries, so it never mutates
    /// while the world is stopped.
    pub fn enter_unsafe(&self) {
        loop {
            // Optimistically count this thread as in an unsafe region, then
            // check for a pending stop. SeqCst pairs with the collector's
            // store(stop)/load(count) so the two cannot miss each other.
            self.unsafe_count.fetch_add(1, Ordering::SeqCst);
            if !self.stop_requested.load(Ordering::SeqCst) {
                return;
            }
            // A stop is pending: back out so the collector can drain to zero,
            // then park until the world resumes and try again.
            let dropped_to_zero = self.unsafe_count.fetch_sub(1, Ordering::SeqCst) == 1;
            let mut inner = self.inner.lock().unwrap();
            if dropped_to_zero {
                // This thread was the last in-flight region; nudge the collector.
                self.cv.notify_all();
            }
            let epoch = inner.epoch;
            while self.stop_requested.load(Ordering::SeqCst) && inner.epoch == epoch {
                inner = self.cv.wait(inner).unwrap();
            }
        }
    }

    /// Leave an unsafe region. If this was the last in-flight region and a
    /// collector is waiting, wake it.
    pub fn exit_unsafe(&self) {
        let was_last = self.unsafe_count.fetch_sub(1, Ordering::SeqCst) == 1;
        if was_last && self.stop_requested.load(Ordering::SeqCst) {
            let _guard = self.inner.lock().unwrap();
            self.cv.notify_all();
        }
    }

    /// Stop the world and return with it held. The caller must first leave both
    /// any unsafe region and the running set ([`exit_running`](Self::exit_running)),
    /// since a collection is triggered from inside the runtime, not from
    /// compiled Raven. After this returns: every in-flight unsafe region has
    /// drained, and every in-Raven thread is parked at a safepoint with a
    /// complete shadow stack, so the caller may mark and sweep. One collector
    /// runs at a time; a contending collector has also left the running set, so
    /// it does not block the active one.
    pub fn stop_the_world(&self) {
        let mut inner = self.inner.lock().unwrap();
        // Serialize collectors: wait out any in-progress stop. The waiting
        // collector has left the running set, so it does not block the active
        // collection.
        while inner.stopping {
            inner = self.cv.wait(inner).unwrap();
        }
        inner.stopping = true;
        self.stop_requested.store(true, Ordering::SeqCst);
        // Wait until no thread is mutating a shadow stack (unsafe regions
        // drained) and every in-Raven thread has parked at a safepoint. New
        // unsafe entries and safepoint arrivals park rather than proceeding, so
        // both conditions are reached as soon as the bounded in-flight work
        // finishes.
        while self.unsafe_count.load(Ordering::SeqCst) != 0
            || self.parked.load(Ordering::SeqCst) != self.running.load(Ordering::SeqCst)
        {
            inner = self.cv.wait(inner).unwrap();
        }
    }

    /// Resume the threads parked by a stop and let collectors contend again.
    pub fn resume_the_world(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.stopping = false;
        inner.epoch = inner.epoch.wrapping_add(1);
        self.stop_requested.store(false, Ordering::SeqCst);
        self.cv.notify_all();
    }
}

impl Default for StopTheWorld {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    /// With no thread in an unsafe region, a stop returns immediately.
    #[test]
    fn stops_immediately_when_no_unsafe_regions() {
        let stw = StopTheWorld::new();
        let start = Instant::now();
        stw.stop_the_world();
        stw.resume_the_world();
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    /// The deadlock the old barrier had: a thread that touched the coordinator
    /// once and then goes idle (never enters another unsafe region) must NOT
    /// block a collection. The old "wait for all to park" model hung here; this
    /// model treats the idle thread as already safe.
    #[test]
    fn idle_threads_do_not_block_a_collection() {
        let stw = Arc::new(StopTheWorld::new());
        let go_idle = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));

        // A worker that does a little unsafe-region work, then spins idle
        // (never entering an unsafe region again) until told to stop.
        let worker = {
            let stw = Arc::clone(&stw);
            let go_idle = Arc::clone(&go_idle);
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                for _ in 0..50 {
                    stw.enter_unsafe();
                    stw.exit_unsafe();
                }
                go_idle.store(true, Ordering::Release);
                while !stop.load(Ordering::Acquire) {
                    std::hint::spin_loop();
                }
            })
        };

        while !go_idle.load(Ordering::Acquire) {
            std::hint::spin_loop();
        }
        // The worker is now idle and not in any unsafe region. A stop must
        // complete promptly rather than wait for the idle worker.
        let start = Instant::now();
        stw.stop_the_world();
        let elapsed = start.elapsed();
        stw.resume_the_world();
        stop.store(true, Ordering::Release);
        worker.join().unwrap();
        assert!(
            elapsed < Duration::from_secs(5),
            "a stop blocked on an idle thread for {elapsed:?}"
        );
    }

    /// While the world is stopped, no thread is inside an unsafe region.
    /// Workers loop entering/exiting regions and assert the stop flag is never
    /// set while they are inside one; a collector stops, marks the window, and
    /// resumes, over many rounds.
    #[test]
    fn no_thread_is_unsafe_while_stopped() {
        const WORKERS: usize = 8;
        const ROUNDS: usize = 300;
        let stw = Arc::new(StopTheWorld::new());
        let world_stopped = Arc::new(AtomicBool::new(false));
        let escaped = Arc::new(AtomicU64::new(0));
        let done = Arc::new(AtomicBool::new(false));

        let mut handles = Vec::new();
        for _ in 0..WORKERS {
            let stw = Arc::clone(&stw);
            let world_stopped = Arc::clone(&world_stopped);
            let escaped = Arc::clone(&escaped);
            let done = Arc::clone(&done);
            handles.push(std::thread::spawn(move || {
                while !done.load(Ordering::Acquire) {
                    stw.enter_unsafe();
                    // Inside the region the world must not be stopped.
                    if world_stopped.load(Ordering::Acquire) {
                        escaped.fetch_add(1, Ordering::Relaxed);
                    }
                    stw.exit_unsafe();
                }
            }));
        }

        for _ in 0..ROUNDS {
            stw.stop_the_world();
            world_stopped.store(true, Ordering::Release);
            std::hint::spin_loop();
            world_stopped.store(false, Ordering::Release);
            stw.resume_the_world();
        }
        done.store(true, Ordering::Release);
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(
            escaped.load(Ordering::Relaxed),
            0,
            "a thread was inside an unsafe region while the world was stopped"
        );
    }

    /// Two collectors contending serialize: a non-atomic counter mutated only
    /// while the world is stopped stays exact.
    #[test]
    fn collectors_serialize() {
        const ROUNDS: usize = 500;
        let stw = Arc::new(StopTheWorld::new());
        let counter = Arc::new(std::cell::UnsafeCell::new(0u64));
        struct Wrap(Arc<std::cell::UnsafeCell<u64>>);
        // SAFETY: the cell is touched only between stop and resume, which the
        // coordinator serializes; this wrapper just moves the Arc across threads.
        unsafe impl Send for Wrap {}

        let mut handles = Vec::new();
        for _ in 0..2 {
            let stw = Arc::clone(&stw);
            let cell = Wrap(Arc::clone(&counter));
            handles.push(std::thread::spawn(move || {
                let cell = cell;
                for _ in 0..ROUNDS {
                    stw.stop_the_world();
                    // SAFETY: exclusive while the world is stopped.
                    unsafe {
                        *cell.0.get() += 1;
                    }
                    stw.resume_the_world();
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        // SAFETY: all threads joined.
        let total = unsafe { *counter.get() };
        assert_eq!(total, (2 * ROUNDS) as u64, "a collector stop overlapped");
    }

    /// Liveness: a collection always completes while workers churn through
    /// unsafe regions, guarded by a watchdog against deadlock.
    #[test]
    fn collections_make_progress_under_churn() {
        const ROUNDS: usize = 200;
        let stw = Arc::new(StopTheWorld::new());
        let done = Arc::new(AtomicBool::new(false));

        let mut handles = Vec::new();
        for _ in 0..6 {
            let stw = Arc::clone(&stw);
            let done = Arc::clone(&done);
            handles.push(std::thread::spawn(move || {
                while !done.load(Ordering::Acquire) {
                    stw.enter_unsafe();
                    std::hint::spin_loop();
                    stw.exit_unsafe();
                }
            }));
        }

        let collector_done = Arc::new(AtomicBool::new(false));
        let tripped = Arc::new(AtomicBool::new(false));
        let watchdog = {
            let collector_done = Arc::clone(&collector_done);
            let tripped = Arc::clone(&tripped);
            std::thread::spawn(move || {
                let start = Instant::now();
                while !collector_done.load(Ordering::Acquire) {
                    if start.elapsed() > Duration::from_secs(30) {
                        tripped.store(true, Ordering::Release);
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
            })
        };

        for _ in 0..ROUNDS {
            stw.stop_the_world();
            stw.resume_the_world();
        }
        collector_done.store(true, Ordering::Release);
        done.store(true, Ordering::Release);
        for h in handles {
            h.join().unwrap();
        }
        watchdog.join().unwrap();
        assert!(
            !tripped.load(Ordering::Acquire),
            "a collection deadlocked under unsafe-region churn"
        );
    }

    /// In-Raven threads must be parked at a safepoint while the world is
    /// stopped. Workers run in-Raven, polling safepoints; the collector is
    /// itself in-Raven and drops out of the running set to collect. No worker
    /// may observe the world stopped between its safepoints.
    #[test]
    fn in_raven_threads_park_at_safepoints_for_a_collection() {
        const WORKERS: usize = 6;
        const ROUNDS: usize = 200;
        let stw = Arc::new(StopTheWorld::new());
        let world_stopped = Arc::new(AtomicBool::new(false));
        let escaped = Arc::new(AtomicU64::new(0));
        let done = Arc::new(AtomicBool::new(false));

        let mut handles = Vec::new();
        for _ in 0..WORKERS {
            let stw = Arc::clone(&stw);
            let world_stopped = Arc::clone(&world_stopped);
            let escaped = Arc::clone(&escaped);
            let done = Arc::clone(&done);
            handles.push(std::thread::spawn(move || {
                stw.enter_running();
                while !done.load(Ordering::Acquire) {
                    stw.safepoint();
                    if world_stopped.load(Ordering::Acquire) {
                        escaped.fetch_add(1, Ordering::Relaxed);
                    }
                }
                stw.exit_running();
            }));
        }

        // The collector is an in-Raven thread that leaves the running set to
        // collect, then rejoins, exactly the allocation-triggered collect path.
        stw.enter_running();
        for _ in 0..ROUNDS {
            stw.exit_running();
            stw.stop_the_world();
            world_stopped.store(true, Ordering::Release);
            std::hint::spin_loop();
            world_stopped.store(false, Ordering::Release);
            stw.resume_the_world();
            stw.enter_running();
        }
        stw.exit_running();

        done.store(true, Ordering::Release);
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(
            escaped.load(Ordering::Relaxed),
            0,
            "an in-Raven thread ran while the world was stopped"
        );
    }

    /// A thread that has left the running set (blocked, in a syscall, or simply
    /// never running compiled Raven like the Rust test harness) must not block a
    /// collection.
    #[test]
    fn non_running_threads_do_not_block_a_collection() {
        let stw = Arc::new(StopTheWorld::new());
        let ready = Arc::new(AtomicBool::new(false));
        let release = Arc::new(AtomicBool::new(false));

        let worker = {
            let stw = Arc::clone(&stw);
            let ready = Arc::clone(&ready);
            let release = Arc::clone(&release);
            std::thread::spawn(move || {
                stw.enter_running();
                stw.safepoint();
                // Leave the running set and spin, never reaching a safepoint.
                stw.exit_running();
                ready.store(true, Ordering::Release);
                while !release.load(Ordering::Acquire) {
                    std::hint::spin_loop();
                }
            })
        };

        while !ready.load(Ordering::Acquire) {
            std::hint::spin_loop();
        }
        // The caller is not in the running set; the worker has left it. The
        // stop must complete promptly despite the spinning worker.
        let start = Instant::now();
        stw.stop_the_world();
        let elapsed = start.elapsed();
        stw.resume_the_world();
        release.store(true, Ordering::Release);
        worker.join().unwrap();
        assert!(
            elapsed < Duration::from_secs(5),
            "a stop blocked on a non-running thread for {elapsed:?}"
        );
    }
}
