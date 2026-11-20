# std/iter

Lazy, single-pass iterator pipelines. `std/iter` gives a `list[T]` an
`iter()` method that turns it into an iterator, a set of lazy adapters (`map`,
`filter`, `take`, `skip`, `enumerate`) that you chain with method calls, and a
set of consumers (`collect`, `count`, `fold`, `any`, `all`, `find`,
`for_each`) that drive a pipeline to completion.

```rust
from std.iter import collect

fun main() {
    let xs = [1, 2, 3, 4, 5]
    let doubled = collect(xs.iter().map((x: int) -> int => x * 2))
    for x in doubled {
        print(x) // 2, 4, 6, 8, 10
    }
}
```

## Importing

The adapters (`map`, `filter`, `take`, `skip`, `enumerate`) are methods on the
iterator types, so they are available once you have an iterator. The consumers
are free functions, so you bring in the ones you use by name:

```rust
from std.iter import collect, fold
```

You can list any of `collect`, `count`, `fold`, `any`, `all`, `find`, and
`for_each` inside the `{ ... }`. The bridge (`iter()` on `list[T]`) and the
adapter methods come along with the module.

## Lazy vs eager

The adapters are **lazy**: building a chain like
`xs.iter().map(f).filter(g)` does no work and touches no element. Each adapter
is a small struct that remembers its source iterator (and, where applicable, a
closure). Nothing runs until a **consumer** pulls elements through. A consumer
walks the chain one element at a time, in a single pass.

```rust
from std.iter import count

fun main() {
    let xs = [1, 2, 3, 4]
    // No mapping happens here; this just describes a pipeline.
    let pipeline = xs.iter().map((x: int) -> int => x * 10)
    // The consumer is what actually pulls elements through.
    print(count(pipeline)) // 4
}
```

Only a consumer that builds a list (`collect`) allocates. The other consumers
return a scalar or an `Option[T]` without allocating an intermediate list.

## The bridge: `list[T]` to an iterator

### `iter(self) -> ListIter[T]`

Defined on `list[T]`. Returns a `ListIter[T]` that walks the list by index.
This is the usual entry point into a pipeline.

```rust
from std.iter import collect

fun main() {
    let xs = [10, 20, 30]
    for x in collect(xs.iter()) {
        print(x) // 10, 20, 30
    }
}
```

### `from_list[T](xs: list[T]) -> ListIter[T]`

The free-function form of the same bridge, for when you prefer a call over a
method. `from_list(xs)` and `xs.iter()` produce the same `ListIter[T]`.

## Adapters

Each adapter is a generic struct over its element type(s) and a bounded source
`S: Iterator[T]`. The adapter methods are defined on every concrete iterator
type, so chaining is uniform: `xs.iter().map(f).filter(g).take(n)`. Each
adapter's `next` pulls from its source on demand.

### `map`

```rust
fun map[U](self, f: fun(T) -> U) -> MapIter[T, U, Self]
```

Apply a closure to each element, yielding the result. The element type can
change (`T` to `U`).

```rust
from std.iter import collect

fun main() {
    let xs = [1, 2, 3]
    let squares = collect(xs.iter().map((x: int) -> int => x * x))
    for s in squares {
        print(s) // 1, 4, 9
    }
}
```

### `filter`

```rust
fun filter(self, pred: fun(T) -> bool) -> Filter[T, Self]
```

Keep only the elements for which `pred` returns `true`. `next` pulls from the
source until the predicate holds or the source is exhausted.

```rust
from std.iter import collect

fun main() {
    let xs = [1, 2, 3, 4, 5, 6]
    let evens = collect(xs.iter().filter((x: int) -> bool => x % 2 == 0))
    for e in evens {
        print(e) // 2, 4, 6
    }
}
```

### `take`

```rust
fun take(self, n: int) -> Take[T, Self]
```

Yield at most the first `n` elements, then stop. Because the pipeline is lazy,
later elements are never produced.

```rust
from std.iter import collect

fun main() {
    let xs = [1, 2, 3, 4, 5]
    for x in collect(xs.iter().take(3)) {
        print(x) // 1, 2, 3
    }
}
```

### `skip`

```rust
fun skip(self, n: int) -> Skip[T, Self]
```

Discard the first `n` elements, then pass the rest through.

```rust
from std.iter import collect

fun main() {
    let xs = [1, 2, 3, 4, 5]
    for x in collect(xs.iter().skip(2)) {
        print(x) // 3, 4, 5
    }
}
```

### `enumerate`

```rust
fun enumerate(self) -> Enumerate[T, Self]
```

Pair each element with its running index, starting at `0`. Raven has no tuples
yet, so `enumerate` yields an `Indexed[T]` record instead of a `(int, T)`
pair:

```rust
class Indexed[T] {
    index: int
    value: T
}
```

Read `.index` and `.value` off each element:

```rust
from std.iter import for_each

fun main() {
    let xs = ["a", "b", "c"]
    for_each(xs.iter().enumerate(), (p: Indexed[str]) => print("${p.index}: ${p.value}"))
}
// 0: a
// 1: b
// 2: c
```

`enumerate` is a terminal adapter in the chaining methods: it produces an
`Enumerate[T, S]`, which you hand directly to a consumer rather than chaining
further adapters onto.

## Consumers

Consumers are free generic functions over any `S: Iterator[T]`. A consumer is
what drives the pipeline, so every pipeline ends in exactly one consumer call.

### `collect[T, S: Iterator[T]](it: S) -> list[T]`

Gather every remaining element into a `list[T]`, in order.

```rust
from std.iter import collect

fun main() {
    let xs = [1, 2, 3]
    let out = collect(xs.iter())
    print(out.len()) // 3
}
```

### `count[T, S: Iterator[T]](it: S) -> int`

Count the remaining elements.

```rust
from std.iter import count

fun main() {
    let xs = [1, 2, 3, 4]
    let odds = count(xs.iter().filter((x: int) -> bool => x % 2 == 1))
    print(odds) // 2
}
```

### `fold[T, A, S: Iterator[T]](it: S, init: A, f: fun(A, T) -> A) -> A`

Left fold: start from `init` and combine the running accumulator with each
element.

```rust
from std.iter import fold

fun main() {
    let xs = [1, 2, 3, 4]
    let sum = fold(xs.iter(), 0, (acc: int, x: int) -> int => acc + x)
    print(sum) // 10
}
```

### `any[T, S: Iterator[T]](it: S, pred: fun(T) -> bool) -> bool`

True when at least one element satisfies `pred`. Stops at the first match.

### `all[T, S: Iterator[T]](it: S, pred: fun(T) -> bool) -> bool`

True when every element satisfies `pred`. Vacuously true for an empty
iterator.

```rust
from std.iter import any, all

fun main() {
    let xs = [2, 4, 6]
    print(any(xs.iter(), (x: int) -> bool => x > 5)) // true
    print(all(xs.iter(), (x: int) -> bool => x % 2 == 0)) // true
}
```

### `find[T, S: Iterator[T]](it: S, pred: fun(T) -> bool) -> Option[T]`

The first element satisfying `pred`, or `None` when none does.

```rust
from std.iter import find

fun main() {
    let xs = [1, 3, 5, 8, 9]
    let first_even = find(xs.iter(), (x: int) -> bool => x % 2 == 0)
    print(
        match first_even {
            case Some(v) { v }
            case None { -1 }
        }, // 8
    )
}
```

### `for_each[T, S: Iterator[T]](it: S, f: fun(T) -> Unit)`

Apply `f` to each element for its side effect.

```rust
from std.iter import for_each

fun main() {
    let xs = [1, 2, 3]
    for_each(xs.iter(), (x: int) => print(x))
}
// 1
// 2
// 3
```

### `sum[S: Iterator[int]](it: S) -> int`

Sum of every remaining element of an `int` iterator. `0` for an empty
iterator.

### `product[S: Iterator[int]](it: S) -> int`

Product of every remaining element of an `int` iterator. `1` for an empty
iterator.

```rust
from std.iter import sum, product

fun main() {
    print(sum([3, 1, 4, 1, 5, 9].iter())) // 23
    print(product([1, 2, 3, 4].iter())) // 24
}
```

### `min[T: Ord, S: Iterator[T]](it: S) -> Option[T]`

The smallest element by `Ord`, or `None` when the iterator is empty.

### `max[T: Ord, S: Iterator[T]](it: S) -> Option[T]`

The largest element by `Ord`, or `None` when the iterator is empty.

```rust
from std.iter import min, max

fun main() {
    let xs = [3, 1, 4, 1, 5, 9]
    print(
        match min(xs.iter()) {
            case Some(v) { v }
            case None { -1 }
        }, // 1
    )
    print(
        match max(xs.iter()) {
            case Some(v) { v }
            case None { -1 }
        }, // 9
    )
}
```

### `position[T, S: Iterator[T]](it: S, pred: fun(T) -> bool) -> Option[int]`

The index of the first element satisfying `pred`, or `None` when none does.

```rust
from std.iter import position

fun main() {
    let xs = [3, 1, 4, 1, 5]
    print(
        match position(xs.iter(), (x: int) -> bool => x == 4) {
            case Some(i) { i }
            case None { -1 }
        }, // 2
    )
}
```

### `nth[T, S: Iterator[T]](it: S, n: int) -> Option[T]`

The element at index `n` (zero-based), or `None` when the iterator is shorter.

```rust
from std.iter import nth

fun main() {
    let xs = [10, 20, 30, 40]
    print(
        match nth(xs.iter(), 2) {
            case Some(v) { v }
            case None { -1 }
        }, // 30
    )
}
```

### `last[T, S: Iterator[T]](it: S) -> Option[T]`

The final element, or `None` when the iterator is empty.

```rust
from std.iter import last

fun main() {
    let xs = [1, 2, 3]
    print(
        match last(xs.iter()) {
            case Some(v) { v }
            case None { -1 }
        }, // 3
    )
}
```

## Worked example: combining adapters and a consumer

A pipeline is built with the adapter methods and then handed to a consumer.
Here `iter().filter(...).map(...).take(...)` describes the work lazily, and
`collect` runs it in a single pass:

```rust
from std.iter import collect

fun main() {
    let xs = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]

    // filter keeps the even numbers, map scales them, take stops at three.
    let even = (x: int) -> bool => x % 2 == 0
    let scale = (x: int) -> int => x * 10
    let pipeline = xs.iter().filter(even).map(scale).take(3)

    for value in collect(pipeline) {
        print(value) // 20, 40, 60
    }
}
```

Because `take(3)` is lazy, the chain only ever pulls enough elements to
produce three results; it never maps `80` or `100`.

You can swap the final consumer to ask a different question of the same
pipeline. With `fold` instead of `collect`:

```rust
from std.iter import fold

fun main() {
    let xs = [1, 2, 3, 4, 5, 6]
    let total = fold(
        xs.iter().filter((x: int) -> bool => x % 2 == 0),
        0,
        (acc: int, x: int) -> int => acc + x,
    )
    print(total) // 12
}
```

## for-loops

A `for x in <iterable>` loop drives any value whose type implements
`Iterator`, calling `next` until it returns `None`. Lists and ranges work
directly; an iterator pipeline works the same way, so you can loop over one
instead of calling a consumer:

```rust
import std.iter

fun main() {
    let xs = [1, 2, 3]
    for x in xs.iter().map((x: int) -> int => x + 100) {
        print(x) // 101, 102, 103
    }
}
```

## See also

- [std/string](string.md) for text methods you can `map` over.
- [std/io](io.md) for printing pipeline results.
- The [language reference](../language-reference.md) for closures, generics,
  and `for` loops.
