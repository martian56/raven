//! Integration test that links against `raven-runtime` from a
//! separate crate boundary and exercises the C ABI surface.
//!
//! Running this test catches two regressions the inline unit tests
//! cannot:
//!
//! 1. The crate must still build as an `rlib` (the linker for this
//!    test binary picks the rlib variant; if `crate-type` regresses
//!    to just `staticlib`, the test refuses to compile).
//! 2. The exported symbols stay reachable through the public Rust
//!    surface that the v2 codegen and tooling will import.

use raven_runtime::{
    raven_alloc, raven_dealloc, raven_print_str, raven_println_str, ObjectHeader, OBJECT_ALIGN,
    TAG_LIST,
};

#[test]
fn alloc_write_dealloc_cycle() {
    let size = 128;
    let ptr = raven_alloc(size, OBJECT_ALIGN);
    assert!(!ptr.is_null(), "expected non-null allocation");

    // Write a recognizable pattern and read it back so a broken
    // allocator that returned an unmapped pointer would segfault here
    // instead of passing.
    unsafe {
        for i in 0..size {
            ptr.add(i).write(i as u8);
        }
        for i in 0..size {
            assert_eq!(ptr.add(i).read(), i as u8);
        }
    }

    raven_dealloc(ptr, size, OBJECT_ALIGN);
}

#[test]
fn header_layout_visible_across_crate_boundary() {
    let header = ObjectHeader::new(TAG_LIST, 3, 4);
    assert_eq!(std::mem::size_of_val(&header), 16);
    assert_eq!(header.tag, TAG_LIST);
    assert_eq!(header.len, 3);
    assert_eq!(header.cap, 4);
}

#[test]
fn print_helpers_accept_empty_input() {
    // Smoke test: the helpers must not abort on empty input. We
    // intentionally do not capture stdout here; this test exists to
    // confirm the symbols link and are callable from an external
    // crate.
    raven_print_str(std::ptr::null(), 0);
    raven_println_str(std::ptr::null(), 0);
}
