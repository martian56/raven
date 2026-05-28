# std/string Spec

## Goal

Provide a useful string-utility surface for Raven v2 programs: case
mapping, length and access, search, transforms, and validation. The
module is bundled Raven source compiled into the program the same way
`std/io` is (see `stdlib.md`). It is written in pure Raven on top of a
small set of byte-level compiler intrinsics, so most of the logic
dogfoods the language itself.

## Import

`std/string` uses the selective-import form the resolver supports:

```raven
import std/string { to_upper, trim, contains, repeat }
```

Each selector binds the bare name to the namespaced function
(`std.string.to_upper`, ...) the compiler merges into the program. The
aliased `import std/string as s` form with `s.member(...)` access is not
supported, the same limitation `std/io` documents.

## Byte versus codepoint semantics

Every function in this module is byte oriented. Indices, lengths, and
slices count UTF-8 bytes, not Unicode code points or grapheme clusters.

* `length(s)` returns the byte count. A string of multi-byte characters
  reports a length larger than its visible character count: `length("é")`
  is 2 because `é` is the two bytes `0xC3 0xA9` in UTF-8.
* `char_at(s, i)` and `substring(s, start, end)` cut on byte boundaries.
  Cutting through the middle of a multi-byte character yields a string
  whose bytes are not valid UTF-8; that is the caller's responsibility.
* `index_of`, `contains`, `starts_with`, `ends_with`, and `replace`
  compare whole byte sequences, so they work correctly on multi-byte
  text as long as the needle and haystack are themselves valid UTF-8
  (the common case). `contains("café", "fé")` is true because the byte
  sequence of `fé` occurs in `café`.
* `to_upper` and `to_lower` map only ASCII letters (`A`..`Z` <->
  `a`..`z`); every other byte, including the bytes of a multi-byte
  character, passes through unchanged.
* `trim` and `is_blank` treat the ASCII whitespace set (space, tab,
  newline, carriage return, vertical tab, form feed) as whitespace.

This byte model keeps v2.0 small and predictable. Code-point and
grapheme-aware operations are deferred (see Out of scope).

## The std/string surface

`stdlib/std/string.rv` exports:

* `length(s: String) -> Int`: byte length of `s`.
* `is_empty(s: String) -> Bool`: true when `s` has zero bytes.
* `char_at(s: String, i: Int) -> String`: the `i`-th byte as a one-byte
  string; out of range yields the empty string.
* `substring(s: String, start: Int, end: Int) -> String`: the half-open
  byte range `[start, end)`, with bounds clamped to `0..length(s)` and a
  `start` past `end` yielding the empty string.
* `concat(a: String, b: String) -> String`: join two strings.
* `to_upper(s: String) -> String`, `to_lower(s: String) -> String`:
  ASCII case mapping.
* `trim(s: String) -> String`: remove leading and trailing ASCII
  whitespace; interior whitespace is kept.
* `is_blank(s: String) -> Bool`: true when `s` is empty or all ASCII
  whitespace.
* `repeat(s: String, n: Int) -> String`: `s` repeated `n` times; a non
  positive `n` yields the empty string.
* `index_of(s: String, needle: String) -> Int`: byte index of the first
  occurrence of `needle`, or `-1` when absent; an empty `needle` returns
  `0`.
* `contains(s: String, needle: String) -> Bool`: true when `needle`
  occurs anywhere in `s`.
* `starts_with(s: String, prefix: String) -> Bool`,
  `ends_with(s: String, suffix: String) -> Bool`: prefix and suffix
  tests; an empty prefix or suffix always matches.
* `replace(s: String, from: String, to: String) -> String`: replace
  every non-overlapping occurrence of `from` with `to`, scanning left to
  right; an empty `from` returns `s` unchanged so the scan terminates.

Two helpers, `is_space_byte` and `matches_at`, are also exported because
the bundled module calls them internally; they are documented but not
part of the intended public surface.

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

Unlike `std/io`, the `std/string` functions call one another (for
example `trim` calls `is_space_byte`, and `index_of` calls
`matches_at`). The stdlib expansion renames each declared function to
`std.string.<name>` and, so a sibling call still resolves, rewrites any
call-site identifier inside a bundled function body that names a sibling
function to the same namespaced name. Local variables and parameters in
the bundled sources never share a name with a sibling function, so the
rewrite is unambiguous. This is implemented in `src/resolve/stdlib.rs`
and is general: any later bundled module whose functions call each other
benefits without extra work.

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
