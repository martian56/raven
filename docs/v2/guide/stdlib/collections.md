# std/collections

Hash-backed `Set<T>` and `Map<K, V>` over keys that implement the prelude
`Eq + Hash` traits. Storage is a hash-bucket layout, so lookup, insert, and
remove are O(1) average. `List<T>` is built into the language (literals,
indexing, `len`, `push`, `pop`, `get`) and needs no import.

```rust
import std/collections

fun main() {
    let seen = Set.new()
    seen.add("ada")
    print(seen.contains("ada"))     // true

    let counts = Map.new()
    counts.set("x", 1)
    print(counts.len())             // 1
}
```

## Importing

```rust
import std/collections
```

Import the whole module (not a selective `{ ... }` list). The idiomatic
constructor is the associated function `new()` on each type, called as
`Type.new()`. A bare `import std/collections` merges the `Set` and `Map`
`impl` blocks, so both the constructors and the instance methods resolve by
type. A selective import would bring in named functions but not the type
`impl` blocks, so `Set.new()` and the methods below would fail to resolve.

Element and key/value types are inferred from the first use: `s.add(1)`
fixes `s` as `Set<Int>`. Where no later use pins them, write the type
arguments on the call:

```rust
import std/collections

fun main() {
    let s = Set<Int>.new()
    let m = Map<String, Int>.new()
}
```

## Supported key types

`Hash` is implemented for `Int`, `Bool`, and `String`, and any user type
that implements both `Eq` and `Hash` works as an element or key. `Char` and
`Float` do not yet implement `Hash`, so `Set<Char>`, `Set<Float>`, and maps
keyed on them do not satisfy the bound.

| Key type | Usable as `Set<T>` / `Map<K, _>` |
|----------|----------------------------------|
| `Int` | yes |
| `Bool` | yes |
| `String` | yes |
| user type with `Eq + Hash` | yes |
| `Char` | no (not hashable) |
| `Float` | no (not hashable) |

## Set and map literals

Set and map values have literal syntax. Both lower to the constructors
above, so they need the same `import std/collections` in scope.

```rust
import std/collections

fun main() {
    let s = {1, 2, 2, 3}            // Set<Int>, dedups to {1, 2, 3}
    let m = ["a": 1, "b": 2]        // Map<String, Int>
    let flags = ["x": true]         // Map<String, Bool>
    let empty: Map<String, Int> = [:]
}
```

Grammar and disambiguation:

- A set literal is `{ e1, e2, ... }`: braces around one or more
  comma-separated expressions. It must carry at least one comma, so a
  single-element set is written `{x,}`. A bare `{ x }` stays a block whose
  tail expression is `x`, and `{}` is an empty block. An empty set is written
  `Set.new()`.
- A map literal is `[ k1: v1, k2: v2, ... ]`: brackets around one or more
  comma-separated `key: value` pairs. The empty map is the distinct `[:]`
  form, so the bare `[]` stays an empty list. A bracket whose first element
  has no top-level `:` is a list (`[1, 2, 3]`); a `:` after the first element
  makes it a map.

Element, key, and value types are inferred from the literal contents:
`{1, 2}` is `Set<Int>`, and `["x": true]` is `Map<String, Bool>`. The set's
`T` and the map's `K` must implement `Eq + Hash`, the same bound the types
carry. `{a, b}` desugars to `Set.new()` then `add(a); add(b)`, and `[k: v]`
desugars to `Map.new()` then `set(k, v)`.

## Set<T: Eq + Hash>

A set of distinct elements. Construct with a set literal `{1, 2}` or with
`Set.new()`.

### `new() -> Set<T>`

A new empty set. Called on the type: `Set.new()`.

### `len(self) -> Int`

The number of elements in the set.

### `is_empty(self) -> Bool`

True when the set has no elements.

### `contains(self, x: T) -> Bool`

True when `x` is in the set. Hashes `x` to a bucket and scans that bucket
with `Eq`.

### `add(self, x: T)`

Add `x` to the set. Does nothing if `x` is already present, so the set never
holds duplicates.

### `remove(self, x: T) -> Bool`

Remove `x` if present, returning whether it was. Order within a bucket is not
preserved.

```rust
import std/collections

fun main() {
    let s = Set.new()
    s.add("a")
    s.add("b")
    s.add("a")                  // already present, no-op

    print(s.len())              // 2
    print(s.contains("b"))      // true
    print(s.remove("b"))        // true
    print(s.remove("z"))        // false
    print(s.is_empty())         // false
}
```

### `to_list(self) -> List<T>`

Every element as a list, in hash-bucket order (not insertion order). This is
the way to enumerate a set: iterate the returned list.

```rust
import std/collections

fun main() {
    let s = {1, 2, 3}
    for x in s.to_list() {
        print(x)        // 1, 2, 3 (bucket order)
    }
}
```

### `union(self, other: Set<T>) -> Set<T>`

A new set with every element of `self` or `other`.

### `intersection(self, other: Set<T>) -> Set<T>`

A new set with the elements in both `self` and `other`.

### `difference(self, other: Set<T>) -> Set<T>`

A new set with the elements of `self` that are not in `other`.

### `is_subset(self, other: Set<T>) -> Bool`

True when every element of `self` is in `other`.

```rust
import std/collections

fun main() {
    let a = {1, 2, 3}
    let b = {2, 3, 4}

    print(a.union(b).len())         // 4
    print(a.intersection(b).len())  // 2
    print(a.difference(b).len())    // 1
    print({2, 3}.is_subset(a))      // true
}
```

## Map<K: Eq + Hash, V>

A map from keys to values. Construct with a map literal `["a": 1]` (or `[:]`
for an empty map) or with `Map.new()`.

### `new() -> Map<K, V>`

A new empty map. Called on the type: `Map.new()`.

### `len(self) -> Int`

The number of entries in the map.

### `is_empty(self) -> Bool`

True when the map has no entries.

### `has(self, k: K) -> Bool`

True when `k` has an entry in the map.

### `get(self, k: K) -> Option<V>`

`Some(v)` when `k` is present, otherwise `None`. Handle it with `match`:

```rust
import std/collections

fun main() {
    let m = Map.new()
    m.set("ada", 1815)

    match m.get("ada") {
        Some(year) -> print(year),      // 1815
        None -> print("missing"),
    }

    match m.get("grace") {
        Some(year) -> print(year),
        None -> print("missing"),       // missing
    }
}
```

### `get_or(self, k: K, default: V) -> V`

The value for `k`, or `default` when the key is absent. The non-`Option`
shortcut for `get` when you have a sensible fallback.

```rust
import std/collections

fun main() {
    let m = ["a": 1]
    print(m.get_or("a", 0))     // 1
    print(m.get_or("z", 0))     // 0
}
```

### `set(self, k: K, v: V)`

Insert `k` with value `v`, or overwrite the existing value when `k` is
already present.

### `keys(self) -> List<K>`

Every key in the map, in hash-bucket order (not insertion order).

### `values(self) -> List<V>`

Every value in the map, aligned with `keys()`: the value at index `i` in
`values()` belongs to the key at index `i` in `keys()`.

### `entries(self) -> List<Entry<K, V>>`

Every key/value pair as a list of `Entry`, in hash-bucket order. Each `Entry`
has a `.key` and a `.value` field.

```rust
import std/collections

fun main() {
    let m = ["a": 1, "b": 2]
    for e in m.entries() {
        print("${e.key}=${e.value}")    // a=1, then b=2 (bucket order)
    }
}
```

### `remove(self, k: K) -> Bool`

Remove the entry for `k` if present, returning whether it was. Order within a
bucket is not preserved.

```rust
import std/collections

fun main() {
    let m = Map.new()
    m.set("a", 1)
    m.set("b", 2)
    m.set("a", 10)              // overwrites

    print(m.len())              // 2
    print(m.has("a"))           // true
    print(m.remove("b"))        // true
    print(m.remove("z"))        // false
}
```

### `clear(self)`

Remove every entry, leaving an empty map.

```rust
import std/collections

fun main() {
    let m = ["a": 1, "b": 2]
    m.clear()
    print(m.len())              // 0
}
```

## Iteration order

Both types store entries in an array of buckets. The table starts with 8
buckets and doubles, rehashing every entry, once the load factor
(`count / bucket_count`) passes 0.75. Because of this, `keys()` and
`values()` follow the current bucket layout, not the order entries were
inserted. Do not rely on iteration order being stable across inserts or
across runs.

## Worked example: word frequencies

```rust
import std/collections
import std/string

fun main() {
    let words = ["red", "blue", "red", "green", "blue", "red"]

    let counts: Map<String, Int> = Map.new()
    let i = 0
    while i < words.len() {
        let w = words[i]
        match counts.get(w) {
            Some(n) -> counts.set(w, n + 1),
            None -> counts.set(w, 1),
        }
        i = i + 1
    }

    let ks = counts.keys()
    let j = 0
    while j < ks.len() {
        let k = ks[j]
        match counts.get(k) {
            Some(n) -> print("${k}: ${n}"),
            None -> print("${k}: 0"),
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
