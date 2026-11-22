# std.env

Access to the process environment: environment variables, command-line
arguments, process exit, and platform info. The primitives bind the
raven-runtime C ABI; the convenience functions are pure Raven on top of them.

```rust
from std.env import get_env_or, os_name

fun main() {
    let level = get_env_or("LOG_LEVEL", "info")
    print(level) // info, unless LOG_LEVEL is set
    print(os_name()) // windows, linux, or macos
}
```

## Importing

```rust
from std.env import get_env, has_env, get_env_or, args, arg_count, arg_at, exit, os_name, arch
```

Pull in just the functions you use, or list all of them as above.

## Environment variables

### `get_env(name: str) -> str`

The value of the environment variable `name`, or `""` when it is unset.
A variable set to the empty string and a variable that is not set both
yield `""`, so `get_env` alone cannot tell them apart: use `has_env` to
distinguish unset from empty.

```rust
from std.env import get_env

fun main() {
    print(get_env("HOME")) // the value, or "" when unset
}
```

### `has_env(name: str) -> bool`

True when `name` is set in the environment, regardless of its value
(including the empty string).

### `get_env_or(name: str, default: str) -> str`

The value of `name`, or `default` when `name` is unset. This is the usual
way to read configuration with a fallback.

```rust
from std.env import get_env_or

fun main() {
    let port = get_env_or("PORT", "8080")
    print(port) // 8080, unless PORT is set
}
```

### `set_env(name: str, value: str) -> bool`

Sets a variable for this program and every program it starts afterwards
with `std.process`. Returns `false` when `name` is empty or contains `=`.

```rust
from std.env import set_env, get_env

fun main() {
    let _ = set_env("GIT_TERMINAL_PROMPT", "0")
    print(get_env("GIT_TERMINAL_PROMPT"))
}
```

## Working directory

### `current_dir() -> str`

The directory the program runs in, with forward slashes on every platform,
or `""` when it cannot be read.

## Command-line arguments

### `arg_count() -> int`

The number of process arguments, always at least 1. Index 0 is the program
path; user-supplied arguments start at index 1.

### `arg_at(i: int) -> str`

The argument at index `i` (index 0 is the program path), or `""` when `i`
is out of range.

### `args() -> list[str]`

All process arguments as a list, with index 0 being the program path.
This is pure Raven: it loops `arg_at` over `0..arg_count()`.

```rust
from std.env import args

fun main() {
    let all = args()
    var i = 1
    while i < all.len() {
        print(all[i]) // each user-supplied argument
        i = i + 1
    }
}
```

## Process exit

### `exit(code: int)`

Terminate the process with status `code`. It does not return; any code
after a call to `exit` is unreachable.

```rust
from std.env import arg_count, exit

fun main() {
    if arg_count() < 2 {
        print("usage: tool <name>")
        exit(1)
    }
    print("ok")
}
```

## Platform info

### `os_name() -> str`

The target operating system: `"windows"`, `"linux"`, or `"macos"`.

### `arch() -> str`

The target CPU architecture, for example `"x86_64"` or `"aarch64"`.

```rust
from std.env import os_name, arch

fun main() {
    print(os_name()) // linux
    print(arch()) // x86_64
}
```

## Worked example: a small greeting tool

```rust
from std.env import args, get_env_or, os_name, exit

fun main() {
    let argv = args()
    if argv.len() < 2 {
        print("usage: greet <name>")
        exit(1)
    }
    let name = argv[1]
    let greeting = get_env_or("GREETING", "Hello")
    print("${greeting}, ${name}! Running on ${os_name()}.")
}
```

## See also

- [std.io](io.md) for reading from standard input and writing to standard
  output and error.
- [std.fs](fs.md) for working with files and paths.
