# std/path

Pure path string manipulation on the POSIX `/` separator. Every function
here is plain string work. Nothing in this module touches the disk: it
never checks whether a file exists, never reads a directory, never
resolves a real path. For anything that talks to the filesystem, reach for
[std/fs](fs.md) instead.

```rust
import std/path { join, basename, dirname, extension, stem, is_absolute }

fun main() {
    let p = join("src", "main.rv")      // src/main.rv
    print(dirname(p))                   // src
    print(basename(p))                  // main.rv
    print(extension(p))                 // rv
}
```

## Importing

The functions have no natural single receiver, so they are free functions
brought in by a selective import:

```rust
import std/path { join, basename, dirname, extension, stem, is_absolute }
```

Pull in only the names you use. The module is built on `std/string`'s
`String` methods, and the import merges that dependency transitively, so
you do not need a separate `import std/string`.

## The path model

A path is a `String` whose components are separated by `/`. That is the
only separator this module understands. Windows backslash (`\`) paths,
drive letters, and UNC paths are out of scope: a `\` is treated as an
ordinary path byte, not a separator.

Indices are byte offsets, consistent with `std/string`. Paths are assumed
to be valid UTF-8 with the separator and the dot appearing only as their
own single-byte ASCII forms.

## Surface

| Function | Result | Summary |
|---|---|---|
| `join(a: String, b: String) -> String` | `String` | join two parts with a single `/` |
| `basename(p: String) -> String` | `String` | final component after the last `/` |
| `dirname(p: String) -> String` | `String` | everything up to the last `/` |
| `extension(p: String) -> String` | `String` | text after the last `.` in the basename |
| `stem(p: String) -> String` | `String` | basename without its extension |
| `is_absolute(p: String) -> Bool` | `Bool` | whether `p` starts with `/` |

## Building a path

### `join(a: String, b: String) -> String`

Join two path parts with a single `/`. The result never doubles the
separator: it stays one `/` whether `a` ends with a slash, `b` begins with
one, or both. If either operand is empty, the other is returned unchanged.

```rust
import std/path { join }

fun main() {
    print(join("src", "main.rv"))       // src/main.rv
    print(join("src/", "main.rv"))      // src/main.rv
    print(join("src", "/main.rv"))      // src/main.rv
    print(join("src/", "/main.rv"))     // src/main.rv
    print(join("", "main.rv"))          // main.rv
    print(join("src", ""))              // src
}
```

## Decomposing a path

### `basename(p: String) -> String`

The final component, everything after the last `/`. When `p` contains no
`/`, the whole string is the basename.

```rust
import std/path { basename }

fun main() {
    print(basename("src/lib/main.rv"))  // main.rv
    print(basename("main.rv"))          // main.rv (no slash)
    print(basename("/etc/hosts"))       // hosts
}
```

### `dirname(p: String) -> String`

Everything up to the last `/`. Two edge cases are worth remembering:

- When there is no `/`, `dirname` returns `"."` (the current directory).
- When the only `/` is at index 0, it returns `"/"` (the root).

```rust
import std/path { dirname }

fun main() {
    print(dirname("src/lib/main.rv"))   // src/lib
    print(dirname("main.rv"))           // . (no slash)
    print(dirname("/hosts"))            // / (slash only at index 0)
}
```

### `extension(p: String) -> String`

The substring after the last `.` in the basename, or `""` when there is no
dot. The dot is inspected on the basename only, so a `.` in a parent
directory name does not count. A leading dot marks a dotfile, not an
extension: the extension of `.gitignore` is `""`.

```rust
import std/path { extension }

fun main() {
    print(extension("archive.tar.gz"))  // gz (last dot wins)
    print(extension("README"))          // "" (no dot)
    print(extension(".gitignore"))      // "" (leading dot, a dotfile)
    print(extension("a.b/main"))        // "" (dot is in the directory part)
}
```

### `stem(p: String) -> String`

The basename with its extension removed: the part `extension` leaves
behind. For a name without an extension, the whole basename is the stem,
and a dotfile keeps its leading dot.

```rust
import std/path { stem }

fun main() {
    print(stem("src/main.rv"))          // main
    print(stem("archive.tar.gz"))       // archive.tar (only the last dot)
    print(stem("README"))               // README (no extension)
    print(stem(".gitignore"))           // .gitignore (dotfile, no extension)
}
```

### `is_absolute(p: String) -> Bool`

True when `p` starts with `/`. The empty string is relative.

```rust
import std/path { is_absolute }

fun main() {
    print(is_absolute("/usr/bin"))      // true
    print(is_absolute("usr/bin"))       // false
    print(is_absolute(""))              // false
}
```

## Worked example: rename a path's extension

This walks a path apart, swaps its extension, and joins it back together
using only documented functions.

```rust
import std/path { join, dirname, stem, is_absolute }

fun with_extension(p: String, ext: String) -> String {
    let dir = dirname(p)
    let name = stem(p).concat(".").concat(ext)
    if dir == "." {
        return name
    }
    return join(dir, name)
}

fun main() {
    print(with_extension("src/report.txt", "md"))   // src/report.md
    print(with_extension("notes.txt", "md"))         // notes.md
    print(is_absolute("/tmp/a"))                      // true
}
```

## Relationship to std/fs

`std/path` and [std/fs](fs.md) split cleanly along one line: `std/path` is
strings, `std/fs` is the disk.

- `std/path` answers questions about the shape of a path string
  (`dirname`, `basename`, `extension`) and builds new ones (`join`). It
  never opens, reads, or stats anything, so its results hold even for
  paths that do not exist.
- [std/fs](fs.md) is where existence checks, reading, writing, and
  directory listing live. When you need to know whether a path is really
  on disk, use `std/fs`.

A common pattern is to compute a target path with `std/path`, then hand
that string to `std/fs` to actually read or write it.

## Not yet covered

A `normalize` function (resolving `.` and `..` segments and collapsing
repeated separators) is deferred and not part of this module today. The
functions above do no such resolution: they operate on the literal bytes
of the path you pass in.

## See also

- [std/fs](fs.md) for filesystem access (existence, reading, writing).
- [std/string](string.md) for the `String` methods this module builds on.
