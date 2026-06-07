# Standard library

Raven ships a batteries-included standard library. Every module is bundled
into the compiler, so there is nothing to install: just `import` what you
need. This page explains how imports work and the conventions shared across
modules, then links to a reference page for each one.

## How modules are imported

A module is imported by name. There are two import styles, and which one you
use depends on what the module provides.

**Selective import** brings named free functions into scope:

```rust
import std/math { sqrt, pow_int }

fun main() {
    print(sqrt(16.0))       // 4
}
```

**Bare import** brings in a module that adds methods or constructors to a
type (it merges an `impl` block or registers a type):

```rust
import std/string                  // adds methods to String
import std/collections             // registers Map.new() / Set.new()

fun main() {
    print("hi".to_upper())         // HI
    let m: Map<String, Int> = Map.new()
    m.set("a", 1)
}
```

Two rules are worth memorizing up front:

- Import `std/string` and `std/collections` **whole** (bare), not with a
  `{ ... }` list. Their methods and constructors come from `impl` blocks
  that only the bare form merges.
- `String.len()` and `String.is_empty()` are built into the compiler and
  work with no import at all. The richer string methods need
  `import std/string`.

## Core traits are always in scope

`std/core` defines the foundational traits (`ToString`, `Eq`, `Ord`,
`Hash`, `Iterator<T>`) and implements them for the primitive types. It is
auto-imported, so you never write `import std/core`:

```rust
fun describe<T: ToString>(x: T) -> String = x.to_string()
```

See [core traits](stdlib/core.md) for the full set.

## Errors use Result and Option

Anything that can fail returns `Result<T, Error>`; anything that can be
absent returns `Option<T>`. There are no exceptions and no `null`. Handle a
`Result` with `match`, or propagate it with the `?` operator inside a
function that itself returns `Result`:

```rust
import std/fs { read }

fun load(path: String) -> Result<String, Error> {
    let text = read(path)?          // returns early on Err
    return Ok(text.trim())
}

fun main() {
    match load("note.txt") {
        Ok(s) -> print(s),
        Err(e) -> print("could not load"),
    }
}
```

See [std/error](stdlib/error.md) for `Error`, `?`, and the `Result`/`Option`
helpers.

## The modules

### Text and formatting

| Module | What it is for |
|--------|----------------|
| [std/string](stdlib/string.md) | methods on `String`: slicing, case, search, replace |
| [std/fmt](stdlib/fmt.md) | padding, joining, number bases, the `Debug` trait |
| [std/regex](stdlib/regex.md) | regular expressions |
| [std/encoding](stdlib/encoding.md) | hex and base64 |

### Data structures and iteration

| Module | What it is for |
|--------|----------------|
| [std/collections](stdlib/collections.md) | hash-backed `Map` and `Set` |
| [std/list](stdlib/list.md) | utility functions over the built-in `List` |
| [std/option](stdlib/option.md) | combinators over the built-in `Option` |
| [std/iter](stdlib/iter.md) | lazy iterator adapters and consumers |
| [std/cmp](stdlib/cmp.md) | sorting and comparison helpers |
| [std/hash](stdlib/hash.md) | non-cryptographic hashing |

### Numbers

| Module | What it is for |
|--------|----------------|
| [std/math](stdlib/math.md) | float and integer math, constants |
| [std/random](stdlib/random.md) | a seeded random number generator |

### Input, output, and the system

| Module | What it is for |
|--------|----------------|
| [std/io](stdlib/io.md) | console input and output |
| [std/fs](stdlib/fs.md) | files and directories |
| [std/path](stdlib/path.md) | path string manipulation (no IO) |
| [std/env](stdlib/env.md) | environment, arguments, platform |
| [std/process](stdlib/process.md) | run external programs |
| [std/time](stdlib/time.md) | timestamps and calendar conversions |

### Networking

| Module | What it is for |
|--------|----------------|
| [std/net](stdlib/net.md) | TCP sockets |
| [std/http](stdlib/http.md) | an HTTP client and server |
| [std/json](stdlib/json.md) | JSON parse and stringify |

### Concurrency, errors, FFI, testing

| Module | What it is for |
|--------|----------------|
| [std/sync](stdlib/sync.md) | channels for goroutines |
| [std/error](stdlib/error.md) | the `Error` type and `Result` helpers |
| [std/ffi](stdlib/ffi.md) | bridge values to C and access raw memory |
| [std/test](stdlib/test.md) | assertions for test programs |
| [std/core](stdlib/core.md) | the always-in-scope core traits |
