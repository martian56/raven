# std/list

Free utility functions over the built-in `List<T>`. `List` ships with
`len`, `get`, `push`, and `pop` as built-in methods; this module adds the
common operations on top of them. Functions that build a new list return a
fresh `List` and never mutate their input.

```rust
import std/list

fun main() {
    print(list.contains([1, 2, 3], 2))      // true
    for x in list.reverse([1, 2, 3]) {
        print(x)                            // 3, then 2, 1
    }
}
```

## Importing

```rust
import std/list
```

A bare `import std/list` binds the module alias, so the functions are called
as `list.contains(...)`. To call them unqualified, list the names in a
selective import:

```rust
import std/list { contains, reverse, range }
```

`List<T>` itself is built into the language and needs no import for literals,
indexing, `len`, `get`, `push`, or `pop`.

## Searching

### `contains<T: Eq>(xs: List<T>, x: T) -> Bool`

True when `xs` holds a value equal to `x`. Bound by `T: Eq`.

### `index_of<T: Eq>(xs: List<T>, x: T) -> Int`

The index of the first element equal to `x`, or `-1` when absent.

```rust
import std/list

fun main() {
    let xs = [10, 20, 30]
    print(list.contains(xs, 20))    // true
    print(list.index_of(xs, 30))    // 2
    print(list.index_of(xs, 99))    // -1
}
```

## Slicing and combining

### `reverse<T>(xs: List<T>) -> List<T>`

A new list with the elements of `xs` in reverse order.

### `slice<T>(xs: List<T>, start: Int, end: Int) -> List<T>`

The half-open range `[start, end)` as a new list, clamped to the list bounds.

```rust
import std/list

fun main() {
    let xs = [10, 20, 30, 40]
    for x in list.reverse(xs) {
        print(x)        // 40, 30, 20, 10
    }
    for x in list.slice(xs, 1, 3) {
        print(x)        // 20, 30
    }
}
```

### `concat<T>(a: List<T>, b: List<T>) -> List<T>`

The elements of `a` followed by the elements of `b`, as a new list.

### `flatten<T>(xss: List<List<T>>) -> List<T>`

Flatten a list of lists into a single list, preserving order.

```rust
import std/list

fun main() {
    for x in list.concat([1, 2], [3, 4]) {
        print(x)        // 1, 2, 3, 4
    }
    for x in list.flatten([[1, 2], [3], [4, 5]]) {
        print(x)        // 1, 2, 3, 4, 5
    }
}
```

## First and last

### `first<T>(xs: List<T>) -> Option<T>`

The first element, or `None` for an empty list.

### `last<T>(xs: List<T>) -> Option<T>`

The last element, or `None` for an empty list.

```rust
import std/list

fun main() {
    let xs = [10, 20, 30]
    match list.first(xs) {
        Some(v) -> print(v),        // 10
        None -> print("empty"),
    }
    match list.last(xs) {
        Some(v) -> print(v),        // 30
        None -> print("empty"),
    }
}
```

## Building new lists

These return a new list and leave their input unchanged.

### `insert<T>(xs: List<T>, i: Int, x: T) -> List<T>`

A new list with `x` inserted at index `i`, clamped to `[0, len]`.

### `remove_at<T>(xs: List<T>, i: Int) -> List<T>`

A new list with the element at index `i` removed. An out-of-range index leaves
the list unchanged.

```rust
import std/list

fun main() {
    let xs = [10, 20, 30]
    for x in list.insert(xs, 1, 15) {
        print(x)        // 10, 15, 20, 30
    }
    for x in list.remove_at(xs, 0) {
        print(x)        // 20, 30
    }
}
```

### `repeat<T>(x: T, n: Int) -> List<T>`

A list containing `x` repeated `n` times. Empty for a non-positive `n`.

### `range(start: Int, end: Int) -> List<Int>`

The integers `[start, end)` as a list. Empty when `start >= end`.

```rust
import std/list

fun main() {
    for x in list.repeat(7, 3) {
        print(x)        // 7, 7, 7
    }
    for x in list.range(0, 4) {
        print(x)        // 0, 1, 2, 3
    }
}
```

## See also

- [std/cmp](cmp.md) for `sort`, `min_of`, and `max_of` over lists.
- [std/iter](../../specs/std-iter.md) for `map`, `filter`, and `fold`.
- The [language reference](../language-reference.md) for list literals,
  indexing, generics, and `Option`.
