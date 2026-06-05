# std/error

An `Error` value type plus a few helpers over the built-in `Result<T, E>`.
The language already gives you `Result`, `Option`, and the `?` operator, so
this module does not invent an error system. It adds a carryable error with a
message and an optional kind, context chaining as an error propagates, and a
small set of free functions for the cases where matching a `Result` by hand
gets verbose.

```rust
import std/error { error, error_kind, is_ok, is_err, unwrap_or, ok, err }

fun divide(a: Int, b: Int) -> Result<Int, Error> {
    if b == 0 {
        return Err(error("divide by zero"))
    }
    return Ok(a / b)
}

fun main() {
    print(unwrap_or(divide(10, 2), -1))   // 5
    print(is_ok(divide(1, 0)))            // false
}
```

## Importing

Import the free functions you use by name:

```rust
import std/error { error, error_kind, is_ok, is_err, unwrap_or, ok, err }
```

`error`, `error_kind`, and the Result helpers (`is_ok`, `is_err`, `unwrap_or`,
`ok`, `err`) are free functions, so list the ones you call. The `Error` methods
(`message`, `kind`, `with_context`, `to_string`) come from the module's `impl`
block, which the import merges, so `e.message()` and `print(e)` work once you
import the module.

## Result, Option, and the `?` operator

`Result<T, E>`, `Option<T>`, and their variants `Ok`, `Err`, `Some`, and `None`
are built into the language. They need no import. The convention this module
leans on is simple:

- A function that can fail returns `Result<T, Error>`. Success is `Ok(value)`,
  failure is `Err(error("..."))`.
- A function that can be absent returns `Option<T>`: `Some(value)` or `None`.

The `?` operator is the primary way to propagate failures. When applied to a
`Result`, `?` unwraps an `Ok` to its value or returns the `Err` from the
enclosing function early. It works the same on `Option`, unwrapping `Some` or
returning `None`. Because `?` returns early, it can only appear in a function
whose return type matches (a `Result` for `?` on a `Result`, an `Option` for
`?` on an `Option`).

A function that uses `?` to chain failing steps:

```rust
import std/error { error, error_kind, is_ok, is_err, unwrap_or, ok, err }

fun positive(n: Int) -> Result<Int, Error> {
    if n <= 0 {
        return Err(error("must be positive"))
    }
    return Ok(n)
}

fun add_positive(a: Int, b: Int) -> Result<Int, Error> {
    let x = positive(a)?    // returns Err early if `a` is not positive
    let y = positive(b)?
    return Ok(x + y)
}
```

Each `?` keeps the happy path flat: if `positive` returns an `Err`,
`add_positive` returns that same `Err` without running the rest of the body.

A caller handles the result with `match`:

```rust
import std/error { error, error_kind, is_ok, is_err, unwrap_or, ok, err }

fun positive(n: Int) -> Result<Int, Error> {
    if n <= 0 {
        return Err(error("must be positive"))
    }
    return Ok(n)
}

fun main() {
    match positive(5) {
        Ok(n) -> print("ok: ${n}"),         // ok: 5
        Err(e) -> print("failed: ${e.to_string()}"),
    }
}
```

`match` forces you to handle both arms, so a failure cannot be ignored by
accident. When you do not need the full match, the Result helpers below cover
the common shortcuts.

Errors here are values, never control flow. `panic(msg)` aborts the process and
is a separate runtime facility, not part of this module. Reach for `Result` and
`Error` when a failure is expected and recoverable, and for `panic` only when a
bug has made the program unable to continue.

## Constructors

### `error(msg: String) -> Error`

Build an `Error` with the given message and an empty kind. This is the usual
constructor for an ad hoc failure.

```rust
import std/error { error, error_kind, is_ok, is_err, unwrap_or, ok, err }

fun check_age(age: Int) -> Result<Int, Error> {
    if age < 0 {
        return Err(error("age cannot be negative"))
    }
    return Ok(age)
}
```

### `error_kind(kind: String, msg: String) -> Error`

Build an `Error` tagged with a kind, for example `"io"` or `"parse"`. The kind
is a free-form `String`; callers can branch on it with `e.kind()`.

```rust
import std/error { error, error_kind, is_ok, is_err, unwrap_or, ok, err }

fun open_config(path: String) -> Result<String, Error> {
    return Err(error_kind("io", "no such file: ${path}"))
}
```

## Error methods

`Error` is a struct holding a `kind` and a `message`, both `String`. These
methods come from the merged `impl` blocks.

### `message(self) -> String`

The message text.

### `kind(self) -> String`

The kind tag, or `""` when the error was built with `error`.

```rust
import std/error { error, error_kind, is_ok, is_err, unwrap_or, ok, err }

fun main() {
    let e = error_kind("parse", "unexpected token")
    print(e.kind())       // parse
    print(e.message())    // unexpected token
}
```

### `with_context(self, ctx: String) -> Error`

A new `Error` whose message is `ctx + ": " + message`, preserving the kind. Use
it to add a higher-level explanation to a lower-level failure as it propagates.
It returns a new `Error` and does not mutate the receiver.

```rust
import std/error { error, error_kind, is_ok, is_err, unwrap_or, ok, err }

fun load_settings(path: String) -> Result<String, Error> {
    return match read_raw(path) {
        Ok(text) -> Ok(text),
        Err(e) -> Err(e.with_context("load settings")),
    }
}
```

If the inner error was `no such file`, the caller now sees
`load settings: no such file`, with the original kind intact.

### `to_string(self) -> String`

The display form: the bare `message` when the kind is empty, otherwise
`kind + ": " + message`. This comes from the `ToString` impl, so `print(e)`
uses it too.

```rust
import std/error { error, error_kind, is_ok, is_err, unwrap_or, ok, err }

fun main() {
    print(error("disk full").to_string())            // disk full
    print(error_kind("io", "disk full").to_string()) // io: disk full
}
```

## Result helpers

Free functions generic over `Result<T, E>`. They work with any error type in
the `E` slot, not only this module's `Error`. Each one matches the Result
internally, so you can avoid writing a `match` for the simple cases.

### `is_ok<T, E>(r: Result<T, E>) -> Bool`

True when `r` is `Ok`.

### `is_err<T, E>(r: Result<T, E>) -> Bool`

True when `r` is `Err`.

### `unwrap_or<T, E>(r: Result<T, E>, default: T) -> T`

The `Ok` value, or `default` when `r` is an `Err`.

```rust
import std/error { error, unwrap_or, is_err }

fun divide(a: Int, b: Int) -> Result<Int, Error> {
    if b == 0 {
        return Err(error("divide by zero"))
    }
    return Ok(a / b)
}

fun main() {
    print(unwrap_or(divide(10, 2), -1))   // 5
    print(unwrap_or(divide(1, 0), -1))    // -1
    print(is_err(divide(1, 0)))           // true
}
```

### `ok<T, E>(r: Result<T, E>) -> Option<T>`

Discard the error: map `Ok(v)` to `Some(v)` and `Err(e)` to `None`. Useful when
you care whether a value is present but not why it failed.

### `err<T, E>(r: Result<T, E>) -> Option<E>`

Keep the error: map `Err(e)` to `Some(e)` and `Ok(v)` to `None`.

```rust
import std/error { error, ok }

fun divide(a: Int, b: Int) -> Result<Int, Error> {
    if b == 0 {
        return Err(error("divide by zero"))
    }
    return Ok(a / b)
}

fun main() {
    match ok(divide(10, 2)) {
        Some(n) -> print("got ${n}"),    // got 5
        None -> print("no value"),
    }
}
```

### `map_ok<T, U, E>(r: Result<T, E>, f: fun(T) -> U) -> Result<U, E>`

Transform the `Ok` value with `f`, passing an `Err` through unchanged. `f` is
a lambda.

```rust
import std/error { error, map_ok }

fun parse(s: String) -> Result<Int, Error> {
    if s == "1" {
        return Ok(1)
    }
    return Err(error("bad"))
}

fun main() {
    match map_ok(parse("1"), fun(x: Int) -> Int = x + 10) {
        Ok(n) -> print(n),              // 11
        Err(e) -> print(e.message()),
    }
}
```

### `map_err<T, E, F>(r: Result<T, E>, f: fun(E) -> F) -> Result<T, F>`

Transform the `Err` value with `f`, passing an `Ok` through unchanged.

```rust
import std/error { error, map_err }

fun parse(s: String) -> Result<Int, Error> {
    if s == "1" {
        return Ok(1)
    }
    return Err(error("bad"))
}

fun main() {
    match map_err(parse("z"), fun(e: Error) -> String = e.message()) {
        Ok(n) -> print(n),
        Err(msg) -> print(msg),         // bad
    }
}
```

### `unwrap_or_else<T, E>(r: Result<T, E>, f: fun(E) -> T) -> T`

The `Ok` value, or the result of applying `f` to the error. The lazy
counterpart to `unwrap_or`, for when the fallback depends on the error or is
expensive to build.

```rust
import std/error { error, unwrap_or_else }

fun parse(s: String) -> Result<Int, Error> {
    if s == "1" {
        return Ok(1)
    }
    return Err(error("bad"))
}

fun main() {
    print(unwrap_or_else(parse("1"), fun(e: Error) -> Int = 0))      // 1
    print(unwrap_or_else(parse("z"), fun(e: Error) -> Int = 0 - 1))  // -1
}
```

## Worked example: a small config loader

This ties the pieces together: failing steps return `Result<T, Error>`, `?`
propagates failures, `with_context` annotates them as they rise, and the caller
recovers with `unwrap_or`.

```rust
import std/error { error, error_kind, is_ok, is_err, unwrap_or, ok, err }

fun check_port(port: Int) -> Result<Int, Error> {
    if port < 1 {
        return Err(error_kind("config", "port must be positive"))
    }
    return Ok(port)
}

fun read_port(port: Int) -> Result<Int, Error> {
    let checked = check_port(port)?
    return Ok(checked)
}

fun with_label(port: Int) -> Result<Int, Error> {
    return match read_port(port) {
        Ok(p) -> Ok(p),
        Err(e) -> Err(e.with_context("read port")),
    }
}

fun main() {
    print(unwrap_or(with_label(8080), 80))    // 8080
    print(unwrap_or(with_label(0), 80))       // 80

    match with_label(-1) {
        Ok(p) -> print("port ${p}"),
        Err(e) -> print(e.to_string()),       // read port: port must be positive
    }
}
```

## See also

- [std/io](io.md) for `print` and `println`, which render an `Error` through its
  `ToString` impl.
- [std/string](string.md) for `concat`, `trim`, and the other text methods used
  alongside error messages.
- The [language reference](../language-reference.md) for `Result`, `Option`,
  pattern matching, and the `?` operator.
