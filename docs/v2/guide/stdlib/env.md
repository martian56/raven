# std/env

Access to the process environment: environment variables, command-line
arguments, process exit, and platform info. The primitives bind the
raven-runtime C ABI; the convenience functions are pure Raven on top of them.

```rust
import std/env { get_env_or, os_name }

fun main() {
    let level = get_env_or("LOG_LEVEL", "info")
    print(level)            // info, unless LOG_LEVEL is set
    print(os_name())        // windows, linux, or macos
}
```

## Importing

```rust
import std/env { get_env, has_env, get_env_or, args, arg_count, arg_at, exit, os_name, arch }
```

Pull in just the functions you use, or list all of them as above.

## Environment variables

### `get_env(name: String) -> String`

The value of the environment variable `name`, or `""` when it is unset.
A variable set to the empty string and a variable that is not set both
yield `""`, so `get_env` alone cannot tell them apart: use `has_env` to
distinguish unset from empty.

```rust
import std/env { get_env }

fun main() {
    print(get_env("HOME"))      // the value, or "" when unset
}
```

### `has_env(name: String) -> Bool`

True when `name` is set in the environment, regardless of its value
(including the empty string).

### `get_env_or(name: String, default: String) -> String`

The value of `name`, or `default` when `name` is unset. This is the usual
way to read configuration with a fallback.

```rust
import std/env { get_env_or }

fun main() {
    let port = get_env_or("PORT", "8080")
    print(port)         // 8080, unless PORT is set
}
```

## Command-line arguments

### `arg_count() -> Int`

The number of process arguments, always at least 1. Index 0 is the program
path; user-supplied arguments start at index 1.

### `arg_at(i: Int) -> String`

The argument at index `i` (index 0 is the program path), or `""` when `i`
is out of range.

### `args() -> List<String>`

All process arguments as a list, with index 0 being the program path.
This is pure Raven: it loops `arg_at` over `0..arg_count()`.

```rust
import std/env { args }

fun main() {
    let all = args()
    let i = 1
    while i < all.len() {
        print(all[i])       // each user-supplied argument
        i = i + 1
    }
}
```

## Process exit

### `exit(code: Int)`

Terminate the process with status `code`. It does not return; any code
after a call to `exit` is unreachable.

```rust
import std/env { arg_count, exit }

fun main() {
    if arg_count() < 2 {
        print("usage: tool <name>")
        exit(1)
    }
    print("ok")
}
```

## Platform info

### `os_name() -> String`

The target operating system: `"windows"`, `"linux"`, or `"macos"`.

### `arch() -> String`

The target CPU architecture, for example `"x86_64"` or `"aarch64"`.

```rust
import std/env { os_name, arch }

fun main() {
    print(os_name())        // linux
    print(arch())           // x86_64
}
```

## Worked example: a small greeting tool

```rust
import std/env { args, get_env_or, os_name, exit }

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

- [std/io](io.md) for reading from standard input and writing to standard
  output and error.
- [std/fs](fs.md) for working with files and paths.
