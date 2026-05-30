# derive

`@derive(...)` is a compile-time attribute that synthesizes trait impls
from a type definition, so a user does not hand write `equals`, `hash`,
`to_string`, or `debug` for a struct or enum. It is the foundation the later
metaprogramming work (macros, reflection) builds on, tracked under issue
#214.

## Syntax

The attribute sits on its own line immediately before a `struct` or `enum`
declaration:

```raven
@derive(Eq, Hash, ToString, Debug)
struct Point { x: Int, y: Int }

@derive(Eq, ToString)
enum Shape {
    Dot,
    Circle(Int),
    Rect(Int, Int),
}
```

The `@` lexes to a dedicated `At` token. The parser reads
`@derive(Name, Name, ...)`, validates that the attribute name is `derive`,
and attaches the trait list to the following struct or enum as a
`derives: Vec<String>`. The attribute is only valid before a `struct` or
`enum`; placing it before any other item, or using any attribute name other
than `derive`, is a parse error. A type with no attribute carries an empty
derive list and is unaffected.

The four supported traits in this slice are `Eq`, `Hash`, `ToString`, and
`Debug`. Naming any other trait (for example `Ord`) is a compile error.

## Expansion

Derive runs as part of stdlib expansion, before name resolution, alongside
the bundled-module merge in `src/resolve/stdlib.rs`. For each derive request
it generates the impl as Raven source text, re-parses it, and appends the
resulting `impl` items to the program. The generated impls then flow through
resolve, type checking, HIR, MIR, and codegen exactly like a hand written
impl, so there is no separate code path to keep in sync.

The generated bodies call the field and payload types' own trait methods
(`equals`, `hash`, `to_string`, `debug`). A field or payload type must
therefore implement the trait being derived; the type checker reports the
missing bound with its normal `trait bound ... is not satisfied` diagnostic.

`@derive(Debug)` produces an `impl Debug`, and the `Debug` trait lives in
`std/fmt` rather than the prelude. The expander force-merges `std/fmt` when
any type derives `Debug`, so the user needs no explicit `import std/fmt`.

## Generated impl shapes

### Eq

```raven
fun equals(self, other: Point) -> Bool
```

* Struct: the conjunction of `self.field.equals(other.field)` over every
  field. A field-less struct yields `true`.
* Enum: `match self` over the variants; each arm matches `other` against the
  same variant (with a `_ -> false` fallback) and compares the payload slots
  pairwise with `equals`, so two values are equal only when they are the same
  variant with equal payloads.

The `other` parameter is annotated with the concrete self type (for example
`Point`, or `Pair<A, B>`) rather than `Self`, because the type checker does
not yet accept `Self` as a non-receiver parameter type.

### Hash

```raven
fun hash(self) -> Int
```

* Struct: folds the field hashes with `h = h * 31 + self.field.hash()`,
  seeded at `17`, matching the String hash style in `stdlib/std/core.rv`.
* Enum: starts from a per-variant seed (the variant index) and folds in each
  payload slot's hash with the same `* 31 +` step. A unit variant hashes to
  its seed.

`Eq` and `Hash` together let a derived type act as a `Map` key or a `Set`
element, since the hash-backed collections require `Eq + Hash` keys.

### ToString

```raven
fun to_string(self) -> String
```

* Struct: `TypeName { field: value, ... }`, where each value is the field's
  own `to_string()`. A field-less struct prints just `TypeName`.
* Enum: a unit variant prints its bare name (`Dot`); a payload variant prints
  `VariantName(p0, p1)` using each payload's `to_string()` (`Circle(3)`,
  `Rect(2, 4)`).

### Debug

```raven
fun debug(self) -> String
```

Same shape as `ToString`, but each field or payload is formatted with
`debug()` instead of `to_string()`. Because `Debug for String` and
`Debug for Char` quote their value (see `stdlib/std/fmt.rv`), a derived
`debug()` quotes string and char members while the derived `to_string()`
does not. For a `User { name: "ann", age: 30 }`:

```
to_string -> User { name: ann, age: 30 }
debug     -> User { name: "ann", age: 30 }
```

## Generics

For a generic type the synthesized impl is generic with the derived trait as
a bound on every type parameter. Deriving `Eq` on

```raven
@derive(Eq)
struct Pair<A, B> { first: A, second: B }
```

generates

```raven
impl<A: Eq, B: Eq> Eq for Pair<A, B> {
    fun equals(self, other: Pair<A, B>) -> Bool {
        return self.first.equals(other.first) && self.second.equals(other.second)
    }
}
```

The bound is required because `equals` on a field of type `A` needs
`A: Eq`. The same rule applies to each trait: `Hash` emits `A: Hash`,
`ToString` emits `A: ToString`, and so on.

## Limitations

* Only `Eq`, `Hash`, `ToString`, and `Debug` are supported. `Ord` and other
  traits are not derivable yet.
* Enum variants with struct-style (named-field) payloads, for example
  `V(a: Int)`, are rejected with a clear error. Unit and tuple variants are
  fully supported.
* `Debug` reuses the `ToString` field shape with `debug()` formatting rather
  than offering a separate layout.

## Follow-ups

Derive is the first metaprogramming slice. Later slices build on it:
declarative and procedural macros, compile-time and runtime reflection, and
a `derive(ToJson/FromJson)` slice on top of `std/json`.
