# Tutorial: testing your code

Raven's test model is deliberately small: a test is a zero-argument function
named `test_*` in a file ending in `_test.rv`. There are no attributes, no
registration macros, and no reflection. `rvpm test` finds those functions,
runs each in its own process, and reports pass or fail by whether its
assertions held. The assertions come from [`std/test`](../stdlib/test.md). This
tutorial writes a function, tests it, makes a test fail on purpose to see the
output, then tests a library and wires the whole thing into CI.

## Step 1: a project with something to test

Start a package and put a function worth testing in it:

```bash
rvpm init calc
cd calc
```

Edit `src/main.rv` so it exports an `add` function:

```rust
// src/main.rv
fun add(a: Int, b: Int) -> Int {
    return a + b
}

fun main() {
    print(add(2, 3))        // 5
}
```

## Step 2: write a test file

A test lives in a `*_test.rv` file and imports the function under test from its
module. Each check is a `test_*` function that calls assertions from
`std/test`:

```rust
// src/add_test.rv
import std/test { assert_eq_int }
import "./main" { add }

fun test_add_basic() {
    assert_eq_int(add(2, 3), 5)
}

fun test_add_negative() {
    assert_eq_int(add(-1, 1), 0)
}
```

A test file has no `main`: `rvpm test` calls each `test_*` function for you.
`import "./main" { add }` pulls in the function from `src/main.rv` (a local
import is a relative path in quotes). Test function names must be unique within
a file.

## Step 3: run the tests

From the package root:

```bash
rvpm test
```

`rvpm test` discovers every `test_*` function in every `*_test.rv` file, runs
each in its own process, and prints a line per test plus a summary:

```
running 2 tests
  ok test_add_basic
  ok test_add_negative
test result: ok. 2 passed; 0 failed
```

Because each test runs in a separate process, one test's failure (a panic from
a failed assertion) cannot take down the others.

## Step 4: see a failure

Assertions that compare values interpolate both operands into the panic
message, so a failure tells you what it expected. Add a deliberately wrong test:

```rust
// src/add_test.rv
import std/test { assert_eq_int }
import "./main" { add }

fun test_add_basic() {
    assert_eq_int(add(2, 3), 5)
}

fun test_add_wrong() {
    assert_eq_int(add(2, 2), 5)     // add(2, 2) is 4, not 5
}
```

Now `rvpm test` reports the failure and exits non-zero:

```
running 2 tests
  ok test_add_basic
  FAIL test_add_wrong (raven panic: assertion failed: 4 != 5)
test result: FAILED. 1 passed; 1 failed
```

The `4 != 5` comes straight from `assert_eq_int` interpolating the mismatched
values. Fix the expected value (or the code) and the test goes back to `ok`.

## Step 5: pick the right assertion

`std/test` has an assertion per common shape. Import the ones you use:

```rust
import std/test { assert, assert_msg, assert_true, assert_false, assert_eq_int, assert_eq_str, assert_eq, assert_ne, assert_some, assert_none }
import std/string

fun test_assertions_tour() {
    assert(1 + 1 == 2)                          // any Bool condition
    assert_msg(7 % 2 == 1, "7 should be odd")   // with a custom message
    assert_true(10 > 3)
    assert_false(3 > 10)

    assert_eq_int(6 * 7, 42)                     // Int equality
    assert_eq_str("ab".concat("c"), "abc")       // String equality, by content
    assert_eq(2 + 2, 4)                          // generic Eq + ToString
    assert_ne("yes", "no")

    assert_some(Some(5))                         // Option is Some
    let empty: Option<Int> = None
    assert_none(empty)
}
```

Use `assert_eq_int` / `assert_eq_str` when you know the type, `assert_eq` /
`assert_ne` for any `Eq + ToString` type, and `assert_msg` when a bare
condition would not explain itself on failure. For floats, use
`assert_eq_float(a, b, eps)` rather than exact `==`, since computed floats
rarely match to the bit.

## Step 6: test the failure paths too

Code that returns `Result` or `Option` should be tested on both outcomes.
`assert_ok` / `assert_err` and `assert_some` / `assert_none` check the variant
without a `match`:

```rust
// src/parse_test.rv
import std/test { assert_ok, assert_err }
import "./main" { parse_count }

fun test_parse_accepts_valid() {
    assert_ok(parse_count("3"))
}

fun test_parse_rejects_invalid() {
    assert_err(parse_count("oops"))
}
```

This assumes a `parse_count(s: String) -> Result<Int, Error>` in `src/main.rv`.
Testing the `Err` path is how you lock in that a failure stays a failure, not a
silent `Ok`.

## Step 7: testing a library

A library has no `src/main.rv`, just modules other packages import. Tests work
the same way: a `*_test.rv` at the package root (or beside the code) imports the
library module and asserts against it. For a library whose entry is `src/lib.rv`:

```rust
// src/lib_test.rv
import std/test { assert_eq_int }
import "./lib" { factorial }

fun test_factorial_base() {
    assert_eq_int(factorial(0), 1)
}

fun test_factorial_small() {
    assert_eq_int(factorial(5), 120)
}
```

`rvpm test` works without a `main.rv`, so a library is fully testable on its
own.

## Step 8: tests in CI

`rvpm test` exits non-zero the moment any test fails, which is exactly the
signal a CI step needs. A minimal job is just:

```bash
rvpm test
```

If every `test_*` passes, the command exits zero and the job is green; the
first failing assertion flips the exit code and the per-test `FAIL` line tells
you which one. If you would rather run a single test program by hand, a
`*_test.rv` you write with its own `main` full of assertions can also be
built directly with `raven build path/to/test.rv -o test_bin` and then run,
where a zero exit means it passed.

## Where to go next

- The [`std/test` reference](../stdlib/test.md) lists every assertion, including
  `assert_eq_float`, `assert_some`, and `assert_ok`, with examples.
- The [rvpm guide](../rvpm.md) covers project layout and how `rvpm test`
  discovers and runs tests.
- The [error-handling tutorial](error-handling.md) explains the `Result` and
  `Option` values that `assert_ok` and `assert_some` check.
