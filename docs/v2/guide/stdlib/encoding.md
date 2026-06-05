# std/encoding

Hex and standard base64 over the bytes of a `String`. These are free
functions that turn a `String`'s raw bytes into encoded text and decode the
inverse.

```rust
import std/encoding { hex_encode, base64_encode, base64_decode }

fun main() {
    print(hex_encode("abc"))        // 616263
    print(base64_encode("abc"))     // YWJj
    print(base64_decode("YWJj"))    // abc
}
```

## Importing

Every entry is a free function, so bind the ones you need with a selective
import:

```rust
import std/encoding { hex_encode, hex_decode, base64_encode, base64_decode }
```

Unlike [std/string](string.md) (which merges an `impl String` block on a bare
import), these are plain functions you call as `hex_encode(s)`, not methods on
`String`.

## A note on bytes

A Raven `String` is a byte buffer. Every function here reads byte values
`0..255` straight out of the input, so they work on arbitrary binary data
carried in a `String`, not only valid UTF-8 text. A decoder likewise returns
the decoded bytes as a `String`. None of these functions return a `Result`:
they each return a `String`, and the decoders map anything unexpected to a
defined value rather than failing (see [Invalid input](#invalid-input)).

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

### `hex_decode(s: String) -> String`

The inverse of `hex_encode`. Reads the input two digits at a time, accepting
lowercase, uppercase, or mixed hex (`0-9`, `a-f`, `A-F`).

```rust
import std/encoding { hex_decode }

fun main() {
    print(hex_decode("616263"))     // abc
    print(hex_decode("4D61"))       // Ma  (uppercase digits)
}
```

Round-tripping is exact for any input:

```rust
import std/encoding { hex_encode, hex_decode }

fun main() {
    let s = "raven"
    print(hex_decode(hex_encode(s)) == s)   // true
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
    print(base64_encode("Man"))     // TWFu
    print(base64_encode("Ma"))      // TWE=   (one byte of padding)
    print(base64_encode("M"))       // TQ==   (two bytes of padding)
}
```

### `base64_decode(s: String) -> String`

The inverse of `base64_encode`. Honors trailing `=` padding to recover how
many bytes the final group yields.

```rust
import std/encoding { base64_decode }

fun main() {
    print(base64_decode("YWJj"))    // abc
    print(base64_decode("TWE="))    // Ma
    print(base64_decode("TQ=="))    // M
}
```

Encode then decode round-trips for any input:

```rust
import std/encoding { base64_encode, base64_decode }

fun main() {
    let s = "Hello, Raven"
    print(base64_decode(base64_encode(s)) == s)   // true
}
```

## Invalid input

The decoders favor a simple, total mapping over rejecting input, so they
never fail. Pass well-formed text from the matching encoder and you always
get the original bytes back.

- `hex_decode`: any byte outside `0-9 a-f A-F` maps to nibble `0`. A lone
  final digit (odd input length) is read as the high nibble of a zero-padded
  byte.
- `base64_decode`: any byte outside the alphabet, other than `=` padding,
  maps to sextet `0`. Trailing `=` padding sets how many bytes the final
  group yields. Input whose length is not a multiple of four decodes the
  leading whole groups and drops a trailing partial group.

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
    let restored = hex_decode(dumped)
    print(restored)                     // Raven 1.0
    print(restored == payload)          // true
}
```

## See also

- [std/string](string.md) for inspecting, slicing, and transforming the
  `String` values these functions read and produce.
- [std/json](json.md) for structured serialization rather than raw byte
  encoding.
