# std/test

Assertions for writing test programs in Raven. A test is an ordinary Raven
program whose `main` calls these assertions: if every assertion holds, the
program runs to completion and exits zero; if one fails, it panics with a
message and aborts with a nonzero exit code.

```raven
import std/test { assert, assert_eq_int }
import std/io { println }

fun main() {
    assert(1 + 1 == 2)
    assert_eq_int(6 * 7, 42)
    println("all passed")
}
```

## Importing

The assertions are free functions, so import the ones you use with a
selective list:

```raven
import std/test { assert, assert_msg, assert_true, assert_false, assert_eq_int, assert_eq_str }
```

## The test model

Raven has no attributes or reflection, so there is no test discovery or
registration framework. A test is just a normal program. Build and run it:
exit zero means every assertion held, and a nonzero exit means one failed.
The panic message naming the failure goes to stderr.

A test runner is whatever invokes the compiled program and checks its exit
code (a shell loop, a CI step, or `rvpm run`). Because a failing assertion
aborts the whole process, the first failure stops the program; assertions
after it do not run.

## Assertions

Each assertion checks a condition and, when it does not hold, panics with a
fixed message and exits nonzero.

### `assert(cond: Bool)`

Fails when `cond` is false. Panic message: `assertion failed`.

```raven
import std/test { assert }

fun main() {
    assert(2 > 1)
    assert("raven".len() == 5)
}
```

### `assert_msg(cond: Bool, msg: String)`

Fails when `cond` is false, panicking with the caller-supplied `msg`. Use
this when you want the failure to explain itself.

```raven
import std/test { assert_msg }

fun main() {
    let n = 7
    assert_msg(n % 2 == 1, "n should be odd")
}
```

### `assert_true(cond: Bool)`

Fails when `cond` is false. Panic message: `assertion failed: expected true`.

### `assert_false(cond: Bool)`

Fails when `cond` is true. Panic message: `assertion failed: expected false`.

```raven
import std/test { assert_true, assert_false }
import std/string

fun main() {
    assert_true("report.csv".ends_with(".csv"))
    assert_false("report.csv".ends_with(".txt"))
}
```

### `assert_eq_int(a: Int, b: Int)`

Fails when `a != b`. The panic message interpolates both operands
(`assertion failed: ${a} != ${b}`), so a failure shows the mismatched
values.

```raven
import std/test { assert_eq_int }

fun double(x: Int) -> Int {
    return x * 2
}

fun main() {
    assert_eq_int(double(21), 42)
}
```

### `assert_eq_str(a: String, b: String)`

Fails when `a != b`, compared by content. The panic message interpolates
both operands (`assertion failed: ${a} != ${b}`).

```raven
import std/test { assert_eq_str }
import std/string

fun main() {
    assert_eq_str("Hello".to_upper(), "HELLO")
}
```

## Worked example: a test program

A test file is a regular `.rv` program with a `main` that asserts. The
program below exercises a couple of helpers and exits zero only if every
check passes:

```raven
import std/test { assert, assert_eq_int, assert_eq_str }
import std/string
import std/io { println }

fun add(a: Int, b: Int) -> Int {
    return a + b
}

fun shout(s: String) -> String {
    return s.trim().to_upper()
}

fun main() {
    assert_eq_int(add(2, 3), 5)
    assert_eq_int(add(-1, 1), 0)
    assert(add(10, 10) > 15)

    assert_eq_str(shout("  hi  "), "HI")
    assert_eq_str(shout("raven"), "RAVEN")

    println("all tests passed")
}
```

Run it directly:

```
raven run tests/math_test.rv
```

If `shout("raven")` ever returned the wrong value, the program would print
the interpolated mismatch to stderr and exit nonzero instead of printing
`all tests passed`.

## Running tests in a project

A test is a normal Raven program, so you run it the same way you run any
other:

- `raven run path/to/test.rv` compiles and runs a single test file.
- `raven build path/to/test.rv` compiles it to a binary you can invoke
  later (handy for CI, where the exit code is the pass/fail signal).
- Inside an `rvpm` project, point `main.rv` (or a dedicated entry) at your
  assertions and use `rvpm run`. See the
  [rvpm guide](../rvpm.md) for project layout and how `rvpm run` builds and
  executes the package entry.

To run several test files, drive them from a shell loop or CI step and check
each exit code; the first nonzero exit marks a failure.

## See also

- [rvpm guide](../rvpm.md) for project structure and running a package.
- [std/string](string.md) for the string methods used in the examples above.
