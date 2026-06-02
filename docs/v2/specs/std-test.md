# std/test Spec

A small assertion surface for writing test programs in Raven. A failed
assertion aborts the process with a message and a nonzero exit code; a
program whose assertions all hold runs to completion and exits zero.

## Test model

Raven has no attributes or reflection yet, so there is no test discovery
or registration framework. A test is an ordinary Raven program whose
`main` calls these assertions:

```raven
import std/test { assert, assert_eq_int }
import std/io { println }

fun main() {
    assert(1 + 1 == 2)
    assert_eq_int(6 * 7, 42)
    println("all passed")
}
```

Build and run the program. Exit zero means every assertion held; a
nonzero exit means an assertion failed, and the panic message names the
failure. A test runner is just whatever invokes the compiled program and
checks its exit code (for example a shell loop or CI step).

## Import

The assertions are free functions, so a selective import binds them:

```raven
import std/test { assert, assert_msg, assert_true, assert_false, assert_eq_int, assert_eq_str }
```

## Surface

| Function | Fails when | Panic message |
|---|---|---|
| `assert(cond: Bool)` | `cond` is false | `assertion failed` |
| `assert_msg(cond: Bool, msg: String)` | `cond` is false | `msg` |
| `assert_true(cond: Bool)` | `cond` is false | `assertion failed: expected true` |
| `assert_false(cond: Bool)` | `cond` is true | `assertion failed: expected false` |
| `assert_eq_int(a: Int, b: Int)` | `a != b` | `assertion failed: ${a} != ${b}` |
| `assert_eq_str(a: String, b: String)` | `a != b` | `assertion failed: ${a} != ${b}` |

`assert_eq_str` relies on `String` `==` comparing contents (equivalent to
the prelude `Eq` `equals`). The integer and string equality messages
interpolate both operands so a failure shows the mismatched values.

A failure lowers to the runtime panic (`raven_panic`), which writes the
message to stderr and exits nonzero. The compiler exposes this through an
internal `__panic(msg: String)` intrinsic that `std/test` calls; users
call the assertions, not `__panic` directly.

## Out of scope

- Test discovery, registration, or attributes.
- A generic `assert_eq<T: Eq + ToString>`: interpolating a value of a
  generic `T` in the panic message is not yet supported by code
  generation, so the concrete `assert_eq_int` and `assert_eq_str` ship
  instead. The generic form is deferred until generic interpolation lands.
- Fixtures, setup/teardown, and parametrized cases.
- Floating point tolerance comparisons.
