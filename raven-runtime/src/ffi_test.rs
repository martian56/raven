//! C-ABI struct-by-value fixtures used only by the FFI golden tests.
//!
//! There is no convenient C standard-library function that takes a small
//! struct by value, so these tiny functions stand in: the Rust compiler
//! lowers their `extern "C"` signatures to the correct platform C ABI, so a
//! Raven program that calls them through `extern "C"` exercises the back
//! end's by-value struct marshalling against a correct oracle. They are a
//! few bytes of code and harmless in a shipped binary.

/// A 16-byte struct of two eightbytes. System V AMD64 and AArch64 pass it in
/// two integer registers; Windows x64 passes it by reference.
#[repr(C)]
pub struct RavenFfiPair {
    pub a: i64,
    pub b: i64,
}

/// Swap the two fields: exercises a 16-byte struct as both argument and
/// return.
#[no_mangle]
pub extern "C" fn raven_ffi_swap_pair(p: RavenFfiPair) -> RavenFfiPair {
    RavenFfiPair { a: p.b, b: p.a }
}

/// A 16-byte struct of four 32-bit ints, the shape of `SDL_Rect`.
#[repr(C)]
pub struct RavenFfiRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// Area from a by-value rect argument.
#[no_mangle]
pub extern "C" fn raven_ffi_rect_area(r: RavenFfiRect) -> i64 {
    (r.w as i64) * (r.h as i64)
}

/// Build a rect, returned by value.
#[no_mangle]
pub extern "C" fn raven_ffi_make_rect(x: i32, y: i32, w: i32, h: i32) -> RavenFfiRect {
    RavenFfiRect { x, y, w, h }
}

/// A 12-byte struct of three 32-bit ints. Windows x64 passes it by
/// reference; System V and AArch64 use two eightbytes where the second holds
/// only 4 valid bytes.
#[repr(C)]
pub struct RavenFfiVec3 {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

/// Sum the fields of a by-value 12-byte struct argument.
#[no_mangle]
pub extern "C" fn raven_ffi_vec3_sum(v: RavenFfiVec3) -> i64 {
    v.x as i64 + v.y as i64 + v.z as i64
}

/// An 8-byte struct of two f32 (a 2D point). System V passes it in one SSE
/// register, AArch64 as a 2-member float HFA, Windows x64 in an integer
/// register.
#[repr(C)]
pub struct RavenFfiPointF {
    pub x: f32,
    pub y: f32,
}

#[no_mangle]
pub extern "C" fn raven_ffi_pointf_sum(p: RavenFfiPointF) -> f64 {
    p.x as f64 + p.y as f64
}

#[no_mangle]
pub extern "C" fn raven_ffi_make_pointf(x: f32, y: f32) -> RavenFfiPointF {
    RavenFfiPointF { x, y }
}

/// A 16-byte struct of two f64. System V: two SSE registers; AArch64: a
/// 2-member double HFA; Windows x64: by reference.
#[repr(C)]
pub struct RavenFfiVec2D {
    pub x: f64,
    pub y: f64,
}

#[no_mangle]
pub extern "C" fn raven_ffi_vec2d_sum(v: RavenFfiVec2D) -> f64 {
    v.x + v.y
}

#[no_mangle]
pub extern "C" fn raven_ffi_make_vec2d(x: f64, y: f64) -> RavenFfiVec2D {
    RavenFfiVec2D { x, y }
}

/// An 8-byte struct mixing an int and a float (one INTEGER eightbyte on
/// System V, general register on AArch64, integer register on Windows).
#[repr(C)]
pub struct RavenFfiMixed {
    pub n: i32,
    pub f: f32,
}

#[no_mangle]
pub extern "C" fn raven_ffi_mixed_sum(m: RavenFfiMixed) -> f64 {
    m.n as f64 + m.f as f64
}

/// An 8-byte inner struct, nested inside `RavenFfiOuter`.
#[repr(C)]
pub struct RavenFfiInner {
    pub a: i32,
    pub b: i32,
}

/// A 12-byte struct with a nested `@repr(C)` struct field and a trailing int.
#[repr(C)]
pub struct RavenFfiOuter {
    pub inner: RavenFfiInner,
    pub c: i32,
}

#[no_mangle]
pub extern "C" fn raven_ffi_outer_sum(o: RavenFfiOuter) -> i64 {
    o.inner.a as i64 + o.inner.b as i64 + o.c as i64
}

#[no_mangle]
pub extern "C" fn raven_ffi_make_outer(a: i32, b: i32, c: i32) -> RavenFfiOuter {
    RavenFfiOuter {
        inner: RavenFfiInner { a, b },
        c,
    }
}

/// Invokes a callback with a userdata pointer (userdata-last, like a glibc
/// `qsort_r` comparator): calls `cb(x, userdata)` and returns the result. Used
/// to exercise passing a Raven closure as a C callback via a trampoline.
#[no_mangle]
pub extern "C" fn raven_ffi_apply_cb(
    cb: extern "C" fn(i64, *mut core::ffi::c_void) -> i64,
    userdata: *mut core::ffi::c_void,
    x: i64,
) -> i64 {
    cb(x, userdata)
}

/// Invokes a callback twice and sums the results, to check a closure survives
/// being called more than once through C.
#[no_mangle]
pub extern "C" fn raven_ffi_apply_cb_twice(
    cb: extern "C" fn(i64, *mut core::ffi::c_void) -> i64,
    userdata: *mut core::ffi::c_void,
    a: i64,
    b: i64,
) -> i64 {
    cb(a, userdata) + cb(b, userdata)
}

/// Calls a callback `n` times (`cb(0, ud) + cb(1, ud) + ...`) and returns the
/// sum, so a callback that allocates triggers garbage collection while the
/// Raven stack is suspended in this C frame.
#[no_mangle]
pub extern "C" fn raven_ffi_sum_cb(
    cb: extern "C" fn(i64, *mut core::ffi::c_void) -> i64,
    userdata: *mut core::ffi::c_void,
    n: i64,
) -> i64 {
    let mut total: i64 = 0;
    let mut i: i64 = 0;
    while i < n {
        total += cb(i, userdata);
        i += 1;
    }
    total
}

/// A 24-byte struct (three i64), larger than two registers. System V passes
/// it in memory on the stack (the MEMORY class); Windows x64 and AArch64 pass
/// it by reference. Returned through a hidden `sret` pointer on every target.
#[repr(C)]
pub struct RavenFfiBig {
    pub a: i64,
    pub b: i64,
    pub c: i64,
}

#[no_mangle]
pub extern "C" fn raven_ffi_big_sum(v: RavenFfiBig) -> i64 {
    v.a + v.b + v.c
}

#[no_mangle]
pub extern "C" fn raven_ffi_make_big(a: i64, b: i64, c: i64) -> RavenFfiBig {
    RavenFfiBig { a, b, c }
}
