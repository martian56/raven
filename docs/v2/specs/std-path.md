# std/path Spec

Path string manipulation on the POSIX `/` separator. Every function is
pure string work: no filesystem access, no allocation beyond the returned
`String`. The functions are built in Raven on `std/string`'s `String`
methods (`length`, `substring`, `concat`, `char_at`, `starts_with`,
`ends_with`); the module `import std/string` and lets stdlib expansion
merge it transitively.

## Model

A path is a `String` whose components are separated by `/`. This is the
only separator the module understands. Windows backslash (`\`) paths and
drive letters are out of scope for v2.0; a `\` is treated as an ordinary
path byte.

## Import

The functions have no natural single receiver, so they are free functions
bound by a selective import:

```rust
import std/path { join, basename, dirname, extension, stem, is_absolute }

fun main() {
    let p = join("a/b", "c.txt")   // a/b/c.txt
    let d = dirname(p)             // a/b
    let f = basename(p)            // c.txt
    let e = extension(p)           // txt
    let s = stem(p)                // c
}
```

## Surface

| Function | Result | Notes |
|---|---|---|
| `join(a, b)` | `String` | join with a single `/`; never doubles when `a` ends with `/` or `b` starts with `/`. An empty operand returns the other unchanged. |
| `basename(p)` | `String` | final component after the last `/`, or the whole string when there is no `/`. |
| `dirname(p)` | `String` | everything up to the last `/`; `"."` when there is no `/`; `"/"` when the only `/` is at index 0. |
| `extension(p)` | `String` | substring after the last `.` in the basename, or `""` when none. A leading dot (a dotfile such as `.gitignore`) is not an extension. |
| `stem(p)` | `String` | basename without its extension. |
| `with_extension(p, ext)` | `String` | replace the extension with `ext` (no leading dot); an empty `ext` removes it. A trailing dot counts as the separator, matching `stem`. |
| `is_absolute(p)` | `Bool` | whether `p` starts with `/`. The empty string is relative. |
| `is_relative(p)` | `Bool` | the negation of `is_absolute`. |
| `components(p)` | `List<String>` | the non-empty `/`-separated segments, in order; leading, trailing, and repeated separators drop out. |
| `normalize(p)` | `String` | resolve `.` and `..` segments and collapse repeated separators, keeping a leading `/` for an absolute path. An empty relative result is `"."`. A `..` that would escape a relative root is kept; one at an absolute root is dropped. |

Indices are byte offsets, consistent with `std/string`. Paths are assumed
to be valid UTF-8 with the separator and dot appearing only as their own
single-byte ASCII forms.

## Out of scope

- Windows `\` separators, drive letters, and UNC paths.
- Filesystem queries (existence, canonicalization); those belong to
  `std/fs`.
