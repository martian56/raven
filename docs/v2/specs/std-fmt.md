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

```raven
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

## Deferred

`format_float(x, places)` (fixed decimal places) is deferred: it needs a
Float-to-Int truncation runtime hook the surface language does not expose, and
is not required for the rest of the module. It can land later as a small
`extern "C"` addition (a `raven_float_trunc` symbol) without changing the
existing surface.
