# std/env Spec

Access to the process environment: command-line arguments, environment
variables, process exit, and platform info. The primitives bind the
raven-runtime C ABI; the convenience functions are pure Raven on top of
them.

## Import

```raven
import std/env { get_env, has_env, get_env_or, args, arg_count, arg_at, exit, os_name, arch }
```

## Surface

### Environment variables

```raven
fun get_env(name: String) -> String
fun has_env(name: String) -> Bool
fun get_env_or(name: String, default: String) -> String
```

`get_env` returns the value of `name`, or `""` when the variable is unset.
A variable set to the empty string and a variable that is not set both
yield `""`, so `get_env` alone cannot tell them apart. `has_env` reports
whether the variable is set, regardless of value, which is how a caller
distinguishes unset from empty.

This is why the surface uses `has_env` rather than returning an
`Option<String>`: constructing an enum across the FFI boundary is avoided,
and the empty-or-unset distinction is recovered with a separate boolean
query. `get_env_or` is pure Raven: it returns `default` when `has_env`
is false and `get_env(name)` otherwise.

A value that is not valid UTF-8 is reported as `""`.

### Command-line arguments

```raven
fun arg_count() -> Int
fun arg_at(i: Int) -> String
fun args() -> List<String>
```

`arg_count` is the number of process arguments, always at least 1.
Index 0 is the program path; the user-supplied arguments start at index 1.
`arg_at(i)` returns the argument at `i`, or `""` when `i` is out of range
(or the argument is not valid UTF-8). `args` is pure Raven: it loops
`arg_at` over `0..arg_count()` into a `List<String>`.

### Process exit

```raven
fun exit(code: Int)
```

`exit` terminates the process with status `code`. It does not return;
any code after a call to `exit` is unreachable. The Raven return type is
declared `Unit` because the surface language has no never type, but the
runtime call (`std::process::exit`) never comes back.

### Platform info

```raven
fun os_name() -> String
fun arch() -> String
```

`os_name` returns one of `"windows"`, `"linux"`, or `"macos"` (and
`"unknown"` on any other target). `arch` returns the CPU architecture,
for example `"x86_64"` or `"aarch64"` (and `"unknown"` when unrecognized).
Both are resolved at compile time in the runtime through `cfg!`.

## FFI path

This module uses `extern "C"` blocks binding raven-runtime symbols
directly, not compiler builtin intrinsics. A Raven `String` is a single
GC pointer (`*mut object::String`) at the ABI, so it crosses the boundary
unchanged in both directions, which lets `extern "C"` carry `String`
arguments and returns without any codegen change. The runtime symbols
(`raven_env_get`, `raven_env_has`, `raven_env_arg_count`,
`raven_env_arg_at`, `raven_env_exit`, `raven_env_os_name`,
`raven_env_arch`) live in `raven-runtime/src/lib.rs`.
