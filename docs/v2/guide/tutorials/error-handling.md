# Tutorial: error handling with Option, Result, and `?`

Raven has no exceptions and no `null`. A value that might be absent has type
`Option<T>`, and an operation that might fail returns `Result<T, E>`. Both are
ordinary enums built into the language, so failure shows up in the type
signature and the compiler makes you handle it. This tutorial works through
absence, failure, the `?` operator that propagates failures, and the
[`std/error`](../stdlib/error.md) helpers that keep the common cases short.
Every step compiles and runs.

## Step 1: absence with `Option`

`Option<T>` is either `Some(value)` or `None`. Use it for a lookup that might
find nothing. You read it back with `match`, which forces you to handle both
the present and the absent case:

```rust
fun unwrap_or(x: Option<Int>, fallback: Int) -> Int {
    return match x {
        None -> fallback,
        Some(n) -> n,
    }
}

fun main() {
    let present = unwrap_or(Some(5), 0)
    let missing = unwrap_or(None, 99)
    print(present + missing)        // 104
}
```

There is no way to read the inner value without first checking which variant
you have, so the "I forgot it might be empty" class of bug simply cannot
compile.

## Step 2: failure with `Result`

`Result<T, E>` is either `Ok(value)` or `Err(error)`. The `E` slot is whatever
error type you choose. A clean default is the `Error` value from `std/error`,
built with `error("message")`:

```rust
import std/error { error, Error }

fun divide(a: Int, b: Int) -> Result<Int, Error> {
    if b == 0 {
        return Err(error("divide by zero"))
    }
    return Ok(a / b)
}

fun main() {
    match divide(10, 2) {
        Ok(v) -> print(v),                  // 5
        Err(e) -> print(e.message()),
    }
    match divide(1, 0) {
        Ok(v) -> print(v),
        Err(e) -> print(e.message()),       // divide by zero
    }
}
```

`Error` carries a message (and an optional kind, see Step 5). The `match`
covers both arms, so a failure can never be silently ignored.

## Step 3: propagating with `?`

Matching every intermediate result gets verbose when one operation feeds the
next. The postfix `?` operator unwraps an `Ok` to its value, or returns the
`Err` from the enclosing function immediately. It keeps the happy path flat:

```rust
import std/error { error, Error }

fun positive(n: Int) -> Result<Int, Error> {
    if n <= 0 {
        return Err(error("must be positive"))
    }
    return Ok(n)
}

fun add_positive(a: Int, b: Int) -> Result<Int, Error> {
    let x = positive(a)?        // returns the Err early if a <= 0
    let y = positive(b)?
    return Ok(x + y)
}

fun main() {
    match add_positive(3, 4) {
        Ok(n) -> print(n),                  // 7
        Err(e) -> print(e.message()),
    }
    match add_positive(3, 0) {
        Ok(n) -> print(n),
        Err(e) -> print(e.message()),       // must be positive
    }
}
```

Because `?` returns early, it can only appear in a function whose return type
matches: `?` on a `Result` needs the function to return a `Result`, and `?` on
an `Option` needs it to return an `Option`. Using `?` in a function that
returns a plain value is a compile error, not a silently dropped failure.

## Step 4: recovering without a full `match`

When you do not need to inspect the error, the `std/error` helpers cover the
common shortcuts so you can skip writing a `match`:

```rust
import std/error { error, unwrap_or, is_ok, is_err, ok }

fun divide(a: Int, b: Int) -> Result<Int, Error> {
    if b == 0 {
        return Err(error("divide by zero"))
    }
    return Ok(a / b)
}

fun main() {
    print(unwrap_or(divide(10, 2), -1))     // 5  (the Ok value)
    print(unwrap_or(divide(1, 0), -1))      // -1 (the fallback)
    print(is_ok(divide(10, 2)))             // true
    print(is_err(divide(1, 0)))             // true

    match ok(divide(8, 4)) {                // Result -> Option, dropping the error
        Some(v) -> print(v),                // 2
        None -> print(0),
    }
}
```

`unwrap_or` gives you the value or a default, `is_ok`/`is_err` test the
variant, and `ok` converts a `Result` to an `Option` when you care whether a
value is present but not why it failed. These are generic over any error type,
not just `Error`.

## Step 5: adding context as errors rise

A low-level failure like `no such file` is more useful with a higher-level
explanation attached. `with_context` returns a new `Error` whose message is
prefixed, keeping the original kind:

```rust
import std/error { error_kind, unwrap_or }

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

fun labeled(port: Int) -> Result<Int, Error> {
    return match read_port(port) {
        Ok(p) -> Ok(p),
        Err(e) -> Err(e.with_context("read port")),
    }
}

fun main() {
    print(unwrap_or(labeled(8080), 80))     // 8080
    match labeled(-1) {
        Ok(p) -> print("port ${p}"),
        Err(e) -> print(e.to_string()),     // read port: port must be positive
    }
}
```

`error_kind("config", ...)` tags the error with a free-form kind you can branch
on later with `e.kind()`. `to_string()` renders an error as `kind: message` (or
just the message when the kind is empty), and `print(e)` uses the same form.

## Step 6: a different error type per layer

The `E` in `Result<T, E>` does not have to be `Error`. A library often defines
a struct that carries structured failure data, and `?` threads it through
unchanged even when the `Ok` and `Err` types differ:

```rust
import std/string

struct Fail {
    code: Int,
    message: String,
}

fun check(n: Int) -> Result<Bool, Fail> {
    if n < 0 {
        return Err(Fail { code: 1, message: "negative" })
    }
    return Ok(true)
}

fun run(n: Int) -> Result<Int, Fail> {
    let _ok = check(n)?         // propagates the Fail struct unchanged
    return Ok(n * 2)
}

fun main() {
    match run(5) {
        Ok(v) -> print(v),                                          // 10
        Err(e) -> print(e.message),
    }
    match run(0 - 3) {
        Ok(v) -> print(v),
        Err(e) -> print(e.message.concat(" code=").concat(e.code.to_string())),
    }
}
```

Here `check` returns `Result<Bool, Fail>` and `run` returns `Result<Int,
Fail>`: the `Ok` types differ, but the `Err` type is the same, so `?` carries a
`Fail` from `check` straight out of `run`. The `Err(e)` arm binds `e` as a
`Fail`, so you can read `e.code` and `e.message`.

## When to use what

- **`Option<T>`** for a value that may be absent, with no reason needed.
- **`Result<T, Error>`** for an operation that may fail, where a message is
  enough.
- **`Result<T, MyStruct>`** when callers need to branch on structured failure
  data.
- **`?`** to propagate, **`match`** to handle, and the `std/error` helpers to
  recover in one line.

Errors in Raven are plain values, never control flow. Reach for `panic(msg)`
only when a bug has made the program unable to continue, not for an expected,
recoverable failure.

## Where to go next

- The [`std/error` reference](../stdlib/error.md) documents every helper,
  including `map_ok`, `map_err`, and `unwrap_or_else`.
- The [language reference](../language-reference.md) covers `Result`, `Option`,
  pattern matching, and the `?` operator.
- The [testing tutorial](testing.md) shows `assert_ok`, `assert_err`,
  `assert_some`, and `assert_none` for checking these types in tests.
