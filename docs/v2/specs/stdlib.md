# Standard Library Spec

## Goal

Define how Raven ships a standard library and implement its first module,
`std/io`. The standard library is written in Raven (`.rv` source) bundled
with the compiler. Bundled modules call a small set of compiler
intrinsics for the operations that cannot be expressed in safe Raven
(writing bytes to stdout, reading a line from stdin). Every later stdlib
module (issues #72 to #80) plugs into the same mechanism this PR builds.

## Pipeline position

```
Source -> Lexer -> Parser -> [stdlib expansion] -> Resolver -> Tycheck -> HIR -> MIR -> Codegen -> Linker
```

Stdlib expansion is a new step between parsing and resolution. It rewrites
the parsed program into a combined file that contains the bundled modules
the program imports plus the user's own items. From that point on the
existing single file pipeline compiles and links everything together with
no per stage special casing.

## Bundled source model

Stdlib modules live under `stdlib/std/` in the repository, one `.rv` file
per module (for example `stdlib/std/io.rv`). Each module is embedded into
the compiler binary with `include_str!`, so the compiler carries its own
standard library and does not depend on a runtime file path or an
installed copy on disk.

The embedding lives in `src/resolve/stdlib.rs`:

```rust
pub const BUNDLED_MODULES: &[(&str, &str)] =
    &[("io", include_str!("../../stdlib/std/io.rv"))];
```

The key is the module path under `std/`. A `std/io` import maps to the
`"io"` entry. Adding a later module is one new row here plus the new
`.rv` file. An import of a module name that has no bundled source is
reported by the resolver as `UnresolvedImport`, the same as today.

## Import to source mapping

When the program imports a bundled module, the compiler:

1. Looks up the embedded source for the module path.
2. Lexes and parses that source into a module file. The virtual source
   path is `<bundled>/std/<module>.rv`, used only for diagnostics.
3. Namespaces every top level function in the module (see below).
4. Merges the namespaced declarations into the program ahead of the
   user's own items.

A module is loaded once. Duplicate imports of the same `std/<module>` (in
the same program, or selecting different names) merge a single copy.

## Multi module compilation and namespacing

The driver builds one combined `ast::File` whose `items` are the bundled
stdlib declarations followed by the user's declarations. The combined file
is owned by the driver and flows through the whole pipeline, so the
resolver, type checker, HIR, MIR, and codegen all see the stdlib functions
as ordinary top level functions defined alongside the user program. The
monomorphizer already roots every non generic top level function, so each
stdlib function is compiled into the object and linked with no extra
reachability analysis.

Namespacing avoids collisions between a stdlib function and a user
function of the same name. A function `f` in module `io` is renamed at the
AST level to `std.io.f`. The separator is a literal `.`, which a user
cannot type in an identifier, so a namespaced name never clashes with a
user declaration. The back end keys every call on the compiled function's
name, so a call site must use the namespaced name rather than the source
spelling; HIR lowering resolves a callee identifier to its declared
function name through the resolution map, which yields `std.io.println`
for a `println(...)` call that bound to the bundled function.

## Import forms

The working import form for a bundled module is the selective import:

```raven
import std/io { println, println_int }
```

The resolver binds each selector (`println`, `println_int`) directly to
the namespaced function (`std.io.println`, `std.io.println_int`). The call
site `println("hi")` is then an ordinary call to a known function, which
the type checker, HIR, MIR, and codegen handle with no member access
machinery. An explicit selector binding wins over a builtin of the same
name, so `import std/io { print }` shadows the builtin `print` and gives
the no newline behavior the module documents.

The aliased form `import std/io as io` with member access `io.println(...)`
is not supported in this release: member resolution through an import
alias is type checker work that is out of scope here. Programs use the
selective import form.

## Intrinsic boundary

Bundled Raven needs three operations below safe Raven: write bytes to
stdout with no newline, write bytes followed by a newline, and read a line
from stdin. These are exposed as a minimal set of compiler intrinsics that
the bundled source calls. The leading `__io_` marks them internal; users
do not write them directly.

| Intrinsic                     | Lowers to runtime symbol | Meaning                          |
|-------------------------------|--------------------------|----------------------------------|
| `__io_print_str(s: String)`   | `raven_print_str`        | write `s` bytes, no newline      |
| `__io_println_str(s: String)` | `raven_println_str`      | write `s` bytes plus a newline   |
| `__io_read_line() -> String`  | `raven_read_line`        | read one line, newline stripped  |

The intrinsics are recognized at three points, mirroring the existing
`print` builtin:

* The resolver bypasses scope lookup for the `__io_*` names so the bundled
  source can call them without importing them.
* The type checker assigns each intrinsic its signature (a `String`
  argument returning `Unit`, or no argument returning `String`).
* The codegen back end pattern matches the mangled name and emits a direct
  call to the matching `raven-runtime` C ABI symbol. The String argument
  is lowered through the same path as the `print` builtin: a string
  literal passes its interned bytes and compile time length, and any other
  `String` value reads its byte pointer and length from the object through
  `raven_string_bytes` and `raven_string_len`.

The runtime gains one new symbol, `raven_read_line() -> *mut String`. It
reads one line from stdin, strips a trailing `\n` (and a preceding `\r`
for Windows line endings), and returns a heap `String`. At end of input it
returns an empty `String`, so a caller always receives a valid pointer.
`raven_print_str` and `raven_println_str` already existed.

This boundary dogfoods the existing string runtime and keeps the intrinsic
surface to three internal names. A future module that needs a new
primitive adds one intrinsic and one runtime symbol the same way.

## The std/io surface

`stdlib/std/io.rv` exports:

* `print(s: String)`: write `s` with no trailing newline.
* `println(s: String)`: write `s` followed by a newline.
* `print_int(n: Int)`: write the base ten rendering of `n`, no newline.
* `println_int(n: Int)`: write the base ten rendering of `n`, plus a newline.
* `int_to_string(n: Int) -> String`: render an `Int` as a `String`.
* `bool_to_string(b: Bool) -> String`: render a `Bool` as `"true"` / `"false"`.
* `input(prompt: String) -> String`: print `prompt` (no newline), read one
  line from stdin, and return it without the trailing newline.
* `read_line() -> String`: read one line from stdin, newline stripped.

The integer and conversion functions are built on string interpolation
(`"${n}"`), which the compiler already lowers to the runtime integer to
string and bool to string conversions. The module thus needs no separate
integer intrinsics.

## Relationship to the print builtins

The compiler keeps `print` and `print_int` as global builtins for
convenience (used by examples and quick programs without an import). For
historical reasons those builtins append a newline. `std/io` is the
blessed user facing surface: its `print` writes with no newline and its
`println` adds one, which is the conventional split. Because an explicit
import binds over a builtin, a program that imports `std/io`'s `print`
gets the module's no newline behavior; a program that does not import it
keeps the builtin.

## How later modules plug in

A new module (for example `std/collections`, `std/math`, `std/fs`):

1. Adds `stdlib/std/<module>.rv` with its functions and types.
2. Adds one `include_str!` row to `BUNDLED_MODULES`.
3. If it needs an operation below safe Raven, adds an internal intrinsic
   and its runtime symbol following the `__io_*` pattern above.

No driver, resolver, type checker, or codegen change is needed for a
module that only adds functions and types built on existing primitives.
The packaging note in `CLAUDE.md` about shipping `lib/*.rv` does not apply
to bundled modules: they are compiled into the binary, so nothing extra
ships.

## Out of scope

* The aliased import form `import std/io as io` with `io.member(...)`
  access. Selective imports are the working form here.
* Variadic `format(template, args...)`. String interpolation already
  covers formatted output, so a variadic `format` is deferred.
* Buffered or unbuffered output control, flushing, and standard error
  helpers (`eprint` / `eprintln`).
* Async I/O and file I/O (`std/fs`, a later module).
* Structs and traits in a bundled module. The `std/io` surface is free
  functions only; later modules that need types exercise that path.
