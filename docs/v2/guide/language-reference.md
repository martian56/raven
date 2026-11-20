# Language reference (v2)

Raven reads like Python with braces. Statements end at a newline (a `;` is
allowed but rarely needed), blocks use `{ }`, and types are checked at
compile time but mostly inferred. A program starts at `fun main()`.

```rust
from std.math import sqrt

class Point {
    x: float = 0.0
    y: float = 0.0

    fun len(self) -> float = sqrt(self.x * self.x + self.y * self.y)
}

fun main() {
    let p = Point(x: 3.0, y: 4.0)
    print("length ${p.len()}")
}
```

## Variables

`let` introduces a binding that cannot be reassigned. `var` introduces one
that can.

```rust
fun main() {
    let name = "raven"
    var count = 0
    count += 1
    print("${name} ${count}")
}
```

Assigning to a `let` binding is a compile error that suggests `var`.
`let` freezes the binding, not the value: a `let` list can still be pushed
to, and a `let` object's fields can still be set.

Types are inferred from the initializer, including from later use: an empty
list takes its element type from what is pushed into it. Annotate when you
want to be explicit.

```rust
let count: int = 0
var names = []
names.push("ann") // names is list[str]
```

At module level (outside any function), `let` and `var` declare globals,
initialized before `main` in declaration order. `const` declares a
compile-time constant whose initializer is a literal or arithmetic over
literals; it is inlined at each use.

```rust
let MAX: int = 100
var counter = 0

fun record() {
    counter += 1
}
```

## Built-in types

| Type | Meaning |
|------|---------|
| `int` | 64-bit signed integer |
| `float` | 64-bit floating point |
| `bool` | `true` or `false` |
| `str` | heap-allocated byte string; literals are UTF-8 |
| `char` | a single Unicode scalar value |
| `list[T]` | growable list |
| `dict[K, V]` | hash map (`K` must implement `Eq` and `Hash`) |
| `set[T]` | hash set |
| `Option[T]` | `Some(value)` or `None` |
| `Result[T, E]` | `Ok(value)` or `Err(error)` |
| `(A, B)` | tuple of up to six values |

`T?` is shorthand for `Option[T]`. Integer literals accept `0xff`, `0b1010`,
`0o17`, and `_` separators such as `1_000_000`.

There are no implicit numeric conversions. Convert with `x as float`,
`x as int` (truncates toward zero), and `x as str`, or with the builtins
`float(x)`, `int(x)`, and `str(x)`. `int("42")` and `float("3.5")` parse text
and panic on bad input; `"42".parse_int()` and `"3.5".parse_float()` return an
`Option` instead. A float always prints with its point: `5.0`.

## Tuples

```rust
fun divmod(a: int, b: int) -> (int, int) {
    return (a / b, a % b)
}

let (q, r) = divmod(17, 5)
let pair = ("ada", 36)
print(pair.0, pair.1)
var a = 1
var b = 2
(a, b) = (b, a)
```

Tuples compare, hash, order, and print by their items, so they work as dict
keys and sort naturally. `()` is the unit value.

## Strings

Strings use double quotes and support escapes (`\n`, `\t`, `\\`, `\"`,
`\u{1F600}`). `${expr}` interpolates any value that implements `ToString`,
which includes every built-in type and, automatically, your own classes.

```rust
let name = "Raven"
print("Hello, ${name}! 2 + 2 = ${2 + 2}")
print("${3.14159:.2} [${name:>8}] [${7:03}]") // 3.14 [   Raven] [007]
```

A format spec after `:` takes an optional fill and alignment (`<`, `>`, `^`),
a `0` for zero padding, a width, and a precision: `${x:.2}`, `${s:<10}`,
`${n:*^9}`.

`+` joins strings, `+=` appends, and `"ab" * 3` repeats. `s[1..4]` slices by
byte offset, `"ell" in s` tests for a substring, and `for c in s` visits each
character. String methods (`len`, `split`, `strip`, `upper`, `lower`,
`replace`, `startswith`, `endswith`, `join`, `index_of`, `parse_int`, and
more) need no import; `" ".join(parts)` joins a list. Triple-quoted strings
(`"""..."""`) are raw and keep newlines.

## Operators

- Arithmetic: `+ - * / %`
- Comparison: `== != < <= > >=`. Comparisons do not chain.
- Logic: `and`, `or`, `not`
- Membership: `x in xs` and `x not in xs` (lists, sets, dict keys, substrings)
- Conversion: `x as int`, `x as float`, `x as str`
- Bitwise: `& | ^ << >>`
- Compound assignment: `+= -= *= /= %= &= |= ^= <<= >>=`
- Conditional value: `a if cond else b`
- Ranges: `a..b` (excludes `b`) and `a..=b` (includes `b`)
- `?` propagates an `Err` or `None` out of the current function

`==` compares values. Every class and enum whose fields can be compared gets
`==` automatically; a type holding a function or trait object needs a
written `impl Eq` before it can be compared.

A class supports an operator by defining its method: `add`, `sub`, `mul`,
`div`, `mod`, and `neg` for arithmetic, `index` and `set_index` for `x[i]`
and `x[i] = v`. Implementing `Ord` (its `compare` method) gives `<`, `<=`,
`>`, and `>=`.

```rust
class Vec2 {
    x: float
    y: float

    fun add(self, o: Vec2) -> Vec2 = Vec2(self.x + o.x, self.y + o.y)

    fun mul(self, k: float) -> Vec2 = Vec2(self.x * k, self.y * k)
}

let v = Vec2(1.0, 2.0) + Vec2(3.0, 4.0) * 2.0
```

## Functions

```rust
fun add(a: int, b: int) -> int {
    return a + b
}

fun square(x: int) -> int = x * x
```

Parameters can have default values, and callers can pass arguments by name:

```rust
fun greet(name: str, greeting: str = "hello", punct: str = "!") -> str {
    return greeting + " " + name + punct
}

fun main() {
    print(greet("ann"))
    print(greet("bob", punct: "?"))
}
```

Every path through a function with a return type must return a value.
`return`, `break`, `continue`, and `panic(msg)` never produce a value, so they
fit anywhere a value is expected.

## Lambdas

A lambda is `params => body`. Parameter types are inferred from where the
lambda is used; write them when there is nothing to infer from.

```rust
let doubled = [1, 2, 3].map(x => x * 2)
let total = [1, 2, 3].fold(0, (acc, x) => acc + x)
let clamp = (x: int) => {
    if x > 100 {
        return 100
    }
    return x
}
```

`return` in a block-bodied lambda leaves the lambda. Lambdas capture the
values of surrounding locals when they are created. A function type is
written `fun(int) -> int`.

## Control flow

```rust
if n > 0 {
    print("positive")
} else if n < 0 {
    print("negative")
} else {
    print("zero")
}

let label = "big" if n > 100 else "small"
```

Loops:

```rust
for x in [3, 5, 7] {
    print(x)
}
for i in 0..10 {
    print(i)
}
while queue.len() > 0 {
    queue.pop()
}
let first = loop {
    break 42
}
```

`for` iterates lists, ranges, and anything with an `iter()`. `loop` yields
the value its `break` carries.

`if let`, `while let`, and `let ... else` test a value against a pattern:

```rust
if let Some(user) = find(id) {
    print(user.name)
}

while let Some(job) = jobs.pop_next() {
    run(job)
}

let port = match text.parse_int() {
    case Some(port) { port }
    case _ {
        return Err(error("bad port"))
    }
}
```

The `else` block of a `let ... else` must leave (`return`, `break`,
`continue`, or `panic`).

## assert

`assert cond` stops the program when `cond` is false. The failure message is
the condition as written; a message after a comma replaces it. No import is
needed.

```rust
fun mean(xs: list[float]) -> float {
    assert len(xs) > 0, "mean of an empty list"
    return sum(xs) / float(len(xs))
}

fun main() {
    assert mean([3.0, 4.0, 5.0]) == 4.0
}
```

A failed `assert x == 5` prints `raven panic: assertion failed: x == 5`.

## defer

`defer expr` runs `expr` when the function returns, in reverse order of
registration. A `defer` inside a nested block still runs at the function's
return. Defers do not run on `panic`.

## Classes

A class has fields and methods. Fields may have default values. Methods take
`self` explicitly; a function without `self` is static.

```rust
class Counter {
    count: int = 0
    step: int = 1

    fun origin() -> Counter = Counter()

    fun bump(self) {
        self.count += self.step
    }
}

fun main() {
    let c = Counter(step: 5)
    c.bump()
    print(c.count) // 5
    print(Counter.origin().count)
}
```

Construct with positional arguments in field order, with named arguments, or
both. Omitted fields take their defaults. `..base` copies the remaining
fields from another value:

```rust
let p = Point(1.0, 2.0)
let q = Point(x: 1.0)
let r = Point(..p, y: 9.0)
```

A class value is a reference: assigning it shares the object. Classes print
and compare by value automatically when their fields allow it.

`impl Name { ... }` adds methods to any type, including built-in ones:

```rust
impl int {
    fun doubled(self) -> int = self * 2
}
```

## Enums and match

```rust
enum Shape {
    Circle(r: float),
    Rect(w: float, h: float),
    Dot,
}

fun area(s: Shape) -> float {
    match s {
        case Circle(r) {
            return 3.14159 * r * r
        }
        case Rect(w, h) {
            return w * h
        }
        case Dot {
            return 0.0
        }
    }
}
```

Variants are constructed with their enum name: `Shape.Circle(2.0)`. Patterns
may name the variant with or without the enum. `match` must cover every case,
and a capitalized name in a pattern must be a variant, so a typo is an error
instead of a silent catch-all.

Patterns: literals, ranges (`1..=9`), `_`, bindings, variants with payloads,
tuples (`case (0, y) { ... }`), class fields (`case Point { x, y } { ... }`),
and guards (`case n if n > 0 { ... }`). Patterns nest to any depth, and the
exhaustiveness check looks inside them:

```rust
fun describe(r: Option[Result[int, str]]) -> str {
    match r {
        case Some(Ok(0)) { return "zero" }
        case Some(Ok(n)) { return "ok ${n}" }
        case Some(Err(e)) { return "error ${e}" }
        case None { return "nothing" }
    }
}
```

A `match` can also produce a value when every case block ends in one.

## Collections

```rust
let xs = [1, 2, 3]
let ages = {"ann": 31, "bob": 27}
let tags = {"a", "b"}
let empty_dict = {}
let empty_set = set()
```

`xs[-1]` is the last item and `xs[1..3]` a slice; `xs + ys` joins two lists
and `[0] * n` repeats one. `d[k]` reads a dict (and panics on a missing key;
`d.get(k)` returns an `Option`), and `d[k] = v` and `d[k] += 1` write it.
`for (k, v) in d` visits a dict's pairs and `for x in s` a set's items, both
in insertion order.

Comprehensions build a collection from a loop:

```rust
let squares = [x * x for x in range(10) if x % 2 == 0]
let lengths = {w: len(w) for w in ["hi", "there"]}
let firsts = {w[0..1] for w in ["ant", "bee", "ape"]}
```

Python's builtins work on these types: `len`, `sum`, `min`, `max`, `sorted`,
`reversed`, `enumerate`, `zip`, `range`, `abs`, `round`, and `print`, which
takes any number of arguments. Each is an ordinary method underneath
(`len(x)` is `x.len()`), so a file can still define its own function by one
of these names.

Lists carry `push`, `pop`, `len`, `map`, `filter`, `fold`, `each`, `any`,
`all`, `find`, `position`, `count`, `first`, `last`, `slice`, `take`,
`skip`, `reversed`, `extend`, `concat`, `sorted_by`, `enumerate`, `zip`,
plus `contains` and `index_of` (elements with `Eq`), `sorted`, `sort`, `min`,
`max` (elements with `Ord`), `sum`, and `join` on `list[str]`. Dicts carry
`get` (an `Option`), `get_or`, `set`, `has`, `remove`, `keys`, `values`,
`items`, and `len`. None of these need an import.

## Option and Result

`Option[T]` is `Some(value)` or `None`. `Result[T, E]` is `Ok(value)` or
`Err(error)`. Both have methods with no import: `is_some`/`is_none`,
`is_ok`/`is_err`, `unwrap`, `expect(msg)`, `unwrap_or`, `unwrap_or_else`,
`map`, `and_then`, `or_else`, `ok_or` (Option), `map_err`, `ok`, `err`
(Result). `unwrap` and `expect` panic on the empty case.

```rust
fun port(text: str) -> Result[int, Error] {
    let n = text.parse_int().ok_or(error("not a number"))?
    return Ok(n)
}
```

## Traits

A trait declares methods; a method can have a default body. A class lists
the traits it implements after `:`.

```rust
trait Shape {
    fun area(self) -> float

    fun describe(self) -> str = "shape with area ${self.area()}"
}

class Circle: Shape {
    r: float

    fun area(self) -> float = 3.14159 * self.r * self.r
}
```

An implementation must provide every method without a default, may not add
methods the trait does not declare, and must match each signature. To
implement a trait for a type declared elsewhere, write
`impl Trait for Type { ... }`.

`dyn Trait` is a value of any type implementing the trait, dispatched at
run time:

```rust
fun total(shapes: list[dyn Shape]) -> float {
    return shapes.map(s => s.area()).sum()
}
```

## Generics

Type parameters go in square brackets, with bounds after `:`:

```rust
fun largest[T: Ord](xs: list[T]) -> Option[T] = xs.max()

class Box[T] {
    value: T

    fun map[U](self, f: fun(T) -> U) -> Box[U] = Box(f(self.value))
}
```

Type arguments are inferred at call sites; write them when needed, as in
`decode[User](text)`. Generic code is compiled once per concrete type.

## Concurrency

`spawn { ... }` starts a goroutine. Goroutines run in parallel across all
cores on an M:N scheduler and communicate through channels from `std.sync`.

```rust
from std.sync import channel

fun main() {
    let ch: Channel[int] = channel()
    spawn {
        for i in 1..=5 {
            ch.send(i)
        }
    }
    var sum = 0
    for _ in 0..5 {
        sum += ch.recv()
    }
    print(sum) // 15
}
```

A `Channel[T]` carries values of any type: classes, strings, tuples. A send on
a full channel and a receive on an empty one block only the goroutine.
`let task = spawn { ... }` keeps a handle, and `task.join()` waits for the
block to finish. When `main` returns, the program exits. `std.sync` also provides
`select_recv`, a mutex, a wait group, and `sleep_millis`.

## Modules and imports

```rust
import std.fs
from std.json import parse, stringify
from "./models" import User, Order as PurchaseOrder
import "github.com/martian56/raven-http" as http
```

- `import std.fs` makes the module's functions reachable as `fs.read(...)`.
- `from M import a, b as c` binds names directly; `as` renames one.
- A quoted path is a local file (`"./models"`) or a package
  (`"github.com/user/repo"`, resolved through rvpm; see the
  [rvpm guide](rvpm.md)).
- The core prelude (traits, `Option`/`Result`/`list`/`str` methods, `dict` and
  `set`) is always available.

Names starting with `_` are private to their module by convention.

## FFI and C types

`extern "C" { ... }` declares foreign function signatures. Call them like
ordinary functions. Arguments and returns use C types.

```rust
extern "C" {
    fun abs(x: CInt) -> CInt
    fun strlen(s: CStr) -> CSize
}

fun main() {
    print(abs(-7)) // 7
    print(strlen(c"hello")) // 5
}
```

C types map to C as follows:

| Raven   | C              | Width |
|---------|----------------|-------|
| `CInt`  | `int`          | 32-bit |
| `CLong` | `long`         | 64-bit on Linux (LP64), 32-bit on Windows (LLP64) |
| `CSize` | `size_t`       | pointer width (64-bit) |
| `CStr`  | `const char *` | pointer width |
| `CFloat` | `float`       | 32-bit |
| `CDouble` | `double`     | 64-bit |
| `CPtr[T]` | `T *`        | pointer width |
| `CFnPtr` | function pointer | pointer width |

A native `int` is accepted where an integer C type is expected, and a
`c"..."` literal where a `CStr` is expected. A native `float` is accepted
where a `CFloat` or `CDouble` is expected; for `CFloat` the value is
narrowed to f32 at the call and a `CFloat` return is widened back to a
`float`. The integer and float C return types satisfy `ToString`, so a
`CInt` or `CDouble` result prints and interpolates directly.

```rust
extern "C" {
    fun sqrtf(x: CFloat) -> CFloat
}

fun main() {
    print(sqrtf(16.0)) // 4
}
```

### Strings across the boundary

To pass a runtime `str` to C, convert it with
[`std.ffi`](standard-library.md#stdffi)'s `to_cstr`; a native `str` is
not itself a valid `const char *`. `from_cstr` reads a `CStr` back into a
`str`. `to_cstr` copies into a buffer outside the GC and does not free
it, so it leaks one buffer per call; hoist the conversion out of a hot
loop.

```rust
from std.ffi import to_cstr, from_cstr

extern "C" {
    fun strlen(s: CStr) -> CSize
}

fun main() {
    print(strlen(to_cstr("hello"))) // 5
    print(from_cstr(to_cstr("roundtrip"))) // roundtrip
}
```

### Raw pointers

`CPtr[T]` is a usable raw pointer. `std.ffi` reads and writes C memory
through it: `alloc[T](count)` reserves a buffer, `free[T](p)` releases it,
`load[T](p)` and `store[T](p, v)` read and write the element at `p`,
`offset[T](p, i)` advances by `i` elements (scaled by `sizeof(T)`),
`null_ptr[T]()` is the null pointer, and `is_null[T](p)` tests it.

```rust
from std.ffi import alloc, free, load, store, offset, is_null, null_ptr

fun main() {
    let buf = alloc[CInt](4)
    store[CInt](buf, 10)
    store[CInt](offset[CInt](buf, 1), 20)
    print(load[CInt](buf)) // 10
    print(load[CInt](offset[CInt](buf, 1))) // 20
    print(is_null[CInt](null_ptr[CInt]())) // true
    free[CInt](buf)
}
```

This memory lives outside the garbage collector. It is never traced or
reclaimed automatically, so the caller owns it and must `free` it. There
are no bounds, null, or use-after-free checks: an out-of-range `offset`
or a `load` through a freed pointer is undefined behavior, exactly as in
C. `T` must be a C scalar (`CInt`, `CLong`, `CSize`, `CFloat`, `CDouble`,
`CStr`) or a native `int`/`float`.

### Callbacks

`CFnPtr` is an untyped C function pointer. A C function that takes a
callback can call back into Raven through one. Pass a non-capturing
top-level function by naming it bare; its parameters and return must all
be C-FFI types so the C ABI is well defined.

A capturing closure can also be used as a callback when the C function
carries a userdata pointer (the common C idiom). Pass the closure both
where the `CFnPtr` is expected and where the userdata `CPtr` is expected:
the compiler emits a trampoline for the function-pointer slot and hands the
closure object as the userdata, and C threads it back to the trampoline,
which invokes the closure with its captured values. The garbage collector
traces the suspended Raven stack through the call, so a closure that
allocates mid-callback is safe.

```rust
from std.ffi import alloc, free, load, store, offset

extern "C" {
    fun raven_ffi_qsort_i32(p: CPtr[CInt], n: CSize, cmp: CFnPtr)
}

fun compare(a: CPtr[CInt], b: CPtr[CInt]) -> CInt {
    return load[CInt](a) - load[CInt](b)
}

fun main() {
    let buf = alloc[CInt](3)
    store[CInt](buf, 30)
    store[CInt](offset[CInt](buf, 1), 10)
    store[CInt](offset[CInt](buf, 2), 20)
    raven_ffi_qsort_i32(buf, 3, compare)
    print(load[CInt](buf)) // 10
    free[CInt](buf)
}
```

`CFnPtr` is untyped: the type checker does not verify the function's
signature against what the C side expects. Matching it is your
responsibility, as in C.

### Structs by value

A C struct can cross the boundary by value. Mark the matching Raven class
`@repr(C)` to give it C memory layout. Its fields may be any C scalars,
integer-class (`CInt`, `CLong`, `CSize`, `CStr`, `CPtr[T]`, `CFnPtr`) or
floating-point (`CFloat`, `CDouble`), and a field may itself be a `@repr(C)`
struct. The back end classifies the struct for the platform C ABI, so a
struct larger than one register, or one with float fields, is passed
correctly (in registers, on the stack, or by reference as the ABI requires)
rather than rejected. You can still pass a `CPtr[...]` to a struct when you
need to share or mutate it by address.

```rust
@repr(C)
class Point {
    x: CInt
    y: CInt
}

extern "C" {
    fun raven_ffi_point_sum(p: Point) -> CInt
    fun raven_ffi_translate(p: Point, dx: CInt, dy: CInt) -> Point
}

fun main() {
    let p = Point(x: 3, y: 4)
    print(raven_ffi_point_sum(p)) // 7
    let q = raven_ffi_translate(p, 1, 2) // {4, 6}
    print(q.x) // 4
    print(q.y) // 6
}
```

The fields stay readable on the Raven side (`q.x`); only the call
boundary marshals the struct by value.

## Metaprogramming

Raven has three metaprogramming tools: `@derive` to synthesize trait
impls, declarative macros to rewrite call sites, and reflection to read
type information.

### @derive

`@derive(...)` sits on its own line before a `class` or `enum` and
synthesizes trait impls from the type definition, so you do not hand write
`equals`, `hash`, `to_string`, or `debug`. The derivable traits are `Eq`,
`Hash`, `ToString`, `Debug`, `ToJson`, and `FromJson`. A field or payload
type must itself implement the trait being derived.

```rust
from std.collections import Map, Set

@derive(Eq, Hash, ToString, Debug)
class Point {
    x: int
    y: int
}

fun main() {
    let p = Point(x: 1, y: 2)
    let q = Point(x: 1, y: 2)
    print(p.equals(q)) // true
    print(p.to_string()) // Point(x: 1, y: 2)

    // Derived Eq + Hash let the class key a dict or join a set.
    let m: dict[Point, int] = Map.new()
    m.set(p, 100)
    print(m.has(q)) // true
    let s: set[Point] = Set.new()
    s.add(p)
    print(s.contains(q)) // true
}
```

`@derive(ToJson, FromJson)` adds JSON serialization built on `std.json`.
`to_json` produces a `JsonValue`, and `from_json` is an associated
function that decodes one back, returning a `Result`. Combine `to_json`
with `stringify`, and `parse` with `from_json`, for a round trip.

```rust
from std.json import JsonValue, stringify, parse

@derive(ToJson, FromJson, Eq)
class User {
    id: int
    name: str
}

fun main() {
    let u = User(id: 7, name: "ada")
    let text = stringify(u.to_json())
    print(text) // {"id":7,"name":"ada"}
    match parse(text) {
        case Ok(v) {
            match User.from_json(v) {
                case Ok(u2) { print(u.equals(u2)) } // true
                case Err(e) { print("decode error") }
            }
        }
        case Err(e) { print("parse error") }
    }
}
```

Object keys keep their insertion order.
Enum variants with named-field payloads are not derivable yet; unit and
tuple variants are. `Ord` derives a `compare` that orders field by field
(struct) or by variant then payload (enum).

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
    print(twice!(n + 1)) // 8
}
```

A repetition group `$(...)*` (zero or more) or `$(...)+` (one or more)
matches a sub-pattern several times, with an optional separator between
the closing `)` and the marker. In the template it expands once per
capture.

```rust
macro sum_all { ($($x:expr), *) => { (0 $(+ ($x)) *) } }

fun main() {
    print(sum_all!(1, 2, 3)) // 6
    print(sum_all!()) // 0
    print(sum_all!(10)) // 10
}
```

A name a template introduces at a `let`, `const`, or `for` binding is
renamed to a fresh name, so a template temporary cannot collide with or
capture a caller's variable of the same spelling.

### Reflection

Compile-time reflection reads type information resolved while the program
compiles. `type_name[T]()` returns the rendered name of a type,
`field_names[T]()` returns a struct's field names in declaration order, and
`field_types[T]()` returns the matching field type names by position. Inside
a generic function each resolves to the concrete type bound to `T` at that
instantiation.

```rust
class Point { x: int, y: int }

fun introspect[T]() {
    print("type ${type_name[T]()}")
    let names = field_names[T]()
    let types = field_types[T]()
```