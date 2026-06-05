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
