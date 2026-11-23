//! In-memory layout, constructors, and accessors for Raven `String`.
//!
//! See `docs/specs/object-layout.md` for the byte-exact field
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
// Codegen emits string literals as static objects with this exact layout.
const _: () = {
    assert!(TAG_STRING == 0x01);
    assert!(std::mem::offset_of!(String, header) == 0);
    assert!(std::mem::offset_of!(String, bytes) == 16);
    assert!(std::mem::size_of::<String>() == 24);
};

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

/// High bit of `header.cap`: `bytes` points into a [`SharedBuf`] whose
/// spare room later concatenations may fill. Static literals never set it.
const SHARED: u32 = 1 << 31;
/// Results at least this long get spare room, so a loop of `s += x` grows
/// the buffer geometrically instead of copying the whole string each time.
const GROW_FROM: usize = 32;
const BUF_HDR: usize = std::mem::size_of::<SharedBuf>();

/// Header in front of a shared byte buffer. `used` is how far any string
/// has written; only a string whose length equals `used` may append in
/// place, so strings sharing the buffer never see each other's bytes.
#[repr(C)]
struct SharedBuf {
    refs: std::sync::atomic::AtomicU32,
    used: std::sync::atomic::AtomicU32,
}

fn shared_header(bytes: *mut u8) -> *const SharedBuf {
    // SAFETY: callers pass the data pointer of a shared buffer, which
    // starts BUF_HDR bytes after its header.
    unsafe { bytes.sub(BUF_HDR) as *const SharedBuf }
}

/// A GC `String` object over an existing buffer.
fn string_over(bytes: *mut u8, len: u32, cap_field: u32) -> *mut String {
    let header_ptr = raven_gc_alloc(size_of_string(), align_of_string(), TAG_STRING) as *mut String;
    if header_ptr.is_null() {
        return ptr::null_mut();
    }
    // SAFETY: header_ptr points to writable, correctly aligned storage.
    unsafe {
        ptr::write(
            header_ptr,
            String {
                header: ObjectHeader::new(TAG_STRING, len, cap_field),
                bytes,
            },
        );
    }
    header_ptr
}

/// Concatenate two strings.
///
/// When `a` sits at the end of a shared buffer with room for `b`, `b` is
/// written in place and the result shares the buffer: `s = s + x` in a
/// loop then costs amortized O(len(x)) instead of copying `s`. Otherwise a
/// fresh buffer is made, with spare room once the result is long enough.
/// Either input may be null, in which case it is treated as the empty
/// string.
#[no_mangle]
pub extern "C" fn raven_string_concat(a: *const String, b: *const String) -> *mut String {
    use std::sync::atomic::Ordering;
    let a_len = raven_string_len(a) as usize;
    let b_len = raven_string_len(b) as usize;
    let total = a_len + b_len;
    let total_u32 = match u32::try_from(total) {
        Ok(v) if v < SHARED / 2 => v,
        _ => return ptr::null_mut(),
    };
    if !a.is_null() && b_len > 0 {
        // SAFETY: a is a live String.
        let (a_cap, a_bytes) = unsafe { ((*a).header.cap, (*a).bytes) };
        if a_cap & SHARED != 0 && total <= (a_cap & !SHARED) as usize {
            let buf = shared_header(a_bytes);
            // SAFETY: a shared String's bytes always have a SharedBuf header.
            let claimed = unsafe {
                (*buf)
                    .used
                    .compare_exchange(a_len as u32, total_u32, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
            };
            if claimed {
                // SAFETY: the CAS reserved [a_len, total) for this call alone,
                // and the buffer holds at least `total` bytes.
                unsafe {
                    ptr::copy_nonoverlapping(raven_string_bytes(b), a_bytes.add(a_len), b_len);
                    (*buf).refs.fetch_add(1, Ordering::AcqRel);
                }
                let out = string_over(a_bytes, total_u32, a_cap);
                if out.is_null() {
                    // SAFETY: undo the reference taken for the failed object.
                    unsafe { (*buf).refs.fetch_sub(1, Ordering::AcqRel) };
                }
                return out;
            }
        }
    }
    if total < GROW_FROM {
        let out = raven_string_new(total_u32);
        if out.is_null() {
            return ptr::null_mut();
        }
        // SAFETY: out has `total` bytes of capacity; the copies are bounded
        // by the inputs' lengths.
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
        return out;
    }
    let cap = (total * 2).min((SHARED - 1) as usize);
    let block = raven_alloc(BUF_HDR + cap, align_of::<SharedBuf>().max(8));
    if block.is_null() {
        return ptr::null_mut();
    }
    // SAFETY: block holds BUF_HDR + cap writable bytes; the header is
    // initialised before any string can see it.
    let bytes = unsafe {
        ptr::write(
            block as *mut SharedBuf,
            SharedBuf {
                refs: std::sync::atomic::AtomicU32::new(1),
                used: std::sync::atomic::AtomicU32::new(total_u32),
            },
        );
        let bytes = block.add(BUF_HDR);
        if a_len > 0 {
            ptr::copy_nonoverlapping(raven_string_bytes(a), bytes, a_len);
        }
        if b_len > 0 {
            ptr::copy_nonoverlapping(raven_string_bytes(b), bytes.add(a_len), b_len);
        }
        bytes
    };
    let out = string_over(bytes, total_u32, SHARED | cap as u32);
    if out.is_null() {
        raven_dealloc(block, BUF_HDR + cap, align_of::<SharedBuf>().max(8));
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

/// Compare two strings by content. Returns `1` when both hold the same
/// bytes (and same length), `0` otherwise. Either pointer may be null,
/// in which case it is treated as the empty string, so two nulls are
/// equal and a null equals an empty string.
///
/// Backs the `==`/`!=` operators on `String`, which compare contents
/// rather than object identity.
#[no_mangle]
pub extern "C" fn raven_string_eq(a: *const String, b: *const String) -> i8 {
    if a == b {
        return 1;
    }
    let a_len = raven_string_len(a) as usize;
    let b_len = raven_string_len(b) as usize;
    if a_len != b_len {
        return 0;
    }
    if a_len == 0 {
        return 1;
    }
    let a_bytes = raven_string_bytes(a);
    let b_bytes = raven_string_bytes(b);
    if a_bytes.is_null() || b_bytes.is_null() {
        return 0;
    }
    // SAFETY: both buffers hold `a_len == b_len` valid bytes.
    let (a_slice, b_slice) = unsafe {
        (
            std::slice::from_raw_parts(a_bytes, a_len),
            std::slice::from_raw_parts(b_bytes, b_len),
        )
    };
    (a_slice == b_slice) as i8
}

/// Lexicographic byte comparison of two `String` values: negative when
/// `a < b`, zero when equal, positive when `a > b`. Either pointer may be
/// null and is treated as the empty string. Backs the `< <= > >=`
/// operators on `String`, which compare contents lexicographically.
#[no_mangle]
pub extern "C" fn raven_string_cmp(a: *const String, b: *const String) -> i64 {
    if a == b {
        return 0;
    }
    let a_len = raven_string_len(a) as usize;
    let b_len = raven_string_len(b) as usize;
    let a_bytes = raven_string_bytes(a);
    let b_bytes = raven_string_bytes(b);
    let a_slice = if a_bytes.is_null() || a_len == 0 {
        &[][..]
    } else {
        // SAFETY: `a_bytes` points at `a_len` valid bytes.
        unsafe { std::slice::from_raw_parts(a_bytes, a_len) }
    };
    let b_slice = if b_bytes.is_null() || b_len == 0 {
        &[][..]
    } else {
        // SAFETY: `b_bytes` points at `b_len` valid bytes.
        unsafe { std::slice::from_raw_parts(b_bytes, b_len) }
    };
    match a_slice.cmp(b_slice) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

/// Allocate a fresh `String` holding the half-open byte range
/// `[start, end)` of `s`. The bounds are signed and clamped to
/// `0..=len` (a negative bound clamps to 0), and a `start` past `end`
/// yields an empty string, so the function never reads out of range.
/// The range is in bytes; slicing through the middle of a multi-byte
/// UTF-8 sequence produces a string whose bytes are not valid UTF-8,
/// which the byte-oriented `std/string` surface documents as the
/// caller's responsibility.
///
/// Backs the `__str_substring` compiler intrinsic.
#[no_mangle]
pub extern "C" fn raven_string_substring(s: *const String, start: i64, end: i64) -> *mut String {
    let len = raven_string_len(s) as usize;
    // A Raven Int is signed, so the bounds arrive as `i64`. Clamp each into
    // `0..=len`: a negative bound clamps up to 0 and an over-long one clamps down
    // to `len`, matching the documented `0..length` clamping.
    let start = start.clamp(0, len as i64) as usize;
    let end = end.clamp(0, len as i64) as usize;
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

/// Allocate a GC `String` rendering of an `f64`. A whole number keeps its
/// `.0` (`7.0`, not `7`) so a float never prints like an int.
#[no_mangle]
pub extern "C" fn raven_float_to_string(value: f64) -> *mut String {
    let mut rendered = format!("{}", value);
    if value.is_finite() && !rendered.contains(['.', 'e', 'E']) {
        rendered.push_str(".0");
    }
    raven_string_from_bytes(rendered.as_ptr(), rendered.len())
}

/// The floating-point remainder of `a / b` with the sign of `a` (C `fmod`):
/// `5.5 % 2.0` is `1.5`, `-5.5 % 2.0` is `-1.5`, and a zero divisor gives NaN.
#[no_mangle]
pub extern "C" fn raven_float_mod(a: f64, b: f64) -> f64 {
    a % b
}

fn string_slice<'a>(s: *const String) -> &'a [u8] {
    let len = raven_string_len(s) as usize;
    let ptr = raven_string_bytes(s);
    if ptr.is_null() || len == 0 {
        &[]
    } else {
        // SAFETY: a Raven String holds `len` initialized bytes.
        unsafe { std::slice::from_raw_parts(ptr, len) }
    }
}

/// Lowercase hex of every byte of `s`.
#[no_mangle]
pub extern "C" fn raven_hex_encode(s: *const String) -> *mut String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let bytes = string_slice(s);
    let mut out = Vec::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize]);
        out.push(HEX[(b & 15) as usize]);
    }
    raven_string_from_bytes(out.as_ptr(), out.len())
}

/// Standard base64 (`+`, `/`, `=` padding) of the bytes of `s`.
#[no_mangle]
pub extern "C" fn raven_base64_encode(s: *const String) -> *mut String {
    const ALPHA: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = string_slice(s);
    let mut out = Vec::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHA[(n >> 18) as usize & 63]);
        out.push(ALPHA[(n >> 12) as usize & 63]);
        out.push(if chunk.len() > 1 {
            ALPHA[(n >> 6) as usize & 63]
        } else {
            b'='
        });
        out.push(if chunk.len() > 2 {
            ALPHA[n as usize & 63]
        } else {
            b'='
        });
    }
    raven_string_from_bytes(out.as_ptr(), out.len())
}

/// Percent-encode every byte of `s` outside RFC 3986's unreserved set
/// (`A-Z a-z 0-9 - _ . ~`), with uppercase hex; a space becomes `%20`.
#[no_mangle]
pub extern "C" fn raven_url_encode(s: *const String) -> *mut String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let bytes = string_slice(s);
    let mut out = Vec::with_capacity(bytes.len());
    for &b in bytes {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b);
        } else {
            out.push(b'%');
            out.push(HEX[(b >> 4) as usize]);
            out.push(HEX[(b & 15) as usize]);
        }
    }
    raven_string_from_bytes(out.as_ptr(), out.len())
}

/// Allocate the JSON form of a number: `{}` formatting (which never prints a
/// `.0`) for a finite value, `null` for infinity or NaN.
#[no_mangle]
pub extern "C" fn raven_json_number(value: f64) -> *mut String {
    if !value.is_finite() {
        return raven_string_from_bytes(b"null".as_ptr(), 4);
    }
    let rendered = format!("{}", value);
    raven_string_from_bytes(rendered.as_ptr(), rendered.len())
}

/// Allocate `s` as a quoted JSON string. Quote, backslash and control bytes
/// are escaped (named escapes where JSON has them, `\u00xx` otherwise), valid
/// UTF-8 passes through, and each byte of an invalid sequence becomes
/// `�`, so the output is always valid JSON.
#[no_mangle]
pub extern "C" fn raven_json_quote(s: *const String) -> *mut String {
    let len = raven_string_len(s) as usize;
    let ptr = raven_string_bytes(s);
    let bytes: &[u8] = if ptr.is_null() || len == 0 {
        &[]
    } else {
        // SAFETY: a Raven String holds `len` initialized bytes.
        unsafe { std::slice::from_raw_parts(ptr, len) }
    };
    let plain = bytes
        .iter()
        .all(|&b| (32..128).contains(&b) && b != b'"' && b != b'\\');
    let mut out: Vec<u8> = Vec::with_capacity(len + 2);
    out.push(b'"');
    if plain {
        out.extend_from_slice(bytes);
    } else {
        for chunk in bytes.utf8_chunks() {
            for &b in chunk.valid().as_bytes() {
                match b {
                    b'"' => out.extend_from_slice(b"\\\""),
                    b'\\' => out.extend_from_slice(b"\\\\"),
                    8 => out.extend_from_slice(b"\\b"),
                    12 => out.extend_from_slice(b"\\f"),
                    b'\n' => out.extend_from_slice(b"\\n"),
                    b'\r' => out.extend_from_slice(b"\\r"),
                    b'\t' => out.extend_from_slice(b"\\t"),
                    0..=31 => {
                        const HEX: &[u8; 16] = b"0123456789abcdef";
                        out.extend_from_slice(b"\\u00");
                        out.push(HEX[(b >> 4) as usize]);
                        out.push(HEX[(b & 15) as usize]);
                    }
                    _ => out.push(b),
                }
            }
            for _ in chunk.invalid() {
                out.extend_from_slice(b"\\uFFFD");
            }
        }
    }
    out.push(b'"');
    raven_string_from_bytes(out.as_ptr(), out.len())
}

/// Allocate a GC `String` rendering `value` with exactly `places` digits
/// after the decimal point (clamped to 0..=64), for `"${x:.2}"`.
#[no_mangle]
pub extern "C" fn raven_float_fixed(value: f64, places: i64) -> *mut String {
    let rendered = format!("{:.*}", places.clamp(0, 64) as usize, value);
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
/// Bytes of the buffer `s` owns; its share of a shared buffer.
pub(crate) unsafe fn buffer_bytes(s: *mut String) -> usize {
    // SAFETY: caller guarantees `s` is a live String.
    let cap = unsafe { (*s).header.cap };
    let bytes = unsafe { (*s).bytes };
    if bytes.is_null() || cap == 0 {
        return 0;
    }
    if cap & SHARED != 0 {
        let buf = shared_header(bytes);
        // SAFETY: a shared String's bytes always have a SharedBuf header.
        let refs = unsafe { (*buf).refs.load(std::sync::atomic::Ordering::Relaxed) }.max(1);
        return (BUF_HDR + (cap & !SHARED) as usize) / refs as usize;
    }
    cap as usize
}

pub(crate) unsafe fn free_buffers(s: *mut String) {
    // SAFETY: caller guarantees `s` is a live String.
    let cap = unsafe { (*s).header.cap };
    let bytes = unsafe { (*s).bytes };
    if bytes.is_null() || cap == 0 {
        return;
    }
    if cap & SHARED != 0 {
        // The last string over a shared buffer frees it.
        let buf = shared_header(bytes);
        // SAFETY: a shared String's bytes always have a SharedBuf header.
        let last = unsafe {
            (*buf)
                .refs
                .fetch_sub(1, std::sync::atomic::Ordering::AcqRel)
        } == 1;
        if last {
            let data = (cap & !SHARED) as usize;
            raven_dealloc(
                buf as *mut u8,
                BUF_HDR + data,
                align_of::<SharedBuf>().max(8),
            );
        }
        return;
    }
    raven_dealloc(bytes, cap as usize, 1);
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
    fn float_to_string_keeps_a_point() {
        let whole = raven_float_to_string(7.0);
        let frac = raven_float_to_string(3.5);
        let neg = raven_float_to_string(-0.25);
        assert_eq!(read(whole), "7.0");
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
    fn string_eq_compares_contents_not_identity() {
        let a = raven_string_from_bytes(b"foo".as_ptr(), 3);
        let b = raven_string_from_bytes(b"f".as_ptr(), 1);
        let oo = raven_string_from_bytes(b"oo".as_ptr(), 2);
        let c = raven_string_concat(b, oo);
        // Distinct objects, same bytes: equal.
        assert_eq!(raven_string_eq(a, c), 1);
        // Same pointer: equal.
        assert_eq!(raven_string_eq(a, a), 1);
        // Different length and different bytes: not equal.
        let bar = raven_string_from_bytes(b"bar".as_ptr(), 3);
        let abcd = raven_string_from_bytes(b"abcd".as_ptr(), 4);
        assert_eq!(raven_string_eq(a, bar), 0);
        assert_eq!(raven_string_eq(a, abcd), 0);
        // Null is the empty string: two nulls equal, null vs empty equal,
        // null vs non-empty not equal.
        let empty = raven_string_new(0);
        assert_eq!(raven_string_eq(std::ptr::null(), std::ptr::null()), 1);
        assert_eq!(raven_string_eq(std::ptr::null(), empty), 1);
        assert_eq!(raven_string_eq(std::ptr::null(), a), 0);
        unsafe {
            drop_string_for_test(a);
            drop_string_for_test(b);
            drop_string_for_test(oo);
            drop_string_for_test(c);
            drop_string_for_test(bar);
            drop_string_for_test(abcd);
            drop_string_for_test(empty);
        }
    }

    #[test]
    fn string_cmp_is_lexicographic() {
        let apple = raven_string_from_bytes(b"apple".as_ptr(), 5);
        let banana = raven_string_from_bytes(b"banana".as_ptr(), 6);
        let app = raven_string_from_bytes(b"app".as_ptr(), 3);
        let empty = raven_string_new(0);
        // a < b, b > a, equal is 0.
        assert!(raven_string_cmp(apple, banana) < 0);
        assert!(raven_string_cmp(banana, apple) > 0);
        assert_eq!(raven_string_cmp(apple, apple), 0);
        // A prefix sorts before the longer string.
        assert!(raven_string_cmp(app, apple) < 0);
        assert!(raven_string_cmp(apple, app) > 0);
        // Null is the empty string: less than any non-empty, equal to empty.
        assert!(raven_string_cmp(std::ptr::null(), apple) < 0);
        assert_eq!(raven_string_cmp(std::ptr::null(), empty), 0);
        unsafe {
            drop_string_for_test(apple);
            drop_string_for_test(banana);
            drop_string_for_test(app);
            drop_string_for_test(empty);
        }
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
        // A negative bound clamps to 0, not to a huge unsigned index (#556).
        let neg_start = raven_string_substring(s, -1, 4);
        assert_eq!(read(neg_start), "hell");
        let neg_end = raven_string_substring(s, 1, -1);
        assert_eq!(read(neg_end), "");
        unsafe {
            drop_string_for_test(s);
            drop_string_for_test(mid);
            drop_string_for_test(tail);
            drop_string_for_test(empty);
            drop_string_for_test(whole);
            drop_string_for_test(neg_start);
            drop_string_for_test(neg_end);
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

    #[test]
    fn appending_to_a_long_string_reuses_its_buffer() {
        let long = "x".repeat(40);
        let a = raven_string_from_bytes(long.as_ptr(), 40);
        let b = raven_string_from_bytes(b"yz".as_ptr(), 2);
        let ab = raven_string_concat(a, b);
        let abb = raven_string_concat(ab, b);
        assert_eq!(read(abb), format!("{long}yzyz"));
        unsafe {
            assert_ne!((*ab).header.cap & SHARED, 0);
            // The second append wrote into the first result's spare room.
            assert_eq!((*abb).bytes, (*ab).bytes);
            drop_string_for_test(abb);
            drop_string_for_test(ab);
            drop_string_for_test(a);
            drop_string_for_test(b);
        }
    }

    #[test]
    fn two_appends_to_one_prefix_do_not_share_bytes() {
        let long = "p".repeat(40);
        let base = raven_string_from_bytes(long.as_ptr(), 40);
        let bang = raven_string_from_bytes(b"!".as_ptr(), 1);
        let prefix = raven_string_concat(base, bang);
        let l = raven_string_from_bytes(b"LEFT".as_ptr(), 4);
        let r = raven_string_from_bytes(b"RIGHT".as_ptr(), 5);
        let left = raven_string_concat(prefix, l);
        let right = raven_string_concat(prefix, r);
        assert_eq!(read(left), format!("{long}!LEFT"));
        assert_eq!(read(right), format!("{long}!RIGHT"));
        assert_eq!(read(prefix), format!("{long}!"));
        unsafe {
            // The first branch took the spare room; the second had to copy.
            assert_eq!((*left).bytes, (*prefix).bytes);
            assert_ne!((*right).bytes, (*prefix).bytes);
            // Freeing every string over the shared buffer releases it once.
            for s in [left, right, prefix, base, bang, l, r] {
                drop_string_for_test(s);
            }
        }
    }

    #[test]
    fn short_results_stay_exact() {
        let a = raven_string_from_bytes(b"ab".as_ptr(), 2);
        let ab = raven_string_concat(a, a);
        unsafe {
            assert_eq!((*ab).header.cap, 4);
            drop_string_for_test(ab);
            drop_string_for_test(a);
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
        // SAFETY: `s` is a live collector-allocated string.
        unsafe { crate::gc::free_for_test(s as *mut crate::object::ObjectHeader) };
    }
}
