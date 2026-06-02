//! Deterministic 64-bit hash used by Map and Set buckets.
//!
//! FNV-1a 64 is chosen because the persisted hash sits inside each
//! bucket and must be reproducible when the table resizes. The
//! function takes no per-process seed and pulls in no extra
//! dependency. See `docs/v2/specs/object-layout.md` for the design
//! note.

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Hash `bytes` with FNV-1a 64.
pub fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut h = FNV_OFFSET;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_yields_offset_basis() {
        assert_eq!(fnv1a_64(b""), FNV_OFFSET);
    }

    #[test]
    fn known_vector_matches_reference() {
        // FNV-1a 64 of "a" per the published reference.
        assert_eq!(fnv1a_64(b"a"), 0xaf63_dc4c_8601_ec8c);
        // FNV-1a 64 of "foobar".
        assert_eq!(fnv1a_64(b"foobar"), 0x8594_4171_f739_67e8);
    }

    #[test]
    fn different_inputs_collide_rarely() {
        // Sanity check, not a guarantee: two short distinct inputs
        // should produce different hashes.
        assert_ne!(fnv1a_64(b"hello"), fnv1a_64(b"world"));
    }
}
