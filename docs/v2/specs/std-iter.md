# std/iter: the lazy iterator pipeline

`std/iter` is bundled Raven source compiled into a program when the user writes `import std/iter { ... }`. It provides lazy, single-pass iterator adapters and the consumers that drive them, built on the `Iterator<T>` trait from the `std/core` prelude:

```
trait Iterator<T> {
    fun next(self) -> Option<T>
}
```

## Design

An adapter is a small struct that stores its source iterator (and, where applicable, a captured closure) and implements `Iterator`. Its `next` pulls from the source on demand, so a chain such as `list.iter().map(f).filter(g)` performs no work until a consumer drives it. Each adapter is monomorphized at its concrete source and element types, and the stored closures are called through statically known function-pointer slots, so a finished pipeline runs in a single pass with no per-stage List allocation. Only a consumer that builds a List (`collect`) allocates.

## Bridge

`ListIter<T>` iterates a `List<T>` by index. `List<T>` gains an `iter()` method (defined with `impl<T> List<T>`) returning a `ListIter<T>`, plus a free `from_list<T>` constructor.

## Adapters

Each adapter is a generic struct over its element types and a bounded source `S: Iterator<T>`, and implements `next`:

- `MapIter<T, U, S>`: stores a closure `fun(T) -> U`; `next` maps each source element.
- `Filter<T, S>`: stores a predicate `fun(T) -> Bool`; `next` pulls until the predicate holds.
- `Take<T, S>`: yields at most n elements.
- `Skip<T, S>`: discards the first n, then passes through.
- `Enumerate<T, S>`: pairs each element with its running index. Raven has no tuples yet, so it yields an `Indexed<T> { index: Int, value: T }` record rather than `(Int, T)`.

## Chaining ergonomics

Raven has no blanket trait impls, so the adapter-constructor methods (`map`, `filter`, `take`, `skip`, `enumerate`) are defined on each concrete iterator type (`ListIter`, `MapIter`, `Filter`, `Take`, `Skip`) rather than once on all `Iterator` types. This is repetitive in the module source but gives users uniform method-call chaining: `xs.iter().map(f).filter(g).take(n)`. A generic-parameter ordering rule applies: a bound that mentions a sibling parameter (`S: Iterator<T>`) must list that parameter first, so each adapter lists its element types before its bounded source type.

## Consumers

Consumers are free generic functions over any `S: Iterator<T>`, since a consumer cannot be a method shared by every adapter without blanket impls:

- `collect<T, S>(it) -> List<T>`
- `count<T, S>(it) -> Int`
- `fold<T, A, S>(it, init, f) -> A`
- `any<T, S>(it, pred) -> Bool`, `all<T, S>(it, pred) -> Bool`
- `find<T, S>(it, pred) -> Option<T>`
- `for_each<T, S>(it, f)`

A pipeline is built with the adapter methods and then handed to a consumer: `collect(xs.iter().map(f).filter(g))`.

## How the element type is recovered for consumers

A consumer's element type `T` appears only in its `S: Iterator<T>` bound and (for `collect`/`fold`) its return type, never in a parameter position. Two mechanisms recover it:

- The type checker links `T` to the element of the concrete source through deferred iterator links (`add_iterator_link` / `solve_iterator_links` in the inference context), so the call site's result type is concrete (`collect(pipeline)` types as `List<Int>`).
- Monomorphization recovers `T` for the instantiation substitution by matching the consumer's declared return type against the concrete call result, since the arguments alone cannot bind a parameter that appears only in the return type or a bound. See the call lowering in `src/mir/lower/expr.rs`.

## for-loops

`for x in <iterable>` drives any value whose type implements `Iterator` by calling `next` until it returns `None`. Lists and ranges keep their direct counter/index lowering (see `docs/v2/specs/hir.md`); an arbitrary iterator, including a generic parameter bounded by `Iterator<T>`, iterates through the trait method. The loop binding takes the bound's element type.

## Out of scope

- `Zip` and `Chain` adapters: deferred.
- Tuples for `Enumerate` (uses the `Indexed<T>` record instead).
- Blanket trait impls that would let the adapter methods and consumers be written once for all `Iterator` types.
- Double-ended and exact-size iteration.
