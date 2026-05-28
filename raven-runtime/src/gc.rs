//! Stop-the-world, single-threaded, tracing mark-and-sweep garbage
//! collector for compiled Raven v2 programs.
//!
//! The collector finds its roots through a shadow stack the code
//! generator maintains: a runtime-owned stack of the addresses of the
//! stack slots that currently hold live GC pointers. Codegen registers
//! a frame's root array on entry and unregisters it on exit. See
//! `docs/v2/specs/gc.md` for the full design and the ABI contract.
//!
//! # Single-threaded assumption
//!
//! v2.0 compiled programs are single threaded. The collector state
//! lives in `thread_local!` cells, so each thread that ever touches the
//! runtime gets its own independent heap and shadow stack. Sharing GC
//! objects across threads is undefined in v2.0; the thread-local form
//! simply keeps the global state sound under Rust's aliasing rules
//! without a lock.

use std::cell::RefCell;

/// A registered root: the address of a stack slot that holds a GC
/// pointer (or null). The collector reads the slot's current value at
/// collection time, so a slot reassigned during the function body is
/// always observed at its live value.
type RootSlot = *mut *mut u8;

thread_local! {
    /// The shadow stack of root slot addresses, shared by the frame API
    /// and the per-slot API.
    static ROOTS: RefCell<Vec<RootSlot>> = const { RefCell::new(Vec::new()) };

    /// Frame boundaries into `ROOTS`. Each entry is the `ROOTS` length
    /// at the moment a frame was entered; leaving a frame truncates
    /// `ROOTS` back to that length.
    static FRAMES: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
}

/// Register a single root slot on the shadow stack.
///
/// `slot` is the address of a stack local holding a GC pointer. It must
/// stay valid (the local must outlive the matching pop) and must be
/// paired with a later `raven_gc_pop_roots`.
///
/// # Safety
///
/// `slot` must point to a writable, properly aligned `*mut u8` that
/// remains live until it is popped.
#[no_mangle]
pub extern "C" fn raven_gc_push_root(slot: *mut *mut u8) {
    if slot.is_null() {
        return;
    }
    ROOTS.with(|r| r.borrow_mut().push(slot));
}

/// Pop the last `n` root slots off the shadow stack.
///
/// Popping more slots than are registered clears the stack rather than
/// underflowing.
#[no_mangle]
pub extern "C" fn raven_gc_pop_roots(n: usize) {
    ROOTS.with(|r| {
        let mut roots = r.borrow_mut();
        let new_len = roots.len().saturating_sub(n);
        roots.truncate(new_len);
    });
}

/// Register a frame's root array on the shadow stack.
///
/// `roots` points to `count` contiguous slot addresses, each the
/// address of a stack local that holds a GC pointer (or null). The
/// array must outlive the matching `raven_gc_leave_frame` call (it
/// normally lives in the caller's frame). Frames nest in strict
/// last-in-first-out order.
///
/// # Safety
///
/// `roots` must point to `count` readable, properly aligned
/// `*mut *mut u8` entries, each of which stays live until the matching
/// `raven_gc_leave_frame`.
#[no_mangle]
pub extern "C" fn raven_gc_enter_frame(roots: *mut *mut u8, count: usize) {
    ROOTS.with(|r| {
        let mut stack = r.borrow_mut();
        FRAMES.with(|f| f.borrow_mut().push(stack.len()));
        if !roots.is_null() {
            for i in 0..count {
                // SAFETY: caller guarantees `roots` has `count` entries.
                let slot = unsafe { roots.add(i) };
                stack.push(slot);
            }
        }
    });
}

/// Unregister the most recently registered frame, truncating the shadow
/// stack back to the boundary recorded by the matching
/// `raven_gc_enter_frame`.
///
/// A call with no open frame is a no-op.
#[no_mangle]
pub extern "C" fn raven_gc_leave_frame() {
    let boundary = FRAMES.with(|f| f.borrow_mut().pop());
    if let Some(boundary) = boundary {
        ROOTS.with(|r| {
            let mut roots = r.borrow_mut();
            // Defensive: never grow past the current length.
            let target = boundary.min(roots.len());
            roots.truncate(target);
        });
    }
}

/// Number of root slots currently registered. Test and diagnostic aid.
#[cfg(test)]
pub(crate) fn root_count() -> usize {
    ROOTS.with(|r| r.borrow().len())
}

#[cfg(test)]
mod shadow_stack_tests {
    use super::*;

    /// Each test runs on its own thread so the thread-local shadow
    /// stack starts empty and does not leak state into sibling tests.
    fn isolated(body: impl FnOnce() + Send + 'static) {
        std::thread::spawn(body).join().unwrap();
    }

    #[test]
    fn push_and_pop_track_root_count() {
        isolated(|| {
            assert_eq!(root_count(), 0);
            let mut a: *mut u8 = std::ptr::null_mut();
            let mut b: *mut u8 = std::ptr::null_mut();
            raven_gc_push_root(&mut a as *mut *mut u8);
            raven_gc_push_root(&mut b as *mut *mut u8);
            assert_eq!(root_count(), 2);
            raven_gc_pop_roots(2);
            assert_eq!(root_count(), 0);
        });
    }

    #[test]
    fn pop_more_than_present_clears_stack() {
        isolated(|| {
            let mut a: *mut u8 = std::ptr::null_mut();
            raven_gc_push_root(&mut a as *mut *mut u8);
            raven_gc_pop_roots(10);
            assert_eq!(root_count(), 0);
        });
    }

    #[test]
    fn frames_nest_last_in_first_out() {
        isolated(|| {
            let mut outer: [*mut u8; 2] = [std::ptr::null_mut(); 2];
            let mut inner: [*mut u8; 3] = [std::ptr::null_mut(); 3];
            raven_gc_enter_frame(outer.as_mut_ptr(), 2);
            assert_eq!(root_count(), 2);
            raven_gc_enter_frame(inner.as_mut_ptr(), 3);
            assert_eq!(root_count(), 5);
            raven_gc_leave_frame();
            assert_eq!(root_count(), 2);
            raven_gc_leave_frame();
            assert_eq!(root_count(), 0);
        });
    }

    #[test]
    fn leave_frame_without_open_frame_is_noop() {
        isolated(|| {
            raven_gc_leave_frame();
            assert_eq!(root_count(), 0);
        });
    }
}
