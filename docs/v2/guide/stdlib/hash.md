# std/hash

Non-cryptographic hashing building blocks: free functions that hash the bytes
of a `String` or mix an `Int`. They are fast, well-known mixing functions meant
for hash tables, checksums, and folding several values into one composite hash.

```rust
import std/hash { fnv1a }

fun main() {
    print(fnv1a("abc"))     // a 64-bit FNV-1a hash, may be negative
}
```

## Not cryptographic

These functions are **not** cryptographic. Do not use them for passwords,
signatures, integrity against an adversary, or any security purpose. They make
no collision-resistance or preimage guarantees. Cryptographic hashes (SHA-2,
BLAKE3, and similar) are out of scope for the standard library and belong in a
package.

What they are good for: keys in a hash table, quick change detection, and
combining the hashes of a struct's fields.

## Importing

Every entry is a free function, so bring in the ones you need with a selective
import:

```rust
import std/hash { fnv1a, djb2, crc32, hash_int, combine, checksum }
```

## A note on integers

Raven `Int` is i64 and arithmetic wraps in two's complement. That wrapping is
the intended behavior here: the FNV multiply, the djb2 multiply, and the
integer mixes all rely on it. Returned hashes use the full i64 range and may be
negative.

`fnv1a` uses the standard FNV-64 offset basis (`14695981039346656037`, written
as its i64 two's-complement value) and prime (`1099511628211`), so it
reproduces the canonical FNV-1a 64-bit vectors (for example `fnv1a("abc")` is
`0xe71fa2190541574b`).

`>>` is an arithmetic (sign-propagating) shift. The integer mixes stay
deterministic under it; the values differ from an unsigned-shift reference, but
that does not matter for a non-cryptographic hash.

## Surface

| Function | Result | Notes |
|---|---|---|
| `fnv1a(s: String)` | `Int` | FNV-1a over the bytes of `s`, 64-bit variant |
| `djb2(s: String)` | `Int` | classic djb2 string hash (`hash * 33 + byte`, seed 5381) |
| `hash_int(n: Int)` | `Int` | splitmix64-style bit-mix so sequential ints scatter |
| `combine(seed: Int, value: Int)` | `Int` | fold two hashes into one (boost `hash_combine`) |
| `crc32(s: String)` | `Int` | CRC-32 IEEE checksum, in `[0, 2^32)` |
| `checksum(s: String)` | `Int` | additive byte checksum; cheap and weak, change detection only |

## Hashing strings

### `fnv1a(s: String) -> Int`

FNV-1a over the bytes of `s`, the 64-bit variant. A good general-purpose string
hash with strong avalanche behavior for short keys. Prefer this when you want
one string hash and are unsure which to pick.

```rust
import std/hash { fnv1a }

fun main() {
    print(fnv1a("abc"))     // 0xe71fa2190541574b as a signed i64
    print(fnv1a(""))        // the offset basis, for the empty string
}
```

### `djb2(s: String) -> Int`

The classic djb2 hash: start at `5381`, then `hash = hash * 33 + byte` for each
byte. Simple and fast; included as a well-known alternative to `fnv1a`.

```rust
import std/hash { djb2 }

fun main() {
    print(djb2("raven"))
}
```

### `crc32(s: String) -> Int`

The CRC-32 IEEE checksum of the bytes of `s`. Unlike the i64 hashes above,
the result is always in `[0, 2^32)` and never negative. A checksum for change
detection, not for security.

```rust
import std/hash { crc32 }

fun main() {
    print(crc32("123456789"))   // 3421780262
}
```

### `checksum(s: String) -> Int`

A plain additive checksum: the sum of the byte values in `s`. Cheaper and
weaker than the hashes above, so use it only for quick change detection (did
this string change?), not for distributing keys across buckets.

```rust
import std/hash { checksum }

fun main() {
    print(checksum("abc"))      // 97 + 98 + 99 = 294
}
```

## Mixing integers

### `hash_int(n: Int) -> Int`

A splitmix64-style bit-mix. Sequential integers (`0, 1, 2, ...`) hash to
scattered, well-spread values, which is what you want before using them as
table indices.

```rust
import std/hash { hash_int }

fun main() {
    print(hash_int(0))
    print(hash_int(1))      // far from hash_int(0)
}
```

### `combine(seed: Int, value: Int) -> Int`

Fold `value` into `seed` and return the mixed result (the boost `hash_combine`
recipe, using the 32-bit golden ratio constant). Chain it to hash a sequence of
values into a single hash:

```rust
import std/hash { hash_int, combine }

fun main() {
    let h = hash_int(1)
    h = combine(h, hash_int(2))
    h = combine(h, hash_int(3))
    print(h)        // one hash that depends on all three values, in order
}
```

Order matters: `combine` is not commutative, so `[1, 2]` and `[2, 1]` produce
different hashes.

## Relationship to the Hash trait

The prelude defines `trait Hash { fun hash(self) -> Int }` with built-in impls
for `Int`, `Bool`, and `String`, which is what [std/collections](collections.md)
uses to key its hash-based containers. These functions are the building blocks
a user's own `Hash` impl can call: hash each field, then fold the results with
`combine`.

```rust
import std/hash { hash_int, combine }

struct Point { x: Int, y: Int }

impl Hash for Point {
    fun hash(self) -> Int {
        let h = hash_int(self.x)
        combine(h, hash_int(self.y))
    }
}
```

## Worked example: hashing a record

Hash a small struct by mixing a string field and an integer field into one
value, suitable for use as a key.

```rust
import std/hash { fnv1a, hash_int, combine }

struct User { name: String, id: Int }

fun hash_user(u: User) -> Int {
    let h = fnv1a(u.name)
    return combine(h, hash_int(u.id))
}

fun main() {
    let a = User { name: "ada", id: 1 }
    let b = User { name: "ada", id: 2 }

    print(hash_user(a) == hash_user(a))     // true, hashing is deterministic
    print(hash_user(a) == hash_user(b))     // false, the id differs
}
```

## See also

- [std/collections](collections.md) for the hash-based containers that build on
  the `Hash` trait.
- [std/string](string.md) for the string methods you may call before hashing.
