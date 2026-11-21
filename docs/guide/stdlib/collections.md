# std/collections

Hash-backed `set[T]` and `dict[K, V]` over keys that implement the prelude
`Eq + Hash` traits. Lookup, insert, and remove are O(1) average, and
iteration follows insertion order. Both are built in, like `list[T]`, so
they need no import.

```rust
fun main() {
    let seen = {"ada"}
    seen.add("grace")
    print("ada" in seen) // true

    let counts = {"x": 1}
    counts["y"] = 2
    print(counts["x"], len(counts)) // 1 2
    for (name, n) in counts {
        print(name, n)
    }
}
```

## Empty collections

`{}` is an empty dict and `set()` an empty set. Their element types come from
the first use, or from an annotation when nothing later pins them:

```rust
fun main() {
    let s: set[int] = set()
    let m: dict[str, int] = {}
    print(len(s), len(m)) // 0 0
}
```

## Supported key types

`Hash` is implemented for `int`, `bool`, and `str`, and any user type
that implements both `Eq` and `Hash` works as an element or key. `char` and
`float` do not yet implement `Hash`, so `set[char]`, `set[float]`, and maps
keyed on them do not satisfy the bound.

| Key type | Usable as `set[T]` / `dict[K, _]` |
|----------|----------------------------------|
| `int` | yes |
| `bool` | yes |
| `str` | yes |
| user type with `Eq + Hash` | yes |
| `char` | no (not hashable) |
| `float` | no (not hashable) |

## Set and map literals

Set and map values have literal syntax:

```rust
fun main() {
    let s = {1, 2, 2, 3} // set[int], dedups to {1, 2, 3}
    let m = {"a": 1, "b": 2} // dict[str, int]
    let flags = {"x": true} // dict[str, bool]
    let empty: dict[str, int] = {}
}
```

How braces read:

- `{a, b}` and `{a}` are sets; `{k: v, ...}` is a dict; `{}` is an empty
  dict and `set()` an empty set.
- `[x for x in xs]`, `{x for x in xs}`, and `{k: v for k in xs}` build a list,
  set, or dict from a loop.

Element, key, and value types are inferred from the literal contents:
`{1, 2}` is `set[int]`, and `{"x": true}` is `dict[str, bool]`. The set's
`T` and the dict's `K` must implement `Eq + Hash`.

## set[T: Eq + Hash]

A set of distinct elements. Construct with a set literal `{1, 2}`, or
`set()` for an empty one.

### `new() -> set[T]`

A new empty set, the same as `set()`.

### `len(self) -> int`

The number of elements in the set.

### `is_empty(self) -> bool`

True when the set has no elements.

### `contains(self, x: T) -> bool`

True when `x` is in the set. Hashes `x` and probes the index table,
comparing with `Eq`.

### `add(self, x: T)`

Add `x` to the set. Does nothing if `x` is already present, so the set never
holds duplicates.

### `remove(self, x: T) -> bool`

Remove `x` if present, returning whether it was. The most recently added
item moves into its position.

```rust
fun main() {
    let s = Set.new()
    s.add("a")
    s.add("b")
    s.add("a") // already present, no-op

    print(s.len()) // 2
    print(s.contains("b")) // true
    print(s.remove("b")) // true
    print(s.remove("z")) // false
    print(s.is_empty()) // false
}
```

### `to_list(self) -> list[T]`

Every element as a list, in insertion order. This is the way to enumerate
a set: iterate the returned list.

```rust
fun main() {
    let s = {1, 2, 3}
    for x in s.to_list() {
        print(x) // 1, 2, 3 (insertion order)
    }
}
```

### `union(self, other: set[T]) -> set[T]`

A new set with every element of `self` or `other`.

### `intersection(self, other: set[T]) -> set[T]`

A new set with the elements in both `self` and `other`.

### `difference(self, other: set[T]) -> set[T]`

A new set with the elements of `self` that are not in `other`.

### `is_subset(self, other: set[T]) -> bool`

True when every element of `self` is in `other`.

```rust
fun main() {
    let a = {1, 2, 3}
    let b = {2, 3, 4}

    print(a.union(b).len()) // 4
    print(a.intersection(b).len()) // 2
    print(a.difference(b).len()) // 1
    print({2, 3}.is_subset(a)) // true
}
```

## dict[K: Eq + Hash, V]

A map from keys to values. Construct with a dict literal `{"a": 1}`, or `{}`
for an empty one. `d[k]` reads a value (panicking when `k` is missing),
`d[k] = v` sets one, and `k in d` tests for a key.

### `new() -> dict[K, V]`

A new empty dict, the same as `{}`.

### `len(self) -> int`

The number of entries in the map.

### `is_empty(self) -> bool`

True when the map has no entries.

### `has(self, k: K) -> bool`

True when `k` has an entry in the map.

### `get(self, k: K) -> Option[V]`

`Some(v)` when `k` is present, otherwise `None`. Handle it with `match`:

```rust
fun main() {
    let m = Map.new()
    m.set("ada", 1815)

    match m.get("ada") {
        case Some(year) { print(year) } // 1815
        case None { print("missing") }
    }

    match m.get("grace") {
        case Some(year) { print(year) }
        case None { print("missing") } // missing
    }
}
```

### `get_or(self, k: K, default: V) -> V`

The value for `k`, or `default` when the key is absent. The non-`Option`
shortcut for `get` when you have a sensible fallback.

```rust
fun main() {
    let m = {"a": 1}
    print(m.get_or("a", 0)) // 1
    print(m.get_or("z", 0)) // 0
}
```

### `set(self, k: K, v: V)`

Insert `k` with value `v`, or overwrite the existing value when `k` is
already present.

### `keys(self) -> list[K]`

Every key in the map, in insertion order.

### `values(self) -> list[V]`

Every value in the map, aligned with `keys()`: the value at index `i` in
`values()` belongs to the key at index `i` in `keys()`.

### `entries(self) -> list[Entry[K, V]]`

Every key/value pair as a list of `Entry`, in insertion order. Each `Entry`
has a `.key` and a `.value` field.

```rust
fun main() {
    let m = {"a": 1, "b": 2}
    for e in m.entries() {
        print("${e.key}=${e.value}") // a=1, then b=2 (insertion order)
    }
}
```

### `remove(self, k: K) -> bool`

Remove the entry for `k` if present, returning whether it was. The most
recently added entry moves into its position.

```rust
fun main() {
    let m = Map.new()
    m.set("a", 1)
    m.set("b", 2)
    m.set("a", 10) // overwrites

    print(m.len()) // 2
    print(m.has("a")) // true
    print(m.remove("b")) // true
    print(m.remove("z")) // false
}
```

### `clear(self)`

Remove every entry, leaving an empty map.

```rust
fun main() {
    let m = {"a": 1, "b": 2}
    m.clear()
    print(m.len()) // 0
}
```

## Iteration order

Both types keep their items in dense lists in insertion order and find them
through an open-addressed index table, which doubles once it is three quarters
full. `keys()`, `values()`, `entries()`, and `to_list()` follow insertion
order. Removing an item moves the most recently inserted one into its place.

## Worked example: word frequencies

```rust
import std.collections
import std.string

fun main() {
    let words = ["red", "blue", "red", "green", "blue", "red"]

    let counts: dict[str, int] = {}
    var i = 0
    while i < words.len() {
        let w = words[i]
        match counts.get(w) {
            case Some(n) { counts.set(w, n + 1) }
            case None { counts.set(w, 1) }
        }
        i = i + 1
    }

    let ks = counts.keys()
    var j = 0
    while j < ks.len() {
        let k = ks[j]
        match counts.get(k) {
            case Some(n) { print("${k}: ${n}") }
            case None { print("${k}: 0") }
        }
        j = j + 1
    }
}
```

## See also

- The [core traits spec](../../specs/core-traits.md) for the `Eq` and `Hash`
  traits these types build on.
- The [std/hash spec](../../specs/std-hash.md) for hashing helpers.
- The [std/iter spec](../../specs/std-iter.md) for working with the lists
  returned by `keys()` and `values()`.
- The [language reference](../language-reference.md) for `Option`, `match`,
  and list literals.
