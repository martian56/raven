# std/string Spec

## Goal

Provide a useful string-utility surface for Raven v2 programs: case
mapping, length and access, search, transforms, and validation. The
surface is method-first: every operation is a method on `String`, defined
in an `impl String` block in bundled Raven source compiled into the
program the same way `std/io` is (see `stdlib.md`). It is written on top
of a small set of byte-level compiler intrinsics, so most of the logic
dogfoods the language itself.

## Import

`std/string` is method-first, so there are no free names to import
selectively. A bare module import merges its `impl String` block into the
program; the methods are then resolved by the receiver type:

```raven
import std/string

fun main() {
    print("hello".to_upper())
}
```

## Byte versus codepoint semantics

Every function in this module is byte oriented. Indices, lengths, and
slices count UTF-8 bytes, not Unicode code points or grapheme clusters.

* `s.length()` returns the byte count. A string of multi-byte characters
  reports a length larger than its visible character count: `"é".length()`
  is 2 because `é` is the two bytes `0xC3 0xA9` in UTF-8.
* `s.char_at(i)` and `s.substring(start, end)` cut on byte boundaries.
  Cutting through the middle of a multi-byte character yields a string
  whose bytes are not valid UTF-8; that is the caller's responsibility.
* `s.index_of`, `s.contains`, `s.starts_with`, `s.ends_with`, and
  `s.replace` compare whole byte sequences, so they work correctly on
  multi-byte text as long as the needle and haystack are themselves valid
  UTF-8 (the common case). `"café".contains("fé")` is true because the
  byte sequence of `fé` occurs in `café`.
* `s.to_upper()` and `s.to_lower()` map only ASCII letters (`A`..`Z` <->
  `a`..`z`); every other byte, including the bytes of a multi-byte
  character, passes through unchanged.
* `s.trim()` and `s.is_blank()` treat the ASCII whitespace set (space,
  tab, newline, carriage return, vertical tab, form feed) as whitespace.

This byte model keeps v2.0 small and predictable. Code-point and
grapheme-aware operations are deferred (see Out of scope).

## The std/string surface

`stdlib/std/string.rv` defines `impl String` with these methods (each
takes `self`):

* `length() -> Int`: byte length.
* `is_empty() -> Bool`: true when the string has zero bytes.
* `char_at(i: Int) -> String`: the `i`-th byte as a one-byte string; out
  of range yields the empty string.
* `substring(start: Int, end: Int) -> String`: the half-open byte range
  `[start, end)`, with bounds clamped to `0..length()` and a `start` past
  `end` yielding the empty string.
* `concat(other: String) -> String`: join two strings.
* `to_upper() -> String`, `to_lower() -> String`: ASCII case mapping.
* `trim() -> String`: remove leading and trailing ASCII whitespace;
  interior whitespace is kept.
* `is_blank() -> Bool`: true when empty or all ASCII whitespace.
* `repeat(n: Int) -> String`: repeated `n` times; a non-positive `n`
  yields the empty string.
* `matches_at(needle: String, at: Int) -> Bool`: true when `needle`'s
  bytes occur starting at byte index `at`.
* `index_of(needle: String) -> Int`: byte index of the first occurrence
  of `needle`, or `-1` when absent; an empty `needle` returns `0`.
* `contains(needle: String) -> Bool`: true when `needle` occurs anywhere.
* `starts_with(prefix: String) -> Bool`, `ends_with(suffix: String) ->
  Bool`: prefix and suffix tests; an empty prefix or suffix always
  matches.
* `replace(from: String, to: String) -> String`: replace every
  non-overlapping occurrence of `from` with `to`, scanning left to right;
  an empty `from` returns the string unchanged so the scan terminates.

A free helper `is_space_byte(b: Int) -> Bool` classifies an ASCII
whitespace byte; the methods call it internally.

## Intrinsic boundary

The module needs byte-level operations below safe Raven. These are
exposed as internal compiler intrinsics whose names begin with `__str_`.
A user does not write them; they call the exported functions. The
intrinsics are recognized at three points, mirroring the `__io_*`
pattern (see `stdlib.md`):

* The resolver bypasses scope lookup for the `__str_*` names.
* The type checker assigns each intrinsic its signature.
* The codegen back end pattern matches the mangled name and emits a
  direct call to the matching `raven-runtime` C ABI symbol.

| Intrinsic                                  | Runtime symbol            | Meaning                                                       |
|--------------------------------------------|---------------------------|--------------------------------------------------------------|
| `__str_len(s: String) -> Int`              | `raven_string_len`        | byte length of `s` (u32 zero-extended to `Int`)              |
| `__str_byte_at(s: String, i: Int) -> Int`  | `raven_string_byte_at`    | byte at index `i` as `0..=255`, or `-1` when out of range    |
| `__str_substring(s, start, end) -> String` | `raven_string_substring`  | clamped half-open byte range `[start, end)`                  |
| `__str_from_byte(b: Int) -> String`        | `raven_string_from_byte`  | one-byte string from the low eight bits of `b`               |
| `__str_concat(a, b) -> String`             | `raven_string_concat`     | concatenate two strings into a fresh string                  |

`raven_string_len` and `raven_string_concat` already existed for the
print path and interpolation. The runtime gains three new symbols:
`raven_string_byte_at`, `raven_string_substring`, and
`raven_string_from_byte`. Each has unit tests in
`raven-runtime/src/object/string.rs`.

## Building strings in Raven

The transforms (`to_upper`, `to_lower`, `repeat`, `replace`) build their
result by repeated `__str_concat`, growing a `String` one piece at a
time. No mutable string builder was added to the runtime: the concat
primitive already produces a fresh GC-managed `String`, the inputs here
are small, and keeping the intrinsic surface minimal was preferred over
a builder. The repeated-concat approach is `O(n^2)` in the result length
for the character-by-character paths (`to_upper`, `to_lower`); for the
typical short strings this module targets that is acceptable in v2.0. A
builder (`raven_string_builder_*`) is a natural later optimization that
would not change this surface.

## Intra-module call resolution

Methods call each other through `self` (for example `contains` calls
`self.index_of`, which calls `self.matches_at`), resolved by the receiver
type. A method also calls the free helper `is_space_byte`; the stdlib
expansion renames each declared free function to `std.string.<name>` and
rewrites sibling free-function call sites inside bundled bodies to the
same namespaced name, so the helper resolves from within a method body.
This is implemented in `src/resolve/stdlib.rs`.

## Out of scope

* Unicode-aware case mapping (locale rules, `ß` to `SS`, Turkish dotless
  `i`, ...). Case mapping is ASCII only.
* Unicode normalization (NFC/NFD/NFKC/NFKD).
* Grapheme-cluster and code-point indexing. All indices are byte
  offsets.
* Splitting, joining a list, and regular expressions. These wait on the
  collection surface and a regex module.
* A mutable string builder in the runtime. The transforms use repeated
  concat instead; a builder is a later optimization.
