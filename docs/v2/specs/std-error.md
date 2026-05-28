# std/error Spec

An `Error` value type plus ergonomic helpers over the built-in
`Result<T, E>`. The language already provides `Result`, `Option`, and the
`?` operator (see `docs/v2/specs/core-traits.md`); this module adds a
carryable error with a message and optional kind, context chaining, and a
small set of free functions over `Result`.

## Import

```raven
import std/error { error, unwrap_or, is_ok }
import std/io { println }

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

Importing the module also merges the `Error` `impl` blocks (including its
`ToString` impl), so `print(e)` renders the message and `e.message()`
resolves by receiver type. `std/error` itself imports `std/string` for
`concat`; that dependency is merged transitively.

## Error

A struct holding a `kind` and a `message`, both `String`.

| Item | Result | Notes |
|---|---|---|
| `error(msg)` | `Error` | constructor with an empty kind |
| `error_kind(kind, msg)` | `Error` | constructor with a kind tag |
| `message()` | `String` | the message text |
| `kind()` | `String` | the kind tag, `""` when unset |
| `with_context(ctx)` | `Error` | new error with message `ctx + ": " + message`, kind preserved |
| `to_string()` | `String` | `message` when the kind is empty, else `kind + ": " + message` |

`with_context` adds a higher-level explanation as an error propagates,
for example `read_file(p).with_context("load config")`. It returns a new
`Error` and does not mutate the receiver.

## Result helpers

Free functions generic over `Result<T, E>`. Each matches the Result with
`match r { Ok(v) -> ..., Err(e) -> ... }`.

| Function | Result | Notes |
|---|---|---|
| `is_ok(r)` | `Bool` | true when `Ok` |
| `is_err(r)` | `Bool` | true when `Err` |
| `unwrap_or(r, default)` | `T` | the Ok value, or `default` on Err |
| `ok(r)` | `Option<T>` | Ok to Some, Err to None |
| `err(r)` | `Option<E>` | Err to Some, Ok to None |

These work with any error type, not just this module's `Error`.

## Relationship to built-in Result, ?, and panic

`Result<T, E>`, `Option<T>`, `Ok`, `Err`, `Some`, and `None` are built in.
The `?` operator unwraps an `Ok`/`Some` or returns the `Err`/`None` early;
it needs no import and is the primary propagation mechanism. This module
adds an error payload to carry in the `E` slot and helpers for the cases
where matching by hand is verbose. `panic(msg)` aborts the process and is
the runtime's facility, not part of this module: errors here are values,
never control flow.

## Out of scope

- Backtraces and source locations.
- Numeric error codes and `errno` mapping.
- Downcasting or a dynamic error trait object.
- `map`/`map_err`/`and_then` combinators (deferred; `match` and `?` cover
  the common cases for now).
