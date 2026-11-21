# Core Trait Prelude

The prelude is the module `std.core`. The compiler merges it into every program before name resolution, so its declarations are always in scope without an import. It defines the small set of traits that make the standard library polymorphic, and implements them for the built-in types.

## Traits

| Trait | Method | Purpose |
|---|---|---|
| `ToString` | `to_string(self) -> str` | Textual rendering. The basis of `print` and of string interpolation for user types. |
| `Eq` | `equals(self, other: Self) -> bool` | Structural equality, behind `==` and `!=`. |
| `Ord` | `compare(self, other: Self) -> int` | Total ordering. Negative when `self` sorts first, zero when equal, positive otherwise. Behind `<`, `<=`, `>`, `>=`, `sort`, `min`, and `max`. |
| `Hash` | `hash(self) -> int` | Stable hash for `dict` keys and `set` elements. A `Hash` type should also be `Eq` so equal values hash equally. |
| `Iterator[T]` | `next(self) -> Option[T]` | A sequence producing values one at a time. The element type is a parameter on the trait. The lazy adapter pipeline that builds on it lives in `std.iter`. |

## Built-in implementations

- `ToString` for `int`, `float`, `bool`, `char`, `str`, and for `list[T]`, `Option[T]`, `Result[T, E]`, and tuples up to six elements whose parts implement it. `ToString for str` is the identity.
- `Eq` for `int`, `float`, `bool`, `char`, `str`, and for `list[T]`, `Option[T]`, `Result[T, E]`, and tuples up to six elements.
- `Ord` for `int`, `float`, `char`, `bool` (`false` sorts before `true`), `str` (lexicographic over bytes), and tuples up to six elements (compared element by element).
- `Hash` for `int` (identity), `bool`, `char`, `str` (a multiplier-31 polynomial rolling hash over the bytes), and tuples up to six elements. `float` has no `Hash` impl, so a float cannot be a dict key.

The impls are written in Raven on top of the language operators and the byte-level string intrinsics.

## User types

A class or enum gets `==`, hashing, and printing automatically when its fields allow it. `@derive(Ord)` adds ordering, and `@derive(Debug, ToJson, FromJson)` the rest; see [derive.md](derive.md). A type can also implement a trait by hand: `class Point: ToString { ... }` inside the class, or `impl ToString for Point { ... }` for a type declared elsewhere.

## Generic dispatch

A function bounded by a prelude trait, for example `fun describe[T: ToString](x: T) -> str = x.to_string()`, resolves `x.to_string()` through the bound and is compiled once per concrete type at each call site. This is static dispatch with no runtime cost.
