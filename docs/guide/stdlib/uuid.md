# std.uuid

Generate, parse, and inspect UUIDs (RFC 9562). Both generators draw from the
operating system's secure random source.

```rust
import std.uuid

fun main() {
    let id = uuid.v7()
    print(len(id), uuid.version(id)) // 36 7
    print(uuid.parse("6BA7B8109DAD11D180B400C04FD430C8").unwrap())
    // 6ba7b810-9dad-11d1-80b4-00c04fd430c8
}
```

## Which version

- `v4()` is fully random.
- `v7()` starts with the Unix time in milliseconds, then random bits, so ids
  sort by creation time. As a database key it keeps new rows at the end of
  the index instead of scattering them, which keeps inserts fast.

## Functions

| Function | Returns |
|----------|---------|
| `v4()`, `v7()` | a new UUID, lowercase and hyphenated |
| `nil()` | `00000000-0000-0000-0000-000000000000` |
| `parse(s) -> Result[str, Error]` | the canonical form of `s`, which may be uppercase, unhyphenated, in braces, or prefixed with `urn:uuid:` |
| `is_valid(s)` | whether `s` parses |
| `version(s)` | the version digit, or -1 if `s` is not a UUID |
| `timestamp_millis(s) -> Result[int, Error]` | when a v7 UUID was made |
