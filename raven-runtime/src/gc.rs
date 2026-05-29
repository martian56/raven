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

use crate::object::structval::{STRUCT_FIELDS_OFFSET, STRUCT_FIELD_SLOT};
use crate::object::{
    free_object_buffers, object_body_layout, Closure, List, Map, ObjectHeader, Set, TAG_BOX,
    TAG_CLOSURE, TAG_LIST, TAG_MAP, TAG_SET, TAG_STRUCT,
};
use crate::object::{MapEntry, SetEntry, BOX_PAYLOAD_OFFSET};
use crate::{raven_alloc, raven_dealloc};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;

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

    /// The all-objects list: the base pointer of every object handed
    /// out by `raven_gc_alloc`. The sweeper walks it once per cycle.
    static HEAP: RefCell<Vec<*mut ObjectHeader>> = const { RefCell::new(Vec::new()) };

    /// Live object-body bytes. Drives the collection threshold.
    static BYTES_ALLOCATED: Cell<usize> = const { Cell::new(0) };

    /// Live object count. Lets tests assert bounded liveness without
    /// measuring flaky OS memory.
    static LIVE_OBJECTS: Cell<usize> = const { Cell::new(0) };

    /// Allocation high-water mark: a collection runs before serving an
    /// allocation that would carry `bytes_allocated` past this. Reset
    /// after each collection to a multiple of the surviving live bytes.
    static THRESHOLD: Cell<usize> = const { Cell::new(INITIAL_THRESHOLD) };

    /// Per-struct-type GC pointer descriptors. The key is the type id the
    /// back-end assigns to each monomorphic struct type; the value is a
    /// bitmask where bit `i` is set when field slot `i` holds a GC
    /// pointer the collector must trace. The back-end registers every
    /// struct type once at program startup, before any struct is built.
    static STRUCT_DESCRIPTORS: RefCell<HashMap<u32, u64>> =
        RefCell::new(HashMap::new());

    /// Per-call-frame stacks of deferred closures, one inner vector per
    /// open call frame. A `defer expr` pushes its thunk closure onto the
    /// top frame; the function epilogue runs and pops the top frame at
    /// every return. Parked closures stay GC-reachable through `mark`,
    /// which visits every pointer in every open defer frame.
    static DEFER_FRAMES: RefCell<Vec<Vec<*mut crate::object::Closure>>> =
        const { RefCell::new(Vec::new()) };
}

/// ABI of a deferred thunk: the runtime calls the closure's lifted body
/// through this pointer, passing the closure's capture environment. The
/// thunk evaluates the deferred expression for its side effects and
/// returns nothing.
type DeferThunk = extern "C" fn(env: *mut u8);

/// Open a fresh defer frame for the current call. The function epilogue
/// must pair it with one `raven_defer_run_frame`.
#[no_mangle]
pub extern "C" fn raven_defer_enter_frame() {
    DEFER_FRAMES.with(|f| f.borrow_mut().push(Vec::new()));
}

/// Register a deferred thunk on the current defer frame.
///
/// `closure` is a `Closure` whose lifted body takes only the capture
/// environment and returns unit. It is parked until the frame runs, and
/// stays GC-reachable in the meantime because `mark` visits it.
///
/// A push with no open frame is a no-op, which keeps a stray `defer`
/// outside any frame from corrupting the stack.
///
/// # Safety
///
/// `closure` must be a live `Closure` produced by `raven_closure_new`.
#[no_mangle]
pub extern "C" fn raven_defer_push(closure: *mut crate::object::Closure) {
    if closure.is_null() {
        return;
    }
    DEFER_FRAMES.with(|f| {
        if let Some(top) = f.borrow_mut().last_mut() {
            top.push(closure);
        }
    });
}

/// Run and pop the current defer frame.
///
/// Invokes the frame's parked thunks in last-in-first-out order, then
/// discards the frame. A thunk that registers another defer appends to
/// the same frame, so it also runs before the frame is dropped, matching
/// Go's behaviour for defers scheduled during a deferred call. A call
/// with no open frame is a no-op.
#[no_mangle]
pub extern "C" fn raven_defer_run_frame() {
    // Take ownership of the top frame so a thunk that pushes a new defer
    // grows the still-open frame; we keep draining until it is empty.
    let mut frame = match DEFER_FRAMES.with(|f| f.borrow_mut().pop()) {
        Some(frame) => frame,
        None => return,
    };
    // Re-open the frame while draining so any defer scheduled by a thunk
    // lands here and runs too. Pop the placeholder afterwards.
    DEFER_FRAMES.with(|f| f.borrow_mut().push(Vec::new()));
    loop {
        let closure = match frame.pop() {
            Some(c) => c,
            None => {
                // Pull in any defers a thunk scheduled while draining.
                let scheduled = DEFER_FRAMES.with(|f| {
                    f.borrow_mut()
                        .last_mut()
                        .map(std::mem::take)
                        .unwrap_or_default()
                });
                if scheduled.is_empty() {
                    break;
                }
                frame = scheduled;
                continue;
            }
        };
        if closure.is_null() {
            continue;
        }
        // SAFETY: a parked closure is a live Closure; its lifted body is a
        // `fn(env)` and its capture buffer is the env argument.
        unsafe {
            let fn_ptr = (*closure).fn_ptr;
            let env = (*closure).captures;
            if !fn_ptr.is_null() {
                let thunk: DeferThunk = std::mem::transmute(fn_ptr);
                thunk(env);
            }
        }
    }
    DEFER_FRAMES.with(|f| {
        f.borrow_mut().pop();
    });
}

/// Visit every parked deferred closure across all open defer frames,
/// marking each so the collector keeps it (and the values it captures)
/// alive while it waits to run.
fn for_each_defer_root(work: &mut Vec<*mut ObjectHeader>) {
    DEFER_FRAMES.with(|f| {
        for frame in f.borrow().iter() {
            for &closure in frame.iter() {
                if mark_object(closure as *mut ObjectHeader) {
                    work.push(closure as *mut ObjectHeader);
                }
            }
        }
    });
}

/// Initial and floor collection threshold in bytes (1 MiB).
const INITIAL_THRESHOLD: usize = 1024 * 1024;

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

/// Register a struct type's GC pointer descriptor.
///
/// `type_id` is the small integer id the back-end assigns to one
/// monomorphic struct type; `ptr_mask` has bit `i` set when field slot
/// `i` holds a GC pointer the collector traces. Registering the same id
/// twice overwrites the prior descriptor, which is harmless because the
/// back-end always registers the same mask for a given id. The back-end
/// emits these calls in the program entry point before any struct is
/// allocated, so every struct the collector ever sees has a descriptor.
#[no_mangle]
pub extern "C" fn raven_struct_register(type_id: u32, ptr_mask: u64) {
    STRUCT_DESCRIPTORS.with(|d| {
        d.borrow_mut().insert(type_id, ptr_mask);
    });
}

/// Look up a struct type's GC pointer descriptor. Returns zero (no
/// pointer fields) when the id was never registered, so an unregistered
/// struct is traced conservatively as having no pointers rather than
/// crashing the collector.
fn struct_descriptor(type_id: u32) -> u64 {
    STRUCT_DESCRIPTORS.with(|d| d.borrow().get(&type_id).copied().unwrap_or(0))
}

/// Number of root slots currently registered. Test and diagnostic aid.
#[cfg(test)]
pub(crate) fn root_count() -> usize {
    ROOTS.with(|r| r.borrow().len())
}

/// Allocate a zeroed object body of `size` bytes aligned to `align`,
/// register it with the collector, and return its base pointer.
///
/// `tag` is the kind's `TAG_*` constant; the constructor writes the
/// full header into the returned memory. The body is zero-filled so an
/// object that is registered before its fields are written never holds
/// a stale pointer the collector might follow. Owned buffers (string
/// bytes, list elements, and so on) are allocated separately through
/// `raven_alloc` and are not registered.
///
/// Returns null on allocation failure or invalid layout.
///
/// # Safety
///
/// The caller (a constructor) must initialise the object header at the
/// returned pointer before the next collection can observe it.
#[no_mangle]
pub extern "C" fn raven_gc_alloc(size: usize, align: usize, tag: u32) -> *mut u8 {
    let _ = tag;
    // Collect before serving an allocation that would cross the
    // threshold, so the heap stays a bounded multiple of the live set.
    let current = BYTES_ALLOCATED.with(|b| b.get());
    let threshold = THRESHOLD.with(|t| t.get());
    if current + size > threshold {
        collect();
    }
    let ptr = raven_alloc(size, align);
    if ptr.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: `raven_alloc` returned `size` writable bytes.
    unsafe { std::ptr::write_bytes(ptr, 0, size) };
    register(ptr as *mut ObjectHeader, size);
    ptr
}

/// Record a freshly allocated object in the all-objects list and bump
/// the live counters.
fn register(header: *mut ObjectHeader, size: usize) {
    HEAP.with(|h| h.borrow_mut().push(header));
    BYTES_ALLOCATED.with(|b| b.set(b.get() + size));
    LIVE_OBJECTS.with(|n| n.set(n.get() + 1));
}

/// Free a single object: its owned buffers, then its body. Decrements
/// the live counters by the body size and one object.
///
/// # Safety
///
/// `header` must point to a live object registered with the collector
/// and not yet freed.
unsafe fn free_one(header: *mut ObjectHeader) {
    // SAFETY: caller guarantees `header` is a live registered object.
    let (size, align) = unsafe { object_body_layout(header) };
    // SAFETY: same guarantee; release owned buffers before the body.
    unsafe { free_object_buffers(header) };
    raven_dealloc(header as *mut u8, size, align);
    BYTES_ALLOCATED.with(|b| b.set(b.get().saturating_sub(size)));
    LIVE_OBJECTS.with(|n| n.set(n.get().saturating_sub(1)));
}

/// Force a full mark-and-sweep collection regardless of the threshold.
///
/// Marks from every shadow-stack root, frees every unmarked object, and
/// resets the threshold to a multiple of the surviving live bytes.
/// Exposed for deterministic testing and for any future explicit
/// collection point; the compiled program never needs to call it.
#[no_mangle]
pub extern "C" fn raven_gc_collect() {
    collect();
}

/// Run one full mark-and-sweep cycle.
fn collect() {
    mark();
    sweep();
    // Reset the threshold to twice the surviving live bytes, never
    // below the floor, so a large live set collects less often and a
    // small one keeps a tight ceiling.
    let live = BYTES_ALLOCATED.with(|b| b.get());
    let next = INITIAL_THRESHOLD.max(live.saturating_mul(2));
    THRESHOLD.with(|t| t.set(next));
}

/// Mark phase: starting from every root, set the mark bit on every
/// reachable object. Tracing uses an explicit work stack so a deep or
/// cyclic graph cannot overflow the native stack.
fn mark() {
    let mut work: Vec<*mut ObjectHeader> = Vec::new();
    for_each_root(|object| {
        if mark_object(object) {
            work.push(object);
        }
    });
    // Closures parked in open defer frames are roots too: they must
    // survive until the function epilogue runs them.
    for_each_defer_root(&mut work);
    while let Some(object) = work.pop() {
        // SAFETY: `object` was reached from a root or another live
        // object, so it is a live registered header.
        unsafe {
            trace_object(object, &mut work);
        }
    }
}

/// Set the mark bit on `object` if it is non-null and not already
/// marked. Returns true when this call marked it (so the caller should
/// trace its children).
fn mark_object(object: *mut ObjectHeader) -> bool {
    if object.is_null() {
        return false;
    }
    // SAFETY: a registered object pointer is a live header.
    let header = unsafe { &mut *object };
    if header.is_marked() {
        return false;
    }
    header.set_mark();
    true
}

/// Visit a slot that may hold a GC pointer: mark the pointee and, when
/// newly marked, push it for tracing.
fn visit_slot(slot: *const *mut u8, work: &mut Vec<*mut ObjectHeader>) {
    // SAFETY: caller guarantees `slot` points to a readable pointer.
    let child = unsafe { *slot } as *mut ObjectHeader;
    if mark_object(child) {
        work.push(child);
    }
}

/// Trace one already-marked object: follow the GC pointers its layout
/// owns, per `docs/v2/specs/object-layout.md`, pushing newly marked
/// children onto `work`.
///
/// # Safety
///
/// `object` must be a live registered header.
unsafe fn trace_object(object: *mut ObjectHeader, work: &mut Vec<*mut ObjectHeader>) {
    // SAFETY: caller guarantees `object` is a live header.
    let tag = unsafe { (*object).tag };
    match tag {
        TAG_LIST => {
            let list = object as *const List;
            // SAFETY: tag confirms the List layout.
            let (flag, len, elements) = unsafe {
                (
                    (*list).elements_are_gc_ptrs,
                    (*list).header.len,
                    (*list).elements,
                )
            };
            if flag != 0 && !elements.is_null() {
                let slots = elements as *const *mut u8;
                for i in 0..len as usize {
                    // SAFETY: the first `len` slots are initialised.
                    visit_slot(unsafe { slots.add(i) }, work);
                }
            }
        }
        TAG_MAP => {
            let map = object as *const Map;
            // SAFETY: tag confirms the Map layout.
            let (keys_flag, values_flag, bucket_count, buckets) = unsafe {
                (
                    (*map).keys_are_gc_ptrs,
                    (*map).values_are_gc_ptrs,
                    (*map).bucket_count,
                    (*map).buckets,
                )
            };
            if !buckets.is_null() && (keys_flag != 0 || values_flag != 0) {
                for i in 0..bucket_count as usize {
                    // SAFETY: `buckets` holds `bucket_count` slots.
                    let entry = unsafe { buckets.add(i) } as *const MapEntry;
                    // SAFETY: each entry slot is initialised.
                    let key = unsafe { (*entry).key };
                    if key.is_null() {
                        continue; // empty or tombstoned slot
                    }
                    if keys_flag != 0 {
                        visit_slot(unsafe { std::ptr::addr_of!((*entry).key) }, work);
                    }
                    if values_flag != 0 {
                        visit_slot(unsafe { std::ptr::addr_of!((*entry).value) }, work);
                    }
                }
            }
        }
        TAG_SET => {
            let set = object as *const Set;
            // SAFETY: tag confirms the Set layout.
            let (flag, bucket_count, buckets) = unsafe {
                (
                    (*set).elements_are_gc_ptrs,
                    (*set).bucket_count,
                    (*set).buckets,
                )
            };
            if flag != 0 && !buckets.is_null() {
                for i in 0..bucket_count as usize {
                    // SAFETY: `buckets` holds `bucket_count` slots.
                    let entry = unsafe { buckets.add(i) } as *const SetEntry;
                    // SAFETY: each entry slot is initialised.
                    let element = unsafe { (*entry).element };
                    if element.is_null() {
                        continue; // empty or tombstoned slot
                    }
                    visit_slot(unsafe { std::ptr::addr_of!((*entry).element) }, work);
                }
            }
        }
        TAG_CLOSURE => {
            let closure = object as *const Closure;
            // SAFETY: tag confirms the Closure layout.
            let (ptr_count, captures) =
                unsafe { ((*closure).capture_ptr_count, (*closure).captures) };
            if ptr_count != 0 && !captures.is_null() {
                let slots = captures as *const *mut u8;
                for i in 0..ptr_count as usize {
                    // SAFETY: the first `ptr_count` capture slots are
                    // pointer-sized GC pointers placed by lowering.
                    visit_slot(unsafe { slots.add(i) }, work);
                }
            }
        }
        TAG_BOX => {
            // SAFETY: tag confirms the Box layout; the flag sits at the
            // header's start and the payload at BOX_PAYLOAD_OFFSET.
            let flag = unsafe { (*(object as *const crate::object::Box)).payload_is_gc_ptr };
            if flag != 0 {
                let payload = (object as *const u8).wrapping_add(BOX_PAYLOAD_OFFSET);
                visit_slot(payload as *const *mut u8, work);
            }
        }
        TAG_STRUCT => {
            // SAFETY: tag confirms the struct layout. `len` is the field
            // count and `cap` is the per-type descriptor id.
            let (field_count, type_id) = unsafe { ((*object).len, (*object).cap) };
            let mask = struct_descriptor(type_id);
            if mask != 0 {
                let fields = (object as *const u8).wrapping_add(STRUCT_FIELDS_OFFSET);
                for i in 0..field_count as usize {
                    if mask & (1u64 << i) != 0 {
                        let slot = fields.wrapping_add(i * STRUCT_FIELD_SLOT);
                        visit_slot(slot as *const *mut u8, work);
                    }
                }
            }
        }
        // TAG_STRING and unknown tags own no GC pointers.
        _ => {}
    }
}

/// Sweep phase: free every unmarked object and clear the mark bit on
/// survivors so the next cycle starts clean.
fn sweep() {
    HEAP.with(|h| {
        let mut heap = h.borrow_mut();
        let mut write = 0usize;
        for read in 0..heap.len() {
            let object = heap[read];
            // SAFETY: every heap entry is a live registered header.
            let marked = unsafe { (*object).is_marked() };
            if marked {
                // SAFETY: live header; clear the mark for next cycle.
                unsafe { (*object).clear_mark() };
                heap[write] = object;
                write += 1;
            } else {
                // SAFETY: unmarked and registered; free it.
                unsafe { free_one(object) };
            }
        }
        heap.truncate(write);
    });
}

/// Visit the current object pointer held by every registered root slot.
/// Null slots and slots whose stored pointer is null are skipped.
fn for_each_root(mut visit: impl FnMut(*mut ObjectHeader)) {
    ROOTS.with(|r| {
        for &slot in r.borrow().iter() {
            if slot.is_null() {
                continue;
            }
            // SAFETY: a registered slot points to a live `*mut u8`.
            let object = unsafe { *slot } as *mut ObjectHeader;
            if !object.is_null() {
                visit(object);
            }
        }
    });
}

/// Live object-body bytes currently tracked by the collector. A
/// diagnostic entry point used by tests; the compiled program does not
/// call it.
#[no_mangle]
pub extern "C" fn raven_gc_bytes_allocated() -> usize {
    BYTES_ALLOCATED.with(|b| b.get())
}

/// Number of live objects currently tracked by the collector. A
/// diagnostic entry point that lets tests assert bounded liveness
/// without measuring flaky OS memory.
#[no_mangle]
pub extern "C" fn raven_gc_live_objects() -> usize {
    LIVE_OBJECTS.with(|n| n.get())
}

/// Unregister a single object from the all-objects list and free it.
/// Used by the object modules' test deallocators so that manually freed
/// objects do not stay in the heap list where a later collection in the
/// same thread would visit a dangling pointer.
///
/// # Safety
///
/// `header` must point to a live object registered with the collector.
#[cfg(test)]
pub(crate) unsafe fn free_for_test(header: *mut ObjectHeader) {
    if header.is_null() {
        return;
    }
    HEAP.with(|h| {
        let mut heap = h.borrow_mut();
        if let Some(idx) = heap.iter().position(|&p| p == header) {
            heap.swap_remove(idx);
        }
    });
    // SAFETY: caller guarantees `header` is a live registered object.
    unsafe { free_one(header) };
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

#[cfg(test)]
mod collector_tests {
    use super::*;
    use crate::object::{
        raven_box_new, raven_box_payload, raven_closure_captures, raven_closure_new,
        raven_list_new, raven_map_buckets, raven_map_new, raven_set_buckets, raven_set_new,
        raven_string_new,
    };

    /// Run each collector test on its own thread so the thread-local
    /// heap, shadow stack, and counters start clean.
    fn isolated(body: impl FnOnce() + Send + 'static) {
        std::thread::spawn(body).join().unwrap();
    }

    /// A dummy closure body pointer.
    extern "C" fn dummy_body() {}

    #[test]
    fn unrooted_object_is_collected() {
        isolated(|| {
            let s = raven_string_new(8);
            assert!(!s.is_null());
            assert_eq!(raven_gc_live_objects(), 1);
            raven_gc_collect();
            assert_eq!(raven_gc_live_objects(), 0);
        });
    }

    #[test]
    fn rooted_object_survives() {
        isolated(|| {
            let s = raven_string_new(8);
            let mut slot: *mut u8 = s as *mut u8;
            raven_gc_push_root(&mut slot as *mut *mut u8);
            raven_gc_collect();
            assert_eq!(raven_gc_live_objects(), 1);
            // The header is still valid and its mark bit was cleared.
            // SAFETY: the rooted object survived the sweep.
            unsafe {
                assert_eq!((*(s)).header.tag, crate::object::TAG_STRING);
                assert!(!(*(s)).header.is_marked());
            }
            raven_gc_pop_roots(1);
            raven_gc_collect();
            assert_eq!(raven_gc_live_objects(), 0);
        });
    }

    #[test]
    fn transitively_reachable_object_survives() {
        isolated(|| {
            // A list of one GC pointer that points at a string. Root the
            // list; the string must survive transitively.
            let inner = raven_string_new(4);
            let list = raven_list_new(8, 8, 1, 1);
            assert!(!inner.is_null() && !list.is_null());
            // SAFETY: list has capacity 1 for one pointer slot.
            unsafe {
                let slots = (*list).elements as *mut *mut u8;
                slots.write(inner as *mut u8);
                (*list).header.len = 1;
            }
            let mut slot: *mut u8 = list as *mut u8;
            raven_gc_push_root(&mut slot as *mut *mut u8);
            assert_eq!(raven_gc_live_objects(), 2);
            raven_gc_collect();
            assert_eq!(raven_gc_live_objects(), 2);
            raven_gc_pop_roots(1);
            raven_gc_collect();
            assert_eq!(raven_gc_live_objects(), 0);
        });
    }

    #[test]
    fn cycle_with_no_root_is_reclaimed() {
        isolated(|| {
            // Two single-slot lists of GC pointers that reference each
            // other. Neither is rooted; mark-sweep reclaims both, which
            // reference counting could not.
            let a = raven_list_new(8, 8, 1, 1);
            let b = raven_list_new(8, 8, 1, 1);
            assert!(!a.is_null() && !b.is_null());
            // SAFETY: each list has one pointer slot.
            unsafe {
                let a_slots = (*a).elements as *mut *mut u8;
                a_slots.write(b as *mut u8);
                (*a).header.len = 1;
                let b_slots = (*b).elements as *mut *mut u8;
                b_slots.write(a as *mut u8);
                (*b).header.len = 1;
            }
            assert_eq!(raven_gc_live_objects(), 2);
            raven_gc_collect();
            assert_eq!(raven_gc_live_objects(), 0);
        });
    }

    #[test]
    fn cycle_survives_while_one_node_is_rooted() {
        isolated(|| {
            let a = raven_list_new(8, 8, 1, 1);
            let b = raven_list_new(8, 8, 1, 1);
            // SAFETY: each list has one pointer slot.
            unsafe {
                ((*a).elements as *mut *mut u8).write(b as *mut u8);
                (*a).header.len = 1;
                ((*b).elements as *mut *mut u8).write(a as *mut u8);
                (*b).header.len = 1;
            }
            let mut slot: *mut u8 = a as *mut u8;
            raven_gc_push_root(&mut slot as *mut *mut u8);
            raven_gc_collect();
            // Rooting one node of the cycle keeps the whole cycle alive.
            assert_eq!(raven_gc_live_objects(), 2);
            raven_gc_pop_roots(1);
            raven_gc_collect();
            assert_eq!(raven_gc_live_objects(), 0);
        });
    }

    #[test]
    fn map_traces_gc_keys_and_values() {
        isolated(|| {
            let key = raven_string_new(2);
            let value = raven_string_new(2);
            let map = raven_map_new(4, 1, 1);
            assert!(!map.is_null());
            // SAFETY: write one live entry into the first bucket.
            unsafe {
                let buckets = raven_map_buckets(map);
                let e = &mut *buckets.add(0);
                e.hash = 1;
                e.key = key as *mut u8;
                e.value = value as *mut u8;
                (*map).header.len = 1;
            }
            let mut slot: *mut u8 = map as *mut u8;
            raven_gc_push_root(&mut slot as *mut *mut u8);
            // map + key + value all live.
            assert_eq!(raven_gc_live_objects(), 3);
            raven_gc_collect();
            assert_eq!(raven_gc_live_objects(), 3);
            raven_gc_pop_roots(1);
            raven_gc_collect();
            assert_eq!(raven_gc_live_objects(), 0);
        });
    }

    #[test]
    fn set_traces_gc_elements() {
        isolated(|| {
            let element = raven_string_new(2);
            let set = raven_set_new(4, 1);
            assert!(!set.is_null());
            // SAFETY: write one live entry into the first bucket.
            unsafe {
                let buckets = raven_set_buckets(set);
                let e = &mut *buckets.add(0);
                e.hash = 1;
                e.element = element as *mut u8;
                (*set).header.len = 1;
            }
            let mut slot: *mut u8 = set as *mut u8;
            raven_gc_push_root(&mut slot as *mut *mut u8);
            assert_eq!(raven_gc_live_objects(), 2);
            raven_gc_collect();
            assert_eq!(raven_gc_live_objects(), 2);
            raven_gc_pop_roots(1);
            raven_gc_collect();
            assert_eq!(raven_gc_live_objects(), 0);
        });
    }

    #[test]
    fn closure_traces_gc_captures() {
        isolated(|| {
            let captured = raven_string_new(2);
            // One pointer-sized capture slot holding a GC pointer.
            let closure = raven_closure_new(dummy_body as *const u8, 8, 8, 1, 1);
            assert!(!closure.is_null());
            // SAFETY: the capture buffer has room for one pointer.
            unsafe {
                let caps = raven_closure_captures(closure) as *mut *mut u8;
                caps.write(captured as *mut u8);
            }
            let mut slot: *mut u8 = closure as *mut u8;
            raven_gc_push_root(&mut slot as *mut *mut u8);
            assert_eq!(raven_gc_live_objects(), 2);
            raven_gc_collect();
            assert_eq!(raven_gc_live_objects(), 2);
            raven_gc_pop_roots(1);
            raven_gc_collect();
            assert_eq!(raven_gc_live_objects(), 0);
        });
    }

    #[test]
    fn box_traces_gc_payload() {
        isolated(|| {
            let inner = raven_string_new(2);
            let boxed = raven_box_new(8, 8, 1);
            assert!(!boxed.is_null());
            // SAFETY: the payload holds one pointer.
            unsafe {
                let payload = raven_box_payload(boxed) as *mut *mut u8;
                payload.write(inner as *mut u8);
            }
            let mut slot: *mut u8 = boxed as *mut u8;
            raven_gc_push_root(&mut slot as *mut *mut u8);
            assert_eq!(raven_gc_live_objects(), 2);
            raven_gc_collect();
            assert_eq!(raven_gc_live_objects(), 2);
            raven_gc_pop_roots(1);
            raven_gc_collect();
            assert_eq!(raven_gc_live_objects(), 0);
        });
    }

    #[test]
    fn struct_traces_gc_pointer_fields() {
        isolated(|| {
            use crate::object::raven_struct_new;
            // Type 1 has two fields: slot 0 is a scalar Int, slot 1 is a
            // GC pointer (bit 1 set).
            raven_struct_register(1, 0b10);
            let inner = raven_string_new(4);
            let s = raven_struct_new(2, 1);
            assert!(!inner.is_null() && !s.is_null());
            // SAFETY: store a scalar in slot 0 and the pointer in slot 1.
            unsafe {
                let fields = crate::object::raven_struct_fields(s) as *mut *mut u8;
                fields.add(0).write(0xDEAD_BEEF as *mut u8);
                fields.add(1).write(inner as *mut u8);
            }
            let mut slot: *mut u8 = s as *mut u8;
            raven_gc_push_root(&mut slot as *mut *mut u8);
            // struct + string both live.
            assert_eq!(raven_gc_live_objects(), 2);
            raven_gc_collect();
            // The scalar slot's pointer-looking integer is not traced, but
            // the string in the pointer slot survives transitively.
            assert_eq!(raven_gc_live_objects(), 2);
            raven_gc_pop_roots(1);
            raven_gc_collect();
            assert_eq!(raven_gc_live_objects(), 0);
        });
    }

    #[test]
    fn struct_with_no_pointer_fields_traces_nothing() {
        isolated(|| {
            use crate::object::raven_struct_new;
            // Type 2 has two scalar fields (empty mask).
            raven_struct_register(2, 0);
            let s = raven_struct_new(2, 2);
            // SAFETY: fill both slots with pointer-looking integers that
            // must not be traced.
            unsafe {
                let fields = crate::object::raven_struct_fields(s) as *mut u64;
                fields.add(0).write(0xDEAD_BEEF_DEAD_BEEF);
                fields.add(1).write(0x1);
            }
            let mut slot: *mut u8 = s as *mut u8;
            raven_gc_push_root(&mut slot as *mut *mut u8);
            raven_gc_collect();
            assert_eq!(raven_gc_live_objects(), 1);
            raven_gc_pop_roots(1);
            raven_gc_collect();
            assert_eq!(raven_gc_live_objects(), 0);
        });
    }

    #[test]
    fn scalar_list_elements_are_not_traced() {
        isolated(|| {
            // A list of scalar Ints (flag 0). Its 8-byte slots hold
            // integers that look like pointers but must not be traced.
            let list = raven_list_new(8, 8, 4, 0);
            // SAFETY: fill slots with arbitrary non-pointer bit patterns.
            unsafe {
                let slots = (*list).elements as *mut u64;
                slots.add(0).write(0xDEAD_BEEF_DEAD_BEEF);
                slots.add(1).write(0x1);
                (*list).header.len = 2;
            }
            let mut slot: *mut u8 = list as *mut u8;
            raven_gc_push_root(&mut slot as *mut *mut u8);
            // Only the list is live; the integer "pointers" are ignored.
            raven_gc_collect();
            assert_eq!(raven_gc_live_objects(), 1);
            raven_gc_pop_roots(1);
            raven_gc_collect();
            assert_eq!(raven_gc_live_objects(), 0);
        });
    }

    #[test]
    fn bounded_liveness_under_churn() {
        isolated(|| {
            // Keep a bounded working set of rooted lists while churning
            // many short-lived strings. Liveness must stay bounded.
            const WORKING_SET: usize = 4;
            let mut roots: [*mut u8; WORKING_SET] = [std::ptr::null_mut(); WORKING_SET];
            for r in roots.iter_mut() {
                *r = raven_list_new(8, 8, 0, 1) as *mut u8;
            }
            raven_gc_enter_frame(roots.as_mut_ptr(), WORKING_SET);

            let mut peak = 0usize;
            for i in 0..5000usize {
                // Allocate a throwaway string that nothing roots.
                let _garbage = raven_string_new(8);
                // Force frequent collection so the churn cannot pile up.
                if i % 64 == 0 {
                    raven_gc_collect();
                    peak = peak.max(raven_gc_live_objects());
                }
            }
            raven_gc_collect();
            // After a final collection only the working set survives.
            assert_eq!(raven_gc_live_objects(), WORKING_SET);
            // Liveness never exceeded the working set plus a small
            // constant of in-flight garbage between collections.
            assert!(
                peak <= WORKING_SET + 64,
                "liveness peaked at {peak}, expected bounded"
            );

            raven_gc_leave_frame();
            raven_gc_collect();
            assert_eq!(raven_gc_live_objects(), 0);
        });
    }

    #[test]
    fn parked_defer_closure_survives_collection() {
        isolated(|| {
            // A closure parked in an open defer frame must survive a
            // collection even with no shadow-stack root, and the GC
            // pointer it captures must survive transitively.
            let captured = raven_string_new(2);
            let closure = raven_closure_new(dummy_body as *const u8, 8, 8, 1, 1);
            // SAFETY: one pointer-sized GC capture slot.
            unsafe {
                let caps = raven_closure_captures(closure) as *mut *mut u8;
                caps.write(captured as *mut u8);
            }
            raven_defer_enter_frame();
            raven_defer_push(closure);
            assert_eq!(raven_gc_live_objects(), 2);
            raven_gc_collect();
            assert_eq!(raven_gc_live_objects(), 2);
            // Running the frame drops the only reference; the closure and
            // its capture are then collectable.
            raven_defer_run_frame();
            raven_gc_collect();
            assert_eq!(raven_gc_live_objects(), 0);
        });
    }

    #[test]
    fn defer_frame_runs_thunks_in_lifo_order() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        // A thunk records its tag; running a frame of two must observe
        // them in reverse registration order.
        static LOG: [AtomicUsize; 2] = [AtomicUsize::new(0), AtomicUsize::new(0)];
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        extern "C" fn record_a(_env: *mut u8) {
            let i = NEXT.fetch_add(1, Ordering::SeqCst);
            LOG[i].store(1, Ordering::SeqCst);
        }
        extern "C" fn record_b(_env: *mut u8) {
            let i = NEXT.fetch_add(1, Ordering::SeqCst);
            LOG[i].store(2, Ordering::SeqCst);
        }
        isolated(|| {
            let a = raven_closure_new(record_a as *const u8, 0, 0, 0, 0);
            let b = raven_closure_new(record_b as *const u8, 0, 0, 0, 0);
            raven_defer_enter_frame();
            raven_defer_push(a);
            raven_defer_push(b);
            raven_defer_run_frame();
            // b was pushed last, so it runs first.
            assert_eq!(LOG[0].load(Ordering::SeqCst), 2);
            assert_eq!(LOG[1].load(Ordering::SeqCst), 1);
        });
    }

    #[test]
    fn allocation_triggers_collection_at_threshold() {
        isolated(|| {
            // Each string body is 24 bytes, so 1 MiB of bodies is about
            // 43.7k objects. Allocating well past that with unrooted
            // strings must trigger at least one automatic collection,
            // keeping liveness below the total allocated count.
            const COUNT: usize = 120_000;
            for _ in 0..COUNT {
                let _garbage = raven_string_new(8);
            }
            assert!(
                raven_gc_live_objects() < COUNT,
                "expected automatic collection to bound liveness, got {}",
                raven_gc_live_objects()
            );
            raven_gc_collect();
            assert_eq!(raven_gc_live_objects(), 0);
        });
    }
}
