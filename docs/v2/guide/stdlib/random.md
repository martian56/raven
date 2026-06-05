# std/random

A small, reproducible pseudo-random generator. `Rng` is a value type holding
the generator state; its methods draw the next value and mutate that state in
place.

```raven
import std/random

fun main() {
    let rng = Rng.new(42)
    print(rng.gen_range(1, 7))      // a dice roll in [1, 7)
}
```

## Importing

```raven
import std/random
```

`std/random` adds a `struct Rng` and its `impl Rng` block, so a bare import
brings the constructors (`Rng.new`, `Rng.from_entropy`) and every method below
into scope. Import the whole module (not a selective `{ ... }` list).

This generator is **not** cryptographically secure. Do not use it for keys,
tokens, or anything where predictability is a risk.

## Reproducibility

The generator is splitmix64: the same seed always yields the same sequence of
draws. That determinism is the point of seeded construction, and it makes
`std/random` a good fit for reproducible tests and simulations.

`Rng.from_entropy()` instead seeds from a runtime source (a high-resolution
timestamp mixed with the process id), so its stream is not reproducible across
runs. Reach for it when you want a fresh, unpredictable sequence.

```raven
import std/random

fun main() {
    let a = Rng.new(7)
    let b = Rng.new(7)
    print(a.next_int() == b.next_int())     // true: same seed, same draw
}
```

## Constructing

### `Rng.new(seed: Int) -> Rng`

A generator seeded with `seed`. Deterministic: two generators built from the
same seed produce identical sequences.

### `Rng.from_entropy() -> Rng`

A generator seeded from a runtime time/pid source. Non-reproducible across
runs.

## Drawing numbers

### `next_int(self) -> Int`

The next raw 64-bit draw. Values span the full `Int` range and may be negative.

```raven
import std/random

fun main() {
    let rng = Rng.new(1)
    print(rng.next_int())       // some i64 value
    print(rng.next_int())       // the next one in the sequence
}
```

### `gen_range(self, lo: Int, hi: Int) -> Int`

A value in the half-open interval `[lo, hi)`: `lo` is possible, `hi` is not.

```raven
import std/random

fun main() {
    let rng = Rng.new(99)
    print(rng.gen_range(0, 6))      // one of 0, 1, 2, 3, 4, 5
}
```

The result is a non-negative draw reduced modulo `(hi - lo)`. The modulo
introduces a small bias when the range does not evenly divide the draw space,
but for typical small ranges the bias is negligible. If `lo >= hi` the function
returns `lo`.

### `next_float(self) -> Float`

A `Float` in `[0.0, 1.0)`. It takes the top 53 bits of a draw and scales by
2^-53 (the full f64 mantissa width), so every representable multiple of 2^-53
in the interval is reachable.

```raven
import std/random

fun main() {
    let rng = Rng.new(3)
    print(rng.next_float())     // e.g. 0.234...
}
```

### `next_bool(self) -> Bool`

`true` or `false` from the low bit of a draw, a fair coin flip.

```raven
import std/random

fun main() {
    let rng = Rng.new(5)
    print(rng.next_bool())      // true or false
}
```

## Working with lists

### `choice<T>(self, xs: List<T>) -> Option<T>`

A random element of `xs`, or `None` when the list is empty. Because the result
is an `Option<T>`, unwrap it with a `match` (or your preferred `Option` helper)
before use.

```raven
import std/random

fun main() {
    let rng = Rng.new(12)
    let colors = ["red", "green", "blue"]

    match rng.choice(colors) {
        Some(c) -> print(c),
        None -> print("empty"),
    }
}
```

### `shuffle<T>(self, xs: List<T>)`

Shuffles `xs` in place using Fisher-Yates. It returns nothing; the list passed
in is reordered.

```raven
import std/random

fun main() {
    let rng = Rng.new(2024)
    let deck = [1, 2, 3, 4, 5]
    rng.shuffle(deck)
    for card in deck {
        print(card)     // some permutation of 1, 2, 3, 4, 5
    }
}
```

## Worked example: a reproducible sampler

A seeded generator makes a test deterministic: pick the same seed and the
output never drifts between runs.

```raven
import std/random

fun main() {
    let rng = Rng.new(7)
    let words = ["alpha", "beta", "gamma", "delta"]

    // Draw three random words.
    let i = 0
    while i < 3 {
        match rng.choice(words) {
            Some(w) -> print(w),
            None -> print("(none)"),
        }
        i = i + 1
    }

    // Shuffle the deck in place and report a coin flip.
    rng.shuffle(words)
    for w in words {
        print(w)
    }
    print(rng.next_bool())
}
```

Run it twice with the same seed and you get byte-for-byte identical output.
Change the `7` to `Rng.from_entropy()` and each run differs.

## See also

- [std/math](math.md) for the numeric functions you will often pair with
  random draws.
- The [language reference](../language-reference.md) for `Option`, `match`, and
  generics.
