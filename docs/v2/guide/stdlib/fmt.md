# std/fmt

String formatting helpers and a `Debug` trait. These are the building blocks
that compose with string interpolation: pad and align fields, repeat and join
strings, render integers in a base, and produce quoted debug output.

```rust
import std/fmt { pad_left, to_hex }
import std/io { println }

fun main() {
    println(pad_left("7", 3, "0"))      // 007
    println(to_hex(255))                // ff
}
```

## No printf

There is no `format("{}", a, b)` function. The v2 surface language has no
varargs and no format-placeholder runtime, so string interpolation `"${expr}"`
is the placeholder mechanism. std/fmt provides the helpers that compose with
it, not a printf replacement.

## Importing

The formatting helpers are free functions, bound with a selective import:

```rust
import std/fmt { repeat, pad_left, pad_right, center, join, to_radix, to_binary, to_octal, to_hex, pad_int }
```

The `Debug` trait is brought in by the same `import std/fmt` (its impls merge
into scope like `std/string`'s methods). Once the module is imported, the
`debug` method dispatches on the receiver's type and needs no explicit
selector:

```rust
import std/fmt

fun main() {
    print("hi".debug())     // "hi"
    print(true.debug())     // true
}
```

## A note on bytes

A Raven `String` is a byte buffer. Padding widths and `to_radix` digit counts
are measured in **UTF-8 bytes**, not code points. For ASCII text a byte equals
a displayed character; for multi-byte text the width is the encoded byte
length. The `fill` argument to the padding functions is assumed to be a
single-byte string; a multi-byte `fill` can overshoot the target width by up
to its length minus one. See [std/string](string.md) for the same byte model.

## String helpers

### `repeat(s: String, n: Int) -> String`

`s` concatenated `n` times. A non-positive `n` yields the empty string.

```rust
import std/fmt { repeat }

fun main() {
    print(repeat("ab", 3))      // ababab
    print(repeat("x", 0))       // (empty)
}
```

### `pad_left(s: String, width: Int, fill: String) -> String`

Left-pad `s` with `fill` until its byte length is at least `width`. A `width`
that does not exceed the current length returns `s` unchanged.

### `pad_right(s: String, width: Int, fill: String) -> String`

Right-pad `s` with `fill` until its byte length is at least `width`.

```rust
import std/fmt { pad_left, pad_right }

fun main() {
    print(pad_left("42", 5, "0"))       // 00042
    print(pad_right("id", 5, "."))      // id...
}
```

### `center(s: String, width: Int, fill: String) -> String`

Center `s` in a field of byte width `width`, padding with `fill`. An odd
amount of padding puts the extra byte on the right.

```rust
import std/fmt { center }

fun main() {
    print(center("hi", 6, "-"))     // --hi--
    print(center("hi", 7, "-"))     // --hi---
}
```

### `join(parts: List<String>, sep: String) -> String`

Join `parts` with `sep` between adjacent elements.

```rust
import std/fmt { join }

fun main() {
    print(join(["a", "b", "c"], ", "))      // a, b, c
}
```

## Number-base formatting

### `to_radix(n: Int, base: Int) -> String`

`n` written in `base`, with a single leading `-` for negatives. `base` must be
in `2..=16`; a `base` outside that range returns the empty string. Zero
renders as `"0"`. Digits are `0-9` then lowercase `a-f`. The conversion works
in the negative domain (accumulating the negative magnitude), so the most
negative i64 has no overflowing negation.

```rust
import std/fmt { to_radix }

fun main() {
    print(to_radix(255, 16))    // ff
    print(to_radix(-42, 16))    // -2a
    print(to_radix(5, 2))       // 101
}
```

### `to_binary(n: Int) -> String`, `to_octal(n: Int) -> String`, `to_hex(n: Int) -> String`

Thin wrappers over `to_radix` for base 2, 8, and 16.

```rust
import std/fmt { to_binary, to_octal, to_hex }

fun main() {
    print(to_binary(6))     // 110
    print(to_octal(64))     // 100
    print(to_hex(3735928559))   // deadbeef
}
```

### `from_radix(s: String, base: Int) -> Option<Int>`

Parse `s` as an integer in `base`, the inverse of `to_radix`. `base` must be in
`2..=16`. An optional leading `+` or `-` sign is accepted. Returns `None` on an
empty string, a digit out of range for the base, or a `base` outside `2..=16`.

```rust
import std/fmt { from_radix }

fun main() {
    print(match from_radix("101", 2) {
        Some(n) -> n,
        None -> -1,
    })      // 5
    print(match from_radix("ff", 16) {
        Some(n) -> n,
        None -> -1,
    })      // 255
    print(match from_radix("zz", 16) {
        Some(n) -> n,
        None -> -1,
    })      // -1 (None: bad digit)
}
```

### `from_hex(s: String) -> Option<Int>`

Parse a hexadecimal string, the inverse of `to_hex`. A thin wrapper over
`from_radix(s, 16)`.

```rust
import std/fmt { from_hex }

fun main() {
    print(match from_hex("deadbeef") {
        Some(n) -> n,
        None -> -1,
    })      // 3735928559
}
```

### `pad_int(n: Int, width: Int) -> String`

Decimal `n` zero-padded to byte width `width`. For a negative `n` the `-`
stays leftmost and the zero fill goes between the sign and the digits, so
`pad_int(-7, 4)` is `"-007"`. A `width` that does not exceed the rendered
length returns the value unchanged.

```rust
import std/fmt { pad_int }

fun main() {
    print(pad_int(7, 4))        // 0007
    print(pad_int(-7, 4))       // -007
    print(pad_int(12345, 3))    // 12345 (already wider than width)
}
```

### `format_float(x: Float, decimals: Int) -> String`

`x` rendered with exactly `decimals` digits after the decimal point, rounded
half up. A `decimals` of `0` or less produces no fractional part and no point.

```rust
import std/fmt { format_float }

fun main() {
    print(format_float(3.14159, 2))     // 3.14
    print(format_float(1.0, 3))         // 1.000
    print(format_float(2.5, 0))         // 3
}
```

## The `Debug` trait

```rust
trait Debug {
    fun debug(self) -> String
}
```

`Debug` is a separate trait from `ToString`. The module ships impls for the
built-in scalar types:

| Receiver | `debug(self)` result |
|----------|----------------------|
| `Int` | delegates to `to_string` |
| `Float` | delegates to `to_string` |
| `Bool` | delegates to `to_string` |
| `Char` | the value wrapped in single quotes |
| `String` | the value wrapped in double quotes |

Char and String quoting does not escape inner quotes: a String containing a
`"` reproduces it literally inside the surrounding quotes. So `"hi".debug()` is
the 4-character string `"hi"`.

```rust
import std/fmt

fun main() {
    print(42.debug())       // 42
    print(true.debug())     // true
    print('x'.debug())      // 'x'
    print("hi".debug())     // "hi"
}
```

## Worked example: a fixed-width table row

```rust
import std/fmt { pad_right, pad_int, join, to_hex }
import std/io { println }

fun row(name: String, count: Int, color: Int) -> String {
    let cells = [
        pad_right(name, 8, " "),
        pad_int(count, 4),
        to_hex(color),
    ]
    return join(cells, " | ")
}

fun main() {
    println(row("red", 7, 16711680))        // red      | 0007 | ff0000
    println(row("green", 128, 65280))       // green    | 0128 | ff00
}
```

## See also

- [std/string](string.md) for the methods on `String` values and the same
  byte model.
- [std/io](io.md) for `print` and `println`.
- The [language reference](../language-reference.md) for string literals and
  interpolation.
