# std/collections Spec

Generic `Set<T>` and `Map<K, V>` over keys that implement the prelude `Eq`
trait. `List<T>` is built into the language (literals, indexing, `len`,
`push`, `pop`, `get`) and needs no import.

## Import

The collection types are used by method, but their constructors are free
functions, so a selective import binds them:

```raven
import std/collections { empty_set, empty_map }

fun main() {
    let s = empty_set()
    s.add(1)
    let m = empty_map()
    m.set("a", 10)
}
```

Importing the module also merges the `Set` and `Map` `impl` blocks, so the
methods resolve by receiver type. Element and key/value types are inferred
from the first use (`s.add(1)` fixes `Set<Int>`).

## Set<T: Eq>

| Method | Result | Notes |
|---|---|---|
| `len()` | `Int` | element count |
| `is_empty()` | `Bool` | |
| `contains(x)` | `Bool` | linear scan via `Eq` |
| `add(x)` | | adds only if absent |
| `remove(x)` | `Bool` | whether it was present; order not preserved |

Construct with `empty_set()`.

## Map<K: Eq, V>

| Method | Result | Notes |
|---|---|---|
| `len()` | `Int` | entry count |
| `is_empty()` | `Bool` | |
| `has(k)` | `Bool` | |
| `get(k)` | `Option<V>` | `None` when absent |
| `set(k, v)` | | inserts or overwrites |
| `remove(k)` | `Bool` | whether it was present; order not preserved |

Construct with `empty_map()`. Keys and values are stored in parallel
`List`s.

## Complexity

Lookup, insert, and remove are O(n): each scans the keys comparing through
`Eq`. This keeps the module small and dependency-free while exercising
generics, trait bounds, and `List`. Backing `Map`/`Set` with the runtime
hash-bucket layout (which already exists with FNV hashing) for O(1)
average operations is a planned optimization that will not change this
surface; it depends on the `Hash` trait being wired through the bucket
intrinsics.

## Out of scope

- Hash-backed storage (the optimization noted above).
- Ordered variants and `Deque<T>`.
- Iteration over a `Set`/`Map` (waits on returning a `List` of entries or
  an iterator adapter).
- Set algebra (union, intersection, difference).
