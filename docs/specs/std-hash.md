# std.hash Spec

Non-cryptographic hashing building blocks: free functions over the bytes of
a `str` and over `int`. These are fast, well-known mixing functions for
hash tables, checksums, and composite-value hashing.

Not cryptographic. Do not use these for passwords, signatures, integrity
against an adversary, or any security purpose. Cryptographic hashes (SHA-2,
BLAKE3, and similar) are out of scope for the standard library and belong in
a package.

## Import

All entries are free functions:

```rust
from std.hash import fnv1a, djb2, hash_int, combine

fun main() {
    let h = fnv1a("abc")
    let g = combine(h, hash_int(42))
}
```

## Surface

| Function | Result | Notes |
|---|---|---|
| `fnv1a(s: str)` | `int` | FNV-1a over the bytes of `s`, 64-bit variant |
| `djb2(s: str)` | `int` | classic djb2 string hash (`hash * 33 + byte`, seed 5381) |
| `hash_int(n: int)` | `int` | splitmix64-style bit-mix so sequential ints scatter |
| `combine(seed, value)` | `int` | combine two hashes into one (boost `hash_combine`) |
| `checksum(s: str)` | `int` | additive byte checksum; cheap and weak, change detection only |
| `crc32(s: str)` | `int` | CRC-32 (IEEE) over the bytes of `s` |

## Integers wrap

Raven `int` is i64 and arithmetic wraps in two's complement. That wrapping is
the intended behavior here: the FNV multiply, the djb2 multiply, and the
integer mixes all rely on it. `fnv1a` uses the standard FNV-64 offset basis
(`14695981039346656037`, written as its i64 two's-complement value) and prime
(`1099511628211`); it reproduces the canonical FNV-1a 64 vectors (for example
`fnv1a("abc")` is `0xe71fa2190541574b`). Returned hashes are full-width i64
and may be negative.

`>>` is an arithmetic (sign-propagating) shift. The integer mixes stay
deterministic under it; the values differ from an unsigned-shift reference but
that does not matter for a non-cryptographic hash.

## Relationship to the Hash trait

The prelude defines `trait Hash { fun hash(self) -> int }` with built-in impls
for `int`, `bool`, `char`, `str`, and tuples. A class whose fields hash gets
`Hash` automatically. These functions are building blocks for a hand-written
`Hash` impl. A class can hash its fields and fold them with
`combine`:

```rust
from std.hash import fnv1a, hash_int, combine

class Point {
    x: int
    y: int
}

impl Hash for Point {
    fun hash(self) -> int {
        let h = hash_int(self.x)
        combine(h, hash_int(self.y))
    }
}
```

## Out of scope

Cryptographic hashing and keyed/seeded MACs. Streaming/incremental hasher
objects. These functions take a whole `str` or `int` and return a final
hash.
