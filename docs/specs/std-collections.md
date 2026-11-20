# std/collections Spec

Generic `Set<T>` and `Map<K, V>` over keys that implement the prelude
`Eq + Hash` traits. Storage keeps insertion order with an open-addressed index, so lookup, insert, and
remove are O(1) average. `List<T>` is built into the language (literals,
indexing, `len`, `push`, `pop`, `get`) and needs no import.

## Import

The idiomatic constructor is the associated function `new()` on each type,
called as `Type.new()`. A plain `import std/collections` is enough, since
the call resolves the function on the named type rather than an imported
name:

```rust
import std/collections

fun main() {
    let s = Set.new()
    s.add(1)
    let m = Map.new()
    m.set("a", 10)
}
```

Importing the module merges the `Set` and `Map` `impl` blocks, so both the
constructors and the instance methods resolve by type. Element and
key/value types are inferred from the first use (`s.add(1)` fixes
`Set<Int>`). Where no later use pins them, write the type arguments on the
call: `Set<Int>.new()`.

The free functions `empty_set()` and `empty_map()` remain available for a
selective import (`import std/collections { empty_set, empty_map }`) and
behave identically to `Set.new()` and `Map.new()`. They are superseded by
the set and map literals below and by the `new()` constructors; new code
should prefer those.

## Set and map literals

Set and map values have literal syntax (issue #156). Both lower to the
constructors above, so they need the same `import std/collections` in
scope.

```rust
import std/collections

fun main() {
    let s = {1, 2, 2, 3}        // Set<Int>, dedups to {1, 2, 3}
    let m = ["a": 1, "b": 2]    // Map<String, Int>
    let flags = ["x": true]     // Map<String, Bool>
    let empty: Map<String, Int> = [:]
}
```

Grammar and disambiguation:

- A set literal is `{ e1, e2, ... }`: braces around one or more
  comma-separated expressions. It must carry at least one comma, so a
  single-element set is written `{x,}`. A bare `{ x }` stays a block whose
  tail expression is `x`, and `{}` is an empty block, both unchanged. An
  empty set is written `Set.new()`.
- A map literal is `[ k1: v1, k2: v2, ... ]`: brackets around one or more
  comma-separated `key: value` pairs. The empty map is the distinct `[:]`
  form, so the bare `[]` stays an empty list. A bracket whose first element
  has no top-level `:` is a list (`[1, 2, 3]`); a `:` after the first
  element makes it a map.

Element, key, and value types are inferred from the literal contents
(`{1, 2}` is `Set<Int>`; `["x": true]` is `Map<String, Bool>`). The set's
`T` and the map's `K` must implement `Eq + Hash`, the same bound the types
carry. The desugaring is the same constructor-plus-insert sequence as
hand-written code: `{a, b}` builds `Set.new()` then `add(a); add(b)`, and
`[k: v]` builds `Map.new()` then `set(k, v)`.

## Set<T: Eq + Hash>

| Method | Result | Notes |
|---|---|---|
| `len()` | `Int` | element count |
| `is_empty()` | `Bool` | |
| `contains(x)` | `Bool` | probe the index table, compare via `Eq` |
| `add(x)` | | adds only if absent |
| `remove(x)` | `Bool` | whether it was present; order not preserved |

Construct with a set literal `{1, 2}`, `Set.new()`, or `empty_set()`.

## Map<K: Eq + Hash, V>

| Method | Result | Notes |
|---|---|---|
| `len()` | `Int` | entry count |
| `is_empty()` | `Bool` | |
| `has(k)` | `Bool` | |
| `get(k)` | `Option<V>` | `None` when absent |
| `set(k, v)` | | inserts or overwrites |
| `keys()` | `List<K>` | every key, insertion order |
| `values()` | `List<V>` | every value, aligned with `keys()` |
| `remove(k)` | `Bool` | whether it was present; order not preserved |

Construct with a map literal `["a": 1]` (or `[:]` for an empty map),
`Map.new()`, or `empty_map()`.

## Storage layout

Both types keep their items in dense lists in insertion order (`dict` has
parallel key, value, and hash lists; `set` has item and hash lists) plus an
index table of positions into those lists. The table size is a power of two;
a slot holds -1 when empty and -2 after a removal. A key's home slot is a
mixed form of its `hash()` masked to the table size, and lookup probes
linearly from there, comparing cached hashes before calling `Eq`.

The table starts with 8 slots and doubles once three quarters of its slots
are used, rebuilding from the dense lists. Removal marks the slot and moves
the most recently inserted item into the vacated position. Iteration
(`keys()`, `values()`, `entries()`, `to_list()`, and `Map` stringify in
std/json) follows insertion order.

## Complexity

Lookup, insert, and remove are O(1) average: a key hashes to its home slot,
and probe runs stay short under the 0.75 load factor. A resize is O(n)
in the entry count but amortizes to O(1) per insert.

## Supported key types

`Hash` is implemented for `Int`, `Bool`, and `String` (see
`docs/specs/core-traits.md`), and any user type that implements both
`Eq` and `Hash` works as a key or element. `Char` and `Float` do not yet
implement `Hash` (`Char` waits on a char-to-int primitive, and a `Float`
hash is deferred), so `Set<Char>`, `Set<Float>`, and maps keyed on them do
not satisfy the bound.

## Out of scope

- Ordered variants and `Deque<T>`.
- A `Set`/`Map` iterator adapter (the eager `keys()`/`values()` lists cover
  current iteration needs).
- Set algebra (union, intersection, difference).
