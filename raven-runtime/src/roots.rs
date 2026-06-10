//! Cross-thread root registry for the multi-threaded collector.
//!
//! Today the shadow-stack roots live in `thread_local!` cells, so a collection
//! can only see the running thread's roots. Once the heap is shared across an
//! OS-thread pool, a collection on one thread must scan *every* thread's roots,
//! or it would free objects another thread still holds (see
//! `docs/v2/specs/concurrency-parallelism.md`). This module is that registry,
//! built and tested on its own before it is wired into the allocator.
//!
//! Each mutator thread owns a [`RootContext`] (its shadow stack) and registers
//! a pointer to it here. The collector enumerates every registered context's
//! roots. Safety of the cross-thread read is provided by the stop-the-world
//! coordinator (`crate::stw`): a collection enumerates the registry only while
//! every other mutator is parked at a safepoint, so no context is mutated
//! during the scan. This module owns only the registry bookkeeping; the
//! coordination lives in `stw`.

use std::sync::Mutex;

/// A registered root: the address of a stack slot holding a GC pointer (or
/// null). The collector reads the slot's current value at collection time.
pub type RootSlot = *mut *mut u8;

/// One thread's shadow stack: the root slots it has registered and the frame
/// boundaries into them. A thread mutates only its own context, and only the
/// collector reads other threads' contexts (while they are parked), so no lock
/// guards a context's contents.
pub struct RootContext {
    /// Addresses of the stack slots holding live GC pointers.
    pub roots: Vec<RootSlot>,
    /// `roots` length at each open frame's entry; leaving a frame truncates
    /// `roots` back to the recorded length.
    pub frames: Vec<usize>,
    /// Deferred-closure addresses, one inner vector per open call frame. Stored
    /// here, alongside the shadow-stack roots, so the collector marks a
    /// goroutine's parked defers through the same per-thread registry that
    /// covers every worker, and so the defers travel with the goroutine when it
    /// migrates worker threads. Held as `*mut u8` to keep this module free of
    /// object-layout types.
    pub defer_frames: Vec<Vec<*mut u8>>,
}

impl RootContext {
    pub const fn new() -> Self {
        RootContext {
            roots: Vec::new(),
            frames: Vec::new(),
            defer_frames: Vec::new(),
        }
    }
}

impl Default for RootContext {
    fn default() -> Self {
        Self::new()
    }
}

/// The set of every live thread's [`RootContext`], by raw pointer. A thread
/// registers its context when it first touches the heap and deregisters when
/// it stops. The collector enumerates them all during a stop-the-world.
pub struct RootRegistry {
    contexts: Mutex<Vec<*mut RootContext>>,
}

// SAFETY: the registry stores raw pointers to per-thread contexts. The pointers
// are only dereferenced by the collector during a stop-the-world, when every
// owning thread is parked and not mutating its context; the `Mutex` guards the
// pointer vector itself. Asserting `Send`/`Sync` lets the registry live in a
// shared global.
unsafe impl Send for RootRegistry {}
unsafe impl Sync for RootRegistry {}

impl RootRegistry {
    pub const fn new() -> Self {
        RootRegistry {
            contexts: Mutex::new(Vec::new()),
        }
    }

    /// Register `ctx` as a live thread's root context. The pointer must stay
    /// valid until a matching [`deregister`](Self::deregister).
    pub fn register(&self, ctx: *mut RootContext) {
        self.contexts.lock().unwrap().push(ctx);
    }

    /// Remove `ctx` from the registry. The collector will no longer scan it.
    pub fn deregister(&self, ctx: *mut RootContext) {
        self.contexts.lock().unwrap().retain(|&p| p != ctx);
    }

    /// Apply `visit` to every non-null root slot in every registered context.
    ///
    /// # Safety
    ///
    /// The caller must ensure no registered context is being mutated for the
    /// duration of the call (the stop-the-world coordinator guarantees this at
    /// collection time). The registered pointers must be valid.
    pub unsafe fn for_each_root_slot(&self, visit: &mut dyn FnMut(RootSlot)) {
        let contexts = self.contexts.lock().unwrap();
        for &ctx in contexts.iter() {
            // SAFETY: a registered pointer is valid and, per the contract, not
            // being mutated for the duration of this call.
            let ctx = unsafe { &*ctx };
            for &slot in &ctx.roots {
                if !slot.is_null() {
                    visit(slot);
                }
            }
            // Parked deferred closures are roots too: hand the collector the
            // address of each slot holding a closure pointer.
            for frame in &ctx.defer_frames {
                for closure_slot in frame.iter() {
                    visit(closure_slot as *const *mut u8 as RootSlot);
                }
            }
        }
    }
}

impl Default for RootRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The core guarantee: a collection enumerates the roots of *every*
    /// registered context, not just one. Two contexts each hold a slot
    /// pointing at a distinct object; enumeration must surface both.
    #[test]
    fn enumerates_roots_across_all_registered_contexts() {
        let reg = RootRegistry::new();
        let mut obj_a: *mut u8 = 0x1000 as *mut u8;
        let mut obj_b: *mut u8 = 0x2000 as *mut u8;
        let mut ctx_a = RootContext {
            roots: vec![&mut obj_a as *mut *mut u8],
            frames: vec![],
            defer_frames: vec![],
        };
        let mut ctx_b = RootContext {
            roots: vec![&mut obj_b as *mut *mut u8],
            frames: vec![],
            defer_frames: vec![],
        };
        reg.register(&mut ctx_a);
        reg.register(&mut ctx_b);

        let mut seen: Vec<*mut u8> = Vec::new();
        // SAFETY: both contexts outlive this call and are not mutated.
        unsafe {
            reg.for_each_root_slot(&mut |slot| seen.push(*slot));
        }
        reg.deregister(&mut ctx_a);
        reg.deregister(&mut ctx_b);

        assert!(seen.contains(&obj_a), "context A's root was not enumerated");
        assert!(seen.contains(&obj_b), "context B's root was not enumerated");
    }

    /// A frame may hold a null slot (an uninitialized root); enumeration skips
    /// it rather than handing the collector a null address to dereference.
    #[test]
    fn null_slots_are_skipped() {
        let reg = RootRegistry::new();
        let mut obj: *mut u8 = 0x4000 as *mut u8;
        let mut ctx = RootContext {
            roots: vec![std::ptr::null_mut(), &mut obj as *mut *mut u8],
            frames: vec![],
            defer_frames: vec![],
        };
        reg.register(&mut ctx);

        let mut seen = 0;
        // SAFETY: the context outlives the call and is not mutated.
        unsafe {
            reg.for_each_root_slot(&mut |_| seen += 1);
        }
        reg.deregister(&mut ctx);
        assert_eq!(seen, 1, "the null slot should have been skipped");
    }

    /// A deregistered context is no longer scanned.
    #[test]
    fn deregistered_context_is_not_enumerated() {
        let reg = RootRegistry::new();
        let mut obj: *mut u8 = 0x3000 as *mut u8;
        let mut ctx = RootContext {
            roots: vec![&mut obj as *mut *mut u8],
            frames: vec![],
            defer_frames: vec![],
        };
        reg.register(&mut ctx);
        reg.deregister(&mut ctx);

        let mut count = 0;
        // SAFETY: nothing registered remains.
        unsafe {
            reg.for_each_root_slot(&mut |_| count += 1);
        }
        assert_eq!(count, 0, "a deregistered context was still enumerated");
    }
}
