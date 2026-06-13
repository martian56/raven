# Language reference (v2)

This page covers every construct in Raven v2. Statements are separated by
newlines, semicolons, or both. Semicolons are optional and rarely used.
There is no top level statement execution: a program runs from `fun main()`.

## Variables

`let` introduces a binding. Bindings are mutable: you can reassign them
and mutate their fields and elements.

```rust
fun main() {
    let n = 10
    n = n + 5
    print(n)            // 15
}
```

A type annotation is optional. When omitted the type is inferred from the
initializer; annotate when the type cannot be inferred (for example an
empty list).

```rust
let count: Int = 0
let names: List<String> = []
```

A `let` at module level (outside any function) is a mutable global: any
function can read and reassign it. Globals are initialized before `main`
runs, in declaration order, so a later global may read an earlier one, and
an initializer may be any expression (including a function call), not only a
constant. A heap-valued global (a `String`, `List`, struct, and so on) is
kept alive for the whole program.

```rust
fun seed() -> Int = 10

let counter: Int = 0        // mutable, shared across functions
let base: Int = seed() * 3  // initialized by a call, before main
let names: List<String> = []

fun record(name: String) {
    names.push(name)
    counter = counter + 1
}
```

`const` introduces an immutable binding: reassigning it (or compound
assigning, like `+=`) is a compile error.

```rust
fun main() {
    const LIMIT = 5
    LIMIT = 6            // error: cannot assign to `LIMIT`, it is a `const`
}
```

At module level a `const` is a compile-time constant: it requires both a
type and a value, and its initializer must be a constant expression (a
literal, or an arithmetic, comparison, bitwise, or boolean combination of
literals), which is folded and inlined at each use site.

```rust
const MAX: Int = 100
const SECS_PER_HOUR: Int = 60 * 60
```

Inside a function body a `const` is an immutable local. It has stack
storage, so its initializer may be any expression (including a function
call), not only a constant one.

```rust
fun main() {
    const DOUBLED = compute()    // runtime value, still immutable
}
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

```rust
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

```rust
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

```rust
let text = """
line one
line two
"""
```

A C string literal `c"..."` produces a `CStr` for FFI. It lowers to a
pointer to a static null terminated buffer (see [FFI](#ffi-and-c-types)).

## Operators

Arithmetic: `+`, `-`, `*`, `/`, `%`.

Comparison: `==`, `!=`, `<`, `>`, `<=`, `>=`. Ordering (`<`, `<=`, `>`,
`>=`) works on `Int`, `Float`, `Char`, and `String` (lexicographic, by
bytes); `==`/`!=` work on any type. Comparisons do not chain: `a < b < c`
is an error.

Logical: `&&`, `||`, `!`.

Bitwise: `&`, `|`, `^`, `~`, `<<`, `>>`.

Ranges produce a range value used by `for`. `a..b` is half open (excludes
`b`); `a..=b` is inclusive.

```rust
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

```rust
fun add(a: Int, b: Int) -> Int {
    return a + b
}
```

A function may use an expression body with `=`, where the trailing
expression is the return value:

```rust
fun square(x: Int) -> Int = x * x
```

Functions can be generic over type parameters; see [generics](#generics-and-trait-bounds).

## Closures and lambdas

A lambda is written with `fun(params) -> Ret = body` or a block body.
Closures capture surrounding locals by value. A function type is written
`fun(ArgTypes) -> Ret`.

```rust
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

```rust
fun make_adder(n: Int) -> fun(Int) -> Int {
    return fun(x: Int) -> Int = x + n
}
```

## Control flow

`if` / `else if` / `else` chooses a branch. It works as a statement and
as an expression that yields a value:

```rust
let label = if n > 0 { "positive" } else { "non-positive" }
```

`while` loops while a condition holds:

```rust
let i = 0
while i < 10 {
    i = i + 1
}
```

`loop` is an unconditional loop. It evaluates to the operand of `break`:

```rust
let first = loop {
    break 42
}
```

`for ... in` iterates a range or a list:

```rust
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
returns. This is function-scoped, like Go's `defer`, not block-scoped: a
`defer` written inside a nested block, an `if`, or a loop still runs at
the function's return, not when the inner block exits. Deferred
expressions run in reverse order of registration (last in, first out),
and only those actually reached at runtime run.

```rust
fun demo() -> Int {
    defer print(1)
    defer print(2)
    return 0
}
// prints 2 then 1
```

Because a defer is function-scoped, the order across nested blocks is the
same LIFO order, measured by when each `defer` statement ran:

```rust
fun f() -> Int {
    print(1)
    if true {
        defer print(2)
        print(3)
    }
    print(4)
    return 0
}
// prints 1 3 4 2: the nested defer fires at f's return, after 4
```

On `return e` the return value is computed first, then the deferred
thunks run, then the function returns. A deferred expression runs for its
side effects and cannot change the return value. Defers do not run on a
`panic`, which aborts the process without unwinding.

## Structs

A struct groups named, typed fields. Construct it with a struct literal,
read fields with `.`, and assign to fields and elements.

```rust
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

```rust
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

```rust
impl Int {
    fun doubled(self) -> Int = self * 2
}
```

## Enums

An enum defines a set of variants. A variant can be a unit variant, a
tuple variant with positional payloads, or a struct variant with named
fields. Construct a variant with `EnumName.Variant` (or
`EnumName.Variant(args)` for a payload). Match with the bare variant name.

```rust
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

You construct a variant only through the qualified form. A bare `Green` or
`Circle(2.0)` in expression position is not a constructor yet; it is read
as a name and fails to resolve. The bare names appear only as match
patterns.

## match

`match` tests a value against patterns top to bottom and yields the
selected arm. Match is exhaustive: every case must be covered. Patterns
include literals, ranges, the wildcard `_`, enum variants binding their
payload, and struct fields. An arm may carry a guard with `if`.

```rust
fun classify(n: Int) -> String {
    return match n {
        0 -> "zero",
        x if x < 0 -> "negative",
        _ -> "positive",
    }
}
```

## List, set, and map literals

A list literal is comma-separated values in brackets, `[1, 2, 3]`, and an
empty list is `[]`. Lists are built in: they index, grow with `push`, and
report `len()`.

A set literal is comma-separated values in braces, `{1, 2, 3}`. A map
literal is comma-separated `key: value` pairs in brackets, `["a": 1,
"b": 2]`. Both come from `std/collections`, so the literals need
`import std/collections` in scope (see the [standard library](standard-library.md#stdcollections)).

```rust
import std/collections

fun main() {
    let s = {1, 2, 2, 3}        // Set<Int>, dedups to {1, 2, 3}
    let m = ["a": 1, "b": 2]    // Map<String, Int>
    print(s.len())              // 3
    print(m.len())              // 2
}
```

The brace and bracket forms overlap with blocks and lists, so a few rules
disambiguate them:

- A set literal needs at least one comma, so a single-element set is
  written with a trailing comma, `{x,}`. A bare `{ x }` is a block whose
  tail expression is `x`, and `{}` is an empty block. An empty set is
  `Set.new()`.
- The empty map is the distinct `[:]` form, since a bare `[]` is an empty
  list. A bracket whose first element has a top-level `:` is a map; one
  without is a list.

A set's element type and a map's key type must implement `Eq + Hash`,
the same bound the collection types carry.

## Traits and impl

A trait declares methods that a type can implement. Implement it with
`impl Trait for Type`. A trait method may have a default body.

```rust
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

```rust
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

```rust
impl<T> Box<T> {
    fun mapped<U>(self, f: fun(T) -> U) -> U = f(self.value)
}
```

## Option and Result

`Option<T>` is `Some(T)` or `None`. `Result<T, E>` is `Ok(T)` or
`Err(E)`. Both are matched with their variant names and built with the
bare constructors `Some`, `None`, `Ok`, and `Err`. The type `T?` is sugar
for `Option<T>`.

```rust
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

```rust
trait Speak {
    fun sound(self) -> Int
}

fun describe(s: dyn Speak) -> Int = s.sound()
```

Use generics with a bound when the concrete type is known at the call
site (no indirection), and `dyn Trait` when you need a single type that
holds different implementers.

## Concurrency

`spawn` starts a goroutine: a lightweight green thread that runs a
`fun() -> Unit` closure. Goroutines run on a cooperative scheduler. The
program multiplexes many of them onto one OS thread, and exactly one runs
at a time. A goroutine runs until it reaches a yield point, then the
scheduler resumes another ready goroutine.

```rust
spawn(fun() -> Unit {
    // goroutine body
})
```

Goroutines communicate over channels from `std/sync`. `channel()` makes
an unbuffered (rendezvous) channel, and `channel_buffered(cap)` makes a
buffered one. `ch.send(v)` sends a value and `ch.recv()` receives one.
A send on a full channel and a recv on an empty one block the goroutine,
yielding to the scheduler until the counterpart operation runs.
`yield_now()` yields explicitly. Channels carry `Int` values in this
release.

```rust
import std/sync { channel }

fun main() {
    let ch = channel()
    spawn(fun() -> Unit {
        let i = 1
        while i <= 5 {
            ch.send(i)
            i = i + 1
        }
    })
    let sum = 0
    let n = 0
    while n < 5 {
        sum = sum + ch.recv()
        n = n + 1
    }
    print(sum)              // 15
}
```

When `main` returns the program exits, and any goroutines still alive are
abandoned. If every goroutine is blocked with none ready, the scheduler
reports a deadlock and exits.

The model is cooperative on a single OS thread in this release: there is
no preemption and no multicore parallelism. A goroutine that blocks in a
runtime IO call (a net read, a file read, an http request) stalls the
whole scheduler, since those calls are synchronous. True multicore
parallelism, `select`, and non-blocking IO are future work.

## Modules and imports

`import` brings in a module. Standard library modules use the `std/...`
path. A selective import binds named items; a bare import merges the
module (used for modules that add methods or constructors).

```rust
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
  rvpm cache (see the [rvpm guide](rvpm.md)).

### Renaming an import

A selector can be renamed with `as`, binding it under a different local
name. This is how you pull in two items that would otherwise share a name.
If two packages both export a `parse` function, rename one or both and the
clash goes away:

```rust
import "github.com/martian56/raven-toml" { parse as parse_toml }
import "github.com/martian56/raven-ini"  { parse as parse_ini }

let cfg = parse_toml(toml_text)
let ini = parse_ini(ini_text)
```

### Calling through a module alias

A whole stdlib import already allows module-qualified calls: after
`import std/fs`, `fs.write(path, data)` is the same function a selector
would bind as a bare `write`. The `as name` alias extends that to local and
package imports, so you can reach a package's functions namespace-style
without listing each one:

```rust
import "github.com/martian56/raven-color" as color

print(color.red("error"))     // same as importing { red } and calling red(...)
```

### Two packages, same type name

Types from different packages are namespaced, so two packages can both
export a type called `Table` without colliding. To use both in one file,
rename one on the way in:

```rust
import "github.com/martian56/raven-table" { Table }
import "github.com/martian56/raven-csv"   { Table as CsvTable }
```

The core traits (`ToString`, `Eq`, `Ord`, `Hash`, `Iterator`) are always
in scope without an import.

## FFI and C types

`extern "C" { ... }` declares foreign function signatures. Call them like
ordinary functions. Arguments and returns use C types.

```rust
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
| `CPtr<T>` | `T *`        | pointer width |
| `CFnPtr` | function pointer | pointer width |

A native `Int` is accepted where an integer C type is expected, and a
`c"..."` literal where a `CStr` is expected. A native `Float` is accepted
where a `CFloat` or `CDouble` is expected; for `CFloat` the value is
narrowed to f32 at the call and a `CFloat` return is widened back to a
`Float`. The integer and float C return types satisfy `ToString`, so a
`CInt` or `CDouble` result prints and interpolates directly.

```rust
extern "C" {
    fun sqrtf(x: CFloat) -> CFloat
}

fun main() {
    print(sqrtf(16.0))           // 4
}
```

### Strings across the boundary

To pass a runtime `String` to C, convert it with
[`std/ffi`](standard-library.md#stdffi)'s `to_cstr`; a native `String` is
not itself a valid `const char *`. `from_cstr` reads a `CStr` back into a
`String`. `to_cstr` copies into a buffer outside the GC and does not free
it, so it leaks one buffer per call; hoist the conversion out of a hot
loop.

```rust
import std/ffi { to_cstr, from_cstr }

extern "C" {
    fun strlen(s: CStr) -> CSize
}

fun main() {
    print(strlen(to_cstr("hello")))          // 5
    print(from_cstr(to_cstr("roundtrip")))   // roundtrip
}
```

### Raw pointers

`CPtr<T>` is a usable raw pointer. `std/ffi` reads and writes C memory
through it: `alloc<T>(count)` reserves a buffer, `free<T>(p)` releases it,
`load<T>(p)` and `store<T>(p, v)` read and write the element at `p`,
`offset<T>(p, i)` advances by `i` elements (scaled by `sizeof(T)`),
`null_ptr<T>()` is the null pointer, and `is_null<T>(p)` tests it.

```rust
import std/ffi { alloc, free, load, store, offset, is_null, null_ptr }

fun main() {
    let buf = alloc<CInt>(4)
    store<CInt>(buf, 10)
    store<CInt>(offset<CInt>(buf, 1), 20)
    print(load<CInt>(buf))                   // 10
    print(load<CInt>(offset<CInt>(buf, 1)))  // 20
    print(is_null<CInt>(null_ptr<CInt>()))   // true
    free<CInt>(buf)
}
```

This memory lives outside the garbage collector. It is never traced or
reclaimed automatically, so the caller owns it and must `free` it. There
are no bounds, null, or use-after-free checks: an out-of-range `offset`
or a `load` through a freed pointer is undefined behavior, exactly as in
C. `T` must be a C scalar (`CInt`, `CLong`, `CSize`, `CFloat`, `CDouble`,
`CStr`) or a native `Int`/`Float`.

### Callbacks

`CFnPtr` is an untyped C function pointer. A C function that takes a
callback can call back into Raven through one. Pass a non-capturing
top-level function by naming it bare. Its parameters and return must all
be C-FFI types so the C ABI is well defined. A capturing closure (a local
of function type) is rejected, since C cannot supply its capture
environment.

```rust
import std/ffi { alloc, free, load, store, offset }

extern "C" {
    fun raven_ffi_qsort_i32(p: CPtr<CInt>, n: CSize, cmp: CFnPtr)
}

fun compare(a: CPtr<CInt>, b: CPtr<CInt>) -> CInt {
    return load<CInt>(a) - load<CInt>(b)
}

fun main() {
    let buf = alloc<CInt>(3)
    store<CInt>(buf, 30)
    store<CInt>(offset<CInt>(buf, 1), 10)
    store<CInt>(offset<CInt>(buf, 2), 20)
    raven_ffi_qsort_i32(buf, 3, compare)
    print(load<CInt>(buf))                   // 10
    free<CInt>(buf)
}
```

`CFnPtr` is untyped: the type checker does not verify the function's
signature against what the C side expects. Matching it is your
responsibility, as in C.

### Small structs by value

A small C struct can cross the boundary by value. Mark the matching Raven
struct `@repr(C)` to give it C memory layout. The supported shape is a
struct whose fields are all integer-class C scalars (`CInt`, `CLong`,
`CSize`, `CStr`, `CPtr<T>`, or `CFnPtr`) and whose total size is at most
8 bytes (one machine register). A larger struct, or one with a float
field, is rejected; pass a `CPtr<...>` to it instead.

```rust
@repr(C)
struct Point {
    x: CInt
    y: CInt
}

extern "C" {
    fun raven_ffi_point_sum(p: Point) -> CInt
    fun raven_ffi_translate(p: Point, dx: CInt, dy: CInt) -> Point
}

fun main() {
    let p = Point { x: 3, y: 4 }
    print(raven_ffi_point_sum(p))            // 7
    let q = raven_ffi_translate(p, 1, 2)     // {4, 6}
    print(q.x)                               // 4
    print(q.y)                               // 6
}
```

The fields stay readable on the Raven side (`q.x`); only the call
boundary marshals the struct by value.

## Metaprogramming

Raven has three metaprogramming tools: `@derive` to synthesize trait
impls, declarative macros to rewrite call sites, and reflection to read
type information.

### @derive

`@derive(...)` sits on its own line before a `struct` or `enum` and
synthesizes trait impls from the type definition, so you do not hand write
`equals`, `hash`, `to_string`, or `debug`. The derivable traits are `Eq`,
`Hash`, `ToString`, `Debug`, `ToJson`, and `FromJson`. A field or payload
type must itself implement the trait being derived.

```rust
import std/collections { Map, Set }

@derive(Eq, Hash, ToString, Debug)
struct Point { x: Int, y: Int }

fun main() {
    let p = Point { x: 1, y: 2 }
    let q = Point { x: 1, y: 2 }
    print(p.equals(q))          // true
    print(p.to_string())        // Point { x: 1, y: 2 }

    // Derived Eq + Hash let the struct key a Map or join a Set.
    let m: Map<Point, Int> = Map.new()
    m.set(p, 100)
    print(m.has(q))             // true
    let s: Set<Point> = Set.new()
    s.add(p)
    print(s.contains(q))        // true
}
```

`@derive(ToJson, FromJson)` adds JSON serialization built on `std/json`.
`to_json` produces a `JsonValue`, and `from_json` is an associated
function that decodes one back, returning a `Result`. Combine `to_json`
with `stringify`, and `parse` with `from_json`, for a round trip.

```rust
import std/json { JsonValue, stringify, parse }

@derive(ToJson, FromJson, Eq)
struct User { id: Int, name: String }

fun main() {
    let u = User { id: 7, name: "ada" }
    let text = stringify(u.to_json())
    print(text)                          // {"id":7,"name":"ada"} (key order is the map layout)
    match parse(text) {
        Ok(v) -> {
            match User.from_json(v) {
                Ok(u2) -> print(u.equals(u2)),   // true
                Err(e) -> print("decode error"),
            }
        },
        Err(e) -> print("parse error"),
    }
}
```

Object key order follows the map hash-bucket layout, not source order.
Enum variants with named-field payloads are not derivable yet; unit and
tuple variants are. `Ord` is not derivable yet.

### Declarative macros

A `macro` definition lists one or more rules, each a token matcher and a
template, and a `name!(...)` call is rewritten by matching the argument
tokens against the first matching rule and splicing the captures into the
template. Macros expand before parsing, in expression position.

A matcher binds metavariables: `$x:expr` captures a balanced expression
and `$x:ident` captures one identifier. The template splices `$x` back in.
Wrap each splice in parentheses where precedence matters.

```rust
macro twice { ($x:expr) => { ($x) + ($x) } }

fun main() {
    let n = 3
    print(twice!(n + 1))        // 8
}
```

A repetition group `$(...)*` (zero or more) or `$(...)+` (one or more)
matches a sub-pattern several times, with an optional separator between
the closing `)` and the marker. In the template it expands once per
capture.

```rust
macro sum_all { ($($x:expr),*) => { (0 $(+ ($x))*) } }

fun main() {
    print(sum_all!(1, 2, 3))    // 6
    print(sum_all!())           // 0
    print(sum_all!(10))         // 10
}
```

A name a template introduces at a `let`, `const`, or `for` binding is
renamed to a fresh name, so a template temporary cannot collide with or
capture a caller's variable of the same spelling.

### Reflection

Compile-time reflection reads type information resolved while the program
compiles. `type_name<T>()` returns the rendered name of a type,
`field_names<T>()` returns a struct's field names in declaration order, and
`field_types<T>()` returns the matching field type names by position. Inside
a generic function each resolves to the concrete type bound to `T` at that
instantiation.

```rust
struct Point { x: Int, y: Int }

fun introspect<T>() {
    print("type ${type_name<T>()}")
    let names = field_names<T>()
    let types = field_types<T>()
    let i = 0
    while i < names.len() {
        print("field ${names[i]}: ${types[i]}")
        i += 1
    }
}

fun main() {
    print(type_name<Int>())     // Int
    introspect<Point>()         // type Point, field x: Int, field y: Int
}
```

For enums, `variant_names<T>()` lists the variant names in declaration order
and `variant_field_types<T>()` gives each variant's payload type names as an
inner list (empty for a unit variant), so the inner length is the variant's
payload arity.

```rust
enum Shape { Circle(radius: Float) Rectangle(w: Float, h: Float) Dot }

variant_names<Shape>()        // ["Circle", "Rectangle", "Dot"]
variant_field_types<Shape>()  // [["Float"], ["Float", "Float"], []]
```

Runtime reflection works over a value whose type is not known statically.
`to_any<T>(v)` boxes a value into an `Any`. Over an `Any`,
`type_name_of(a)` reads its runtime type name, `field_names_of(a)` lists
its struct fields, `get_field(a, name)` reads one field back as an
`Option<Any>`, and `cast<T>(a)` recovers a concrete value as an
`Option<T>` (`None` for the wrong `T`).

```rust
struct User { id: Int, name: String }

fun describe(a: Any) {
    print("type: ${type_name_of(a)}")
    for f in field_names_of(a) {
        match get_field(a, f) {
            Some(v) -> {
                match cast<Int>(v) {
                    Some(n) -> print("field ${f} = ${n}"),
                    None -> {
                        match cast<String>(v) {
                            Some(s) -> print("field ${f} = ${s}"),
                            None -> print("field ${f} = ?"),
                        }
                    },
                }
            },
            None -> print("field ${f}: missing"),
        }
    }
}

fun main() {
    let u = User { id: 7, name: "ada" }
    describe(to_any<User>(u))
    // type: User, field id = 7, field name = ada
}
```
