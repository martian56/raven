# std/iter

Lazy, single-pass iterator pipelines. `std/iter` gives a `List<T>` an
`iter()` method that turns it into an iterator, a set of lazy adapters (`map`,
`filter`, `take`, `skip`, `enumerate`) that you chain with method calls, and a
set of consumers (`collect`, `count`, `fold`, `any`, `all`, `find`,
`for_each`) that drive a pipeline to completion.

```rust
import std/iter { collect }

fun main() {
    let xs = [1, 2, 3, 4, 5]
    let doubled = collect(xs.iter().map(fun(x: Int) -> Int = x * 2))
    for x in doubled {
        print(x)        // 2, 4, 6, 8, 10
    }
}
```

## Importing

The adapters (`map`, `filter`, `take`, `skip`, `enumerate`) are methods on the
iterator types, so they are available once you have an iterator. The consumers
are free functions, so you bring in the ones you use by name:

```rust
import std/iter { collect, fold }
```

You can list any of `collect`, `count`, `fold`, `any`, `all`, `find`, and
`for_each` inside the `{ ... }`. The bridge (`iter()` on `List<T>`) and the
adapter methods come along with the module.

## Lazy vs eager

The adapters are **lazy**: building a chain like
`xs.iter().map(f).filter(g)` does no work and touches no element. Each adapter
is a small struct that remembers its source iterator (and, where applicable, a
closure). Nothing runs until a **consumer** pulls elements through. A consumer
walks the chain one element at a time, in a single pass.

```rust
import std/iter { count }

fun main() {
    let xs = [1, 2, 3, 4]
    // No mapping happens here; this just describes a pipeline.
    let pipeline = xs.iter().map(fun(x: Int) -> Int = x * 10)
    // The consumer is what actually pulls elements through.
    print(count(pipeline))      // 4
}
```

Only a consumer that builds a list (`collect`) allocates. The other consumers
return a scalar or an `Option<T>` without allocating an intermediate list.

## The bridge: `List<T>` to an iterator

### `iter(self) -> ListIter<T>`

Defined on `List<T>`. Returns a `ListIter<T>` that walks the list by index.
This is the usual entry point into a pipeline.

```rust
import std/iter { collect }

fun main() {
    let xs = [10, 20, 30]
    for x in collect(xs.iter()) {
        print(x)                    // 10, 20, 30
    }
}
```

### `from_list<T>(xs: List<T>) -> ListIter<T>`

The free-function form of the same bridge, for when you prefer a call over a
method. `from_list(xs)` and `xs.iter()` produce the same `ListIter<T>`.

## Adapters

Each adapter is a generic struct over its element type(s) and a bounded source
`S: Iterator<T>`. The adapter methods are defined on every concrete iterator
type, so chaining is uniform: `xs.iter().map(f).filter(g).take(n)`. Each
adapter's `next` pulls from its source on demand.

### `map`

```rust
fun map<U>(self, f: fun(T) -> U) -> MapIter<T, U, Self>
```

Apply a closure to each element, yielding the result. The element type can
change (`T` to `U`).

```rust
import std/iter { collect }

fun main() {
    let xs = [1, 2, 3]
    let squares = collect(xs.iter().map(fun(x: Int) -> Int = x * x))
    for s in squares {
        print(s)    // 1, 4, 9
    }
}
```

### `filter`

```rust
fun filter(self, pred: fun(T) -> Bool) -> Filter<T, Self>
```

Keep only the elements for which `pred` returns `true`. `next` pulls from the
source until the predicate holds or the source is exhausted.

```rust
import std/iter { collect }

fun main() {
    let xs = [1, 2, 3, 4, 5, 6]
    let evens = collect(xs.iter().filter(fun(x: Int) -> Bool = x % 2 == 0))
    for e in evens {
        print(e)        // 2, 4, 6
    }
}
```

### `take`

```rust
fun take(self, n: Int) -> Take<T, Self>
```

Yield at most the first `n` elements, then stop. Because the pipeline is lazy,
later elements are never produced.

```rust
import std/iter { collect }

fun main() {
    let xs = [1, 2, 3, 4, 5]
    for x in collect(xs.iter().take(3)) {
        print(x)                            // 1, 2, 3
    }
}
```

### `skip`

```rust
fun skip(self, n: Int) -> Skip<T, Self>
```

Discard the first `n` elements, then pass the rest through.

```rust
import std/iter { collect }

fun main() {
    let xs = [1, 2, 3, 4, 5]
    for x in collect(xs.iter().skip(2)) {
        print(x)                            // 3, 4, 5
    }
}
```

### `enumerate`

```rust
fun enumerate(self) -> Enumerate<T, Self>
```

Pair each element with its running index, starting at `0`. Raven has no tuples
yet, so `enumerate` yields an `Indexed<T>` record instead of a `(Int, T)`
pair:

```rust
struct Indexed<T> {
    index: Int,
    value: T,
}
```

Read `.index` and `.value` off each element:

```rust
import std/iter { for_each }

fun main() {
    let xs = ["a", "b", "c"]
    for_each(
        xs.iter().enumerate(),
        fun(p: Indexed<String>) -> Unit = print("${p.index}: ${p.value}"),
    )
    // 0: a
    // 1: b
    // 2: c
}
```

`enumerate` is a terminal adapter in the chaining methods: it produces an
`Enumerate<T, S>`, which you hand directly to a consumer rather than chaining
further adapters onto.

## Consumers

Consumers are free generic functions over any `S: Iterator<T>`. A consumer is
what drives the pipeline, so every pipeline ends in exactly one consumer call.

### `collect<T, S: Iterator<T>>(it: S) -> List<T>`

Gather every remaining element into a `List<T>`, in order.

```rust
import std/iter { collect }

fun main() {
    let xs = [1, 2, 3]
    let out = collect(xs.iter())
    print(out.len())                // 3
}
```

### `count<T, S: Iterator<T>>(it: S) -> Int`

Count the remaining elements.

```rust
import std/iter { count }

fun main() {
    let xs = [1, 2, 3, 4]
    let odds = count(xs.iter().filter(fun(x: Int) -> Bool = x % 2 == 1))
    print(odds)     // 2
}
```

### `fold<T, A, S: Iterator<T>>(it: S, init: A, f: fun(A, T) -> A) -> A`

Left fold: start from `init` and combine the running accumulator with each
element.

```rust
import std/iter { fold }

fun main() {
    let xs = [1, 2, 3, 4]
    let sum = fold(xs.iter(), 0, fun(acc: Int, x: Int) -> Int = acc + x)
    print(sum)      // 10
}
```

### `any<T, S: Iterator<T>>(it: S, pred: fun(T) -> Bool) -> Bool`

True when at least one element satisfies `pred`. Stops at the first match.

### `all<T, S: Iterator<T>>(it: S, pred: fun(T) -> Bool) -> Bool`

True when every element satisfies `pred`. Vacuously true for an empty
iterator.

```rust
import std/iter { any, all }

fun main() {
    let xs = [2, 4, 6]
    print(any(xs.iter(), fun(x: Int) -> Bool = x > 5))      // true
    print(all(xs.iter(), fun(x: Int) -> Bool = x % 2 == 0)) // true
}
```

### `find<T, S: Iterator<T>>(it: S, pred: fun(T) -> Bool) -> Option<T>`

The first element satisfying `pred`, or `None` when none does.

```rust
import std/iter { find }

fun main() {
    let xs = [1, 3, 5, 8, 9]
    let first_even = find(xs.iter(), fun(x: Int) -> Bool = x % 2 == 0)
    print(match first_even {
        Some(v) -> v,
        None -> -1,
    })      // 8
}
```

### `for_each<T, S: Iterator<T>>(it: S, f: fun(T) -> Unit)`

Apply `f` to each element for its side effect.

```rust
import std/iter { for_each }

fun main() {
    let xs = [1, 2, 3]
    for_each(xs.iter(), fun(x: Int) -> Unit = print(x))
    // 1
    // 2
    // 3
}
```

## Worked example: combining adapters and a consumer

A pipeline is built with the adapter methods and then handed to a consumer.
Here `iter().filter(...).map(...).take(...)` describes the work lazily, and
`collect` runs it in a single pass:

```rust
import std/iter { collect }

fun main() {
    let xs = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]

    // filter keeps the even numbers, map scales them, take stops at three.
    let even = fun(x: Int) -> Bool = x % 2 == 0
    let scale = fun(x: Int) -> Int = x * 10
    let pipeline = xs.iter().filter(even).map(scale).take(3)

    for value in collect(pipeline) {
        print(value)        // 20, 40, 60
    }
}
```

Because `take(3)` is lazy, the chain only ever pulls enough elements to
produce three results; it never maps `80` or `100`.

You can swap the final consumer to ask a different question of the same
pipeline. With `fold` instead of `collect`:

```rust
import std/iter { fold }

fun main() {
    let xs = [1, 2, 3, 4, 5, 6]
    let total = fold(
        xs.iter().filter(fun(x: Int) -> Bool = x % 2 == 0),
        0,
        fun(acc: Int, x: Int) -> Int = acc + x,
    )
    print(total)        // 12
}
```

## for-loops

A `for x in <iterable>` loop drives any value whose type implements
`Iterator`, calling `next` until it returns `None`. Lists and ranges work
directly; an iterator pipeline works the same way, so you can loop over one
instead of calling a consumer:

```rust
import std/iter

fun main() {
    let xs = [1, 2, 3]
    for x in xs.iter().map(fun(x: Int) -> Int = x + 100) {
        print(x)        // 101, 102, 103
    }
}
```

## See also

- [std/string](string.md) for text methods you can `map` over.
- [std/io](io.md) for printing pipeline results.
- The [language reference](../language-reference.md) for closures, generics,
  and `for` loops.
