# std/math

Numeric constants and functions. Everything in `std/math` is a free
function, so you bring names into scope with a selective import:

```rust
import std/math { sqrt, pow_int }

fun main() {
    print(sqrt(2.0))        // 1.4142135623730951
    print(pow_int(2, 10))   // 1024
}
```

## Importing

```rust
import std/math { sqrt, pow, abs_int, pi }
```

List exactly the names you use inside the `{ ... }`. The transcendental and
rounding functions (`sqrt`, `sin`, `floor`, ...) bind to the platform C math
library, so their accuracy is the host libm's. The integer helpers,
`pow_int`, and the constants are pure Raven.

## Int and Float are separate

Raven has no implicit `Int` to `Float` conversion, and there is no numeric
overloading. Integer math and float math therefore come as two distinct sets
of functions, named apart (`min_int` vs `min`, `abs_int` vs `abs`). Pass an
`Int` to the `_int` helpers and a `Float` to the float helpers. The language
has no numeric cast yet, so the rounding functions (`floor`, `round`, and
friends) return a whole-valued `Float`, not an `Int`.

## Integer functions

All take and return `Int`.

### `abs_int(x: Int) -> Int`

Absolute value.

### `min_int(a: Int, b: Int) -> Int` and `max_int(a: Int, b: Int) -> Int`

The smaller and the larger of two integers.

### `clamp_int(x: Int, lo: Int, hi: Int) -> Int`

Clamp `x` into the closed range `[lo, hi]`: return `lo` if `x < lo`, `hi` if
`x > hi`, otherwise `x`.

### `pow_int(base: Int, exp: Int) -> Int`

`base` raised to `exp`, computed by squaring. A negative `exp` has no integer
result and returns `0`. The result is exact when it fits in a 64-bit signed
integer; overflow is not detected.

```rust
import std/math { abs_int, min_int, max_int, clamp_int, pow_int }

fun main() {
    print(abs_int(0 - 7))           // 7
    print(min_int(3, 9))            // 3
    print(max_int(3, 9))            // 9
    print(clamp_int(15, 0, 10))     // 10
    print(pow_int(2, 8))            // 256
}
```

## Float functions

All take and return `Float`.

### `abs(x: Float) -> Float`

Absolute value (the C `fabs`).

### `min(a: Float, b: Float) -> Float` and `max(a: Float, b: Float) -> Float`

The smaller and the larger of two floats.

### `clamp(x: Float, lo: Float, hi: Float) -> Float`

Clamp `x` into the closed range `[lo, hi]`.

### `sqrt(x: Float) -> Float`

Square root.

### `pow(base: Float, exp: Float) -> Float`

`base` raised to `exp`.

### `exp(x: Float) -> Float`

`e` raised to `x`.

### `ln(x: Float) -> Float`

Natural logarithm (the C `log`).

### `floor(x: Float) -> Float`, `ceil(x: Float) -> Float`, `round(x: Float) -> Float`, `trunc(x: Float) -> Float`

Round toward minus infinity, toward plus infinity, to the nearest whole
value, and toward zero. Each returns a whole-valued `Float`.

### `sin(x: Float) -> Float` and `cos(x: Float) -> Float`

Sine and cosine, with the angle in radians.

```rust
import std/math { sqrt, pow, exp, ln, abs, min, max, clamp, floor, ceil, round, trunc, sin, cos }

fun main() {
    print(sqrt(2.0))            // 1.4142135623730951
    print(pow(2.0, 10.0))       // 1024.0
    print(abs(0.0 - 3.5))       // 3.5
    print(min(2.5, 1.5))        // 1.5
    print(clamp(9.0, 0.0, 1.0)) // 1.0
    print(floor(2.7))           // 2.0
    print(ceil(2.1))            // 3.0
    print(round(2.5))           // 3.0
    print(trunc(0.0 - 2.7))     // -2.0
}
```

## Constants

The language has no `Float` const items yet, so the constants are
zero-argument functions returning `Float`. Call them with `()`.

### `pi() -> Float`

3.141592653589793

### `e() -> Float`

2.718281828459045

### `tau() -> Float`

6.283185307179586 (a full turn, `2 * pi`).

```rust
import std/math { pi, e, tau, sin }

fun main() {
    print(pi())             // 3.141592653589793
    print(tau())            // 6.283185307179586
    print(sin(pi()))        // ~0 (libm rounding)
}
```

## Worked example: distance between two points

`sqrt` plus `pow` gives the Euclidean distance between two points.

```rust
import std/math { sqrt, pow }

fun distance(x1: Float, y1: Float, x2: Float, y2: Float) -> Float {
    let dx = x2 - x1
    let dy = y2 - y1
    return sqrt(pow(dx, 2.0) + pow(dy, 2.0))
}

fun main() {
    print(distance(0.0, 0.0, 3.0, 4.0))     // 5.0
}
```

## See also

- [std/cmp](cmp.md) for `min`/`max`/`clamp` over the generic `Ord` trait
  rather than numbers directly.
- [std/random](random.md) for random number generation.
- The [language reference](../language-reference.md) for the `Int` and
  `Float` types and numeric literals.
