# std/encoding Spec

Hex and standard base64 over the bytes of a `String`. Free functions that
encode a `String`'s raw bytes to text and decode the inverse.

## Byte model

A Raven `String` is a byte buffer. These functions treat the input as raw
bytes (`__str_byte_at` reads byte values 0..255), so they work on arbitrary
binary data carried in a `String`, not only valid UTF-8 text. A decoder
likewise returns the decoded bytes as a `String`.

## Import

All entries are free functions, bound with a selective import:

```rust
import std/encoding { hex_encode, hex_decode, base64_encode, base64_decode }

fun main() {
    let h = hex_encode("abc")          // "616263"
    let b = base64_encode("abc")       // "YWJj"
    let back = base64_decode(b)        // "abc"
}
```

## Surface

| Function | Result | Notes |
|---|---|---|
| `hex_encode(s: String)` | `String` | each input byte to two lowercase hex digits |
| `hex_decode(s: String)` | `String` | inverse; accepts lowercase, uppercase, or mixed |
| `base64_encode(s: String)` | `String` | standard alphabet (`A-Z a-z 0-9 + /`), `=` padding |
| `base64_decode(s: String)` | `String` | inverse; honors `=` padding |

## Known vectors

`hex_encode("abc")` is `616263`. `base64_encode` of `"abc"`, `"Man"`,
`"Ma"`, and `"M"` is `YWJj`, `TWFu`, `TWE=`, and `TQ==` (the last two show
one and two bytes of trailing input producing `=` padding). Encode then
decode round-trips: `base64_decode(base64_encode(s)) == s`.

## Invalid input

The decoders favor a simple, total mapping over rejecting input:

- `hex_decode`: digits outside `0-9 a-f A-F` map to nibble `0`. An odd final
  digit is read as the high nibble of a zero-padded byte.
- `base64_decode`: bytes outside the alphabet (other than `=` padding) map to
  sextet `0`. Trailing `=` padding sets how many bytes the final group
  yields. Input whose length is not a multiple of four decodes the leading
  whole groups and drops a trailing partial group.

## Out of scope

URL-safe base64 (the `-_` alphabet) and base64 line wrapping. Both can be
added later without changing the existing functions.

## Deferred

`utf8` and `csv` (named in the tracking issue) are deferred. A Raven `String`
is already a UTF-8 byte buffer, so a dedicated `utf8` codec adds little here,
and `csv` needs more surface (quoting, delimiters, record streaming) than
this module covers. They can land as separate work.
