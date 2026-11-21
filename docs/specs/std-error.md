# std.error Spec

An `Error` value type plus helpers over the built-in `Result[T, E]`. The language already provides `Result`, `Option`, and the `?` operator (see [core-traits.md](core-traits.md)); this module adds a carryable error with a message and optional kind, context chaining, and a few free functions over `Result`.

## Import

```rust
from std.error import error, unwrap_or, is_ok

fun divide(a: int, b: int) -> Result[int, Error] {
    if b == 0 {
        return Err(error("divide by zero"))
    }
    return Ok(a / b)
}

fun main() {
    print(unwrap_or(divide(10, 2), -1)) // 5
    print(is_ok(divide(1, 0))) // false
}
```

Importing the module also brings in the `Error` methods and its `ToString` impl, so `print(e)` renders the message and `e.message()` works.

## Error

A class holding a `kind` and a `message`, both `str`.

| Item | Result | Notes |
|---|---|---|
| `error(msg)` | `Error` | constructor with an empty kind |
| `error_kind(kind, msg)` | `Error` | constructor with a kind tag |
| `message()` | `str` | the message text |
| `kind()` | `str` | the kind tag, `""` when unset |
| `with_context(ctx)` | `Error` | new error with message `ctx + ": " + message`, kind preserved |
| `to_string()` | `str` | `message` when the kind is empty, else `kind + ": " + message` |

`with_context` adds a higher-level explanation as an error propagates, for example `read_config(p).with_context("load config")`. It returns a new `Error` and does not change the receiver.

## Result helpers

Free functions generic over `Result[T, E]`:

| Function | Result | Notes |
|---|---|---|
| `is_ok(r)` | `bool` | true when `Ok` |
| `is_err(r)` | `bool` | true when `Err` |
| `unwrap_or(r, default)` | `T` | the Ok value, or `default` on Err |
| `unwrap_or_else(r, f)` | `T` | the Ok value, or `f(e)` on Err |
| `ok(r)` | `Option[T]` | Ok to Some, Err to None |
| `err(r)` | `Option[E]` | Err to Some, Ok to None |
| `map_ok(r, f)` | `Result[U, E]` | apply `f` to an Ok value |
| `map_err(r, f)` | `Result[T, F]` | apply `f` to an Err value |

These work with any error type, not just this module's `Error`. `Result` and `Option` also have methods of their own in the prelude (`is_ok`, `unwrap`, `unwrap_or`, `map`, `map_err`, `and_then`, `ok_or`), so `r.map(f)` needs no import.

## Relationship to Result, ?, and panic

`Result[T, E]`, `Option[T]`, `Ok`, `Err`, `Some`, and `None` are built in. The `?` operator unwraps an `Ok` or `Some`, or returns the `Err` or `None` early; in `main`, a failing `?` stops the program with the error. This module adds an error payload for the `E` slot and helpers for cases where a `match` would be verbose. `panic(msg)` aborts the process: errors here are values, never control flow.

## Not included

- Backtraces and source locations.
- Numeric error codes and `errno` mapping.
- Downcasting or a dynamic error trait object.
