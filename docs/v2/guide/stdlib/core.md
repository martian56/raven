# std/core

The foundational traits every Raven program builds on. `std/core` is the
prelude: the compiler merges it into every program before name resolution, so
its traits and impls are always in scope. You never write `import std/core`,
and there is nothing to import selectively.

```raven
fun main() {
    print(42.to_string())           // 42
    print(3.compare(7) < 0)         // true (3 sorts before 7)
}
```

These traits make the rest of the standard library polymorphic. A function
bounded by one of them, for example `fun f<T: ToString>(x: T)`, works for any
type that implements it, including your own.

## The traits

| Trait | Method | Purpose |
|-------|--------|---------|
| `ToString` | `to_string(self) -> String` | Generic textual rendering. The basis of generic printing and of string interpolation for user types. |
| `Eq` | `equals(self, other: Self) -> Bool` | Structural equality. Reflexive, symmetric, and transitive for the built-in impls. |
| `Ord` | `compare(self, other: Self) -> Int` | Total ordering. Negative when `self` sorts first, zero when equal, positive otherwise. |
| `Hash` | `hash(self) -> Int` | Stable hash for hash maps and sets. A `Hash` type should also be `Eq` so equal values hash equally. |
| `Iterator<T>` | `next(self) -> Option<T>` | A sequence producing values one at a time. The element type `T` is a parameter on the trait. |

`Self` in a signature means the implementing type, so `Eq for Int` reads as
`equals(self, other: Int) -> Bool`.

## ToString

```raven
trait ToString {
    fun to_string(self) -> String
}
```

Renders a value as text. `print` and string interpolation both render any
value whose type implements `ToString`, so implementing it is what lets a type
print.

Built in for `Int`, `Float`, `Bool`, `Char`, and `String`. The scalar impls
render through interpolation (`"${self}"`); `ToString for String` returns the
string unchanged.

```raven
fun main() {
    print(42.to_string())           // 42
    print(true.to_string())         // true
    print(3.14.to_string())         // 3.14

    let count = 3
    print("count = ${count}")       // count = 3 (interpolation uses ToString)
}
```

## Eq

```raven
trait Eq {
    fun equals(self, other: Self) -> Bool
}
```

Structural equality. Built in for `Int`, `Float`, `Bool`, `Char`, and
`String`. The scalar impls compare with `==`; `String` compares byte by byte.

```raven
fun main() {
    print(2.equals(2))              // true
    print("ab".equals("ab"))        // true
    print("ab".equals("ba"))        // false
}
```

## Ord

```raven
trait Ord {
    fun compare(self, other: Self) -> Int
}
```

Total ordering. `compare` returns a negative `Int` when `self` sorts before
`other`, zero when they are equal, and a positive `Int` when `self` sorts
after. Built in for `Int`, `Float`, `Char`, `Bool` (`false` sorts before
`true`), and `String` (lexicographic over bytes).

```raven
fun main() {
    print(3.compare(7))             // -1
    print(7.compare(3))             //  1
    print(5.compare(5))             //  0
    print("apple".compare("apply")) // -1
}
```

Use the sign of the result rather than the exact magnitude: only `< 0`,
`== 0`, and `> 0` are guaranteed.

```raven
fun max<T: Ord>(a: T, b: T) -> T {
    if a.compare(b) >= 0 {
        return a
    }
    return b
}

fun main() {
    print(max(3, 7))                // 7
    print(max("cat", "ant"))        // cat
}
```

## Hash

```raven
trait Hash {
    fun hash(self) -> Int
}
```

A stable hash for keying hash maps and sets. Built in for `Int` (identity),
`Bool` (`0` or `1`), and `String` (a multiplier-31 rolling hash over the
bytes). A `Hash` type should also be `Eq`, so that equal values produce equal
hashes.

`Hash for Char` and `Hash for Float` are not provided yet.

```raven
fun main() {
    print(7.hash())                 // 7
    print(true.hash())              // 1
}
```

## Iterator&lt;T&gt;

```raven
trait Iterator<T> {
    fun next(self) -> Option<T>
}
```

A sequence that yields values one at a time. `next` returns `Some(value)`
while elements remain and `None` when the sequence is exhausted. The element
type `T` is a generic parameter on the trait (Raven has no associated types
yet). The lazy adapter pipeline that builds on this lives in
[std/iter](iter.md).

## Using traits as generic bounds

Write a trait after a type parameter to require that the argument implements
it. Inside the function you can then call the trait's methods on that
parameter. Dispatch is resolved statically at each call site, so there is no
runtime overhead.

```raven
fun describe<T: ToString>(x: T) -> String {
    return "value: ${x.to_string()}"
}

fun main() {
    print(describe(42))             // value: 42
    print(describe(true))           // value: true
    print(describe("hi"))           // value: hi
}
```

A type participates in a bound by implementing the trait. Implement it for your
own struct or enum and that type becomes usable everywhere the bound appears:

```raven
struct Point {
    x: Int,
    y: Int,
}

impl ToString for Point {
    fun to_string(self) -> String = "(${self.x}, ${self.y})"
}

fun describe<T: ToString>(x: T) -> String {
    return "value: ${x.to_string()}"
}

fun main() {
    print(describe(Point { x: 1, y: 2 }))   // value: (1, 2)
}
```

## Deriving for user types

Most of the time you do not hand-write these impls. `@derive(...)` on a struct
or enum generates them from the fields:

```raven
@derive(Eq, Hash, ToString, Debug)
struct User {
    id: Int,
    name: String,
}
```

`Eq`, `Hash`, `ToString`, and `Debug` are derivable; `Ord` is not derivable
yet and must be written by hand. Every field must already implement the trait
being derived. See the `@derive` section of the
[language reference](../language-reference.md) for the full list and rules.

## Worked example: generic comparison and sorting key

```raven
struct Score {
    name: String,
    points: Int,
}

impl Ord for Score {
    fun compare(self, other: Score) -> Int = self.points.compare(other.points)
}

impl ToString for Score {
    fun to_string(self) -> String = "${self.name}: ${self.points}"
}

// Works for Int, String, Score, or anything else that is Ord.
fun higher<T: Ord + ToString>(a: T, b: T) -> String {
    if a.compare(b) >= 0 {
        return a.to_string()
    }
    return b.to_string()
}

fun main() {
    let ada = Score { name: "Ada", points: 90 }
    let bob = Score { name: "Bob", points: 75 }
    print(higher(ada, bob))         // Ada: 90
    print(higher(3, 8))             // 8
}
```

## See also

- [std/cmp](cmp.md) for comparison helpers built on `Ord`.
- [std/collections](collections.md) for maps and sets that key on `Eq` and `Hash`.
- [std/iter](iter.md) for the lazy adapter pipeline over `Iterator<T>`.
- The [language reference](../language-reference.md) for traits, generics,
  bounds, and `@derive`.
