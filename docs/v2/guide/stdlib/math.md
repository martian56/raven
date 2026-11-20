# std/math

Numeric constants and functions. Everything in `std/math` is a free
function, so you bring names into scope with a selective import:

```rust
from std.math import sqrt, pow_int

fun main() {
    print(sqrt(2.0)) // 1.4142135623730951
    print(pow_int(2, 10)) // 1024
}
```

## Importing

```rust
from std.math import sqrt, pow, abs_int, pi
```

List exactly the names you use inside the `{ ... }`. The transcendental and
rounding functions (`sqrt`, `sin`, `floor`, ...) bind to the platform C math
library, so their accuracy is the host libm's. The integer helpers,
`pow_int`, and the constants are pure Raven.

## int and float are separate

Raven has no implicit `int` to `float` conversion, and there is no numeric
overloading. Integer math and float math therefore come as two distinct sets
of functions, named apart (`min_int` vs `min`, `abs_int` vs `abs`). Pass an
`int` to the `_int` helpers and a `float` to the float helpers. The language
has no numeric cast yet, so the rounding functions (`floor`, `round`, and
friends) return a whole-valued `float`, not an `int`.

## Integer functions

All take and return `int`.

### `abs_int(x: int) -> int`

Absolute value.

### `min_int(a: int, b: int) -> int` and `max_int(a: int, b: int) -> int`

The smaller and the larger of two integers.

### `clamp_int(x: int, lo: int, hi: int) -> int`

Clamp `x` into the closed range `[lo, hi]`: return `lo` if `x < lo`, `hi` if
`x > hi`, otherwise `x`.

### `pow_int(base: int, exp: int) -> int`

`base` raised to `exp`, computed by squaring. A negative `exp` has no integer
result and returns `0`. The result is exact when it fits in a 64-bit signed
integer; a result that would overflow aborts with `pow_int overflow` rather
than wrapping.

```rust
from std.math import abs_int, min_int, max_int, clamp_int, pow_int

fun main() {
    print(abs_int(0 - 7)) // 7
    print(min_int(3, 9)) // 3
    print(max_int(3, 9)) // 9
    print(clamp_int(15, 0, 10)) // 10
    print(pow_int(2, 8)) // 256
}
```

### `gcd(a: int, b: int) -> int`

Greatest common divisor of `a` and `b`, always non-negative. `gcd(0, 0)` is
`0`.

### `lcm(a: int, b: int) -> int`

Least common multiple of `a` and `b`. Zero when either argument is zero.

```rust
from std.math import gcd, lcm

fun main() {
    print(gcd(12, 18)) // 6
    print(lcm(4, 6)) // 12
}
```

### `sign_int(n: int) -> int`

The sign of an integer: `1` when positive, `-1` when negative, `0` for zero.

```rust
from std.math import sign_int

fun main() {
    print(sign_int(5)) // 1
    print(sign_int(0 - 5)) // -1
    print(sign_int(0)) // 0
}
```

## Float functions

All take and return `float`.

### `abs(x: float) -> float`

Absolute value (the C `fabs`).

### `min(a: float, b: float) -> float` and `max(a: float, b: float) -> float`

The smaller and the larger of two floats.

### `clamp(x: float, lo: float, hi: float) -> float`

Clamp `x` into the closed range `[lo, hi]`.

### `sqrt(x: float) -> float`

Square root.

### `pow(base: float, exp: float) -> float`

`base` raised to `exp`.

### `fmod(a: float, b: float) -> float`

Floating-point remainder of `a / b`, with the sign of `a`. The `%` operator
does not work on `float`, so this is the way to take a float remainder.

```rust
from std.math import fmod

fun main() {
    print(fmod(7.5, 2.0)) // 1.5
}
```

### `cbrt(x: float) -> float`

Cube root.

### `hypot(a: float, b: float) -> float`

`sqrt(a*a + b*b)`, computed without intermediate overflow.

```rust
from std.math import cbrt, hypot

fun main() {
    print(cbrt(27.0)) // 3.0
    print(hypot(3.0, 4.0)) // 5.0
}
```

### `sign(x: float) -> float`

The sign of a float: `1.0` when positive, `-1.0` when negative, `0.0` for
zero.

```rust
from std.math import sign

fun main() {
    print(sign(0.0 - 3.0)) // -1.0
    print(sign(2.5)) // 1.0
}
```

### `exp(x: float) -> float`

`e` raised to `x`.

### `ln(x: float) -> float`

Natural logarithm (the C `log`).

### `log2(x: float) -> float`

Base-2 logarithm.

### `log10(x: float) -> float`

Base-10 logarithm.

```rust
from std.math import log2

fun main() {
    print(log2(8.0)) // 3.0
}
```

### `floor(x: float) -> float`, `ceil(x: float) -> float`, `round(x: float) -> float`, `trunc(x: float) -> float`

Round toward minus infinity, toward plus infinity, to the nearest whole
value, and toward zero. Each returns a whole-valued `float`.

### `sin(x: float) -> float`, `cos(x: float) -> float`, and `tan(x: float) -> float`

Sine, cosine, and tangent, with the angle in radians.

### `asin(x: float) -> float`, `acos(x: float) -> float`, `atan(x: float) -> float`

Inverse sine, cosine, and tangent. Results are in radians.

### `atan2(y: float, x: float) -> float`

The angle in radians of the point `(x, y)` from the positive x-axis, using the
signs of both arguments to land in the correct quadrant.

```rust
from std.math import asin, acos, atan, atan2

fun main() {
    print(asin(1.0)) // 1.5707963267948966
    print(acos(1.0)) // 0
    print(atan(1.0)) // 0.7853981633974483
    print(atan2(1.0, 1.0)) // 0.7853981633974483
}
```

### `sinh(x: float) -> float`, `cosh(x: float) -> float`, `tanh(x: float) -> float`

Hyperbolic sine, cosine, and tangent.

```rust
from std.math import sinh, cosh, tanh

fun main() {
    print(sinh(0.0)) // 0
    print(cosh(0.0)) // 1
    print(tanh(0.0)) // 0
}
```

### `to_radians(deg: float) -> float` and `to_degrees(rad: float) -> float`

Convert between degrees and radians.

```rust
from std.math import to_radians, to_degrees

fun main() {
    print(to_radians(180.0)) // 3.141592653589793
    print(to_degrees(3.141592653589793)) // 180
}
```

```rust
from std.math import sqrt, pow, exp, ln, abs, min, max, clamp, floor, ceil, round, trunc, sin, cos

fun main() {
    print(sqrt(2.0)) // 1.4142135623730951
    print(pow(2.0, 10.0)) // 1024.0
    print(abs(0.0 - 3.5)) // 3.5
    print(min(2.5, 1.5)) // 1.5
    print(clamp(9.0, 0.0, 1.0)) // 1.0
    print(floor(2.7)) // 2.0
    print(ceil(2.1)) // 3.0
    print(round(2.5)) // 3.0
    print(trunc(0.0 - 2.7)) // -2.0
}
```

## Special values

### `infinity() -> float` and `nan() -> float`

Positive infinity and a NaN (not-a-number) value.

### `is_nan(x: float) -> bool`

True when `x` is NaN. A NaN is never equal to itself, so a plain `==` test does
not detect it; use this.

### `is_inf(x: float) -> bool`

True when `x` is positive or negative infinity.

```rust
from std.math import infinity, nan, is_nan, is_inf

fun main() {
    print(is_nan(nan())) // true
    print(is_inf(infinity())) // true
    print(is_nan(1.0)) // false
}
```

## Constants

The language has no `float` const items yet, so the constants are
zero-argument functions returning `float`. Call them with `()`.

### `pi() -> float`

3.141592653589793

### `e() -> float`

2.718281828459045

### `tau() -> float`

6.283185307179586 (a full turn, `2 * pi`).

```rust
from std.math import pi, e, tau, sin

fun main() {
    print(pi()) // 3.141592653589793
    print(tau()) // 6.283185307179586
    print(sin(pi())) // ~0 (libm rounding)
}
```

## Worked example: distance between two points

`sqrt` plus `pow` gives the Euclidean distance between two points.

```rust
from std.math import sqrt, pow

fun distance(x1: float, y1: float, x2: float, y2: float) -> float {
    let dx = x2 - x1
    let dy = y2 - y1
    return sqrt(pow(dx, 2.0) + pow(dy, 2.0))
}

fun main() {
    print(distance(0.0, 0.0, 3.0, 4.0)) // 5.0
}
```

## See also

- [std/cmp](cmp.md) for `min`/`max`/`clamp` over the generic `Ord` trait
  rather than numbers directly.
- [std/random](random.md) for random number generation.
- The [language reference](../language-reference.md) for the `int` and
  `float` types and numeric literals.
