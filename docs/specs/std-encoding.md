# std/encoding Spec

Hex and standard base64 over the bytes of a `String`. Free functions that
encode a `String`'s raw bytes to text and decode the inverse.

## Byte model

A Raven `String` is a byte buffer. These functions treat the input as raw
bytes (`__str_byte_at` reads byte values 0..255), so they work on arbitrary
binary data carried in a `String`, not only valid UTF-8 text. A decoder
returns `Result<String, Error>`: the decoded bytes on success, or an `Error`
tagged `"encoding"` when the input is malformed.

## Import

All entries are free functions, bound with a selective import:

```rust
import std/encoding { hex_encode, hex_decode, base64_encode, base64_decode }

fun main() {
    let h = hex_encode("abc")          // "616263"
    let b = base64_encode("abc")       // "YWJj"
    let back = base64_decode(b)        // Ok("abc")
}
```

## Surface

| Function | Result | Notes |
|---|---|---|
| `hex_encode(s: String)` | `String` | each input byte to two lowercase hex digits |
| `hex_decode(s: String)` | `Result<String, Error>` | inverse; accepts lower, upper, or mixed; Err on an odd length or a non-hex byte |
| `base64_encode(s: String)` | `String` | standard alphabet (`A-Z a-z 0-9 + /`), `=` padding |
| `base64_decode(s: String)` | `Result<String, Error>` | inverse; honors `=` padding; Err on a bad length, a non-alphabet byte, or misplaced padding |
| `base32_decode(s: String)` | `Result<String, Error>` | inverse; honors `=` padding; Err on a bad length, a non-alphabet byte, or data after padding |

## Known vectors

`hex_encode("abc")` is `616263`. `base64_encode` of `"abc"`, `"Man"`,
`"Ma"`, and `"M"` is `YWJj`, `TWFu`, `TWE=`, and `TQ==` (the last two show
one and two bytes of trailing input producing `=` padding). Encode then
decode round-trips: `base64_decode(base64_encode(s)) == s`.

## Invalid input

The decoders reject malformed input with an `Err` rather than silently
producing wrong bytes:

- `hex_decode`: an odd length, or any byte outside `0-9 a-f A-F`, is an `Err`.
- `base64_decode`: a length that is not a multiple of four, any byte outside
  the alphabet other than `=` padding, or padding before the final group (or a
  third-position pad without a fourth), is an `Err`.
- `base32_decode`: a length that is not a multiple of eight, any byte outside
  the alphabet other than `=` padding, or any data byte after a padding byte, is
  an `Err`.

## Out of scope

URL-safe base64 (the `-_` alphabet) and base64 line wrapping. Both can be
added later without changing the existing functions.

## Deferred

`utf8` and `csv` (named in the tracking issue) are deferred. A Raven `String`
is already a UTF-8 byte buffer, so a dedicated `utf8` codec adds little here,
and `csv` needs more surface (quoting, delimiters, record streaming) than
this module covers. They can land as separate work.
