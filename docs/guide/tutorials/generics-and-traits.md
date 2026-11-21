# Tutorial: generics and traits

Generics let one piece of code work over many types; traits describe what a
type can do and let you write code against that capability. Together they are
Raven's tools for reuse without giving up static types: a generic is
monomorphized (specialized) per concrete type at compile time, so there is no
runtime type check and no boxing unless you ask for it. This tutorial builds
from a generic container to bounded functions, trait objects, and derived
implementations. Every step compiles and runs.

## Step 1: a generic class

A type parameter in square brackets makes a class hold any type. `Box[T]` wraps
one value of type `T`:

```rust
class Box[T] {
    value: T
}

fun main() {
    let n = Box(value: 7)
    let s = Box(value: "raven")
    print(n.value) // 7
    print(s.value) // raven
}
```

You never write the type argument when constructing: the compiler infers `T`
from the value you pass, so `Box(value: 7)` is a `Box[int]` and
`Box(value: "raven")` is a `Box[str]`. Each instantiation gets its own layout and its
own garbage-collector descriptor, so wrapping an `int` and wrapping a `str`
are both fully typed.

## Step 2: generic methods on a generic type

Methods written inside a generic class work for every instantiation. A
method's body can use `T` as the element type:

```rust
class Box[T] {
    value: T

    fun unwrap(self) -> T = self.value
}

fun main() {
    let a = Box(value: 42)
    let b = Box(value: "raven")
    print(a.unwrap()) // 42     (unwrap specialized at T = int)
    print(b.unwrap()) // raven  (unwrap specialized at T = str)
}
```

`unwrap` is compiled once per concrete `T` it is called with. You can also add
methods for a *specific* instantiation with a concrete `impl`:

```rust
impl Box[int] {
    fun doubled(self) -> int = self.value * 2
}
```

`doubled` exists only on `Box[int]`; calling it on a `Box[str]` is a compile
error.

## Step 3: a trait

A trait names a set of methods a type can provide. Implement it for a type with
`impl Trait for Type`:

```rust
trait Speak {
    fun sound(self) -> int
}

class Dog {}
class Cat {}

impl Speak for Dog {
    fun sound(self) -> int = 1
}

impl Speak for Cat {
    fun sound(self) -> int = 2
}

fun main() {
    let d = Dog()
    let c = Cat()
    print(d.sound()) // 1
    print(c.sound()) // 2
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
    fun sound(self) -> int
}

class Dog {}
impl Speak for Dog {
    fun sound(self) -> int = 1
}

fun loudness[T: Speak](x: T) -> int = x.sound() * 10

fun main() {
    print(loudness(Dog())) // 10
}
```

The standard library leans on this. `ToString` is part of the always-imported
prelude, with built-in implementations for the primitives, so a bound of
`[T: ToString]` accepts ints, bools, and any type you implement it for:

```rust
class Point {
    x: int
    y: int
}

impl ToString for Point {
    fun to_string(self) -> str = "(${self.x}, ${self.y})"
}

fun describe[T: ToString](x: T) -> str = x.to_string()

fun main() {
    print(describe(42)) // 42
    print(describe(true)) // true
    let p = Point(x: 3, y: 4)
    print(describe(p)) // (3, 4)
}
```

No `import std.core` line is needed: the prelude with `ToString` is always in
scope.

## Step 5: dynamic dispatch with `dyn Trait`

A bounded generic produces a separate specialization per type, which is fast but
means the type is fixed at each call site. When you instead want one function to
accept values of *different* concrete types at runtime (a heterogeneous
collection, a plugin slot), use `dyn Trait`. It is a fat pointer (data plus a
vtable), and the call dispatches through the vtable:

```rust
trait Speak {
    fun sound(self) -> int
}

class Dog {}
class Cat {}

impl Speak for Dog {
    fun sound(self) -> int = 1
}

impl Speak for Cat {
    fun sound(self) -> int = 2
}

fun describe(s: dyn Speak) -> int = s.sound()

fun main() {
    print(describe(Dog())) // 1
    print(describe(Cat())) // 2
}
```

The rule of thumb: reach for `<T: Trait>` by default (static dispatch, no
overhead), and for `dyn Trait` only when you genuinely need to mix concrete
types behind one type at runtime.

## Step 6: method-level type parameters

A method can introduce its own type parameter, separate from the type's. Here
`mapped[U]` transforms a `Box[T]` into a `U` using a function you pass:

```rust
class Box[T] {
    value: T

    fun mapped[U](self, f: fun(T) -> U) -> U = f(self.value)
}

fun main() {
    let b = Box(value: 21)
    let doubled = b.mapped((x: int) -> int => x * 2)
    let is_big = b.mapped((x: int) -> bool => x > 10)
    print(doubled) // 42
    print(is_big) // true
}
```

Calling `mapped` at two different `U` on the same `Box[int]` compiles to two
distinct specializations. The closures use the arrow form `(x: int) -> int => x * 2`; the
annotations are optional when the types can be inferred.

## Step 7: classes as dict keys

A type used as a dict key must satisfy the key bounds `Eq` and `Hash`:

```rust
class Box[T] {
    value: T
}

fun main() {
    let m = {Box(value: 7): 42}
    match m.get(Box(value: 7)) {
        case Some(v) { print(v) } // 42
        case None { print(0) }
    }
}
```

A class whose fields all compare and hash gets `==` and hashing
automatically, so it works as a dict key or set element with no extra code.
`@derive(Ord)` adds ordering, and `@derive(ToJson, FromJson)` adds JSON
conversion.

## Putting it together

These features compose: a generic container, a trait its element is bounded by,
and a function that works over both. The snippet below stores any
`ToString` value and renders it:

```rust
class Labeled[T: ToString] {
    name: str
    value: T

    fun show(self) -> str = "${self.name}=${self.value.to_string()}"
}

fun main() {
    let a = Labeled(name: "count", value: 42)
    let b = Labeled(name: "ready", value: true)
    print(a.show()) // count=42
    print(b.show()) // ready=true
}
```

The bound on the `impl[T: ToString]` block means `show` is available only when
`T` can be turned into a string, which is exactly when its body
(`self.value.to_string()`) makes sense.

## Where to go next

- The [language reference](../language-reference.md) covers generics, traits,
  `dyn Trait`, and `@derive` in full.
- [`std.cmp`](../stdlib/cmp.md) and [`std.hash`](../stdlib/hash.md) define the
  `Ord`, `Eq`, and `Hash` traits the derives target.
- The [modeling-data tutorial](task-tracker.md) uses classes, enums, and
  `match` together, the data side that complements the abstraction tools here.
