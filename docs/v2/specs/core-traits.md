# Core Trait Prelude

The prelude is the module `std/core`. The compiler merges it into every program before name resolution, so its declarations are always in scope without an `import std/core` line. It defines the small set of traits that make the standard library polymorphic, and implements them for the built-in types.

## Traits

| Trait | Method | Purpose |
|---|---|---|
| `ToString` | `to_string(self) -> String` | Generic textual rendering. The basis of generic printing and of string interpolation for user types. |
| `Eq` | `equals(self, other: Self) -> Bool` | Structural equality. Reflexive, symmetric, transitive for the built-in impls. |
| `Ord` | `compare(self, other: Self) -> Int` | Total ordering. Negative when `self` sorts first, zero when equal, positive otherwise. |
| `Hash` | `hash(self) -> Int` | Stable hash for hash maps and sets. A `Hash` type should also be `Eq` so equal values hash equally. |
| `Iterator<T>` | `next(self) -> Option<T>` | A sequence producing values one at a time. The element type is a generic parameter on the trait (Raven has no associated types yet). The lazy adapter pipeline that builds on this lives in `std/iter`. |

## Built-in implementations

- `ToString` for `Int`, `Float`, `Bool`, `Char`, and `String`. The scalar impls render through string interpolation (`"${self}"`), which the compiler lowers to the per-type runtime conversions (`raven_int_to_string` and friends). The runtime owns the digits; the trait owns the dispatch. `ToString for String` is the identity.
- `Eq` for `Int`, `Float`, `Bool`, `Char`, and `String`. Scalars compare with `==`; `String` compares byte by byte through the `__str_len` and `__str_byte_at` intrinsics.
- `Ord` for `Int`, `Float`, `Char`, `Bool` (false sorts before true), and `String` (lexicographic over bytes).
- `Hash` for `Int` (identity), `Bool` (0 or 1), and `String` (a multiplier-31 polynomial rolling hash over the bytes). `Hash for Char` and `Hash for Float` are deferred (see below).

The non-`ToString` impls are written in pure Raven on top of the language operators and the byte-level string intrinsics, so they require no new runtime symbol.

## Generic dispatch

A function bounded by a prelude trait, for example `fun describe<T: ToString>(x: T) -> String = x.to_string()`, resolves `x.to_string()` through the bound and monomorphizes to the concrete impl at each call site. This is static dispatch with no runtime overhead. A user type participates by implementing the trait: `impl ToString for Point { ... }`.

## Printing and interpolation

`print` and string interpolation render any value whose type implements `ToString`. User types print as soon as they implement `ToString`. The `_int`-suffixed print builtins that predated the trait are removed as part of the io method-first conversion.

## Out of scope (deferred)

- `Hash for Char` and `Hash for Float`: need a `Char`-to-`Int` primitive and a defined float-bit hash; deferred until those land.
- Associated types on `Iterator` (the element type is a trait type parameter for now).
- Derivation (`derive(Eq, Hash)`): user types implement the traits explicitly for now.
- The lazy iterator adapter pipeline: specified in `std/iter`.
