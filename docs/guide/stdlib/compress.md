# std.compress

gzip, zlib, and raw DEFLATE. Data is bytes in a `str`.

```rust
import std.compress

fun main() {
    let body = "hello " * 1000
    let packed = compress.gzip(body)
    print(len(body), len(packed) < 100) // 6000 true
    print(compress.gunzip(packed).unwrap() == body) // true
}
```

## Formats

| Compress | Decompress | Used by |
|----------|------------|---------|
| `gzip(data)`, `gzip_level(data, level)` | `gunzip(data)`, `gunzip_limit(data, limit)` | HTTP `Content-Encoding: gzip`, `.gz` files |
| `zlib(data)`, `zlib_level(data, level)` | `unzlib(data)`, `unzlib_limit(data, limit)` | HTTP `Content-Encoding: deflate`, PNG |
| `deflate(data)`, `deflate_level(data, level)` | `inflate(data)`, `inflate_limit(data, limit)` | zip entries, WebSocket compression |

The level runs from 0 (store) through 1 (fastest) to 9 (smallest); the
plain forms use 6. `gunzip` joins concatenated gzip members.

## Size limits

A few kilobytes of compressed data can expand to gigabytes. Decompression
stops with an error once the output passes a limit: 256 MiB for the plain
forms, or the `limit` you pass to the `_limit` forms. When the input comes
from a client, pass the most you are willing to hold:

```rust
import std.compress

fun main() {
    let bomb = compress.gzip("0" * 1000000)
    print(compress.gunzip_limit(bomb, 65536).is_err()) // true
}
```

Corrupt input and output past the limit are errors of kind `"compress"`.
