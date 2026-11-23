# std.collections

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

`Hash` is implemented for `int`, `bool`, `char`, `str`, and tuples of those.
A class or enum whose fields all hash gets `Eq` and `Hash` automatically, so it
works as an element or key too. `float` does not implement `Hash`, so
`set[float]` and dicts keyed on a float do not satisfy the bound.

| Key type | Usable as `set[T]` / `dict[K, _]` |
|----------|----------------------------------|
| `int`, `bool`, `char`, `str` | yes |
| tuples of hashable types | yes |
| a class or enum whose fields hash | yes |
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
    let s: set[str] = set()
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

Every element as a list, in insertion order. A `for` loop over the set
itself visits the same elements in the same order.

```rust
fun main() {
    let s = {1, 2, 3}
    for x in s {
        print(x) // 1, 2, 3 (insertion order)
    }
    print(s.to_list()) // [1, 2, 3]
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
    let m = {"ada": 1815}

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
already present. `m[k] = v` is the same call.

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
    let m = {"a": 1, "b": 2}
    m["a"] = 10 // overwrites

    print(len(m)) // 2
    print("a" in m) // true
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
fun main() {
    let words = ["red", "blue", "red", "green", "blue", "red"]

    var counts: dict[str, int] = {}
    for w in words {
        counts[w] = counts.get_or(w, 0) + 1
    }

    for (word, n) in counts {
        print("${word}: ${n}") // red: 3, blue: 2, green: 1
    }
}
```

## Deque[T]

A double-ended queue. `push_back`, `push_front`, `pop_back`, and `pop_front`
run in amortized constant time; the pops return `None` when it is empty.
`peek_front` and `peek_back` look without removing, and `to_list` returns
the items front to back.

```rust
from std.collections import Deque

fun main() {
    let d: Deque[int] = Deque.new()
    d.push_back(2)
    d.push_front(1)
    d.push_back(3)
    print(d.to_list()) // [1, 2, 3]
    print(d.pop_front().unwrap()) // 1
}
```

## PriorityQueue[T: Ord]

A binary heap that pops its smallest item first by `Ord.compare`. `push` and
`pop` take logarithmic time; `peek` looks at the smallest without removing
it. For largest first, give the items an `Ord` that compares in reverse.
Items that compare equal come out in no guaranteed order.

```rust
from std.collections import PriorityQueue

class Job {
    priority: int
    name: str
}

impl Ord for Job {
    fun compare(self, other: Job) -> int = self.priority - other.priority
}

fun main() {
    let q: PriorityQueue[Job] = PriorityQueue.new()
    q.push(Job(priority: 5, name: "report"))
    q.push(Job(priority: 1, name: "email"))
    print(q.pop().unwrap().name) // email
}
```

## See also

- The [core traits spec](../../specs/core-traits.md) for the `Eq` and `Hash`
  traits these types build on.
- The [std.hash spec](../../specs/std-hash.md) for hashing helpers.
- The [std.iter spec](../../specs/std-iter.md) for lazy pipelines over the
  lists returned by `keys()` and `values()`.
- The [language reference](../language-reference.md) for `Option`, `match`,
  and list literals.
