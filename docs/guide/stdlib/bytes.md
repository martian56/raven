# std.bytes

Build and read binary data for wire protocols and file formats. A Raven
`str` holds bytes, so a `ByteWriter` produces a `str` and a `ByteReader`
reads one.

```rust
from std.bytes import ByteReader, ByteWriter

fun main() {
    // A length-prefixed message: type byte, 32-bit big-endian length, body.
    let msg = ByteWriter.new().u8(81).u32_be(13).cstr("SELECT 1").finish()
    let r = ByteReader.new(msg)
    print(r.u8().unwrap(), r.u32_be().unwrap(), r.cstr().unwrap()) // 81 13 SELECT 1
}
```

## ByteWriter

Every method returns the writer, so calls chain.

| Method | Writes |
|--------|--------|
| `u8(v)` | one byte (the low 8 bits of `v`) |
| `u16_be(v)`, `u16_le(v)` | 2 bytes, big- or little-endian |
| `u32_be(v)`, `u32_le(v)` | 4 bytes |
| `i64_be(v)`, `i64_le(v)` | 8 bytes, the full `int` |
| `bytes(s)` | the bytes of `s` as they are |
| `cstr(s)` | `s` and a NUL byte |

`len()` is the number of bytes written and `finish()` returns them. The
writer stays usable after `finish`.

## ByteReader

Each read takes bytes from the front and returns `Result`: asking for more
bytes than remain is an error (kind `"bytes"`) rather than a crash or a short
read.

| Method | Reads |
|--------|-------|
| `u8()` | one byte, 0 to 255 |
| `u16_be()`, `u16_le()`, `u32_be()`, `u32_le()` | unsigned integers |
| `i16_be()`, `i32_be()` | signed integers, sign-extended |
| `i64_be()`, `i64_le()` | 8-byte integers |
| `bytes(n)` | the next `n` bytes |
| `cstr()` | bytes up to the next NUL, consuming the NUL |
| `skip(n)` | moves past `n` bytes |
| `rest()` | everything not yet read |

`remaining()`, `at_end()`, and `position()` report where the reader is.

```rust
from std.bytes import ByteReader

fun main() {
    let r = ByteReader.new("ab")
    print(r.u32_be().is_err()) // true: only 2 bytes remain
}
```
