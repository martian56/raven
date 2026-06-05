# std/math

Numeric constants and functions, importable with `import std/math { ... }`.
The module is bundled into the compiler as Raven source and merged into a
program the same way the other `std/*` modules are.

## Surface

Functions are free functions. Constants are zero-argument functions
returning `Float` (the language has no `Float` const items yet).

Constants:

| Function | Value |
|----------|-------|
| `pi() -> Float`  | 3.141592653589793 |
| `e() -> Float`   | 2.718281828459045 |
| `tau() -> Float` | 6.283185307179586 |

Integer functions:

| Function | Notes |
|----------|-------|
| `abs_int(x: Int) -> Int` | absolute value |
| `min_int(a: Int, b: Int) -> Int` | smaller of two |
| `max_int(a: Int, b: Int) -> Int` | larger of two |
| `clamp_int(x: Int, lo: Int, hi: Int) -> Int` | clamp into `[lo, hi]` |
| `pow_int(base: Int, exp: Int) -> Int` | power by squaring; a negative `exp` returns 0 |

Float functions:

| Function | Notes |
|----------|-------|
| `abs(x: Float) -> Float` | absolute value (C `fabs`) |
| `min(a: Float, b: Float) -> Float` | smaller of two |
| `max(a: Float, b: Float) -> Float` | larger of two |
| `clamp(x: Float, lo: Float, hi: Float) -> Float` | clamp into `[lo, hi]` |
| `sqrt(x: Float) -> Float` | square root |
| `pow(base: Float, exp: Float) -> Float` | power |
| `exp(x: Float) -> Float` | e raised to x |
| `ln(x: Float) -> Float` | natural logarithm (C `log`) |
| `log10(x: Float) -> Float` | base-10 logarithm |
| `sin`, `cos`, `tan` `(x: Float) -> Float` | radians |
| `floor`, `ceil`, `trunc`, `round` `(x: Float) -> Float` | rounding to a whole-valued Float |

Free functions, not methods. The language does not yet need an `impl Float`
surface for these, and free functions import cleanly through the existing
stdlib selector mechanism.

`min`/`max`/`clamp` overlap `std/cmp`, but those operate over the generic
`Ord` trait while these operate on numbers directly. The integer and float
variants are named distinctly (`min` vs `min_int`) because the language has
no overloading.

## Implementation: FFI to the C math library

The transcendental and rounding functions bind directly to the C runtime
math library through `extern "C"`:

```rust
extern "C" {
    fun sqrt(x: Float) -> Float
    fun pow(base: Float, exp: Float) -> Float
    ...
}
```

A Raven `Float` is a 64-bit IEEE double, the same as C `double`, so it is
passed and returned across the C ABI directly. The FFI spec
(`docs/v2/specs/ffi.md`) lists only integer and pointer FFI primitives and
no `CDouble`, but `Float` itself is accepted in an `extern` signature and
lowers to the f64 ABI type, which matches C `double`.

This was verified before committing to the approach. A probe declaring
`extern "C" { fun sqrt(x: Float) -> Float }` and calling `sqrt(2.0)`
compiled, linked against the CRT on `x86_64-pc-windows-msvc`, and printed
`1.4142135623730951`. Further probes confirmed `pow` (two doubles), `sin`,
`cos`, `tan`, `exp`, `log`, `log10`, `floor`, `ceil`, `trunc`, `round`, and
`fabs` all resolve and return correct values.

So the FFI path is used rather than a pure-Raven series approximation:
results are the platform libm's, with its accuracy, and there is no series
truncation to document. On `x86_64-pc-windows-msvc` these symbols come from
the CRT (`msvcrt`/`ucrt`), which the linker already puts on the link line;
no extra link flags are needed.

`ln` and `abs` are thin Raven wrappers over the C `log` and `fabs` symbols,
renamed to the conventional Raven spellings. The integer functions,
`pow_int`, and the constants are pure Raven.

## Accuracy

The transcendentals are the platform C library's, so accuracy matches the
host libm (correctly rounded or near-correctly-rounded doubles on a normal
desktop target). `pow_int` is exact for results that fit in a 64-bit signed
integer; it does not detect overflow.

## Out of scope

* Complex numbers.
* Arbitrary-precision / big integers.
* Statistics (mean, variance, and similar).
* Random number generation (tracked separately as `std/random`, issue #129).
* Methods on `Float`/`Int` (`x.sqrt()`); the surface is free functions.
* Float-to-Int and Int-to-Float conversions; the language has no numeric
  cast yet, so the rounding functions return a whole-valued `Float`.
