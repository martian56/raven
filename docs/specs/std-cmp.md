# std/cmp Spec

Free generic functions for ordering and sorting, built on the prelude
`Ord` trait (`fun compare(self, other: Self) -> Int`, negative when
`self < other`, zero when equal, positive when `self > other`). `List<T>`
is built into the language and needs no import.

## Import

These functions have no natural single receiver, so they are free
functions bound by a selective import:

```rust
import std/cmp { sort, max, min, max_of }

fun main() {
    let xs = [5, 2, 8, 1]
    let s = sort(xs)
    print(min(3, 7))
    print(max_of(xs))
}
```

## Surface

| Function | Result | Notes |
|---|---|---|
| `min<T: Ord>(a, b)` | `T` | the lesser; returns `a` on a tie |
| `max<T: Ord>(a, b)` | `T` | the greater; returns `a` on a tie |
| `clamp<T: Ord>(x, lo, hi)` | `T` | `lo` if `x < lo`, `hi` if `x > hi`, else `x` |
| `sort<T: Ord>(xs)` | `List<T>` | a new ascending list; the input is not mutated |
| `sorted_by<T>(xs, cmp)` | `List<T>` | sort by an explicit comparator `fun(T, T) -> Int`; no `Ord` bound |
| `max_of<T: Ord>(xs)` | `Option<T>` | largest element, `None` when empty |
| `min_of<T: Ord>(xs)` | `Option<T>` | smallest element, `None` when empty |

`sort` delegates to `sorted_by` with the comparator
`fun(a, b) -> Int = a.compare(b)`. Use `sorted_by` directly to sort by a
custom key or in descending order (for example `fun(a, b) -> Int = b - a`).

## Ordering vs comparator

`min`, `max`, `clamp`, `sort`, `max_of`, and `min_of` use the `Ord` bound
so any type that implements `compare` is usable, including user structs.
`sorted_by` takes the comparator as a value, so it needs no bound and can
order types that have no single natural ordering.

## Complexity

`sort` and `sorted_by` use selection sort: O(n^2) comparisons. This keeps
the module small and dependency-free while exercising generics, trait
bounds, closures, and `List`. A faster sort is a planned optimization that
will not change this surface.

## Out of scope

- Binary search and `contains_sorted`.
- Partial orders (`PartialOrd`) and NaN-aware float ordering.
- A stable-sort guarantee: selection sort here is not stable, so the
  relative order of elements that compare equal is unspecified.
- In-place sorting that mutates the input list.
