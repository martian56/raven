# std/random Spec

A small, reproducible pseudo-random generator. `Rng` is a value type
holding the generator state; its methods draw the next value and mutate
the state in place.

## Algorithm

The generator is splitmix64. Each draw advances a 64-bit state by the
constant `0x9E3779B97F4A7C15` and runs the result through two
xor-shift-multiply rounds and a final xor-shift. splitmix64 is fast, has
a full 2^64 period, and passes the usual statistical batteries, which
makes it a good default for seeding and general non-cryptographic use.

Raven `Int` is i64 and its arithmetic wraps two's-complement, so
splitmix64's unsigned add and multiply map directly onto `+` and `*` (the
multiplier constants are written as their signed i64 forms). The `>>`
operator is arithmetic (sign-propagating), so the finalizer's logical
right shifts go through a small `ushr` helper that masks off the bits the
sign would otherwise fill.

This generator is not cryptographically secure. Do not use it for keys,
tokens, or anything where predictability is a risk.

## Reproducibility

`Rng.new(seed)` is deterministic: two generators created with the same
seed produce the identical sequence of draws. This is the reproducibility
guarantee and the reason the seeded constructor exists separately from
entropy seeding.

`Rng.from_entropy()` seeds from a runtime source (a high-resolution
timestamp mixed with the process id), so its stream is not reproducible
across runs. Use it when you want a fresh, unpredictable sequence.

## Import

```raven
import std/random

fun main() {
    let rng = Rng.new(42)
    let n = rng.next_int()
    let d = rng.gen_range(0, 6)        // a value in [0, 6)
    let f = rng.next_float()           // a value in [0.0, 1.0)
}
```

## Surface

| Method | Result | Notes |
|---|---|---|
| `Rng.new(seed: Int)` | `Rng` | Deterministic seeding; same seed, same sequence. |
| `Rng.from_entropy()` | `Rng` | Non-reproducible seed from a time/pid source. |
| `next_int(self)` | `Int` | The next raw 64-bit draw. |
| `gen_range(self, lo, hi)` | `Int` | Uniform-ish in `[lo, hi)`. |
| `next_float(self)` | `Float` | In `[0.0, 1.0)`, 53-bit precision. |
| `next_bool(self)` | `Bool` | Low bit of a draw. |
| `choice<T>(self, xs)` | `Option<T>` | A random element, `None` if empty. |
| `shuffle<T>(self, xs)` | (unit) | In-place Fisher-Yates. |

## Notes

`gen_range(lo, hi)` reduces a non-negative draw modulo `(hi - lo)`. A
modulo introduces a small bias when the range does not evenly divide the
draw space (2^63 after the sign bit is masked off), but for typical small
ranges the bias is negligible. The interval is half-open: `lo` is
possible, `hi` is not. If `lo >= hi` the function returns `lo`.

`next_float` takes the top 53 bits of a draw and scales by 2^-53, the
full f64 mantissa width, so every representable multiple of 2^-53 in
`[0.0, 1.0)` is reachable.

The Int-to-Float construction and the entropy seed are provided by two
`extern "C"` runtime symbols (`raven_int_to_float`, `raven_random_entropy`)
because the v2 surface language has no Int-to-Float cast and no clock
primitive. They are resolved at link time against the runtime staticlib,
the same way `std/math` binds libm.
