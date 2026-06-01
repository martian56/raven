//! Cross-crate integration tests for the tracing collector.
//!
//! These play the role the v2 code generator (issue #67) will play:
//! they register roots on the shadow stack, allocate objects through the
//! collector's constructors, force collections, and assert that
//! reachable objects survive while unreachable ones (including cycles)
//! are reclaimed. They run through the public C ABI surface only, so a
//! failure here means an exported symbol regressed or the collector's
//! observable behaviour changed.
//!
//! Every test runs on its own thread because the collector state is
//! thread local: a fresh thread gives each test a clean heap, shadow
//! stack, and counters.

use raven_runtime::{
    raven_box_new, raven_box_payload, raven_closure_captures, raven_closure_new, raven_gc_collect,
    raven_gc_enter_frame, raven_gc_leave_frame, raven_gc_live_objects, raven_gc_pop_roots,
    raven_gc_push_root, raven_list_new, raven_map_buckets, raven_map_new, raven_string_new,
};

/// Run a test body on a dedicated thread so the thread-local collector
/// state starts empty.
fn isolated(body: impl FnOnce() + Send + 'static) {
    std::thread::spawn(body).join().unwrap();
}

extern "C" fn dummy_body() {}

#[test]
fn rooted_survives_unrooted_collected_cross_crate() {
    isolated(|| {
        let kept = raven_string_new(8);
        let dropped = raven_string_new(8);
        assert!(!kept.is_null() && !dropped.is_null());
        assert_eq!(raven_gc_live_objects(), 2);

        let mut slot: *mut u8 = kept as *mut u8;
        raven_gc_push_root(&mut slot as *mut *mut u8);
        let _ = dropped; // intentionally unrooted

        raven_gc_collect();
        // Only the rooted string survives.
        assert_eq!(raven_gc_live_objects(), 1);

        raven_gc_pop_roots(1);
        raven_gc_collect();
        assert_eq!(raven_gc_live_objects(), 0);
    });
}

#[test]
fn cycle_is_reclaimed_cross_crate() {
    isolated(|| {
        // Two single-slot pointer lists referencing each other, unrooted.
        let a = raven_list_new(8, 8, 1, 1);
        let b = raven_list_new(8, 8, 1, 1);
        assert!(!a.is_null() && !b.is_null());
        // SAFETY: each list has one pointer slot.
        unsafe {
            let a_slots = raven_runtime::raven_list_elements(a) as *mut *mut u8;
            a_slots.write(b as *mut u8);
            (*a).header.len = 1;
            let b_slots = raven_runtime::raven_list_elements(b) as *mut *mut u8;
            b_slots.write(a as *mut u8);
            (*b).header.len = 1;
        }
        assert_eq!(raven_gc_live_objects(), 2);
        // Mark-sweep reclaims the cycle that refcounting would leak.
        raven_gc_collect();
        assert_eq!(raven_gc_live_objects(), 0);
    });
}

#[test]
fn frame_api_roots_a_graph_cross_crate() {
    isolated(|| {
        // Build a graph: a map with one GC key and value, a closure with
        // one GC capture, and a box wrapping a GC pointer, plus their
        // leaf strings. Root the three containers through one frame and
        // confirm the whole graph survives.
        let map_key = raven_string_new(2);
        let map_value = raven_string_new(2);
        let map = raven_map_new(4, 1, 1);
        // SAFETY: write one live entry into the first bucket.
        unsafe {
            let buckets = raven_map_buckets(map);
            let e = &mut *buckets.add(0);
            e.hash = 1;
            e.key = map_key as *mut u8;
            e.value = map_value as *mut u8;
            (*map).header.len = 1;
        }

        let captured = raven_string_new(2);
        let closure = raven_closure_new(dummy_body as *const u8, 8, 8, 1, 1);
        // SAFETY: the capture buffer holds one pointer.
        unsafe {
            let caps = raven_closure_captures(closure) as *mut *mut u8;
            caps.write(captured as *mut u8);
        }

        let boxed_inner = raven_string_new(2);
        let boxed = raven_box_new(8, 8, 1);
        // SAFETY: the payload holds one pointer.
        unsafe {
            let payload = raven_box_payload(boxed) as *mut *mut u8;
            payload.write(boxed_inner as *mut u8);
        }

        // map, map_key, map_value, closure, captured, boxed, boxed_inner.
        assert_eq!(raven_gc_live_objects(), 7);

        // The container pointers live in their own slots; the frame's root
        // array holds the *addresses* of those slots, matching the ABI
        // codegen emits (each entry is a slot address the collector
        // dereferences to read the live pointer).
        let mut slots: [*mut u8; 3] = [map as *mut u8, closure as *mut u8, boxed as *mut u8];
        let mut roots: [*mut *mut u8; 3] = [
            &mut slots[0] as *mut *mut u8,
            &mut slots[1] as *mut *mut u8,
            &mut slots[2] as *mut *mut u8,
        ];
        raven_gc_enter_frame(roots.as_mut_ptr() as *mut *mut u8, roots.len());
        raven_gc_collect();
        assert_eq!(raven_gc_live_objects(), 7);

        raven_gc_leave_frame();
        raven_gc_collect();
        assert_eq!(raven_gc_live_objects(), 0);
    });
}

#[test]
fn bounded_liveness_stress_cross_crate() {
    isolated(|| {
        // Root a small working set, then churn many unrooted strings.
        // Liveness must stay bounded and the working set must survive.
        const WORKING_SET: usize = 8;
        // The working-set pointers live in their own slots; the root array
        // holds the *addresses* of those slots, matching the frame ABI
        // codegen emits.
        let mut slots: [*mut u8; WORKING_SET] = [std::ptr::null_mut(); WORKING_SET];
        for s in slots.iter_mut() {
            *s = raven_string_new(8) as *mut u8;
        }
        let mut roots: [*mut *mut u8; WORKING_SET] = [std::ptr::null_mut(); WORKING_SET];
        for (r, s) in roots.iter_mut().zip(slots.iter_mut()) {
            *r = s as *mut *mut u8;
        }
        raven_gc_enter_frame(roots.as_mut_ptr() as *mut *mut u8, WORKING_SET);

        let mut peak = 0usize;
        for i in 0..20_000usize {
            let _garbage = raven_string_new(8);
            if i % 128 == 0 {
                raven_gc_collect();
                peak = peak.max(raven_gc_live_objects());
            }
        }
        raven_gc_collect();
        assert_eq!(raven_gc_live_objects(), WORKING_SET);
        assert!(
            peak <= WORKING_SET + 128,
            "liveness peaked at {peak}, expected bounded"
        );

        raven_gc_leave_frame();
        raven_gc_collect();
        assert_eq!(raven_gc_live_objects(), 0);
    });
}
