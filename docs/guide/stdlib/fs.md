# std.fs

Filesystem access: read and write files, create and remove directories,
list and size entries, query existence, and manipulate path strings. The
fallible operations return `Result[T, Error]`; the queries return a plain
`bool`; the path helpers are pure string work with no runtime call.

```rust
from std.fs import write, read

fun main() {
    match write("greeting.txt", "hello") {
        case Ok(_) { print("wrote it") }
        case Err(e) { print(e.message()) }
    }
}
```

## Importing

```rust
from std.fs import read, write, append, read_lines, copy, remove_file, create_dir, create_dir_all, remove_dir, list_dir, walk, size, exists, is_file, is_dir, join, dirname, basename, split
```

`std.fs` is a set of free functions, so use a selective import and list the
names you want. A bare `import std.fs` does not bring the functions into
scope. Import only what a given file uses:

```rust
from std.fs import read, exists
```

## The error model

Fallible operations return `Result[T, Error]`. On failure the `Error` is an
std.error value tagged with kind `"io"`. Its message names the operation
and the path, then gives the operating system error text, for example
`read notes.txt: No such file or directory`.

There are no sentinel return values: a missing file does not come back as an
empty string or a `-1`, it comes back as an `Err`. The two ways to consume a
`Result` are a `match` and the `?` operator.

Handle each arm explicitly with `match`:

```rust
from std.fs import read

fun main() {
    match read("config.txt") {
        case Ok(text) { print(text) }
        case Err(e) { print(e.message()) }
    }
}
```

Or propagate the error to the caller with `?`, which unwraps `Ok` and
returns early on `Err`:

```rust
from std.fs import read, write

fun copy(src: str, dst: str) -> Result[bool, Error] {
    let text = read(src)?
    return write(dst, text)
}
```

## Reading and writing

### `read(path: str) -> Result[str, Error]`

Read the whole file at `path` and return it as a single `str`.

```rust
from std.fs import read

fun main() {
    match read("notes.txt") {
        case Ok(text) { print(text) }
        case Err(e) { print(e.message()) }
    }
}
```

### `write(path: str, contents: str) -> Result[bool, Error]`

Create or truncate `path`, then write `contents`. Returns `Ok(true)` on
success. The `bool` payload is always `true`; it exists only so the success
arm carries a value (the surface uses `Result[bool, Error]` rather than a
unit payload).

```rust
from std.fs import write

fun main() {
    match write("out.txt", "first line\n") {
        case Ok(_) { print("ok") }
        case Err(e) { print(e.message()) }
    }
}
```

### `append(path: str, contents: str) -> Result[bool, Error]`

Write `contents` to the end of `path`, creating the file when it is absent.
Returns `Ok(true)` on success.

```rust
from std.fs import append

fun log_line(line: str) -> Result[bool, Error] {
    return append("app.log", line.concat("\n"))
}
```

### `read_lines(path: str) -> Result[list[str], Error]`

Read the file at `path` and split it into lines, handling both `\n` and
`\r\n` line endings. The trailing newline does not produce an empty final
entry.

```rust
from std.fs import read_lines

fun main() {
    match read_lines("config.txt") {
        case Ok(lines) {
            for line in lines {
                print(line)
            }
        }
        case Err(e) { print(e.message()) }
    }
}
```

### `copy(src: str, dst: str) -> Result[bool, Error]`

Copy the contents of `src` to `dst`, creating or truncating `dst`. Returns
`Ok(true)` on success.

```rust
from std.fs import copy

fun main() {
    match copy("notes.txt", "notes.bak") {
        case Ok(_) { print("copied") }
        case Err(e) { print(e.message()) }
    }
}
```

## Directory and file operations

### `remove_file(path: str) -> Result[bool, Error]`

Remove the file at `path`. Returns `Ok(true)` on success.

### `create_dir(path: str) -> Result[bool, Error]`

Create the directory at `path`, including any missing parent directories, so
creating a nested path in one call succeeds. Returns `Ok(true)` on success.

```rust
from std.fs import create_dir

fun main() {
    match create_dir("build/cache/tmp") {
        case Ok(_) { print("created") }
        case Err(e) { print(e.message()) }
    }
}
```

### `create_dir_all(path: str) -> Result[bool, Error]`

Create the directory at `path` along with any missing parents, like
`mkdir -p`. An already existing directory is not an error. Returns
`Ok(true)` on success.

```rust
from std.fs import create_dir_all

fun main() {
    match create_dir_all("build/cache/tmp") {
        case Ok(_) { print("created") }
        case Err(e) { print(e.message()) }
    }
}
```

### `remove_dir(path: str) -> Result[bool, Error]`

Remove the directory at `path` and all of its contents recursively. Returns
`Ok(true)` on success.

### `list_dir(path: str) -> Result[list[str], Error]`

Return the entry names (not full paths) of the directory at `path`. An empty
directory yields an empty list, not a one-element list containing `""`.

```rust
from std.fs import list_dir

fun main() {
    match list_dir(".") {
        case Ok(names) {
            for name in names {
                print(name)
            }
        }
        case Err(e) { print(e.message()) }
    }
}
```

### `walk(path: str) -> Result[list[str], Error]`

Recursively list every path under `path`, depth first. Each entry is a full
path (the directory prefix joined to the name), and subdirectories appear
before their contents.

```rust
from std.fs import walk

fun main() {
    match walk("src") {
        case Ok(entries) {
            for entry in entries {
                print(entry)
            }
        }
        case Err(e) { print(e.message()) }
    }
}
```

### `size(path: str) -> Result[int, Error]`

Return the file size at `path` in bytes.

```rust
from std.fs import size

fun main() {
    match size("out.txt") {
        case Ok(n) { print(n) }
        case Err(e) { print("could not stat the file") }
    }
}
```

The `?` operator is the shorter form, but it only works inside a function that
itself returns `Result`, not in `main`:

```rust
from std.fs import size

fun total(a: str, b: str) -> Result[int, Error] {
    return Ok(size(a)? + size(b)?)
}
```

### `modified(path: str) -> int`

When `path` was last changed, in milliseconds since 1970-01-01 UTC, or `-1`
when it does not exist or cannot be read. Compare two readings to notice that
another program changed a file.

```rust
from std.fs import modified

fun main() {
    let stamp = modified("config.toml")
    print(stamp > 0)
}
```

## Boolean checks

```rust
fun exists(path: str) -> bool
fun is_file(path: str) -> bool
fun is_dir(path: str) -> bool
fun is_symlink(path: str) -> bool
```

These return a plain `bool`, never a `Result`. A missing path is a normal
`false`, not an `Err`. `is_file` is `false` for a path that exists but is not
a regular file (a directory, say), and `is_dir` is `false` for anything that
is not a directory. `is_symlink` reports whether the path itself is a symbolic
link, without following it, so it stays `true` for a link whose target is
missing.

```rust
from std.fs import exists, is_dir, read

fun main() {
    if exists("config.txt") {
        match read("config.txt") {
            case Ok(text) { print(text) }
            case Err(e) { print(e.message()) }
        }
    }
    print(is_dir("build"))
}
```

## Path helpers

```rust
fun join(a: str, b: str) -> str
fun basename(p: str) -> str
fun dirname(p: str) -> str
fun split(p: str) -> list[str]
```

These are pure string operations with no runtime call and no `Result`. They
use `/` as the separator. Forward slash works on Windows for these std ops,
so the operating system separator is not detected. They overlap with
[std.path](../../specs/std-path.md); std.fs duplicates them so the module is
usable without a second import.

### `join(a: str, b: str) -> str`

Join two path segments with a single `/`, collapsing a trailing slash on `a`
or a leading slash on `b` so the result never doubles the separator. An empty
segment returns the other unchanged.

```rust
from std.fs import join

fun main() {
    print(join("build", "out.txt")) // build/out.txt
    print(join("build/", "/out.txt")) // build/out.txt
}
```

### `basename(p: str) -> str`

The final component of `p` (everything after the last `/`). A path with no
slash returns unchanged.

```rust
from std.fs import basename

fun main() {
    print(basename("a/b/c.txt")) // c.txt
    print(basename("c.txt")) // c.txt
}
```

### `dirname(p: str) -> str`

Everything before the last `/`. A path with no slash returns `"."`; a path
whose only slash is at the front returns `"/"`.

```rust
from std.fs import dirname

fun main() {
    print(dirname("a/b/c.txt")) // a/b
    print(dirname("c.txt")) // .
}
```

### `split(p: str) -> list[str]`

Split `p` on `/` into its components, dropping empty pieces produced by
leading, trailing, or repeated separators.

```rust
from std.fs import split

fun main() {
    let parts = split("/a//b/c/") // ["a", "b", "c"]
    for part in parts {
        print(part)
    }
}
```

## Renaming and temporary files

- `rename(from, to)` moves a file or directory, replacing `to` if it is a file.
- `write_atomic(path, contents)` writes a temporary file beside `path` and
  renames it into place, so a reader sees the old file or the new one, never
  half of it.
- `temp_dir()` is the system's temporary directory, and
  `make_temp_dir(prefix)` creates a new, uniquely named directory in it.
  Remove it with `remove_dir` when done.

## Reading and writing a piece at a time

`read` and `write` move whole files. For a large file, or a log you keep
appending to, open a `File`: `open(path)` to read, `create(path)` to write
(truncating), or `open_append(path)`. A `File` has `read(n)` (up to `n` bytes,
`""` at the end), `read_line()` (`Some(line)` without its line ending, or
`None` at the end), `write(data)`, `flush()`, `seek(pos)`, and `close()`.
Writes are buffered until `flush` or `close`, and reading a file opened for
writing (or the reverse) is an error.

```rust
import std.fs

fun main() {
    let dir = fs.make_temp_dir("demo-").unwrap()
    let path = fs.join(dir, "events.log")
    let out = fs.create(path).unwrap()
    out.write("started\nstopped\n").unwrap()
    out.close().unwrap()
    let f = fs.open(path).unwrap()
    while true {
        match f.read_line().unwrap() {
            case Some(line) { print(line) }
            case None { break }
        }
    }
    f.close().unwrap()
    fs.remove_dir(dir).unwrap()
}
```

## Worked example: write then read a file

Write a file, confirm it exists, then read it back. Each fallible step uses
`?` to bail out on the first error, and the caller decides what to do with it.

```rust
from std.fs import write, read, exists
from std.error import error_kind

fun round_trip() -> Result[str, Error] {
    write("scratch.txt", "saved value")?
    if not exists("scratch.txt") {
        return Err(error_kind("io", "file vanished after write"))
    }
    return read("scratch.txt")
}

fun main() {
    match round_trip() {
        case Ok(text) { print(text) } // saved value
        case Err(e) { print(e.message()) }
    }
}
```

## See also

- [std.path](../../specs/std-path.md) for the same `join`, `dirname`, and
  `basename` helpers when you do not also need filesystem access.
- [std.error](../../specs/std-error.md) for `Error`, `error_kind`, and reading
  the `kind` and `message` of an `Err`.
- [std.io](io.md) for reading from and writing to the console instead of files.
