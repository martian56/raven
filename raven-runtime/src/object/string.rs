//! In-memory layout, constructors, and accessors for Raven `String`.
//!
//! See `docs/v2/specs/object-layout.md` for the byte-exact field
//! offsets the back-end relies on.

use super::{ObjectHeader, OBJECT_ALIGN, TAG_STRING};
use crate::gc::raven_gc_alloc;
use crate::{raven_alloc, raven_dealloc};
use std::mem::align_of;
use std::ptr;

/// UTF-8 string object. The header carries `len` (byte count) and
/// `cap` (allocated byte capacity); the `bytes` pointer owns the
/// payload buffer.
#[repr(C)]
pub struct String {
    /// Standard 16-byte object header. `tag == TAG_STRING`.
    pub header: ObjectHeader,
    /// Owned buffer of `header.cap` bytes. The first `header.len`
    /// bytes are valid UTF-8. Null when `header.cap == 0`.
    pub bytes: *mut u8,
}

/// Allocate a fresh `String` with the requested byte capacity.
///
/// The header is initialised with `tag = TAG_STRING`, `len = 0`,
/// `cap = cap`. The byte buffer is zero-filled.
///
/// Returns null when the allocation fails.
#[no_mangle]
pub extern "C" fn raven_string_new(cap: u32) -> *mut String {
    // Allocate the owned byte buffer first so a body-allocation failure
    // does not leave a half-registered object in the collector.
    let bytes_ptr = if cap == 0 {
        ptr::null_mut()
    } else {
        let p = raven_alloc(cap as usize, 1);
        if p.is_null() {
            return ptr::null_mut();
        }
        // SAFETY: the allocator just gave us `cap` writable bytes.
        unsafe { ptr::write_bytes(p, 0, cap as usize) };
        p
    };
    let header_ptr = raven_gc_alloc(size_of_string(), align_of_string(), TAG_STRING) as *mut String;
    if header_ptr.is_null() {
        if !bytes_ptr.is_null() {
            raven_dealloc(bytes_ptr, cap as usize, 1);
        }
        return ptr::null_mut();
    }
    // SAFETY: header_ptr points to writable, correctly aligned storage.
    unsafe {
        ptr::write(
            header_ptr,
            String {
                header: ObjectHeader::new(TAG_STRING, 0, cap),
                bytes: bytes_ptr,
            },
        );
    }
    header_ptr
}

/// Return the byte length of the string, i.e. `header.len`.
///
/// Returns zero when `s` is null.
#[no_mangle]
pub extern "C" fn raven_string_len(s: *const String) -> u32 {
    if s.is_null() {
        return 0;
    }
    // SAFETY: caller passes a pointer obtained from a constructor.
    unsafe { (*s).header.len }
}

/// Return a pointer to the string's UTF-8 byte buffer.
///
/// The buffer is valid for `raven_string_len(s)` bytes. Returns null
/// when `s` is null.
#[no_mangle]
pub extern "C" fn raven_string_bytes(s: *const String) -> *const u8 {
    if s.is_null() {
        return ptr::null();
    }
    // SAFETY: caller passes a pointer obtained from a constructor.
    unsafe { (*s).bytes }
}

/// Concatenate two strings into a freshly allocated string.
///
/// The result has `len = a.len + b.len` and a capacity equal to that
/// length. Either input may be null, in which case it is treated as
/// the empty string.
#[no_mangle]
pub extern "C" fn raven_string_concat(a: *const String, b: *const String) -> *mut String {
    let a_len = raven_string_len(a) as usize;
    let b_len = raven_string_len(b) as usize;
    let total = a_len + b_len;
    let total_u32 = match u32::try_from(total) {
        Ok(v) => v,
        Err(_) => return ptr::null_mut(),
    };
    let out = raven_string_new(total_u32);
    if out.is_null() {
        return ptr::null_mut();
    }
    // SAFETY: out was just constructed with `total_u32` capacity. We
    // copy `a_len` bytes from a's buffer then `b_len` bytes from b's
    // buffer, both bounded by the caller-supplied lengths.
    unsafe {
        let dst = (*out).bytes;
        if a_len > 0 {
            ptr::copy_nonoverlapping(raven_string_bytes(a), dst, a_len);
        }
        if b_len > 0 {
            ptr::copy_nonoverlapping(raven_string_bytes(b), dst.add(a_len), b_len);
        }
        (*out).header.len = total_u32;
    }
    out
}

/// Build a GC-managed `String` from a borrowed UTF-8 byte slice.
///
/// Allocates a fresh `String` with capacity equal to `len`, copies the
/// `len` bytes from `ptr`, and sets the length. A zero `len` (or null
/// `ptr`) yields an empty string. Returns null on allocation failure.
///
/// The back-end calls this to promote a static string literal into a
/// heap String value so every `String`-typed local is a real GC object
/// the collector can trace and the concat path can consume.
///
/// # Safety
///
/// `ptr` must point to `len` initialized UTF-8 bytes, or `len` must be
/// zero.
#[no_mangle]
pub extern "C" fn raven_string_from_bytes(ptr: *const u8, len: usize) -> *mut String {
    let len_u32 = match u32::try_from(len) {
        Ok(v) => v,
        Err(_) => return ptr::null_mut(),
    };
    let out = raven_string_new(len_u32);
    if out.is_null() {
        return ptr::null_mut();
    }
    if len_u32 > 0 && !ptr.is_null() {
        // SAFETY: out has `len` bytes of capacity, and the caller
        // guarantees `ptr` points to `len` initialized bytes.
        unsafe {
            ptr::copy_nonoverlapping(ptr, (*out).bytes, len);
            (*out).header.len = len_u32;
        }
    }
    out
}

/// Return the byte at index `i` of `s` as a value in `0..=255`, or `-1`
/// when `i` is out of range (or `s` is null). The index is a byte
/// offset, not a code point or grapheme index; see the `std/string`
/// spec for the byte-vs-codepoint semantics.
///
/// Backs the `__str_byte_at` compiler intrinsic, which the bundled
/// `std/string` source uses to scan a string byte by byte.
#[no_mangle]
pub extern "C" fn raven_string_byte_at(s: *const String, i: usize) -> i32 {
    let len = raven_string_len(s) as usize;
    if i >= len {
        return -1;
    }
    let bytes = raven_string_bytes(s);
    if bytes.is_null() {
        return -1;
    }
    // SAFETY: `i < len` and the buffer holds `len` valid bytes.
    let byte = unsafe { *bytes.add(i) };
    byte as i32
}

/// Allocate a fresh `String` holding the half-open byte range
/// `[start, end)` of `s`. The bounds are clamped to `0..=len` and a
/// `start` past `end` yields an empty string, so the function never
/// reads out of range. The range is in bytes; slicing through the
/// middle of a multi-byte UTF-8 sequence produces a string whose bytes
/// are not valid UTF-8, which the byte-oriented `std/string` surface
/// documents as the caller's responsibility.
///
/// Backs the `__str_substring` compiler intrinsic.
#[no_mangle]
pub extern "C" fn raven_string_substring(
    s: *const String,
    start: usize,
    end: usize,
) -> *mut String {
    let len = raven_string_len(s) as usize;
    let start = start.min(len);
    let end = end.min(len);
    if start >= end {
        return raven_string_new(0);
    }
    let bytes = raven_string_bytes(s);
    if bytes.is_null() {
        return raven_string_new(0);
    }
    // SAFETY: `start < end <= len`, so the source range is in bounds.
    unsafe { raven_string_from_bytes(bytes.add(start), end - start) }
}

/// Allocate a fresh one-byte `String` from the low eight bits of
/// `byte`. Used by the `std/string` case-mapping and builder paths to
/// turn a computed byte value back into a string before concatenation.
///
/// Backs the `__str_from_byte` compiler intrinsic.
#[no_mangle]
pub extern "C" fn raven_string_from_byte(byte: i32) -> *mut String {
    let b = (byte & 0xff) as u8;
    raven_string_from_bytes(&b as *const u8, 1)
}

/// Allocate a GC `String` whose bytes are the decimal rendering of a
/// signed 64-bit integer. Negatives carry a leading `-`; zero renders
/// as `0`.
#[no_mangle]
pub extern "C" fn raven_int_to_string(value: i64) -> *mut String {
    let mut buf = [0u8; 20];
    let s = format_i64_into(value, &mut buf);
    raven_string_from_bytes(s.as_ptr(), s.len())
}

/// Allocate a GC `String` rendering of a boolean. The C ABI passes the
/// value as an `i8`; any nonzero value is `true`.
#[no_mangle]
pub extern "C" fn raven_bool_to_string(value: i8) -> *mut String {
    let s: &[u8] = if value != 0 { b"true" } else { b"false" };
    raven_string_from_bytes(s.as_ptr(), s.len())
}

/// Allocate a GC `String` rendering of an `f64`, matching Rust's
/// default `{}` float formatting (so `7.0` renders as `7`).
#[no_mangle]
pub extern "C" fn raven_float_to_string(value: f64) -> *mut String {
    let rendered = format!("{}", value);
    raven_string_from_bytes(rendered.as_ptr(), rendered.len())
}

/// Allocate a GC `String` holding a single Unicode scalar value. The C
/// ABI passes the `Char` as a `u32` code point; an invalid code point
/// renders as the Unicode replacement character.
#[no_mangle]
pub extern "C" fn raven_char_to_string(value: u32) -> *mut String {
    let ch = char::from_u32(value).unwrap_or('\u{FFFD}');
    let mut buf = [0u8; 4];
    let s = ch.encode_utf8(&mut buf);
    raven_string_from_bytes(s.as_ptr(), s.len())
}

/// Format `value` into `buf` as base-ten ASCII and return the written
/// slice. Twenty bytes hold the widest `i64` (`-9223372036854775808`).
/// Mirrors the `format_i64` helper in the crate root; kept local so the
/// string conversions do not reach across modules.
fn format_i64_into(value: i64, buf: &mut [u8; 20]) -> &str {
    let negative = value < 0;
    let mut magnitude = value.unsigned_abs();
    let mut pos = buf.len();
    loop {
        pos -= 1;
        buf[pos] = b'0' + (magnitude % 10) as u8;
        magnitude /= 10;
        if magnitude == 0 {
            break;
        }
    }
    if negative {
        pos -= 1;
        buf[pos] = b'-';
    }
    // SAFETY: every written byte is an ASCII digit or '-'.
    unsafe { std::str::from_utf8_unchecked(&buf[pos..]) }
}

/// Size, in bytes, of the in-memory `String` object on the host
/// target. Used by constructors and by the collector's free routine.
pub(crate) const fn size_of_string() -> usize {
    std::mem::size_of::<String>()
}

/// Alignment, in bytes, of the in-memory `String` object.
pub(crate) const fn align_of_string() -> usize {
    let a = align_of::<String>();
    if a > OBJECT_ALIGN {
        a
    } else {
        OBJECT_ALIGN
    }
}

/// Free a `String`'s owned byte buffer. The object body is freed by the
/// collector after this call; this routine only releases the separate
/// `bytes` allocation.
///
/// # Safety
///
/// `s` must point to a live `String` produced by `raven_string_new`.
pub(crate) unsafe fn free_buffers(s: *mut String) {
    // SAFETY: caller guarantees `s` is a live String.
    let cap = unsafe { (*s).header.cap };
    let bytes = unsafe { (*s).bytes };
    if !bytes.is_null() && cap > 0 {
        raven_dealloc(bytes, cap as usize, 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{align_of, offset_of, size_of};

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn string_size_and_offsets_match_spec() {
        assert_eq!(size_of::<String>(), 24);
        assert_eq!(offset_of!(String, header), 0);
        assert_eq!(offset_of!(String, bytes), 16);
        assert!(align_of::<String>() >= 8);
    }

    #[test]
    fn new_zero_capacity_yields_empty_string() {
        let s = raven_string_new(0);
        assert!(!s.is_null());
        // SAFETY: s came from the constructor.
        unsafe {
            assert_eq!((*s).header.tag, TAG_STRING);
            assert_eq!((*s).header.len, 0);
            assert_eq!((*s).header.cap, 0);
            assert!((*s).bytes.is_null());
        }
        // Manual cleanup until the GC arrives.
        unsafe { drop_string_for_test(s) };
    }

    #[test]
    fn new_with_capacity_zero_fills_buffer() {
        let cap = 16u32;
        let s = raven_string_new(cap);
        assert!(!s.is_null());
        // SAFETY: s came from the constructor with `cap` bytes.
        unsafe {
            assert_eq!((*s).header.cap, cap);
            for i in 0..cap as usize {
                assert_eq!((*s).bytes.add(i).read(), 0);
            }
        }
        unsafe { drop_string_for_test(s) };
    }

    #[test]
    fn len_and_bytes_accessors_match_header() {
        let s = raven_string_new(8);
        assert!(!s.is_null());
        // SAFETY: write into the just-allocated buffer.
        unsafe {
            ptr::copy_nonoverlapping(b"hi".as_ptr(), (*s).bytes, 2);
            (*s).header.len = 2;
        }
        assert_eq!(raven_string_len(s), 2);
        let bytes = raven_string_bytes(s);
        assert!(!bytes.is_null());
        // SAFETY: bytes points to two valid bytes "hi".
        let slice = unsafe { std::slice::from_raw_parts(bytes, 2) };
        assert_eq!(slice, b"hi");
        unsafe { drop_string_for_test(s) };
    }

    #[test]
    fn concat_joins_inputs() {
        let a = raven_string_new(5);
        let b = raven_string_new(5);
        assert!(!a.is_null() && !b.is_null());
        // SAFETY: a and b have capacity 5 each.
        unsafe {
            ptr::copy_nonoverlapping(b"hel".as_ptr(), (*a).bytes, 3);
            (*a).header.len = 3;
            ptr::copy_nonoverlapping(b"lo".as_ptr(), (*b).bytes, 2);
            (*b).header.len = 2;
        }
        let joined = raven_string_concat(a, b);
        assert!(!joined.is_null());
        assert_eq!(raven_string_len(joined), 5);
        // SAFETY: joined has len 5.
        let slice = unsafe { std::slice::from_raw_parts(raven_string_bytes(joined), 5) };
        assert_eq!(slice, b"hello");
        unsafe {
            drop_string_for_test(joined);
            drop_string_for_test(a);
            drop_string_for_test(b);
        }
    }

    #[test]
    fn concat_with_null_treats_input_as_empty() {
        let a = raven_string_new(2);
        // SAFETY: a has capacity 2.
        unsafe {
            ptr::copy_nonoverlapping(b"hi".as_ptr(), (*a).bytes, 2);
            (*a).header.len = 2;
        }
        let joined = raven_string_concat(a, std::ptr::null());
        assert!(!joined.is_null());
        assert_eq!(raven_string_len(joined), 2);
        let slice = unsafe { std::slice::from_raw_parts(raven_string_bytes(joined), 2) };
        assert_eq!(slice, b"hi");
        unsafe {
            drop_string_for_test(joined);
            drop_string_for_test(a);
        }
    }

    #[test]
    fn null_accessors_are_safe() {
        assert_eq!(raven_string_len(std::ptr::null()), 0);
        assert!(raven_string_bytes(std::ptr::null()).is_null());
    }

    /// Read a String object's bytes into a Rust `String` for assertions.
    fn read(s: *const String) -> std::string::String {
        let len = raven_string_len(s) as usize;
        if len == 0 {
            return std::string::String::new();
        }
        // SAFETY: s is a live String with `len` valid UTF-8 bytes.
        let slice = unsafe { std::slice::from_raw_parts(raven_string_bytes(s), len) };
        std::str::from_utf8(slice).unwrap().to_string()
    }

    #[test]
    fn from_bytes_copies_payload() {
        let src = b"hello";
        let s = raven_string_from_bytes(src.as_ptr(), src.len());
        assert!(!s.is_null());
        assert_eq!(read(s), "hello");
        unsafe { drop_string_for_test(s) };
    }

    #[test]
    fn from_bytes_empty_is_empty_string() {
        let s = raven_string_from_bytes(std::ptr::null(), 0);
        assert!(!s.is_null());
        assert_eq!(raven_string_len(s), 0);
        assert_eq!(read(s), "");
        unsafe { drop_string_for_test(s) };
    }

    #[test]
    fn int_to_string_handles_zero_and_negatives() {
        for (value, want) in [
            (0i64, "0"),
            (7, "7"),
            (-7, "-7"),
            (i64::MAX, "9223372036854775807"),
            (i64::MIN, "-9223372036854775808"),
        ] {
            let s = raven_int_to_string(value);
            assert!(!s.is_null());
            assert_eq!(read(s), want, "int_to_string({value})");
            unsafe { drop_string_for_test(s) };
        }
    }

    #[test]
    fn bool_to_string_renders_true_and_false() {
        let t = raven_bool_to_string(1);
        let f = raven_bool_to_string(0);
        // Any nonzero byte is true.
        let other = raven_bool_to_string(-1);
        assert_eq!(read(t), "true");
        assert_eq!(read(f), "false");
        assert_eq!(read(other), "true");
        unsafe {
            drop_string_for_test(t);
            drop_string_for_test(f);
            drop_string_for_test(other);
        }
    }

    #[test]
    fn float_to_string_matches_default_formatting() {
        let whole = raven_float_to_string(7.0);
        let frac = raven_float_to_string(3.5);
        let neg = raven_float_to_string(-0.25);
        assert_eq!(read(whole), "7");
        assert_eq!(read(frac), "3.5");
        assert_eq!(read(neg), "-0.25");
        unsafe {
            drop_string_for_test(whole);
            drop_string_for_test(frac);
            drop_string_for_test(neg);
        }
    }

    #[test]
    fn char_to_string_encodes_ascii_and_multibyte() {
        let a = raven_char_to_string('A' as u32);
        let euro = raven_char_to_string('€' as u32);
        let invalid = raven_char_to_string(0xD800); // lone surrogate
        assert_eq!(read(a), "A");
        assert_eq!(read(euro), "€");
        assert_eq!(read(invalid), "\u{FFFD}");
        unsafe {
            drop_string_for_test(a);
            drop_string_for_test(euro);
            drop_string_for_test(invalid);
        }
    }

    #[test]
    fn byte_at_returns_byte_or_minus_one() {
        let s = raven_string_from_bytes(b"abc".as_ptr(), 3);
        assert_eq!(raven_string_byte_at(s, 0), b'a' as i32);
        assert_eq!(raven_string_byte_at(s, 2), b'c' as i32);
        assert_eq!(raven_string_byte_at(s, 3), -1);
        assert_eq!(raven_string_byte_at(s, 99), -1);
        assert_eq!(raven_string_byte_at(std::ptr::null(), 0), -1);
        unsafe { drop_string_for_test(s) };
    }

    #[test]
    fn substring_extracts_clamped_range() {
        let s = raven_string_from_bytes(b"hello".as_ptr(), 5);
        let mid = raven_string_substring(s, 1, 4);
        assert_eq!(read(mid), "ell");
        // Bounds are clamped, and start past end yields empty.
        let tail = raven_string_substring(s, 3, 99);
        assert_eq!(read(tail), "lo");
        let empty = raven_string_substring(s, 4, 2);
        assert_eq!(read(empty), "");
        let whole = raven_string_substring(s, 0, 5);
        assert_eq!(read(whole), "hello");
        unsafe {
            drop_string_for_test(s);
            drop_string_for_test(mid);
            drop_string_for_test(tail);
            drop_string_for_test(empty);
            drop_string_for_test(whole);
        }
    }

    #[test]
    fn from_byte_builds_one_byte_string() {
        let a = raven_string_from_byte(b'Z' as i32);
        assert_eq!(read(a), "Z");
        assert_eq!(raven_string_len(a), 1);
        // Only the low eight bits are used.
        let masked = raven_string_from_byte(0x141); // low byte is 0x41 = 'A'
        assert_eq!(read(masked), "A");
        unsafe {
            drop_string_for_test(a);
            drop_string_for_test(masked);
        }
    }

    #[test]
    fn concat_result_is_gc_managed_and_correct() {
        let a = raven_string_from_bytes(b"foo".as_ptr(), 3);
        let b = raven_string_from_bytes(b"bar".as_ptr(), 3);
        let joined = raven_string_concat(a, b);
        assert!(!joined.is_null());
        assert_eq!(read(joined), "foobar");
        unsafe {
            drop_string_for_test(joined);
            drop_string_for_test(a);
            drop_string_for_test(b);
        }
    }

    /// Test-only deallocator. The real free path lands with the GC
    /// (issue #64); for unit tests we want valgrind-clean teardown.
    ///
    /// # Safety
    ///
    /// `s` must come from `raven_string_new` and not be freed yet.
    unsafe fn drop_string_for_test(s: *mut String) {
        if s.is_null() {
            return;
        }
        // SAFETY: matches the construction layout.
        unsafe { free_buffers(s) };
        raven_dealloc(s as *mut u8, size_of_string(), align_of_string());
    }
}
