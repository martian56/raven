//! Stop-the-world coordination for the multi-threaded collector.
//!
//! The garbage collector is stop-the-world: before it marks and sweeps the
//! shared heap, every other mutator thread must reach a **safepoint** and park,
//! so the collector sees a quiescent heap and a consistent set of roots. This
//! module is that coordination, built and tested on its own before it guards
//! real memory (see `docs/v2/specs/concurrency-parallelism.md`).
//!
//! Why a cooperative safepoint protocol and not a single global lock: the
//! shadow-stack root operations (`enter_frame`/`leave_frame`) run on every
//! function call and must stay lock-free, or every call would pay a mutex
//! round trip (measured at roughly an order of magnitude slowdown on
//! call-heavy code). So mutators run lock-free and instead *poll* a single
//! atomic at coarse points (each allocation, and later loop back-edges); when a
//! collection is pending the poll parks the thread at a point where its roots
//! are consistent. The collector waits for every other registered thread to
//! park, runs, then resumes them.
//!
//! The common case is cheap: when no collection is pending, [`poll`] is a
//! single relaxed atomic load and returns. A single-threaded program (one
//! registered thread, which is also the only collector) never waits at all:
//! `stop_the_world` finds zero other threads and returns immediately.
//!
//! [`poll`]: StopTheWorld::poll

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};

/// Coordinates a stop-the-world pause across the registered mutator threads.
///
/// A mutator thread calls [`register`](Self::register) once it may touch the
/// heap and [`deregister`](Self::deregister) when it stops, and [`poll`](Self::poll)
/// at each safepoint. A thread that needs to collect calls
/// [`stop_the_world`](Self::stop_the_world), does the collection while it holds
/// the world stopped, then [`resume_the_world`](Self::resume_the_world).
pub struct StopTheWorld {
    /// Fast-path flag read at every safepoint without locking. False in the
    /// common case, so [`poll`](Self::poll) returns after one atomic load.
    /// Written only under `inner`'s lock, with release ordering, so a thread
    /// that observes `true` and then locks sees the matching `Inner` state.
    stop_requested: AtomicBool,
    inner: Mutex<Inner>,
    /// Signaled when a thread parks (so a waiting collector re-checks the
    /// count) and when the world resumes (so parked threads wake). One condvar
    /// is enough because every waiter re-tests its predicate in a loop.
    cv: Condvar,
}

struct Inner {
    /// Threads that may touch the heap and therefore must be stopped before a
    /// collection. The collector is itself registered (a collection is
    /// triggered by an allocation), so the count includes the collector.
    registered: usize,
    /// Threads currently parked at a safepoint.
    parked: usize,
    /// True while one collector holds the world stopped; serializes collectors
    /// so only one stop is in flight at a time.
    stopping: bool,
    /// Bumped on each resume. A parked thread waits for it to change, so a
    /// resume that happens between the thread deciding to park and actually
    /// waiting cannot be lost.
    epoch: u64,
}

impl StopTheWorld {
    pub const fn new() -> Self {
        StopTheWorld {
            stop_requested: AtomicBool::new(false),
            inner: Mutex::new(Inner {
                registered: 0,
                parked: 0,
                stopping: false,
                epoch: 0,
            }),
            cv: Condvar::new(),
        }
    }

    /// Register the calling thread as a mutator that must be stopped for a
    /// collection. If a collection is already in progress the new thread parks
    /// immediately, so it cannot run free (and mutate the heap) during a stop.
    pub fn register(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.registered += 1;
        self.park_while_stopping(inner);
    }

    /// Deregister the calling thread (it will no longer touch the heap). If a
    /// collector is waiting for the world to stop, removing this thread may
    /// complete the count, so wake the collector to re-check.
    pub fn deregister(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.registered -= 1;
        // The leaving thread is running (not parked), so `parked` is unchanged
        // but the target `registered - 1` dropped; a waiting collector may now
        // be satisfied.
        if inner.stopping {
            self.cv.notify_all();
        }
    }

    /// A safepoint poll. Returns immediately when no collection is pending (the
    /// common case, one relaxed load). Otherwise the thread parks until the
    /// collection finishes, so the collector observes it stopped with
    /// consistent roots.
    pub fn poll(&self) {
        if !self.stop_requested.load(Ordering::Acquire) {
            return;
        }
        let inner = self.inner.lock().unwrap();
        self.park_while_stopping(inner);
    }

    /// Park the calling thread while a stop is in progress, counting it as
    /// parked so a waiting collector can make progress. Consumes the held lock
    /// guard and returns once the world is running again. Used by both
    /// [`poll`](Self::poll) and [`register`](Self::register).
    fn park_while_stopping(&self, mut inner: std::sync::MutexGuard<'_, Inner>) {
        while inner.stopping {
            inner.parked += 1;
            // Wake a collector that may be waiting for the last thread to park.
            self.cv.notify_all();
            let epoch = inner.epoch;
            // Wait for a resume (epoch change). The loop guards against
            // spurious wakeups and against a new stop starting before this
            // thread leaves the park.
            while inner.stopping && inner.epoch == epoch {
                inner = self.cv.wait(inner).unwrap();
            }
            inner.parked -= 1;
        }
    }

    /// Stop every other registered thread and return with the world held
    /// stopped. The caller must be a registered mutator (a collection is
    /// triggered from an allocation), so it waits for the other
    /// `registered - 1` threads to park. After this returns, no other
    /// registered thread is running until [`resume_the_world`](Self::resume_the_world);
    /// the caller may mark and sweep the shared heap exclusively.
    pub fn stop_the_world(&self) {
        let mut inner = self.inner.lock().unwrap();
        // Only one collector at a time. If another already holds the world,
        // park here (this thread is idle, not mutating, so it counts toward
        // the parked total) until that collection resumes, then contend again.
        // Counting a contending collector as parked is essential: otherwise an
        // active collector would wait forever for a thread that is itself
        // blocked trying to collect and never reaches a `poll` safepoint.
        while inner.stopping {
            inner.parked += 1;
            self.cv.notify_all();
            let epoch = inner.epoch;
            while inner.stopping && inner.epoch == epoch {
                inner = self.cv.wait(inner).unwrap();
            }
            inner.parked -= 1;
        }
        // No collection in progress and the lock is held continuously from
        // here, so this thread alone claims the stop.
        inner.stopping = true;
        self.stop_requested.store(true, Ordering::Release);
        // Wait until every other registered thread has parked. `registered`
        // includes the caller, which is at a safepoint by definition.
        while inner.parked < inner.registered.saturating_sub(1) {
            inner = self.cv.wait(inner).unwrap();
        }
        // Hold the invariant by leaving `stopping` true; parked threads will
        // not leave their park until the epoch bumps in `resume_the_world`.
    }

    /// Resume the threads parked by [`stop_the_world`](Self::stop_the_world).
    pub fn resume_the_world(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.stopping = false;
        inner.epoch = inner.epoch.wrapping_add(1);
        self.stop_requested.store(false, Ordering::Release);
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
    use std::sync::atomic::{AtomicU64, AtomicUsize};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    /// A single-threaded program registers one thread (itself) and collects
    /// with no other threads to wait for: stop returns at once.
    #[test]
    fn single_thread_stops_immediately() {
        let stw = StopTheWorld::new();
        stw.register();
        // No other registered threads, so this must not block.
        let start = Instant::now();
        stw.stop_the_world();
        stw.resume_the_world();
        assert!(start.elapsed() < Duration::from_secs(1));
        stw.deregister();
    }

    /// `poll` is a no-op when no stop is pending.
    #[test]
    fn poll_without_stop_is_a_noop() {
        let stw = StopTheWorld::new();
        stw.register();
        for _ in 0..1000 {
            stw.poll();
        }
        stw.deregister();
    }

    /// The core invariant: while one thread holds the world stopped, no other
    /// registered thread runs its critical region. Each worker increments a
    /// plain (non-atomic) counter only when not stopped; the collector mutates
    /// it under the stop. If the stop were not exclusive, the unsynchronized
    /// accesses would race and the final accounting would not add up. Run many
    /// rounds to shake out timing bugs.
    #[test]
    fn stop_is_exclusive_of_mutators() {
        const WORKERS: usize = 8;
        const ROUNDS: usize = 200;
        let stw = Arc::new(StopTheWorld::new());
        // Number of collections observed by each worker as "world running".
        let progress = Arc::new(AtomicU64::new(0));
        // A flag set true only while the world is stopped; a worker that ever
        // sees it true at a safepoint has escaped the stop (a bug).
        let world_stopped = Arc::new(AtomicBool::new(false));
        let escaped = Arc::new(AtomicUsize::new(0));

        let mut handles = Vec::new();
        for _ in 0..WORKERS {
            let stw = Arc::clone(&stw);
            let progress = Arc::clone(&progress);
            let world_stopped = Arc::clone(&world_stopped);
            let escaped = Arc::clone(&escaped);
            handles.push(std::thread::spawn(move || {
                stw.register();
                for _ in 0..ROUNDS * WORKERS {
                    stw.poll();
                    // Between safepoints the world must be running for us.
                    if world_stopped.load(Ordering::Acquire) {
                        escaped.fetch_add(1, Ordering::Relaxed);
                    }
                    progress.fetch_add(1, Ordering::Relaxed);
                }
                stw.deregister();
            }));
        }

        // The collector thread.
        let coll = {
            let stw = Arc::clone(&stw);
            let world_stopped = Arc::clone(&world_stopped);
            std::thread::spawn(move || {
                stw.register();
                for _ in 0..ROUNDS {
                    stw.stop_the_world();
                    // Exclusive region: set, then clear, the stopped flag. No
                    // worker may observe it set at a safepoint.
                    world_stopped.store(true, Ordering::Release);
                    // A little work to widen the window.
                    std::hint::spin_loop();
                    world_stopped.store(false, Ordering::Release);
                    stw.resume_the_world();
                }
                stw.deregister();
            })
        };

        for h in handles {
            h.join().unwrap();
        }
        coll.join().unwrap();
        assert_eq!(
            escaped.load(Ordering::Relaxed),
            0,
            "a worker ran its safepoint region while the world was stopped"
        );
    }

    /// Liveness: a collection always completes even as workers register and
    /// deregister (model goroutines starting and finishing). A deadlock would
    /// hang the test; guard it with a watchdog thread.
    #[test]
    fn collections_make_progress_under_churn() {
        const ROUNDS: usize = 100;
        let stw = Arc::new(StopTheWorld::new());
        let done = Arc::new(AtomicBool::new(false));

        // Workers that come and go, polling while alive.
        let mut handles = Vec::new();
        for w in 0..6 {
            let stw = Arc::clone(&stw);
            let done = Arc::clone(&done);
            handles.push(std::thread::spawn(move || {
                while !done.load(Ordering::Acquire) {
                    stw.register();
                    // Short-lived: poll a handful of times, then leave.
                    for _ in 0..(w + 1) * 3 {
                        stw.poll();
                    }
                    stw.deregister();
                }
            }));
        }

        let watchdog_tripped = Arc::new(AtomicBool::new(false));
        let collector_done = Arc::new(AtomicBool::new(false));
        let watchdog = {
            let tripped = Arc::clone(&watchdog_tripped);
            let collector_done = Arc::clone(&collector_done);
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

        // The collector keeps stopping the world while workers churn.
        let stw_c = Arc::clone(&stw);
        stw_c.register();
        for _ in 0..ROUNDS {
            stw_c.stop_the_world();
            stw_c.resume_the_world();
        }
        stw_c.deregister();
        collector_done.store(true, Ordering::Release);

        done.store(true, Ordering::Release);
        for h in handles {
            h.join().unwrap();
        }
        watchdog.join().unwrap();
        assert!(
            !watchdog_tripped.load(Ordering::Acquire),
            "a collection deadlocked under register/deregister churn"
        );
    }

    /// Two collectors contending serialize: the world is never stopped by both
    /// at once. Each increments a shared non-atomic counter inside its stop;
    /// if the stops overlapped the count would be lost.
    #[test]
    fn collectors_serialize() {
        const ROUNDS: usize = 500;
        let stw = Arc::new(StopTheWorld::new());
        // A counter guarded only by mutual exclusion of the stop region.
        let counter = Arc::new(std::cell::UnsafeCell::new(0u64));
        struct Send2(Arc<std::cell::UnsafeCell<u64>>);
        // SAFETY: access to the cell happens only inside `stop_the_world`/
        // `resume_the_world`, which the coordinator serializes; this wrapper
        // just lets the Arc cross the thread boundary for the test.
        unsafe impl Send for Send2 {}

        let mut handles = Vec::new();
        for _ in 0..2 {
            let stw = Arc::clone(&stw);
            let cell = Send2(Arc::clone(&counter));
            handles.push(std::thread::spawn(move || {
                let cell = cell;
                stw.register();
                for _ in 0..ROUNDS {
                    stw.stop_the_world();
                    // SAFETY: exclusive while the world is stopped.
                    unsafe {
                        let p = cell.0.get();
                        *p += 1;
                    }
                    stw.resume_the_world();
                }
                stw.deregister();
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        // SAFETY: all threads joined.
        let total = unsafe { *counter.get() };
        assert_eq!(total, (2 * ROUNDS) as u64, "a collector stop overlapped");
    }
}
