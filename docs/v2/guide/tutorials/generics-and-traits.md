# Tutorial: generics and traits

Generics let one piece of code work over many types; traits describe what a
type can do and let you write code against that capability. Together they are
Raven's tools for reuse without giving up static types: a generic is
monomorphized (specialized) per concrete type at compile time, so there is no
runtime type check and no boxing unless you ask for it. This tutorial builds
from a generic container to bounded functions, trait objects, and derived
implementations. Every step compiles and runs.

## Step 1: a generic struct

A type parameter in angle brackets makes a struct hold any type. `Box<T>` wraps
one value of type `T`:

```rust
struct Box<T> {
    value: T,
}

fun main() {
    let n = Box { value: 7 }
    let s = Box { value: "raven" }
    print(n.value)          // 7
    print(s.value)          // raven
}
```

You never write the type argument when constructing: the compiler infers `T`
from the value you pass, so `Box { value: 7 }` is a `Box<Int>` and `Box { value:
"raven" }` is a `Box<String>`. Each instantiation gets its own layout and its
own garbage-collector descriptor, so wrapping an `Int` and wrapping a `String`
are both fully typed.

## Step 2: generic methods on a generic type

An `impl<T>` block adds methods that work for every instantiation. A method's
body can use `T` as the element type:

```rust
struct Box<T> {
    value: T,
}

impl<T> Box<T> {
    fun unwrap(self) -> T = self.value
}

fun main() {
    let a = Box { value: 42 }
    let b = Box { value: "raven" }
    print(a.unwrap())       // 42     (unwrap specialized at T = Int)
    print(b.unwrap())       // raven  (unwrap specialized at T = String)
}
```

`unwrap` is compiled once per concrete `T` it is called with. You can also add
methods for a *specific* instantiation with a concrete `impl`:

```rust
impl Box<Int> {
    fun doubled(self) -> Int = self.value * 2
}
```

`doubled` exists only on `Box<Int>`; calling it on a `Box<String>` is a compile
error.

## Step 3: a trait

A trait names a set of methods a type can provide. Implement it for a type with
`impl Trait for Type`:

```rust
trait Speak {
    fun sound(self) -> Int
}

struct Dog {}
struct Cat {}

impl Speak for Dog {
    fun sound(self) -> Int = 1
}

impl Speak for Cat {
    fun sound(self) -> Int = 2
}

fun main() {
    let d = Dog {}
    let c = Cat {}
    print(d.sound())        // 1
    print(c.sound())        // 2
}
```

`Dog` and `Cat` share no data, but both satisfy `Speak`, so any code that needs
"something that can `sound`" accepts either.

## Step 4: bounded generic functions

A trait becomes useful as a *bound*: `<T: Speak>` means "any `T` that
implements `Speak`." The function can then call the trait's methods on its
argument, and the call is dispatched statically (resolved at compile time, no
vtable):

```rust
trait Speak {
    fun sound(self) -> Int
}

struct Dog {}
impl Speak for Dog {
    fun sound(self) -> Int = 1
}

fun loudness<T: Speak>(x: T) -> Int = x.sound() * 10

fun main() {
    print(loudness(Dog {}))     // 10
}
```

The standard library leans on this. `ToString` is part of the always-imported
prelude, with built-in implementations for the primitives, so a bound of
`<T: ToString>` accepts ints, bools, and any type you implement it for:

```rust
struct Point {
    x: Int,
    y: Int,
}

impl ToString for Point {
    fun to_string(self) -> String = "(${self.x}, ${self.y})"
}

fun describe<T: ToString>(x: T) -> String = x.to_string()

fun main() {
    print(describe(42))             // 42
    print(describe(true))           // true
    let p = Point { x: 3, y: 4 }
    print(describe(p))              // (3, 4)
}
```

No `import std/core` line is needed: the prelude with `ToString` is always in
scope.

## Step 5: dynamic dispatch with `dyn Trait`

A bounded generic produces a separate specialization per type, which is fast but
means the type is fixed at each call site. When you instead want one function to
accept values of *different* concrete types at runtime (a heterogeneous
collection, a plugin slot), use `dyn Trait`. It is a fat pointer (data plus a
vtable), and the call dispatches through the vtable:

```rust
trait Speak {
    fun sound(self) -> Int
}

struct Dog {}
struct Cat {}

impl Speak for Dog {
    fun sound(self) -> Int = 1
}

impl Speak for Cat {
    fun sound(self) -> Int = 2
}

fun describe(s: dyn Speak) -> Int = s.sound()

fun main() {
    print(describe(Dog {}))     // 1
    print(describe(Cat {}))     // 2
}
```

The rule of thumb: reach for `<T: Trait>` by default (static dispatch, no
overhead), and for `dyn Trait` only when you genuinely need to mix concrete
types behind one type at runtime.

## Step 6: method-level type parameters

A method can introduce its own type parameter, separate from the type's. Here
`mapped<U>` transforms a `Box<T>` into a `U` using a function you pass:

```rust
struct Box<T> {
    value: T,
}

impl<T> Box<T> {
    fun mapped<U>(self, f: fun(T) -> U) -> U = f(self.value)
}

fun main() {
    let b = Box { value: 21 }
    let doubled = b.mapped(fun(x: Int) -> Int = x * 2)
    let is_big = b.mapped(fun(x: Int) -> Bool = x > 10)
    print(doubled)          // 42
    print(is_big)           // true
}
```

Calling `mapped` at two different `U` on the same `Box<Int>` compiles to two
distinct specializations. The closures use the single-expression form
`fun(x: Int) -> Int = x * 2`, which is how a closure that returns a value is
written.

## Step 7: deriving common implementations

Writing equality and hashing by hand is tedious, so `@derive` generates them. A
type used as a `Map` key must satisfy the key bounds `Eq` and `Hash`; deriving
both makes a generic struct a valid key:

```rust
import std/collections

@derive(Eq, Hash)
struct Box<T> {
    value: T,
}

fun main() {
    let m: Map<Box<Int>, Int> = Map.new()
    m.set(Box { value: 7 }, 42)
    match m.get(Box { value: 7 }) {
        Some(v) -> print(v),        // 42
        None -> print(0),
    }
}
```

`@derive(Eq, Hash)` writes the `equals` and `hash` methods for you; `@derive`
also understands `Ord` (ordering) among others. Without the derive, using
`Box<Int>` as a key is a clear type error rather than a failure deep in code
generation.

## Putting it together

These features compose: a generic container, a trait its element is bounded by,
and a function that works over both. The snippet below stores any
`ToString` value and renders it:

```rust
struct Labeled<T> {
    name: String,
    value: T,
}

impl<T: ToString> Labeled<T> {
    fun show(self) -> String = "${self.name}=${self.value.to_string()}"
}

fun main() {
    let a = Labeled { name: "count", value: 42 }
    let b = Labeled { name: "ready", value: true }
    print(a.show())         // count=42
    print(b.show())         // ready=true
}
```

The bound on the `impl<T: ToString>` block means `show` is available only when
`T` can be turned into a string, which is exactly when its body
(`self.value.to_string()`) makes sense.

## Where to go next

- The [language reference](../language-reference.md) covers generics, traits,
  `dyn Trait`, and `@derive` in full.
- [`std/cmp`](../stdlib/cmp.md) and [`std/hash`](../stdlib/hash.md) define the
  `Ord`, `Eq`, and `Hash` traits the derives target.
- The [modeling-data tutorial](task-tracker.md) uses structs, enums, and
  `match` together, the data side that complements the abstraction tools here.
