# std.crypto

Hashes, HMAC, key derivation, secure random bytes, constant-time comparison,
and password hashing. The work runs in the runtime on
[ring](https://github.com/briansmith/ring), the same library behind
[std.tls](tls.md).

```rust
from std.crypto import sha256_hex, hash_password, verify_password, random_hex

fun main() {
    print(sha256_hex("abc")) // ba7816bf...f20015ad
    let stored = hash_password("hunter2")
    print(verify_password("hunter2", stored)) // true
    let token = random_hex(32) // 64 hex characters
    print(len(token))
}
```

## Bytes and hex

A Raven `str` holds bytes, so the plain forms (`sha256`, `hmac_sha256`,
`pbkdf2_sha256`, `random_bytes`) return raw bytes. The `_hex` forms and
`to_hex` give lowercase hex text, which is what you store or print. For
base64, pass raw bytes to `base64_encode` from [std.encoding](encoding.md).

## Functions

| Function | Returns |
|----------|---------|
| `sha256(data)`, `sha384(data)`, `sha512(data)` | the digest as raw bytes |
| `sha256_hex(data)`, `sha512_hex(data)` | the digest as hex |
| `hmac_sha256(key, data)`, `hmac_sha512(key, data)` | the MAC as raw bytes |
| `hmac_sha256_hex(key, data)` | the MAC as hex |
| `pbkdf2_sha256(password, salt, iterations, length)` | `length` derived bytes (1 to 1024), or `""` when an argument is out of range |
| `random_bytes(n)` | `n` bytes (0 to 65536) from the operating system's secure random source |
| `random_hex(n)` | `n` secure random bytes as `2n` hex characters |
| `to_hex(data)` | the bytes of `data` as hex |
| `constant_time_eq(a, b)` | whether `a` and `b` are equal, in time that depends only on their lengths |
| `hash_password(password)` | a salted hash to store |
| `verify_password(password, stored)` | whether `password` matches `stored` |

## Hashing in pieces

`Hasher` hashes data as it arrives, so a large file or stream never has to
be in memory at once. Start one with `Hasher.sha256()`, `sha384()`,
`sha512()`, `hmac_sha256(key)`, or `hmac_sha512(key)`; feed it with
`update(data)` (which chains), and read the result once with `digest()` or
`hex_digest()`.

```rust
from std.crypto import Hasher

fun main() {
    let h = Hasher.sha256()
    for chunk in ["first part, ", "second part"] {
        h.update(chunk)
    }
    print(h.hex_digest().length()) // 64
}
```

## Passwords

`hash_password` derives 32 bytes with PBKDF2-HMAC-SHA256 over a fresh 16-byte
random salt at 600,000 iterations (the OWASP recommendation for
PBKDF2-HMAC-SHA256), and returns
them as one string:

```text
pbkdf2-sha256$600000$<salt hex>$<hash hex>
```

Store that string as is. `verify_password` reads the iteration count and salt
back from it, so hashes made today keep verifying if a later release raises
the default. A malformed stored value never matches, and neither does one
whose salt or hash was shortened. Hashing and verifying are slow on purpose
(a large fraction of a second each), so a stolen hash is costly to guess. They run as a blocking call, so other goroutines keep
running and the collector is not held up.

## Tokens and signatures

For a session or API token, hand out `random_hex(32)` and store only its
`sha256_hex`, so a leaked database does not leak live tokens. To check a
signature or token, compare with `constant_time_eq`, never `==`, so the
comparison time does not reveal how many leading bytes matched:

```rust
from std.crypto import hmac_sha256_hex, constant_time_eq

fun signed(secret: str, body: str) -> str {
    return hmac_sha256_hex(secret, body)
}

fun valid(secret: str, body: str, signature: str) -> bool {
    return constant_time_eq(signed(secret, body), signature)
}

fun main() {
    let sig = signed("key", "payload")
    print(valid("key", "payload", sig)) // true
}
```

`std.random` is a fast generator for simulations and shuffles and is not
safe for secrets; use `random_bytes` or `random_hex` for anything an attacker
must not guess.
