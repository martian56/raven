# derive

`@derive(...)` is a compile-time attribute that synthesizes trait impls
from a type definition, so a user does not hand write `compare`, `debug`,
`to_json`, or `from_json` for a class or enum.

## Implicit derives

Every class and enum whose fields support it gets `Eq`, `Hash`, and
`ToString` without any attribute (`add_implicit_derives` in
`src/resolve/derive.rs`). A type keeps its own impl when it writes the method
itself (`equals`, `hash`, or `to_string`) or implements the trait explicitly,
and a type is skipped when a field's type lacks the trait. `@repr(C)` classes
and enums with named-field payloads are not derived implicitly.

## Syntax

The attribute sits on its own line immediately before a `class` or `enum`
declaration:

```rust
@derive(Ord, Debug)
class Point {
    x: int
    y: int
}

@derive(Eq, ToString)
enum Shape {
    Dot,
    Circle(int),
    Rect(int, int),
}
```

The `@` lexes to a dedicated `At` token. The parser reads
`@derive(Name, Name, ...)`, validates that the attribute name is `derive`,
and attaches the trait list to the following class or enum as a
`derives: Vec<String>`. The attribute is only valid before a `class` or
`enum`; placing it before any other item, or using any attribute name other
than `derive`, is a parse error. A type with no attribute carries an empty
derive list and is unaffected.

The supported traits are `Eq`, `Ord`, `Hash`, `ToString`, `Debug`, `ToJson`, and
`FromJson`. Naming any other trait (for example `Clone`) is a compile error.
`ToJson` and `FromJson` provide JSON serialization on top of `std.json`; see
their section below and the [std.json spec](../guide/stdlib/json.md).

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
`std.fmt` rather than the prelude. The expander force-merges `std.fmt` when
any type derives `Debug`, so the user needs no explicit `import std.fmt`.
Likewise `@derive(ToJson)` and `@derive(FromJson)` reference the `JsonValue`
tree and the JSON traits in `std.json`, so the expander force-merges
`std.json` (which transitively pulls in `std.error` and `std.collections`)
when any type derives one of them.

## Generated impl shapes

### Eq

```rust
fun equals(self, other: Point) -> bool
```

* Struct: the conjunction of `self.field.equals(other.field)` over every
  field. A field-less class yields `true`.
* Enum: `match self` over the variants; each arm matches `other` against the
  same variant (with a `_ -> false` fallback) and compares the payload slots
  pairwise with `equals`, so two values are equal only when they are the same
  variant with equal payloads.

The `other` parameter is annotated with the concrete self type (for example
`Point`, or `Pair[A, B]`) rather than `Self`, because the type checker does
not yet accept `Self` as a non-receiver parameter type.

The `==` and `!=` operators on any type that implements `Eq` (whether derived
or hand-written) dispatch to its `equals` method, so equality is by value, not
by object identity; `!=` negates the result. HIR lowering rewrites the operator
to the method call, the same way `print` routes a non-`str` through
`to_string`. A primitive keeps the native machine compare, and a `str` keeps
its byte-equality path. A type with no `Eq` impl, such as a class holding a
function, keeps the identity compare.

The built-in generic types `Option[T]`, `Result[T, E]`, and `list[T]` implement
`Eq` in `std.core`, and `set[T]` and `dict[K, V]` in `std.collections` (these
two compare order-independently), so `==`/`!=` work on them by value when the
element type implements `Eq`.

### Ord

```rust
fun compare(self, other: Self) -> int
```

Returns a negative number when `self < other`, zero when equal, and a positive
number when `self > other`, matching `std.core`'s `Ord`. Each field or payload
type must itself implement `Ord`.

* Struct: compare fields in declaration order, returning the first non-zero
  field `compare` and `0` when every field is equal. `Point { x, y }` orders by
  `x` first, then `y`.
* Enum: compare the variant's declaration order first (an earlier variant sorts
  before a later one), then, for two values of the same variant, compare the
  payload slots in order. `Shape.Dot` sorts before `Shape.Circle(_)`, and
  `Circle(2)` before `Circle(5)`.

`Ord` pairs with `std.cmp` (`sort`, `sorted_by`, `min`, `max`) so a derived type
can be sorted without a hand-written comparator.

### Hash

```rust
fun hash(self) -> int
```

* Struct: folds the field hashes with `h = h * 31 + self.field.hash()`,
  seeded at `17`, matching the str hash style in `stdlib/std/core.rv`.
* Enum: starts from a per-variant seed (the variant index) and folds in each
  payload slot's hash with the same `* 31 +` step. A unit variant hashes to
  its seed.

`Eq` and `Hash` together let a derived type act as a `Map` key or a `Set`
element, since the hash-backed collections require `Eq + Hash` keys.

### ToString

```rust
fun to_string(self) -> str
```

* Struct: `TypeName { field: value, ... }`, where each value is the field's
  own `to_string()`. A field-less class prints just `TypeName`.
* Enum: a unit variant prints its bare name (`Dot`); a payload variant prints
  `VariantName(p0, p1)` using each payload's `to_string()` (`Circle(3)`,
  `Rect(2, 4)`).

### Debug

```rust
fun debug(self) -> str
```

Same shape as `ToString`, but each field or payload is formatted with
`debug()` instead of `to_string()`. Because `Debug for str` and
`Debug for char` quote their value (see `stdlib/std/fmt.rv`), a derived
`debug()` quotes string and char members while the derived `to_string()`
does not. For a `User { name: "ann", age: 30 }`:

```
to_string -> User { name: ann, age: 30 }
debug     -> User { name: "ann", age: 30 }
```

### ToJson

```rust
fun to_json(self) -> JsonValue
```

* Struct: a JSON object keyed by field name, each value the field's own
  `to_json()`. `Point { x: 1, y: 2 }` serializes to `{"x":1,"y":2}`.
* Enum: a tagged object `{"tag": "Variant", "values": [p0, p1, ...]}`, where
  the payload slots are each value's `to_json()` and a unit variant has an
  empty `values` array. `Shape.Rect(2, 5)` serializes to
  `{"tag":"Rect","values":[2,5]}`, and `Shape.Dot` to
  `{"tag":"Dot","values":[]}`.

Object keys follow the field declaration order. Combine `to_json` with `stringify` to get a `str`.

### FromJson

```rust
fun from_json(j: JsonValue) -> Result[Self, Error]
```

`from_json` is an associated function (it takes no `self`), so it is called
as `Point.from_json(j)`. The `FromJson` trait declares the method with `Self`
in the return, but the generated impl writes the concrete type, because the
type checker does not yet accept `Self` as a non-receiver type in a method
signature (the same limitation `Eq` works around). So `@derive(FromJson)` on
`Point` generates `impl FromJson for Point { fun from_json(j) -> Result[Point,
Error] { ... } }`.

* Struct: read each field from the object by name, decode it to the field's
  declared type, propagate a missing or wrong-typed field as an `Err`, then
  construct the class.
* Enum: read the `tag` string, dispatch to the matching variant, decode each
  payload slot positionally from the `values` array, and return an `Err` on an
  unknown tag.

The derived `from_json` calls a small set of helper free functions that the
derive pass emits into the program once (a generic decode dispatcher plus
object/array accessors). They cannot live in `std.json` because a bundled
free function is namespaced (`std.json.f`) and so not callable by its bare
name from generated source.

### Scalar, List, and Option impls

`std.json` hand-writes the `ToJson`/`FromJson` impls that field recursion
bottoms out on: `int`, `float`, `bool`, `str`, `list[T]`, and `Option[T]`
(for a `T` that is itself `ToJson`/`FromJson`). `int` and `float` both serialize to a JSON
number; `bool` to a JSON bool; `str` to a JSON string; `list[T]` to a JSON
array; and `Option[T]` to `null` or the inner value. An `int` round-trips
through `float` (JSON has one number type) and loses precision beyond 2^53,
the IEEE 754 double mantissa. The derive only generates impls for user
classes and enums; it never generates impls for the built-in types.

## Generics

For a generic type the synthesized impl is generic with the derived trait as
a bound on every type parameter. Deriving `Eq` on

```rust
@derive(Eq)
class Pair[A, B] {
    first: A
    second: B
}
```

generates

```rust
impl[A: Eq, B: Eq] Eq for Pair[A, B] {
    fun equals(self, other: Pair[A, B]) -> bool {
        return self.first.equals(other.first) and self.second.equals(other.second)
    }
}
```

The bound is required because `equals` on a field of type `A` needs
`A: Eq`. The same rule applies to each trait: `Hash` emits `A: Hash`,
`ToString` emits `A: ToString`, `ToJson` emits `A: ToJson`, `FromJson` emits
`A: FromJson`, and so on. So `@derive(ToJson)` on `Pair[A, B]` generates
`impl[A: ToJson, B: ToJson] ToJson for Pair[A, B]`.

## Limitations

* Only `Eq`, `Ord`, `Hash`, `ToString`, `Debug`, `ToJson`, and `FromJson` are
  supported. Other traits are not derivable yet.
* Enum variants with named-field payloads, for example `V(a: int)`, are
  rejected with a clear error. Unit and tuple variants are
  fully supported.
* `Debug` reuses the `ToString` field shape with `debug()` formatting rather
  than offering a separate layout.
* A derived `FromJson` reads only the keys it declares; extra object members
  are ignored, and a `Number` decodes to `int` by truncation toward zero.
