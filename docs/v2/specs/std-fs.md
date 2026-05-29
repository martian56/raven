# std/fs Spec

Filesystem operations: reading and writing files, directory creation and
removal, listing, size, and existence queries, plus pure path string
manipulation. The primitives bind the raven-runtime C ABI; the wrappers
add the Result/Error model in pure Raven on top.

## Import

```raven
import std/fs { read, write, append, remove_file, create_dir, remove_dir, list_dir, size, exists, is_file, is_dir, join, dirname, basename, split }
```

## Error model

Fallible operations return `Result<T, Error>`. The error is an std/error
`Error` tagged with kind `"io"`. Raven has no type alias, so the `IoError`
of the issue is realized as `Error` with kind `"io"`; std/fs builds it as
an `Error` struct literal directly. The message is a short context prefix
(the operation name) joined to the operating system error text.

There are no sentinel return values. The runtime keeps a thread-local
last-error string that each fallible op clears on success and sets to the
OS message on failure; `raven_fs_last_error()` returns it, and the Raven
wrapper turns a non-empty last error into an `Err`.

## Fallible surface

```raven
fun read(path: String) -> Result<String, Error>
fun write(path: String, contents: String) -> Result<Bool, Error>
fun append(path: String, contents: String) -> Result<Bool, Error>
fun remove_file(path: String) -> Result<Bool, Error>
fun create_dir(path: String) -> Result<Bool, Error>
fun remove_dir(path: String) -> Result<Bool, Error>
fun list_dir(path: String) -> Result<List<String>, Error>
fun size(path: String) -> Result<Int, Error>
```

`read` returns the whole file as a String. `write` creates or truncates
`path` then writes `contents`. `append` writes to the end of `path`,
creating it when absent.

The ops with no natural payload (`write`, `append`, `remove_file`,
`create_dir`, `remove_dir`) return `Result<Bool, Error>` with `Ok(true)`
on success rather than `Result<Unit, Error>`. The Bool is always `true`;
it exists only so the success arm carries a value.

`create_dir` creates intermediate directories (it uses create_dir_all), so
creating a nested path in one call succeeds. `remove_dir` removes the
directory and all of its contents recursively.

`list_dir` returns the entry names (not full paths). Across the FFI
boundary the runtime joins the names with `\n` into a single String, and
the Raven wrapper splits it into a `List<String>`. An empty directory
yields an empty list, not a one-element list containing `""`.

`size` is the file size in bytes.

## Non-fallible queries

```raven
fun exists(path: String) -> Bool
fun is_file(path: String) -> Bool
fun is_dir(path: String) -> Bool
```

These return a plain `Bool` and never an error: a missing path is a normal
`false`, not an `Err`. A path that exists but is not a regular file is
`false` from `is_file`, and likewise for `is_dir`.

## Path manipulation

```raven
fun join(a: String, b: String) -> String
fun dirname(path: String) -> String
fun basename(path: String) -> String
fun split(path: String) -> List<String>
```

These are pure Raven string operations with no runtime call and no Result.
They use `/` as the separator. Forward slash works on Windows for these
std ops, so the OS separator is not detected. `split` drops empty
components from leading, trailing, or repeated separators. This overlaps
with std/path (which offers the same join/dirname/basename on `/`); std/fs
duplicates them so the module is usable without a second import.

## FFI path

This module uses `extern "C"` blocks binding raven-runtime symbols
directly, not compiler builtin intrinsics. A Raven `String` is a single GC
pointer at the ABI, so it crosses the boundary unchanged in both
directions; `Bool` maps to Rust `bool` and `Int` to `i64`. The runtime
symbols (`raven_fs_read`, `raven_fs_write`, `raven_fs_append`,
`raven_fs_remove_file`, `raven_fs_create_dir`, `raven_fs_remove_dir`,
`raven_fs_list`, `raven_fs_size`, `raven_fs_exists`, `raven_fs_is_file`,
`raven_fs_is_dir`, `raven_fs_last_error`) live in
`raven-runtime/src/lib.rs`.
