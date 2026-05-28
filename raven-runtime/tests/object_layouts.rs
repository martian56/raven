//! Cross-crate integration tests for the per-kind object layouts.
//!
//! These exercise the constructor and accessor surface through the
//! public Rust API exactly as the v2 codegen and stdlib glue will. A
//! failure here means either a `crate-type` regression (the rlib no
//! longer links) or a layout symbol stopped being reachable.
//!
//! The byte-exact `size_of` and `offset_of!` assertions live in the
//! inline unit tests; this file confirms behaviour through the ABI.

use raven_runtime::{
    raven_box_new, raven_box_payload, raven_closure_captures, raven_closure_fn_ptr,
    raven_closure_new, raven_list_elements, raven_list_len, raven_list_new, raven_list_push,
    raven_map_bucket_count, raven_map_buckets, raven_map_new, raven_set_bucket_count,
    raven_set_buckets, raven_set_new, raven_string_bytes, raven_string_concat, raven_string_len,
    raven_string_new, TAG_BOX, TAG_CLOSURE, TAG_LIST, TAG_MAP, TAG_SET, TAG_STRING,
};

#[test]
fn string_constructor_and_concat_cross_crate() {
    let a = raven_string_new(3);
    let b = raven_string_new(3);
    assert!(!a.is_null() && !b.is_null());
    // SAFETY: a and b each have capacity 3.
    unsafe {
        assert_eq!((*a).header.tag, TAG_STRING);
        std::ptr::copy_nonoverlapping(b"foo".as_ptr(), (*a).bytes, 3);
        (*a).header.len = 3;
        std::ptr::copy_nonoverlapping(b"bar".as_ptr(), (*b).bytes, 3);
        (*b).header.len = 3;
    }
    let joined = raven_string_concat(a, b);
    assert_eq!(raven_string_len(joined), 6);
    // SAFETY: joined has 6 valid bytes.
    let slice = unsafe { std::slice::from_raw_parts(raven_string_bytes(joined), 6) };
    assert_eq!(slice, b"foobar");
}

#[test]
fn list_push_grows_cross_crate() {
    let l = raven_list_new(8, 8, 0);
    assert!(!l.is_null());
    // SAFETY: l is a fresh List.
    unsafe {
        assert_eq!((*l).header.tag, TAG_LIST);
    }
    for v in 0u64..32 {
        raven_list_push(l, &v as *const u64 as *const u8);
    }
    assert_eq!(raven_list_len(l), 32);
    // SAFETY: l holds 32 u64 elements.
    unsafe {
        let slots = raven_list_elements(l) as *const u64;
        for v in 0u64..32 {
            assert_eq!(*slots.add(v as usize), v);
        }
    }
}

#[test]
fn map_constructor_cross_crate() {
    let m = raven_map_new(10);
    assert!(!m.is_null());
    // 10 rounds up to 16.
    assert_eq!(raven_map_bucket_count(m), 16);
    // SAFETY: m is a fresh Map.
    unsafe {
        assert_eq!((*m).header.tag, TAG_MAP);
        assert_eq!((*m).header.cap, 16);
    }
    let buckets = raven_map_buckets(m);
    assert!(!buckets.is_null());
    // SAFETY: 16 zeroed MapEntry slots.
    unsafe {
        for i in 0..16 {
            let e = &*buckets.add(i);
            assert_eq!(e.hash, 0);
            assert!(e.key.is_null());
            assert!(e.value.is_null());
        }
    }
}

#[test]
fn set_constructor_cross_crate() {
    let s = raven_set_new(3);
    assert!(!s.is_null());
    // 3 rounds up to 4.
    assert_eq!(raven_set_bucket_count(s), 4);
    // SAFETY: s is a fresh Set.
    unsafe {
        assert_eq!((*s).header.tag, TAG_SET);
        assert_eq!((*s).header.cap, 4);
    }
    let buckets = raven_set_buckets(s);
    assert!(!buckets.is_null());
    // SAFETY: 4 zeroed SetEntry slots.
    unsafe {
        for i in 0..4 {
            let e = &*buckets.add(i);
            assert_eq!(e.hash, 0);
            assert!(e.element.is_null());
        }
    }
}

extern "C" fn closure_body() {}

#[test]
fn closure_constructor_cross_crate() {
    let fp = closure_body as *const u8;
    let c = raven_closure_new(fp, 16, 8, 2);
    assert!(!c.is_null());
    // SAFETY: c is a fresh Closure with 16 capture bytes.
    unsafe {
        assert_eq!((*c).header.tag, TAG_CLOSURE);
        assert_eq!((*c).header.len, 2);
    }
    assert_eq!(raven_closure_fn_ptr(c), fp);
    let captures = raven_closure_captures(c);
    assert!(!captures.is_null());
    // SAFETY: captures points to 16 zeroed, writable bytes.
    unsafe {
        for i in 0..16 {
            assert_eq!(captures.add(i).read(), 0);
        }
        (captures as *mut u64).write(7);
        assert_eq!((captures as *const u64).read(), 7);
    }
}

#[test]
fn box_constructor_cross_crate() {
    let b = raven_box_new(8, 8);
    assert!(!b.is_null());
    // SAFETY: b is a fresh Box with an 8-byte payload.
    unsafe {
        assert_eq!((*b).header.tag, TAG_BOX);
        assert_eq!((*b).header.len, 8);
        assert_eq!((*b).header.cap, 1);
    }
    let payload = raven_box_payload(b);
    assert!(!payload.is_null());
    // SAFETY: payload points to 8 zeroed, writable bytes.
    unsafe {
        assert_eq!((payload as *const u64).read(), 0);
        (payload as *mut u64).write(0xFEED);
        assert_eq!((payload as *const u64).read(), 0xFEED);
    }
}
