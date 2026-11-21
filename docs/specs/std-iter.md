# std.iter: the lazy iterator pipeline

`std.iter` is bundled Raven source compiled into a program that imports it, as in `from std.iter import collect, fold`. It provides lazy, single-pass iterator adapters and the consumers that drive them, built on the `Iterator[T]` trait from the `std.core` prelude:

```rust
trait Iterator[T] {
    fun next(self) -> Option[T]
}
```

## Design

An adapter is a small class that stores its source iterator (and, where applicable, a captured closure) and implements `next`. Its `next` pulls from the source on demand, so a chain such as `xs.iter().map(f).filter(g)` performs no work until a consumer drives it. Each adapter is compiled once per concrete source and element type, so a finished pipeline runs in a single pass with no intermediate list. Only a consumer that builds a list (`collect`) allocates.

## Bridge

`ListIter[T]` iterates a `list[T]` by index. `list[T]` gains an `iter()` method returning a `ListIter[T]`, and `from_list(xs)` is the free-function form.

## Adapters

Each adapter is a generic class over its element types and a bounded source `S: Iterator[T]`:

- `MapIter[T, U, S]`: stores a closure `fun(T) -> U`; `next` maps each source element.
- `Filter[T, S]`: stores a predicate `fun(T) -> bool`; `next` pulls until the predicate holds.
- `Take[T, S]`: yields at most n elements.
- `Skip[T, S]`: discards the first n, then passes through.
- `Enumerate[T, S]`: pairs each element with its running index as an `Indexed[T]` record with `index` and `value` fields.

For plain lists, `for (i, x) in xs.enumerate()` is the simpler form and needs no import.

## Chaining

Raven has no blanket trait impls, so the adapter methods (`map`, `filter`, `take`, `skip`, `enumerate`) are defined on each concrete iterator class rather than once for every `Iterator`. That is repetitive in the module source but gives uniform chaining: `xs.iter().map(f).filter(g).take(n)`. A bound that mentions a sibling parameter (`S: Iterator[T]`) must list that parameter first, so each adapter lists its element types before its bounded source type.

## Consumers

Consumers are free generic functions over any `S: Iterator[T]`:

- `collect(it) -> list[T]`
- `count(it) -> int`
- `fold(it, init, f) -> A`
- `any(it, pred) -> bool`, `all(it, pred) -> bool`
- `find(it, pred) -> Option[T]`, `position(it, pred) -> Option[int]`
- `nth(it, n) -> Option[T]`, `last(it) -> Option[T]`
- `min(it)`, `max(it)` for an `Ord` element, `sum(it)` and `product(it)` for `int`
- `for_each(it, f)`

A pipeline is built with the adapter methods and then handed to a consumer:

```rust
from std.iter import collect

fun main() {
    let xs = [1, 2, 3, 4, 5]
    print(collect(xs.iter().map(x => x * 10).filter(x => x > 20))) // [30, 40, 50]
}
```

## How a consumer's element type is found

A consumer's element type `T` appears only in its `S: Iterator[T]` bound and its return type, never in a parameter. The type checker links `T` to the element of the concrete source (`add_iterator_link` and `solve_iterator_links` in the inference context), so `collect(pipeline)` types as `list[int]`. Checks that need the element type, such as printing the result, run after those links are solved.

## for loops

`for x in it` drives any value whose type implements `Iterator` by calling `next` until it returns `None`. Lists and ranges keep their direct index and counter loops; an arbitrary iterator, including a generic parameter bounded by `Iterator[T]`, iterates through the trait method.

## Not included

- `Zip` and `Chain` adapters.
- Blanket trait impls that would let the adapter methods and consumers be written once for all `Iterator` types.
- Double-ended and exact-size iteration.
