# std/fs

Filesystem access: read and write files, create and remove directories,
list and size entries, query existence, and manipulate path strings. The
fallible operations return `Result<T, Error>`; the queries return a plain
`Bool`; the path helpers are pure string work with no runtime call.

```rust
import std/fs { write, read }

fun main() {
    match write("greeting.txt", "hello") {
        Ok(_) -> print("wrote it"),
        Err(e) -> print(e.message()),
    }
}
```

## Importing

```rust
import std/fs { read, write, append, remove_file, create_dir, remove_dir, list_dir, size, exists, is_file, is_dir, join, dirname, basename, split }
```

`std/fs` is a set of free functions, so use a selective import and list the
names you want. A bare `import std/fs` does not bring the functions into
scope. Import only what a given file uses:

```rust
import std/fs { read, exists }
```

## The error model

Fallible operations return `Result<T, Error>`. On failure the `Error` is an
std/error value tagged with kind `"io"`. Its message is a short context
prefix (the operation name) joined to the operating system error text, for
example `read: No such file or directory`.

There are no sentinel return values: a missing file does not come back as an
empty string or a `-1`, it comes back as an `Err`. The two ways to consume a
`Result` are a `match` and the `?` operator.

Handle each arm explicitly with `match`:

```rust
import std/fs { read }

fun main() {
    match read("config.txt") {
        Ok(text) -> print(text),
        Err(e) -> print(e.message()),
    }
}
```

Or propagate the error to the caller with `?`, which unwraps `Ok` and
returns early on `Err`:

```rust
import std/fs { read, write }

fun copy(src: String, dst: String) -> Result<Bool, Error> {
    let text = read(src)?
    return write(dst, text)
}
```

## Reading and writing

### `read(path: String) -> Result<String, Error>`

Read the whole file at `path` and return it as a single `String`.

```rust
import std/fs { read }

fun main() {
    match read("notes.txt") {
        Ok(text) -> print(text),
        Err(e) -> print(e.message()),
    }
}
```

### `write(path: String, contents: String) -> Result<Bool, Error>`

Create or truncate `path`, then write `contents`. Returns `Ok(true)` on
success. The `Bool` payload is always `true`; it exists only so the success
arm carries a value (the surface uses `Result<Bool, Error>` rather than a
unit payload).

```rust
import std/fs { write }

fun main() {
    match write("out.txt", "first line\n") {
        Ok(_) -> print("ok"),
        Err(e) -> print(e.message()),
    }
}
```

### `append(path: String, contents: String) -> Result<Bool, Error>`

Write `contents` to the end of `path`, creating the file when it is absent.
Returns `Ok(true)` on success.

```rust
import std/fs { append }

fun log_line(line: String) -> Result<Bool, Error> {
    return append("app.log", line.concat("\n"))
}
```

## Directory and file operations

### `remove_file(path: String) -> Result<Bool, Error>`

Remove the file at `path`. Returns `Ok(true)` on success.

### `create_dir(path: String) -> Result<Bool, Error>`

Create the directory at `path`, including any missing parent directories, so
creating a nested path in one call succeeds. Returns `Ok(true)` on success.

```rust
import std/fs { create_dir }

fun main() {
    match create_dir("build/cache/tmp") {
        Ok(_) -> print("created"),
        Err(e) -> print(e.message()),
    }
}
```

### `remove_dir(path: String) -> Result<Bool, Error>`

Remove the directory at `path` and all of its contents recursively. Returns
`Ok(true)` on success.

### `list_dir(path: String) -> Result<List<String>, Error>`

Return the entry names (not full paths) of the directory at `path`. An empty
directory yields an empty list, not a one-element list containing `""`.

```rust
import std/fs { list_dir }

fun main() {
    match list_dir(".") {
        Ok(names) -> {
            for name in names {
                print(name)
            }
        }
        Err(e) -> print(e.message()),
    }
}
```

### `size(path: String) -> Result<Int, Error>`

Return the file size at `path` in bytes.

```rust
import std/fs { size }

fun main() {
    match size("out.txt") {
        Ok(n) -> print(n),
        Err(e) -> print("could not stat the file"),
    }
}
```

The `?` operator is the shorter form, but it only works inside a function that
itself returns `Result`, not in `main`:

```rust
import std/fs { size }

fun total(a: String, b: String) -> Result<Int, Error> {
    return Ok(size(a)? + size(b)?)
}
```

## Boolean checks

```rust
fun exists(path: String) -> Bool
fun is_file(path: String) -> Bool
fun is_dir(path: String) -> Bool
```

These return a plain `Bool`, never a `Result`. A missing path is a normal
`false`, not an `Err`. `is_file` is `false` for a path that exists but is not
a regular file (a directory, say), and `is_dir` is `false` for anything that
is not a directory.

```rust
import std/fs { exists, is_dir, read }

fun main() {
    if exists("config.txt") {
        match read("config.txt") {
            Ok(text) -> print(text),
            Err(e) -> print(e.message()),
        }
    }
    print(is_dir("build"))
}
```

## Path helpers

```rust
fun join(a: String, b: String) -> String
fun basename(p: String) -> String
fun dirname(p: String) -> String
fun split(p: String) -> List<String>
```

These are pure string operations with no runtime call and no `Result`. They
use `/` as the separator. Forward slash works on Windows for these std ops,
so the operating system separator is not detected. They overlap with
[std/path](../../specs/std-path.md); std/fs duplicates them so the module is
usable without a second import.

### `join(a: String, b: String) -> String`

Join two path segments with a single `/`, collapsing a trailing slash on `a`
or a leading slash on `b` so the result never doubles the separator. An empty
segment returns the other unchanged.

```rust
import std/fs { join }

fun main() {
    print(join("build", "out.txt"))     // build/out.txt
    print(join("build/", "/out.txt"))   // build/out.txt
}
```

### `basename(p: String) -> String`

The final component of `p` (everything after the last `/`). A path with no
slash returns unchanged.

```rust
import std/fs { basename }

fun main() {
    print(basename("a/b/c.txt"))    // c.txt
    print(basename("c.txt"))        // c.txt
}
```

### `dirname(p: String) -> String`

Everything before the last `/`. A path with no slash returns `"."`; a path
whose only slash is at the front returns `"/"`.

```rust
import std/fs { dirname }

fun main() {
    print(dirname("a/b/c.txt"))     // a/b
    print(dirname("c.txt"))         // .
}
```

### `split(p: String) -> List<String>`

Split `p` on `/` into its components, dropping empty pieces produced by
leading, trailing, or repeated separators.

```rust
import std/fs { split }

fun main() {
    let parts = split("/a//b/c/")   // ["a", "b", "c"]
    for part in parts {
        print(part)
    }
}
```

## Worked example: write then read a file

Write a file, confirm it exists, then read it back. Each fallible step uses
`?` to bail out on the first error, and the caller decides what to do with it.

```rust
import std/fs { write, read, exists }
import std/error { error_kind }

fun round_trip() -> Result<String, Error> {
    write("scratch.txt", "saved value")?
    if !exists("scratch.txt") {
        return Err(error_kind("io", "file vanished after write"))
    }
    return read("scratch.txt")
}

fun main() {
    match round_trip() {
        Ok(text) -> print(text),       // saved value
        Err(e) -> print(e.message()),
    }
}
```

## See also

- [std/path](../../specs/std-path.md) for the same `join`, `dirname`, and
  `basename` helpers when you do not also need filesystem access.
- [std/error](../../specs/std-error.md) for `Error`, `error_kind`, and reading
  the `kind` and `message` of an `Err`.
- [std/io](io.md) for reading from and writing to the console instead of files.
