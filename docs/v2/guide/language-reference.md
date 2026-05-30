# Language reference (v2)

This page covers every construct in Raven v2. Statements are separated by
newlines, semicolons, or both. Semicolons are optional and rarely used.
There is no top level statement execution: a program runs from `fun main()`.

## Variables

`let` introduces a binding. Bindings are mutable: you can reassign them
and mutate their fields and elements.

```raven
fun main() {
    let n = 10
    n = n + 5
    print(n)            // 15
}
```

A type annotation is optional. When omitted the type is inferred from the
initializer; annotate when the type cannot be inferred (for example an
empty list).

```raven
let count: Int = 0
let names: List<String> = []
```

`const` is a compile time constant declared at module level. It requires
both a type and a value.

```raven
const MAX: Int = 100
```

## Primitive types

Type names are PascalCase.

| Type     | Meaning |
|----------|---------|
| `Int`    | 64-bit signed integer |
| `Float`  | 64-bit floating point |
| `Bool`   | `true` or `false` |
| `String` | UTF-8 text, heap allocated |
| `Char`   | a single Unicode scalar value |
| `Unit`   | the empty value, written `()` |

```raven
let i: Int = 42
let f: Float = 3.14
let b: Bool = true
let s: String = "raven"
let c: Char = 'x'
```

Integer literals accept bases: `0xff`, `0b1010`, `0o17`, and underscores
for grouping such as `1_000_000`.

## Strings and interpolation

A regular string uses double quotes and processes escapes (`\n`, `\t`,
`\\`, `\"`, `\x41`, `\u{1F600}`). String interpolation embeds an
expression with `${...}`:

```raven
fun main() {
    let name = "Raven"
    let a = 3
    let b = 4
    print("Hello, ${name}!")
    print("sum is ${a + b}")
}
```

A block string uses triple quotes and is raw: no escapes are processed
and newlines are preserved exactly.

```raven
let text = """
line one
line two
"""
```

A C string literal `c"..."` produces a `CStr` for FFI. It lowers to a
pointer to a static null terminated buffer (see [FFI](#ffi-and-c-types)).

## Operators

Arithmetic: `+`, `-`, `*`, `/`, `%`.

Comparison: `==`, `!=`, `<`, `>`, `<=`, `>=`. Comparisons do not chain:
`a < b < c` is an error.

Logical: `&&`, `||`, `!`.

Bitwise: `&`, `|`, `^`, `~`, `<<`, `>>`.

Ranges produce a range value used by `for`. `a..b` is half open (excludes
`b`); `a..=b` is inclusive.

```raven
for x in 0..5 {        // 0, 1, 2, 3, 4
    print(x)
}
```

The postfix `?` operator propagates the error case of a `Result` or the
`None` case of an `Option`, returning early from the enclosing function.

Compound assignment operators apply an operation in place: `+=`, `-=`,
`*=`, `/=`, `%=`, `&=`, `|=`, `^=`, `<<=`, `>>=`.

## Functions

A function declares typed parameters and an optional return type. A
function with no return type returns `Unit`.

```raven
fun add(a: Int, b: Int) -> Int {
    return a + b
}
```

A function may use an expression body with `=`, where the trailing
expression is the return value:

```raven
fun square(x: Int) -> Int = x * x
```

Functions can be generic over type parameters; see [generics](#generics-and-trait-bounds).

## Closures and lambdas

A lambda is written with `fun(params) -> Ret = body` or a block body.
Closures capture surrounding locals by value. A function type is written
`fun(ArgTypes) -> Ret`.

```raven
fun apply(f: fun(Int) -> Int, x: Int) -> Int {
    return f(x)
}

fun main() {
    let factor = 3
    let triple = fun(x: Int) -> Int = x * factor
    print(apply(triple, 7))      // 21
}
```

A closure can be returned, carrying its captured values:

```raven
fun make_adder(n: Int) -> fun(Int) -> Int {
    return fun(x: Int) -> Int = x + n
}
```

## Control flow

`if` / `else if` / `else` chooses a branch. It works as a statement and
as an expression that yields a value:

```raven
let label = if n > 0 { "positive" } else { "non-positive" }
```

`while` loops while a condition holds:

```raven
let i = 0
while i < 10 {
    i = i + 1
}
```

`loop` is an unconditional loop. It evaluates to the operand of `break`:

```raven
let first = loop {
    break 42
}
```

`for ... in` iterates a range or a list:

```raven
let xs = [3, 5, 7]
let total = 0
for v in xs {
    total = total + v
}
```

`break` exits the nearest loop (optionally carrying a value for `loop`),
`continue` skips to the next iteration, and `return` exits the function.

## defer

`defer` schedules an expression to run when the enclosing function
returns. Deferred expressions run in reverse order of registration
(last in, first out), and only those actually reached at runtime run.

```raven
fun demo() -> Int {
    defer print(1)
    defer print(2)
    return 0
}
// prints 2 then 1
```

## Structs

A struct groups named, typed fields. Construct it with a struct literal,
read fields with `.`, and assign to fields and elements.

```raven
struct Point { x: Int, y: Int }

fun main() {
    let p = Point { x: 3, y: 4 }
    print(p.x + p.y)        // 7
    p.x = 10
}
```

Methods are declared in an `impl` block. A method takes `self` as its
first parameter. A function in an `impl` block without `self` is an
associated function, the idiomatic constructor, called as `Type.func()`.

```raven
struct Counter { n: Int }

impl Counter {
    fun new() -> Counter {
        return Counter { n: 0 }
    }

    fun bump(self) {
        self.n = self.n + 1
    }
}

fun main() {
    let c = Counter.new()
    c.bump()
    print(c.n)              // 1
}
```

Methods can also be declared on built in types:

```raven
impl Int {
    fun doubled(self) -> Int = self * 2
}
```

## Enums

An enum defines a set of variants. A variant can be a unit variant, a
tuple variant with positional payloads, or a struct variant with named
fields. Construct a variant with `EnumName.Variant` (or
`EnumName.Variant(args)` for a payload). Match with the bare variant name.

```raven
enum Color {
    Red,
    Green,
    Blue,
}

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
    let c = Color.Green
    print(area(Shape.Circle(2.0)))
}
```

## match

`match` tests a value against patterns top to bottom and yields the
selected arm. Match is exhaustive: every case must be covered. Patterns
include literals, ranges, the wildcard `_`, enum variants binding their
payload, and struct fields. An arm may carry a guard with `if`.

```raven
fun classify(n: Int) -> String {
    return match n {
        0 -> "zero",
        x if x < 0 -> "negative",
        _ -> "positive",
    }
}
```

## Traits and impl

A trait declares methods that a type can implement. Implement it with
`impl Trait for Type`. A trait method may have a default body.

```raven
trait Speak {
    fun sound(self) -> Int
}

struct Dog {}

impl Speak for Dog {
    fun sound(self) -> Int = 1
}
```

`impl Type { ... }` adds inherent methods and associated functions;
`impl Trait for Type { ... }` provides a trait implementation.

## Generics and trait bounds

Functions, structs, enums, and impl blocks can take type parameters in
angle brackets. A bound `T: Trait` constrains a parameter to types that
implement the trait. Use `+` to require several bounds.

```raven
fun show<T: ToString>(label: String, x: T) -> String {
    return "${label}=${x}"
}

struct Box<T> {
    value: T
}

impl<T> Box<T> {
    fun unwrap(self) -> T = self.value
}
```

Generic code is monomorphized: a distinct machine specialization is
emitted for each concrete type the program uses. A method can introduce
its own type parameters separate from the type's:

```raven
impl<T> Box<T> {
    fun mapped<U>(self, f: fun(T) -> U) -> U = f(self.value)
}
```

## Option and Result

`Option<T>` is `Some(T)` or `None`. `Result<T, E>` is `Ok(T)` or
`Err(E)`. Both are matched with their variant names and built with the
bare constructors `Some`, `None`, `Ok`, and `Err`. The type `T?` is sugar
for `Option<T>`.

```raven
fun divide(a: Int, b: Int) -> Result<Int, Error> {
    if b == 0 {
        return Err(error("divide by zero"))
    }
    return Ok(a / b)
}

fun unwrap_or(x: Option<Int>, fallback: Int) -> Int {
    return match x {
        None -> fallback,
        Some(n) -> n,
    }
}
```

The `?` operator unwraps `Ok`/`Some` or returns the `Err`/`None` early,
which keeps error handling flat. `error` and the `Result` helpers live in
[`std/error`](standard-library.md#stderror).

## dyn Trait

`dyn Trait` is a trait object: a value of any type that implements the
trait, dispatched at runtime through a vtable. Passing a concrete value
where `dyn Trait` is expected boxes it as a fat pointer.

```raven
trait Speak {
    fun sound(self) -> Int
}

fun describe(s: dyn Speak) -> Int = s.sound()
```

Use generics with a bound when the concrete type is known at the call
site (no indirection), and `dyn Trait` when you need a single type that
holds different implementers.

## Modules and imports

`import` brings in a module. Standard library modules use the `std/...`
path. A selective import binds named items; a bare import merges the
module (used for modules that add methods or constructors).

```raven
import std/io { println }
import std/collections
import "./helpers"
import "github.com/martian56/raven-http" as http
```

Forms:

- `import std/io { println }` binds the named items directly.
- `import std/string` merges the module's `impl String` block so String
  methods resolve by receiver type.
- `import std/collections` is a whole module import; `Map` and `Set` are
  reached as `Map.new()` and `Set.new()` rather than through a selector.
- `import "./helpers"` loads a local module relative to the current file.
- `import "github.com/<user>/<repo>"` resolves a dependency through the
  rvpm cache (see the [rvpm guide](rvpm.md)). Add `as name` for an alias.

The core traits (`ToString`, `Eq`, `Ord`, `Hash`, `Iterator`) are always
in scope without an import.

## FFI and C types

`extern "C" { ... }` declares foreign function signatures. Call them like
ordinary functions. Arguments and returns use C types.

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

C types map to C as follows:

| Raven   | C              | Width |
|---------|----------------|-------|
| `CInt`  | `int`          | 32-bit |
| `CLong` | `long`         | 64-bit |
| `CSize` | `size_t`       | pointer width (64-bit) |
| `CStr`  | `const char *` | pointer width |
| `CFloat` | `float`       | 32-bit |
| `CDouble` | `double`     | 64-bit |

A native `Int` is accepted where an integer C type is expected, and a
`c"..."` literal where a `CStr` is expected. A native `Float` is accepted
where a `CFloat` or `CDouble` is expected; for `CFloat` the value is
narrowed to f32 at the call and a `CFloat` return is widened back to a
`Float`. To pass a runtime `String`
to C, convert it with [`std/ffi`](standard-library.md#stdffi)'s
`to_cstr`; a native `String` is not itself a valid `const char *`.
