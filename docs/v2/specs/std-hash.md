# std/hash Spec

Non-cryptographic hashing building blocks: free functions over the bytes of
a `String` and over `Int`. These are fast, well-known mixing functions for
hash tables, checksums, and composite-value hashing.

Not cryptographic. Do not use these for passwords, signatures, integrity
against an adversary, or any security purpose. Cryptographic hashes (SHA-2,
BLAKE3, and similar) are out of scope for the standard library and belong in
a package, per the stdlib charter.

## Import

All entries are free functions, bound with a selective import:

```raven
import std/hash { fnv1a, djb2, hash_int, combine }

fun main() {
    let h = fnv1a("abc")
    let g = combine(h, hash_int(42))
}
```

## Surface

| Function | Result | Notes |
|---|---|---|
| `fnv1a(s: String)` | `Int` | FNV-1a over the bytes of `s`, 64-bit variant |
| `djb2(s: String)` | `Int` | classic djb2 string hash (`hash * 33 + byte`, seed 5381) |
| `hash_int(n: Int)` | `Int` | splitmix64-style bit-mix so sequential ints scatter |
| `combine(seed, value)` | `Int` | combine two hashes into one (boost `hash_combine`) |
| `checksum(s: String)` | `Int` | additive byte checksum; cheap and weak, change detection only |

## Integers wrap

Raven `Int` is i64 and arithmetic wraps in two's complement. That wrapping is
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

The prelude defines `trait Hash { fun hash(self) -> Int }` with built-in impls
for `Int`, `Bool`, and `String`. These functions are building blocks a user's
own `Hash` impl can call. A struct can hash its fields and fold them with
`combine`:

```raven
import std/hash { fnv1a, hash_int, combine }

struct Point { x: Int, y: Int }

impl Hash for Point {
    fun hash(self) -> Int {
        let h = hash_int(self.x)
        combine(h, hash_int(self.y))
    }
}
```

## Out of scope

Cryptographic hashing and keyed/seeded MACs. Streaming/incremental hasher
objects. These functions take a whole `String` or `Int` and return a final
hash.
