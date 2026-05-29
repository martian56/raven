# Migrating from v1 to v2

Raven v2 is a compiled language with a stricter type system, generics,
traits, sum types, and a real package manager. Almost every line of a v1
program needs small edits to compile under v2, and a few v1 idioms (error
sentinels, C-style loops, `export`) are replaced by new constructs. This
guide walks the breaking changes one at a time, pairing a v1 snippet with
the equivalent v2 snippet, and finishes by mapping the v1 example programs
to their v2 counterparts.

A v1 user should be able to read this in about half an hour and then port
their own code. For the full v2 surface, see the
[language reference](guide/language-reference.md).

## Quick reference

| Topic | v1 | v2 |
|-------|----|----|
| Primitive types | `int`, `float`, `bool`, `string`, `void` | `Int`, `Float`, `Bool`, `String`, `Unit` |
| Arrays | `int[]`, `string[]` | `List<Int>`, `List<String>` |
| Statement end | `;` required | newline (semicolons optional, rarely used) |
| Program start | top level statements, trailing `main();` | execution begins at `fun main()` |
| Variable type | always annotated | inferred, annotate when needed |
| Constants | none (use `let`) | `const NAME: T = value` |
| Visibility | `export` marks public | no `export`, declared items are importable |
| For loop | `for (let i = 0; i < n; i = i + 1)` | `for i in 0..n` |
| Else chain | `elseif` | `else if` |
| Enum variant | `Color::Red` | `Color.Red` |
| Errors | sentinel returns (`0`, `""`, `-1`) | `Result<T, E>`, `Option<T>`, `?` |
| Strings in text | `format("{}", x)` | `"${x}"` interpolation |
| Imports | `import math;`, `import x from "p"` | `import std/math { abs }`, `import "./x"` |
| Tooling | `rvpm init/run/fmt` | `rvpm init/add/build/run/fmt` plus `rv.toml` |

## Types are PascalCase

Primitive type names changed from lowercase to PascalCase, and `void`
became `Unit`.

v1:

```raven
let name: string = "Raven";
let count: int = 0;
let ratio: float = 0.5;
let ready: bool = true;
```

v2:

```raven
let name: String = "Raven"
let count: Int = 0
let ratio: Float = 0.5
let ready: Bool = true
```

v2 adds `Char` (a single Unicode scalar, written `'x'`) and `Unit` (the
empty value, written `()`). A function with no return type returns `Unit`,
so a v1 `-> void` annotation is dropped entirely in v2.

## No semicolons and no top level code

v1 ended every statement with `;` and ran top level statements directly,
often with a trailing `main();` call. v2 separates statements with
newlines (semicolons are optional and rarely used), has no top level
statement execution, and starts the program at `fun main()`.

v1:

```raven
let message: string = "Hello, Raven!";
print(message);
```

v2:

```raven
fun main() {
    let message = "Hello, Raven!"
    print(message)
}
```

A v1 file that defined `main` and then called it loses the trailing call:

v1:

```raven
fun main() -> void {
    print("hi");
}
main();
```

v2:

```raven
fun main() {
    print("hi")
}
```

## Variables and constants

In v1 every `let` required an explicit type. In v2 the type is inferred
from the initializer; annotate only when it cannot be inferred (for
example an empty list).

v1:

```raven
let count: int = 0;
let names: string[] = [];
```

v2:

```raven
let count = 0
let names: List<String> = []
```

v2 bindings are mutable: you can reassign a `let` and mutate its fields and
elements, the same as v1 `let`. For a module level compile time constant,
v2 adds `const`, which requires both a type and a value:

```raven
const MAX: Int = 100
```

## Functions

v2 keeps the `fun name(params) -> Ret { ... }` shape but uses PascalCase
types, drops `-> void`, and omits semicolons. It also adds an expression
body with `=` for one liners.

v1:

```raven
fun add(a: int, b: int) -> int {
    return a + b;
}

fun greet(name: string) -> void {
    print(name);
}
```

v2:

```raven
fun add(a: Int, b: Int) -> Int {
    return a + b
}

fun greet(name: String) {
    print(name)
}

fun square(n: Int) -> Int = n * n
```

The last block expression is also the return value, so `return` is
optional at the tail of a function:

```raven
fun classify(age: Int) -> String {
    if age < 18 {
        "Too young"
    } else {
        "Adult"
    }
}
```

v2 adds first class closures and function types, which v1 did not have:

```raven
fun apply(f: fun(Int) -> Int, x: Int) -> Int = f(x)

fun main() {
    let factor = 3
    let triple = fun(x: Int) -> Int = x * factor
    print(apply(triple, 7))      // 21
}
```

## Control flow

The C-style `for` is gone. v2 has `for x in <range or list>`. Ranges are
`a..b` (half open, excludes `b`) and `a..=b` (inclusive).

v1:

```raven
let i: int = 0;
while (i < 5) {
    print(i);
    i = i + 1;
}

for (let j: int = 0; j < 5; j = j + 1) {
    print(j);
}
```

v2:

```raven
let i = 0
while i < 5 {
    print(i)
    i = i + 1
}

for j in 0..5 {
    print(j)
}
```

The loop and `if` conditions no longer need parentheses. The v1 `elseif`
keyword becomes two words, `else if`:

v1:

```raven
if (age < 18) {
    print("Too young");
} elseif (age < 30) {
    print("Young adult");
} else {
    print("Mature");
}
```

v2:

```raven
if age < 18 {
    print("Too young")
} else if age < 30 {
    print("Young adult")
} else {
    print("Mature")
}
```

v2 adds two more loop forms. `loop` is an unconditional loop whose value is
the operand of `break`, and `break`/`continue` work in every loop:

```raven
let first = loop {
    break 42
}
```

Unlike v1, `if` is also an expression in v2, so it can produce a value
directly:

```raven
let label = if n > 0 { "positive" } else { "non-positive" }
```

## Collections

v1 had fixed, one type arrays written `int[]`, `string[]`, and so on. v2
replaces them with the generic `List<T>`, and adds `Map<K, V>` and
`Set<T>` from `std/collections`.

v1:

```raven
let numbers: int[] = [1, 2, 3];
let words: string[] = ["a", "b"];
```

v2:

```raven
let numbers: List<Int> = [1, 2, 3]
let words: List<String> = ["a", "b"]
```

`Map` and `Set` come from a module import and are created with their
associated functions:

```raven
import std/collections

fun main() {
    let s = Set.new()
    s.add(1)
    s.add(2)
    s.add(2)
    print(s.len())          // 2

    let m = Map.new()
    m.set("a", 10)
    m.set("b", 20)
    print(m.len())          // 2
    match m.get("a") {
        Some(v) -> print(v)
        None -> print(0)
    }
}
```

`Map.get` returns an `Option`, which is matched rather than compared to a
sentinel. That pattern is the theme of the next section.

## Errors use Result and Option, and there is no null

v1 signaled failure with sentinel return values: `0`, `-1`, an empty
string, or a magic flag the caller had to remember to check. v2 makes
fallibility part of the type. `Result<T, E>` is `Ok(T)` or `Err(E)`,
`Option<T>` is `Some(T)` or `None`, and there is no `null`. The postfix
`?` operator unwraps the success case or returns the failure early.

v1, sentinel style:

```raven
// returns 0 to mean "cannot divide"
fun divide(a: int, b: int) -> int {
    if (b == 0) {
        return 0;
    }
    return a / b;
}
```

v2, typed errors:

```raven
import std/error { error }

fun divide(a: Int, b: Int) -> Result<Int, Error> {
    if b == 0 {
        return Err(error("divide by zero"))
    }
    return Ok(a / b)
}

fun main() {
    match divide(10, 2) {
        Ok(v) -> print(v)
        Err(e) -> print(e.message())
    }
}
```

`?` keeps multi step error handling flat: each call returns early on
`Err`, so the body reads like the happy path.

```raven
fun pipeline(a: Int, b: Int) -> Result<Int, Error> {
    let x = divide(a, b)?
    let y = divide(x, 2)?
    return Ok(y)
}
```

Where v1 might return an empty string for "not found", v2 returns
`Option<T>` and the caller matches it. `T?` is sugar for `Option<T>`.

```raven
fun unwrap_or(x: Option<Int>, fallback: Int) -> Int {
    return match x {
        None -> fallback,
        Some(n) -> n,
    }
}
```

## Strings

v1 built up text with `format("{}", x)` and a handful of built in helpers.
v2 has string interpolation, `"${expr}"`, which embeds any expression, and
moves string operations onto methods.

v1:

```raven
let name: string = "Raven";
print(format("Hello, {}!", name));
print(format("sum is {}", 3 + 4));
```

v2:

```raven
fun main() {
    let name = "Raven"
    print("Hello, ${name}!")
    print("sum is ${3 + 4}")
}
```

String methods such as `to_upper`, `to_lower`, `trim`, `repeat`,
`replace`, `substring`, `contains`, `index_of`, and `concat` live in
`std/string`. A file must `import std/string` to call them, which merges
the `impl String` block so the methods resolve by receiver type.

```raven
import std/string

fun main() {
    print("  hello world  ".trim())
    print("raven".to_upper())
    print("a-b-c".replace("-", "+"))
    print("ab".repeat(3))
    print("hello".substring(1, 4))
    if "hello world".contains("world") {
        print("yes")
    }
}
```

A v2 block string uses triple quotes and is raw (no escapes, newlines
preserved):

```raven
let text = """
line one
line two
"""
```

## Modules and imports

v1 imported a whole module by bare name (`import math;`) or a path string
with `from`. v2 standard library modules live under a `std/...` path, and
imports come in a few shapes.

v1:

```raven
import math;
import str from "str";
import { trim, capitalize } from "str";
```

v2:

```raven
import std/math { abs_int, min_int, max_int }
import std/string
import std/io { println }
import "./helpers" { greet }
import "github.com/martian56/raven-http" as http
```

Forms in v2:

- `import std/io { println }` binds the named free functions directly.
- `import std/string` merges a module that adds methods or constructors
  (for example the `impl String` block), so methods resolve by receiver.
- `import std/collections` is a whole module import; `Map` and `Set` are
  reached as `Map.new()` and `Set.new()`.
- `import "./helpers"` loads a local module relative to the current file.
  A selective form `import "./helpers" { greet, Counter }` binds named
  items from it.
- `import "github.com/<user>/<repo>"` resolves a dependency through the
  rvpm cache; add `as name` for an alias.

The core traits (`ToString`, `Eq`, `Ord`, `Hash`, `Iterator`) are always
in scope without an import.

## What is new in v2

These constructs have no v1 equivalent. They are the reason a v2 port is
worth doing, not just a syntax sweep.

### Enums with payloads and match

v1 enums were plain tags, referenced as `Color::Red`, and converted from
strings with `enum_from_string`. v2 uses `EnumName.Variant`, lets a
variant carry data, and matches with the bare variant name.

```raven
enum Shape {
    Circle(Float),
    Square(Float),
}

fun area(s: Shape) -> Float {
    return match s {
        Circle(r) -> r * r * 3.0,
        Square(w) -> w * w,
    }
}

fun main() {
    print(area(Shape.Circle(2.0)))
}
```

`match` is exhaustive (every case must be covered) and supports literals,
ranges, the wildcard `_`, struct fields, and guards with `if`:

```raven
fun classify(n: Int) -> String {
    return match n {
        0 -> "zero",
        x if x < 0 -> "negative",
        _ -> "positive",
    }
}
```

### Traits and impl

A trait declares methods a type can implement; `impl Trait for Type`
provides the implementation, and `impl Type { ... }` adds inherent methods
and associated functions (the idiomatic constructor, called `Type.new()`).

```raven
struct Point { x: Int, y: Int }

impl ToString for Point {
    fun to_string(self) -> String = "(${self.x}, ${self.y})"
}

struct Counter { n: Int }

impl Counter {
    fun new() -> Counter = Counter { n: 0 }
    fun bump(self) {
        self.n = self.n + 1
    }
}
```

### Generics with bounds, and dyn Trait

Functions, structs, enums, and impl blocks can take type parameters in
angle brackets. A bound `T: Trait` constrains the parameter; use `+` for
several bounds. Generic code is monomorphized per concrete type.

```raven
fun describe<T: ToString>(x: T) -> String = x.to_string()

struct Box<T> {
    value: T
}

impl<T> Box<T> {
    fun unwrap(self) -> T = self.value
}
```

`dyn Trait` is a trait object: one type that holds any implementer,
dispatched at runtime. Use a generic bound when the concrete type is known
at the call site, `dyn Trait` when it is not.

```raven
trait Speak {
    fun sound(self) -> Int
}

fun describe(s: dyn Speak) -> Int = s.sound()
```

### defer

`defer` schedules an expression to run when the enclosing function
returns, in reverse order of registration. It is the v2 way to do cleanup
that v1 had to place by hand at each return.

```raven
fun demo() -> Int {
    defer print(1)
    defer print(2)
    return 0
}
// prints 2 then 1
```

### Lazy iterators

`std/iter` provides lazy adapters (`map`, `filter`) and consumers
(`collect`, `fold`, `count`) over any `Iterator`. `list.iter()` bridges a
`List` into the pipeline.

```raven
import std/iter { collect, fold, count }

fun main() {
    let xs = [1, 2, 3, 4, 5, 6]
    let kept = collect(xs.iter().map(fun(x: Int) -> Int = x * 10).filter(fun(y: Int) -> Bool = y > 20))
    print_int(kept.len())
}
```

### C FFI

`extern "C" { ... }` declares foreign function signatures called like
ordinary functions, using C types (`CInt`, `CLong`, `CSize`, `CStr`,
`CDouble`). A `c"..."` literal produces a `CStr`.

```raven
extern "C" {
    fun abs(x: CInt) -> CInt
    fun strlen(s: CStr) -> CSize
}

fun main() {
    print(abs(-7))               // 7
    print(strlen(c"hello"))      // 5
}
```

To pass a runtime `String` to C, convert it with `std/ffi`'s `to_cstr`; a
native `String` is not itself a valid `const char *`.

## Tooling and packaging

v1 had `rvpm` with `init`, `run`, and `fmt`, where `install` and `add`
were stubs. v2 promotes `rvpm` to a full package manager backed by an
`rv.toml` manifest, an `rv.lock` lock file, and a shared dependency cache.

A v2 package has this layout:

```
my_app/
  rv.toml
  rv.lock              # written by rvpm, pins resolved dependencies
  src/
    main.rv            # must define fun main()
```

The manifest:

```toml
[package]
name = "demo"
version = "0.1.0"
edition = "v2"

[dependencies]
"github.com/martian56/raven-http" = "1.0"

[fmt]
indent_width = 4
wrap_width = 100
```

Commands:

- `rvpm init [name]` scaffolds `rv.toml` and `src/main.rv`.
- `rvpm add github.com/<user>/<repo>@<version>` records a dependency and
  writes `rv.lock`.
- `rvpm install` resolves the manifest against the lock and fills the
  cache.
- `rvpm update [path]` re-resolves and rewrites `rv.lock`.
- `rvpm build` compiles `src/main.rv` to `target/raven-out/<name>`.
- `rvpm run [args]` builds, then runs the produced binary.
- `rvpm fmt` formats the package sources.

The built binary is native: v2 compiles to a real executable rather than
running through a tree walking interpreter.

## Porting the v1 examples

The v2 example programs live under `examples/v2/`. The table maps each
notable v1 example to its v2 form, followed by a few fully worked
translations.

| v1 example | v2 example | Notes |
|------------|-----------|-------|
| `hello.rv` | `examples/v2/hello.rv` | wrapped in `fun main()` |
| `arithmetic.rv` | `examples/v2/arithmetic.rv` | PascalCase types, no semicolons |
| `conditionals.rv` | `examples/v2/conditionals.rv` | `elseif` to `else if`, `if` as expression |
| `boolean_logic.rv` | folded into `examples/v2/comprehensive.rv` | same `&&`, `\|\|`, `!` operators |
| `loops.rv` | `examples/v2/loops.rv` | C-style `for` to `for j in 0..5` |
| `functions.rv` | `examples/v2/functions.rv` | adds `= expr` bodies |
| `enum_demo.rv` | `examples/v2/enum_demo.rv` | `::` to `.`, `match` instead of `enum_from_string` |
| `comprehensive.rv` | `examples/v2/comprehensive.rv` | structs, loops, conditionals together |
| `simple_calculator.rv` | `examples/v2/calculator.rv` | non-interactive, `else if` chain over operators |
| `standard_library_demo.rv` | `examples/v2/standard_library_demo.rv` | `std/...` imports, String methods |
| `builtins_pure.rv` | `examples/v2/list_ops.rv`, `examples/v2/use_string.rv` | length and parsing via methods |
| `builtins_fs.rv` | `examples/v2/use_fs.rv` | `std/fs` returning `Result` |

### Worked: loops

v1:

```raven
let i: int = 0;
while (i < 5) {
    print(i);
    i = i + 1;
}

for (let j: int = 0; j < 5; j = j + 1) {
    print(j);
}
```

v2 (`examples/v2/loops.rv`):

```raven
fun main() {
    let i = 0
    while i < 5 {
        print_int(i)
        i = i + 1
    }

    for j in 0..5 {
        print_int(j)
    }
}
```

### Worked: a calculator

The v1 `simple_calculator.rv` was interactive and used nested `if`/`else`
blocks and `format`. The v2 version dispatches on an operator string with
an `else if` chain.

v1 (the calculation core):

```raven
let result: float = 0.0;
if (operation == "+") {
    result = num1 + num2;
} else {
    if (operation == "-") {
        result = num1 - num2;
    } else {
        if (operation == "*") {
            result = num1 * num2;
        } else {
            if (operation == "/") {
                result = num1 / num2;
            }
        }
    }
}
print(format("Result: {} {} {} = {}", num1, operation, num2, result));
```

v2 (`examples/v2/calculator.rv`):

```raven
fun apply(op: String, a: Float, b: Float) -> Float {
    if op == "+" {
        a + b
    } else if op == "-" {
        a - b
    } else if op == "*" {
        a * b
    } else {
        a / b
    }
}

fun main() {
    let a = 10.0
    let b = 5.0
    print(apply("+", a, b))
    print(apply("-", a, b))
    print(apply("*", a, b))
    print(apply("/", a, b))
}
```

### Worked: an enum demo

v1 used `::` to reach a variant, stored it in a typed binding, and
converted strings to variants with `enum_from_string`.

v1:

```raven
enum HttpStatus {
    OK,
    NotFound,
    InternalError,
    BadRequest
}

fun main() -> void {
    let status: HttpStatus = HttpStatus::OK;
    print(format("Status: {}", status));
    let parsed: HttpStatus = enum_from_string("HttpStatus", "NotFound");
    print(format("Parsed: {}", parsed));
}

main();
```

v2 (`examples/v2/enum_demo.rv`) reaches a variant with `.` and turns a
variant into text with an exhaustive `match`:

```raven
enum HttpStatus {
    Ok,
    NotFound,
    InternalError,
    BadRequest,
}

fun status_name(s: HttpStatus) -> String {
    return match s {
        Ok -> "Ok",
        NotFound -> "NotFound",
        InternalError -> "InternalError",
        BadRequest -> "BadRequest",
    }
}

fun main() {
    print(status_name(HttpStatus.Ok))
    print(status_name(HttpStatus.NotFound))
    print(status_name(HttpStatus.InternalError))
}
```

### Worked: the standard library tour

v1 imported modules by bare name and called free functions through a
namespace (`math.abs`, `str.trim`). v2 imports named items from a
`std/...` path and calls String operations as methods.

v1:

```raven
import math;
import str from "str";

fun main() -> void {
    print(format("abs({}) = {}", -10, math.abs(-10)));
    let text: string = "  hello world  ";
    print(format("Trimmed: '{}'", str.trim(text)));
}

main();
```

v2 (`examples/v2/standard_library_demo.rv`):

```raven
import std/math { abs_int, min_int, max_int, pow_int }
import std/string

fun main() {
    print_int(abs_int(-10))
    print_int(min_int(-10, 15))
    print_int(max_int(-10, 15))
    print("  hello world  ".trim())
    print("raven".to_upper())
}
```

## A porting checklist

When converting a v1 file, work through this list:

1. Wrap top level statements in `fun main()` and delete any trailing
   `main();` call.
2. Replace type names: `int` to `Int`, `float` to `Float`, `bool` to
   `Bool`, `string` to `String`, drop `-> void`.
3. Remove statement terminating semicolons.
4. Change `int[]` and friends to `List<Int>`; create maps and sets with
   `Map.new()` and `Set.new()` after `import std/collections`.
5. Rewrite C-style `for` as `for x in a..b`, and `elseif` as `else if`.
6. Turn `EnumName::Variant` into `EnumName.Variant`, and replace
   `enum_from_string` and tag comparisons with `match`.
7. Replace `format("{}", x)` with `"${x}"`, and route string operations
   through methods after `import std/string`.
8. Replace error sentinels with `Result`/`Option` and the `?` operator;
   remove any use of `null`.
9. Update imports to the `std/...`, `./local`, and `github.com/...`
   forms, dropping `export` and `from`.
10. Move the project under `rvpm`: an `rv.toml` manifest with the entry at
    `src/main.rv`, built with `rvpm build` or run with `rvpm run`.
</content>
