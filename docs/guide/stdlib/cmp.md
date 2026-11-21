# std.cmp

Free generic functions for ordering and sorting. They build on the prelude
`Ord` trait, so any type that implements `compare` works with them, including
your own classes.

```rust
from std.cmp import min, max, sort

fun main() {
    print(min(3, 7)) // 3
    print(max(3, 7)) // 7
    for x in sort([5, 2, 8, 1]) {
        print(x) // 1, then 2, 5, 8
    }
}
```

## Importing

These functions have no natural single receiver, so they come in through a
selective import: list the names you want inside `{ ... }`.

```rust
from std.cmp import sort, sorted_by, min, max, clamp, max_of, min_of
```

`list[T]` is built into the language and needs no import.

## The `Ord` bound

Most functions here are bound by `T: Ord`. The prelude `Ord` trait is one
method:

```rust
fun compare(self, other: Self) -> int
```

`compare` returns a negative `int` when `self < other`, zero when they are
equal, and a positive `int` when `self > other`. Built-in types like `int`
and `str` already satisfy `Ord`, and any class that implements `compare`
becomes usable with every `Ord`-bound function below.

The one exception is `sorted_by`, which takes the comparator as a value
instead of relying on the bound (see below).

## Comparing two values

### `min[T: Ord](a: T, b: T) -> T`

The lesser of `a` and `b`. On a tie (`a` and `b` compare equal) it returns
`a`.

### `max[T: Ord](a: T, b: T) -> T`

The greater of `a` and `b`. On a tie it returns `a`.

```rust
from std.cmp import min, max

fun main() {
    print(min(10, 4)) // 4
    print(max("apple", "pear")) // pear
}
```

### `clamp[T: Ord](x: T, lo: T, hi: T) -> T`

Constrain `x` to the range `[lo, hi]`: returns `lo` when `x < lo`, `hi` when
`x > hi`, and `x` otherwise.

```rust
from std.cmp import clamp

fun main() {
    print(clamp(15, 0, 10)) // 10
    print(clamp(-3, 0, 10)) // 0
    print(clamp(7, 0, 10)) // 7
}
```

## Sorting

### `sort[T: Ord](xs: list[T]) -> list[T]`

A new list with the elements of `xs` in ascending order. The input is not
mutated. `sort` delegates to `sorted_by` using `a.compare(b)` as the
comparator.

```rust
from std.cmp import sort

fun main() {
    let xs = [5, 2, 8, 1]
    for x in sort(xs) {
        print(x) // 1, 2, 5, 8
    }
    print(xs[0]) // 5 (the original list is unchanged)
}
```

### `sorted_by[T](xs: list[T], cmp: fun(T, T) -> int) -> list[T]`

Sort `xs` by an explicit comparator, returning a new list. There is no `Ord`
bound here: because the comparator is passed in as a value, `sorted_by` can
order types that have no single natural ordering, or order an `Ord` type in a
different way.

The comparator follows the same convention as `compare`: given two elements
`a` and `b`, return a negative `int` when `a` should come before `b`, zero
when their order does not matter, and a positive `int` when `a` should come
after `b`. To sort descending, flip the comparison.

```rust
from std.cmp import sorted_by

fun main() {
    let xs = [5, 2, 8, 1]
    let desc = sorted_by(xs, (a: int, b: int) -> int => b - a)
    for x in desc {
        print(x) // 8, 5, 2, 1
    }
}
```

Both `sort` and `sorted_by` use selection sort, which is O(n^2) comparisons
and is not stable: the relative order of elements that compare equal is
unspecified.

## Reducing a list

### `max_of[T: Ord](xs: list[T]) -> Option[T]`

The largest element of `xs`, or `None` when the list is empty. Wrapping the
result in `Option` is what lets the empty case be represented without a
sentinel value.

### `min_of[T: Ord](xs: list[T]) -> Option[T]`

The smallest element of `xs`, or `None` when the list is empty.

Because both return an `Option[T]`, you match on the result to handle the
empty list:

```rust
from std.cmp import max_of, min_of

fun main() {
    let scores = [42, 17, 99, 8]
    match max_of(scores) {
        case Some(best) { print(best) } // 99
        case None { print("no scores") }
    }

    let empty: list[int] = []
    match min_of(empty) {
        case Some(lo) { print(lo) }
        case None { print("empty") } // empty
    }
}
```

## Worked example: sort and pick the top

```rust
from std.cmp import sort, max_of

fun main() {
    let scores = [42, 17, 99, 8, 56]

    let ranked = sort(scores)
    for s in ranked {
        print(s) // 8, 17, 42, 56, 99
    }

    match max_of(scores) {
        case Some(best) { print("top score: ${best}") } // top score: 99
        case None { print("no scores yet") }
    }
}
```

## See also

- [std.string](string.md) for text, whose `str` values are `Ord` and so
  work with every function here.
- The [language reference](../language-reference.md) for generics, trait
  bounds, closures, and `Option`.
