//! std.crypto primitives over `ring`: digests, HMAC, PBKDF2, secure random
//! bytes, constant-time comparison, and password hashing.

use crate::object;
use ring::{digest, hmac, pbkdf2, rand::SecureRandom};
use std::num::NonZeroU32;
use std::slice;

const KDF_ROUNDS: u32 = 210_000;
const KDF_SALT_LEN: usize = 16;
const KDF_HASH_LEN: usize = 32;
const KDF_SCHEME: &str = "pbkdf2-sha256";

fn bytes_of(s: *const object::String) -> Vec<u8> {
    if s.is_null() {
        return Vec::new();
    }
    let ptr = object::raven_string_bytes(s);
    let len = object::raven_string_len(s) as usize;
    if ptr.is_null() || len == 0 {
        return Vec::new();
    }
    // SAFETY: a Raven String holds `len` initialized bytes.
    unsafe { slice::from_raw_parts(ptr, len) }.to_vec()
}

fn new_string(bytes: &[u8]) -> *mut object::String {
    object::raven_string_from_bytes(bytes.as_ptr(), bytes.len())
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(DIGITS[(b >> 4) as usize] as char);
        out.push(DIGITS[(b & 15) as usize] as char);
    }
    out
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

fn algorithm(name: &[u8]) -> Option<&'static digest::Algorithm> {
    match name {
        b"sha256" => Some(&digest::SHA256),
        b"sha384" => Some(&digest::SHA384),
        b"sha512" => Some(&digest::SHA512),
        _ => None,
    }
}

fn hmac_algorithm(name: &[u8]) -> Option<hmac::Algorithm> {
    match name {
        b"sha256" => Some(hmac::HMAC_SHA256),
        b"sha384" => Some(hmac::HMAC_SHA384),
        b"sha512" => Some(hmac::HMAC_SHA512),
        _ => None,
    }
}

/// Digest of `data` with `alg` ("sha256", "sha384", "sha512") as raw bytes,
/// or an empty String for an unknown algorithm.
#[no_mangle]
pub extern "C" fn raven_crypto_digest(
    alg: *const object::String,
    data: *const object::String,
) -> *mut object::String {
    let Some(a) = algorithm(&bytes_of(alg)) else {
        return new_string(&[]);
    };
    let data = bytes_of(data);
    let d = crate::gc::blocking(|| digest::digest(a, &data));
    new_string(d.as_ref())
}

/// HMAC of `data` under `key` as raw bytes, or empty for an unknown algorithm.
#[no_mangle]
pub extern "C" fn raven_crypto_hmac(
    alg: *const object::String,
    key: *const object::String,
    data: *const object::String,
) -> *mut object::String {
    let Some(a) = hmac_algorithm(&bytes_of(alg)) else {
        return new_string(&[]);
    };
    let (key, data) = (bytes_of(key), bytes_of(data));
    let tag = crate::gc::blocking(|| hmac::sign(&hmac::Key::new(a, &key), &data));
    new_string(tag.as_ref())
}

/// PBKDF2-HMAC-SHA256 of `password` and `salt`, `length` bytes long. Returns
/// empty when `iterations` or `length` is not positive.
#[no_mangle]
pub extern "C" fn raven_crypto_pbkdf2_sha256(
    password: *const object::String,
    salt: *const object::String,
    iterations: i64,
    length: i64,
) -> *mut object::String {
    let (Some(iters), true) = (
        u32::try_from(iterations).ok().and_then(NonZeroU32::new),
        (1..=1024).contains(&length),
    ) else {
        return new_string(&[]);
    };
    let (password, salt) = (bytes_of(password), bytes_of(salt));
    let mut out = vec![0u8; length as usize];
    crate::gc::blocking(|| {
        pbkdf2::derive(
            pbkdf2::PBKDF2_HMAC_SHA256,
            iters,
            &salt,
            &password,
            &mut out,
        )
    });
    new_string(&out)
}

fn fill_random(buf: &mut [u8]) -> bool {
    ring::rand::SystemRandom::new().fill(buf).is_ok()
}

/// `n` bytes from the operating system's secure random source. Returns empty
/// when `n` is not in 0..=65536 or the source fails.
#[no_mangle]
pub extern "C" fn raven_crypto_random_bytes(n: i64) -> *mut object::String {
    if !(0..=65536).contains(&n) {
        return new_string(&[]);
    }
    let mut buf = vec![0u8; n as usize];
    if !fill_random(&mut buf) {
        return new_string(&[]);
    }
    new_string(&buf)
}

/// Whether `a` and `b` hold the same bytes, in time that depends only on
/// their lengths.
#[no_mangle]
pub extern "C" fn raven_crypto_equal(a: *const object::String, b: *const object::String) -> bool {
    let (a, b) = (bytes_of(a), bytes_of(b));
    if a.len() != b.len() {
        return false;
    }
    let diff = a.iter().zip(&b).fold(0u8, |acc, (x, y)| acc | (x ^ y));
    std::hint::black_box(diff) == 0
}

/// Hash a password for storage as `pbkdf2-sha256$<iterations>$<salt hex>$<hash hex>`.
/// Returns empty if the random source fails.
#[no_mangle]
pub extern "C" fn raven_crypto_hash_password(
    password: *const object::String,
) -> *mut object::String {
    let password = bytes_of(password);
    let mut salt = [0u8; KDF_SALT_LEN];
    if !fill_random(&mut salt) {
        return new_string(&[]);
    }
    let iters = NonZeroU32::new(KDF_ROUNDS).unwrap();
    let mut out = [0u8; KDF_HASH_LEN];
    crate::gc::blocking(|| {
        pbkdf2::derive(
            pbkdf2::PBKDF2_HMAC_SHA256,
            iters,
            &salt,
            &password,
            &mut out,
        )
    });
    let stored = format!("{KDF_SCHEME}${KDF_ROUNDS}${}${}", hex(&salt), hex(&out));
    new_string(stored.as_bytes())
}

/// Whether `password` matches a hash from `raven_crypto_hash_password`.
/// A malformed stored hash never matches.
#[no_mangle]
pub extern "C" fn raven_crypto_verify_password(
    password: *const object::String,
    stored: *const object::String,
) -> bool {
    let password = bytes_of(password);
    let stored = bytes_of(stored);
    let Ok(stored) = std::str::from_utf8(&stored) else {
        return false;
    };
    let parts: Vec<&str> = stored.split('$').collect();
    let [scheme, iters, salt, hash] = parts[..] else {
        return false;
    };
    let (Some(iters), Some(salt), Some(hash)) = (
        iters.parse::<u32>().ok().and_then(NonZeroU32::new),
        unhex(salt),
        unhex(hash),
    ) else {
        return false;
    };
    if scheme != KDF_SCHEME || hash.is_empty() {
        return false;
    }
    crate::gc::blocking(|| {
        pbkdf2::verify(pbkdf2::PBKDF2_HMAC_SHA256, iters, &salt, &password, &hash).is_ok()
    })
}

/// Lowercase hex of the bytes of `data`.
#[no_mangle]
pub extern "C" fn raven_crypto_hex(data: *const object::String) -> *mut object::String {
    new_string(hex(&bytes_of(data)).as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips() {
        assert_eq!(hex(&[0, 15, 255]), "000fff");
        assert_eq!(unhex("000fff"), Some(vec![0, 15, 255]));
        assert_eq!(unhex("0g"), None);
        assert_eq!(unhex("abc"), None);
    }
}
