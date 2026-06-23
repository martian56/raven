# std/encoding

Hex, base64, base32, and URL percent encoding over the bytes of a `String`.
These are free functions that turn a `String`'s raw bytes into encoded text
and decode the inverse.

```rust
import std/encoding { hex_encode, base64_encode }

fun main() {
    print(hex_encode("abc"))        // 616263
    print(base64_encode("abc"))     // YWJj
}
```

## Importing

Every entry is a free function, so bind the ones you need with a selective
import:

```rust
import std/encoding { hex_encode, hex_decode, base64_encode, base64_decode, url_encode, url_decode, base32_encode, base32_decode }
```

Unlike [std/string](string.md) (which merges an `impl String` block on a bare
import), these are plain functions you call as `hex_encode(s)`, not methods on
`String`.

## A note on bytes

A Raven `String` is a byte buffer. Every function here reads byte values
`0..255` straight out of the input, so they work on arbitrary binary data
carried in a `String`, not only valid UTF-8 text.

The encoders and `url_decode` always succeed and return a `String`. The
`hex_decode`, `base64_decode`, and `base32_decode` decoders validate their
input and return `Result<String, Error>`: well-formed text from the matching
encoder decodes to `Ok(bytes)`, and malformed text (a bad character, a wrong
length, or stray padding) returns an `Err` rather than silently dropping or
zeroing data.

## Hex

### `hex_encode(s: String) -> String`

Encode each input byte as two lowercase hex digits. The result is twice as
long as the input.

```rust
import std/encoding { hex_encode }

fun main() {
    print(hex_encode("abc"))        // 616263
    print(hex_encode(""))           // (empty)
}
```

### `hex_decode(s: String) -> Result<String, Error>`

The inverse of `hex_encode`. Reads the input two digits at a time, accepting
lowercase, uppercase, or mixed hex (`0-9`, `a-f`, `A-F`). A non-hex character
or an odd number of digits is an `Err`.

```rust
import std/encoding { hex_decode }

fun main() {
    match hex_decode("616263") {
        Ok(bytes) -> print(bytes),       // abc
        Err(e) -> print(e.message()),
    }
}
```

Round-tripping is exact for any input:

```rust
import std/encoding { hex_encode, hex_decode }

fun main() {
    let s = "raven"
    match hex_decode(hex_encode(s)) {
        Ok(back) -> print(back == s),    // true
        Err(e) -> print(e.message()),
    }
}
```

## Base64

### `base64_encode(s: String) -> String`

Encode the input bytes with the standard base64 alphabet (`A-Z`, `a-z`,
`0-9`, `+`, `/`) and `=` padding. Every three input bytes become four output
characters; one or two trailing bytes are padded with `=`.

```rust
import std/encoding { base64_encode }

fun main() {
    print(base64_encode("abc"))     // YWJj
    print(base64_encode("Ma"))      // TWE=   (one byte of padding)
    print(base64_encode("M"))       // TQ==   (two bytes of padding)
}
```

### `base64_decode(s: String) -> Result<String, Error>`

The inverse of `base64_encode`. Honors trailing `=` padding to recover how
many bytes the final group yields. A length that is not a multiple of four, a
character outside the alphabet, or misplaced padding is an `Err`.

```rust
import std/encoding { base64_decode }

fun main() {
    match base64_decode("YWJj") {
        Ok(bytes) -> print(bytes),       // abc
        Err(e) -> print(e.message()),
    }
}
```

Encode then decode round-trips for any input:

```rust
import std/encoding { base64_encode, base64_decode }

fun main() {
    let s = "Hello, Raven"
    match base64_decode(base64_encode(s)) {
        Ok(back) -> print(back == s),    // true
        Err(e) -> print(e.message()),
    }
}
```

## URL percent encoding

### `url_encode(s: String) -> String`

Percent-encode the input bytes per RFC 3986. Unreserved characters (`A-Z`,
`a-z`, `0-9`, `-`, `.`, `_`, `~`) pass through; every other byte becomes
`%XX` with uppercase hex digits. A space becomes `%20`.

```rust
import std/encoding { url_encode }

fun main() {
    print(url_encode("a b/c"))      // a%20b%2Fc
    print(url_encode("hi~there"))   // hi~there  (unreserved pass through)
}
```

### `url_decode(s: String) -> String`

The inverse of `url_encode`. Reads each `%XX` escape back into its byte and
leaves other characters untouched. It always returns a `String`.

```rust
import std/encoding { url_encode, url_decode }

fun main() {
    print(url_decode("a%20b%2Fc"))          // a b/c
    let s = "name=John Doe&id=7"
    print(url_decode(url_encode(s)) == s)   // true
}
```

## Base32

### `base32_encode(s: String) -> String`

Encode the input bytes with the RFC 4648 base32 alphabet (`A-Z`, `2-7`) and
`=` padding. Every five input bytes become eight output characters; a partial
final group is padded with `=`.

```rust
import std/encoding { base32_encode }

fun main() {
    print(base32_encode("foobar"))  // MZXW6YTBOI======
}
```

### `base32_decode(s: String) -> Result<String, Error>`

The inverse of `base32_encode`. Honors trailing `=` padding to recover how
many bytes the final group yields. A character outside the alphabet, a wrong
length, or stray padding is an `Err`.

```rust
import std/encoding { base32_encode, base32_decode }

fun main() {
    let s = "Hello, Raven"
    match base32_decode(base32_encode(s)) {
        Ok(back) -> print(back == s),    // true
        Err(e) -> print(e.message()),
    }
}
```

## Invalid input

The decoders validate and reject malformed input rather than guessing at a
value, so a typo or a truncated string surfaces as an `Err` you can handle.

- `hex_decode`: a character outside `0-9 a-f A-F`, or an odd number of digits,
  is an `Err`.
- `base64_decode`: a length that is not a multiple of four, a character
  outside the alphabet, or `=` padding anywhere but the end is an `Err`.
- `base32_decode`: the same shape of rule for the base32 alphabet and its
  eight-character groups.

Round-tripping output from the matching encoder always decodes to `Ok`.

## Out of scope

URL-safe base64 (the `-_` alphabet) and base64 line wrapping are not provided.
Both can be added later without changing the existing functions.

## Worked example: a hex dump round trip

```rust
import std/encoding { hex_encode, hex_decode }

fun main() {
    let payload = "Raven 1.0"

    // Encode to a hex string you could log or store.
    let dumped = hex_encode(payload)
    print(dumped)                       // 526176656e20312e30

    // Decode it back to the original bytes.
    match hex_decode(dumped) {
        Ok(restored) -> {
            print(restored)             // Raven 1.0
            print(restored == payload)  // true
        },
        Err(e) -> print(e.message()),
    }
}
```

## See also

- [std/string](string.md) for inspecting, slicing, and transforming the
  `String` values these functions read and produce.
- [std/json](json.md) for structured serialization rather than raw byte
  encoding.
