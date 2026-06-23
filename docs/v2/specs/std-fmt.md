# std/fmt Spec

String formatting helpers and a `Debug` trait. Free functions that build
padded fields, repeat and join strings, and render integers in a base, plus
a `Debug` trait with impls for the built-in scalar types.

## No printf

The v2 surface language has no varargs, no derive macros, and no
format-placeholder runtime, so there is no `format("{}", a, b)` function.
String interpolation `"${expr}"` is the placeholder mechanism: any non-scalar
expression interpolated in a string routes through `ToString` (see
`docs/v2/specs/core-traits.md`). std/fmt provides the building blocks that
compose with interpolation, not a printf replacement.

## Byte model

A Raven `String` is a byte buffer. The padding widths and `to_radix` digit
counts are measured in UTF-8 bytes through `__str_len`, not code points. For
ASCII text a byte equals a displayed character; for multi-byte text the width
is the encoded byte length. The `fill` argument to the padding functions is
assumed to be a single-byte string; a multi-byte `fill` can overshoot the
target width by up to its length minus one.

## Import

All formatting helpers are free functions, bound with a selective import. The
`Debug` trait's `debug` method dispatches by the receiver's type and needs no
explicit selector.

```rust
import std/fmt { repeat, pad_left, pad_right, center, join, to_radix, to_binary, to_octal, to_hex, pad_int }
import std/io { println }

fun main() {
    println(pad_left("7", 3, "0"))   // 007
    println(to_hex(255))             // ff
    println(to_radix(-42, 16))       // -2a
    println("hi".debug())            // "hi"
}
```

## Surface

| Function | Result | Notes |
|---|---|---|
| `repeat(s, n)` | `String` | `s` concatenated `n` times; `n <= 0` yields `""`. |
| `pad_left(s, width, fill)` | `String` | left-pad to byte width with `fill`. |
| `pad_right(s, width, fill)` | `String` | right-pad to byte width with `fill`. |
| `center(s, width, fill)` | `String` | center in byte width; an odd pad puts the extra byte on the right. |
| `join(parts, sep)` | `String` | join a `List<String>` with `sep` between elements. |
| `to_radix(n, base)` | `String` | `n` in `base`, lowercase digits, leading `-` for negatives. |
| `to_binary(n)` / `to_octal(n)` / `to_hex(n)` | `String` | thin wrappers for base 2, 8, 16. |
| `from_radix(s, base)` | `Option<Int>` | parse `s` in `base` (2..=16) with an optional sign; `None` on an empty string, a bad digit, an out-of-range `base`, or a value past `i64`. |
| `from_hex(s)` | `Option<Int>` | `from_radix(s, 16)`. |
| `format_float(x, decimals)` | `String` | `x` with exactly `decimals` digits after the point, rounded half up. A non-finite `x` is `"NaN"`, `"inf"`, or `"-inf"`. |
| `pad_int(n, width)` | `String` | decimal zero-padded to byte width; sign kept leftmost. |

| Trait | Method | Notes |
|---|---|---|
| `Debug` | `debug(self) -> String` | impls for Int, Float, Bool, Char, String. |

## Radix

`base` must be in `2..=16`. A `base` outside that range returns the empty
string. Digits are `0-9` then lowercase `a-f`. Zero renders as `"0"`. A
negative `n` gets a single leading `-`. The conversion accumulates the
negative magnitude (working in the negative domain) so the most negative i64
has no overflowing negation.

## pad_int sign handling

`pad_int` zero-pads the decimal form to `width` bytes. For a negative `n` the
`-` stays leftmost and the zero fill goes between the sign and the digits, so
`pad_int(-7, 4)` is `"-007"`. A `width` that does not exceed the rendered
length returns the value unchanged.

## Debug

`Debug` is a separate trait from `ToString`. Int, Float, and Bool delegate to
their `to_string`. Char is wrapped in single quotes and String in double
quotes (so `"hi".debug()` is the 4-character string `"hi"`). Inner quotes are
not escaped: a String containing a `"` reproduces it literally inside the
surrounding quotes.

## format_float

`format_float(x, decimals)` renders `x` with exactly `decimals` digits after
the decimal point, rounding half up: `format_float(3.14159, 2)` is `"3.14"`
and `format_float(-2.5, 0)` is `"-3"`. A non-finite input has no decimal form,
so NaN renders as `"NaN"` and an infinity as `"inf"` or `"-inf"`. The rounding
is done in pure Raven over the float digits, with no Float-to-Int runtime hook.

## Radix parsing

`from_radix(s, base)` is the inverse of `to_radix`: it parses `s` in `base`
(2..=16) with an optional leading sign and returns `Option<Int>`, yielding
`None` on an empty string, a non-digit, an out-of-range `base`, or a value
outside `i64`. It accumulates toward the sign's end of the range so the most
negative i64 parses. `from_hex(s)` is `from_radix(s, 16)`.
