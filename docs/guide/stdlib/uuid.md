# std.uuid

Generate, parse, and inspect UUIDs (RFC 9562). A `Uuid` is a value: equal ids
compare equal, ids sort, and a `Uuid` can key a dict or set. Both generators
draw from the operating system's secure random source.

```rust
import std.uuid

fun main() {
    let id = uuid.v7()
    print(id.to_string().length(), id.version()) // 36 7
    let same = uuid.parse(id.to_string()).unwrap()
    print(id == same) // true
    print(uuid.parse("6BA7B8109DAD11D180B400C04FD430C8").unwrap())
    // 6ba7b810-9dad-11d1-80b4-00c04fd430c8
}
```

## Which version

- `v4()` is fully random.
- `v7()` starts with the Unix time in milliseconds, then random bits. Ids
  made in different milliseconds sort by creation time, so as a database key
  new rows land at the end of the index instead of scattering, which keeps
  inserts fast. Two ids from the same millisecond have no set order, and a
  system clock set backward can make a later id sort first.

## Functions

| Function | Returns |
|----------|---------|
| `v4()`, `v7()` | a new `Uuid` |
| `nil()` | the all-zero `Uuid` |
| `parse(s) -> Result[Uuid, Error]` | `s` parsed; it may be uppercase, unhyphenated, in braces, or prefixed with `urn:uuid:` in any case |
| `is_valid(s)` | whether `s` parses |

## Uuid methods

| Method | Returns |
|--------|---------|
| `to_string()` | lowercase and hyphenated |
| `version()` | the version digit: 4 or 7 for ids made here |
| `is_rfc_variant()` | whether the variant bits are the RFC 9562 ones |
| `is_nil()` | whether it is the all-zero id |
| `timestamp_millis() -> Result[int, Error]` | when a v7 id was made; an error for any other version or variant |
| `bytes()` | the 16 raw bytes |
