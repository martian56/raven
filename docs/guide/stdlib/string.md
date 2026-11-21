# std.string

Methods for working with text. `std.string` adds an `impl str` block, so
a bare `import std.string` brings every method below into scope as a method
on any `str` value.

```rust
import std.string

fun main() {
    let name = "  Ada Lovelace  "
    print(name.trim().to_upper()) // ADA LOVELACE
}
```

## Importing

```rust
import std.string
```

Import the whole module (not a selective `{ ... }` list): the methods are
merged from an `impl str` block, and the bare import is what merges it.

Two methods are built into the compiler and need **no** import:

| Built in | Meaning |
|----------|---------|
| `str.len()` | byte length (alias of `length`) |
| `str.is_empty()` | `len() == 0` |

Everything else (`concat`, `to_upper`, `substring`, `replace`, ...) comes
from `import std.string`.

## A note on bytes

Indices, lengths, and slices count **UTF-8 bytes**, not Unicode code points.
For plain ASCII text a byte is a character, so this rarely matters; for text
with multi-byte characters, an index addresses one byte of the encoding.
Case mapping (`to_upper` / `to_lower`) is ASCII only and leaves other bytes
unchanged.

## Inspecting

### `length(self) -> int`

The number of UTF-8 bytes in the string. `len` is a built-in alias.

```rust
import std.string

fun main() {
    print("raven".length()) // 5
    print("raven".len()) // 5 (built in, no import needed)
}
```

### `is_empty(self) -> bool`

True when the string has no bytes. Also available built in.

### `is_blank(self) -> bool`

True when the string is empty or contains only ASCII whitespace (space, tab,
newline, carriage return, vertical tab, form feed).

```rust
import std.string

fun main() {
    print("   \t\n".is_blank()) // true
    print(" x ".is_blank()) // false
}
```

## Slicing

### `char_at(self, i: int) -> str`

The single byte at index `i`, returned as a one-byte string. An out-of-range
index yields the empty string. (For multi-byte characters this returns one
byte of the encoding, not the whole character.)

### `byte_at(self, i: int) -> int`

The raw byte value (0..255) at index `i`, or `-1` when `i` is out of range.
Where `char_at` gives a one-byte string, `byte_at` gives the numeric byte.

```rust
import std.string

fun main() {
    print("raven".byte_at(0)) // 114
    print("raven".byte_at(9)) // -1
}
```

### `substring(self, start: int, end: int) -> str`

The half-open byte range `[start, end)`, clamped to `0..length`. A `start`
at or past `end` yields the empty string.

```rust
import std.string

fun main() {
    print("raven".substring(1, 4)) // ave
    print("raven".char_at(0)) // r
}
```

## Case and whitespace

### `to_upper(self) -> str` and `to_lower(self) -> str`

ASCII case mapping. Bytes `a`-`z` and `A`-`Z` are mapped; every other byte
is left as is.

### `trim(self) -> str`

Remove leading and trailing ASCII whitespace. Interior whitespace is kept.

```rust
import std.string

fun main() {
    print("Hello".to_upper()) // HELLO
    print("  hi  ".trim()) // hi
}
```

### `repeat(self, n: int) -> str`

The string concatenated `n` times. A non-positive `n` yields the empty
string.

```rust
import std.string

fun main() {
    print("ab".repeat(3)) // ababab
}
```

### `trim_start(self) -> str` and `trim_end(self) -> str`

Remove leading (`trim_start`) or trailing (`trim_end`) ASCII whitespace only,
leaving the other end untouched.

```rust
import std.string

fun main() {
    print("  hi  ".trim_start()) // "hi  "
    print("  hi  ".trim_end()) // "  hi"
}
```

### `reverse(self) -> str`

The string with its bytes reversed. For ASCII this reverses characters; a
multi-byte character's encoding is reversed byte-wise.

```rust
import std.string

fun main() {
    print("raven".reverse()) // nevar
}
```

## Searching

### `index_of(self, needle: str) -> int`

The byte index of the first occurrence of `needle`, or `-1` when it is
absent. An empty needle matches at `0`.

### `last_index_of(self, needle: str) -> int`

The byte index of the last occurrence of `needle`, or `-1` when it is absent.
An empty needle matches at `length`.

### `count(self, needle: str) -> int`

The number of non-overlapping occurrences of `needle`, scanning left to right.
An empty needle counts as `0`.

```rust
import std.string

fun main() {
    print("a-b-a".last_index_of("a")) // 4
    print("a-b-a".count("a")) // 2
}
```

### `contains(self, needle: str) -> bool`

True when `needle` occurs anywhere in the string.

### `starts_with(self, prefix: str) -> bool` and `ends_with(self, suffix: str) -> bool`

Prefix and suffix tests, compared by bytes.

### `matches_at(self, needle: str, at: int) -> bool`

True when the bytes of `needle` occur starting at byte index `at`. The
building block the other search methods use; handy for hand-written
scanning.

```rust
import std.string

fun main() {
    let path = "report.csv"
    print(path.ends_with(".csv")) // true
    print(path.index_of(".")) // 6
    print(path.contains("port")) // true
}
```

## Transforming

### `concat(self, other: str) -> str`

Join two strings. String interpolation (`"${a}${b}"`) is usually clearer for
building up text, but `concat` is the explicit method form.

### `replace(self, from: str, to: str) -> str`

Replace every non-overlapping occurrence of `from` with `to`, scanning left
to right. An empty `from` returns the input unchanged.

```rust
import std.string

fun main() {
    print("a-b-c".replace("-", "+")) // a+b+c
    print("one".concat(" ").concat("two")) // one two
}
```

## Splitting

### `split(self, sep: str) -> list[str]`

Split on every non-overlapping occurrence of `sep`, left to right. Consecutive
separators produce empty pieces. An empty `sep` returns the whole string as a
single element.

```rust
import std.string

fun main() {
    for p in "a,b,c".split(",") {
        print(p) // a, then b, c
    }
}
```

### `split_whitespace(self) -> list[str]`

Split on runs of ASCII whitespace, discarding empty pieces. Leading and
trailing whitespace produce no empty elements.

```rust
import std.string

fun main() {
    for w in "  one   two  ".split_whitespace() {
        print(w) // one, then two
    }
}
```

### `lines(self) -> list[str]`

Split into lines on `\n`, stripping a trailing `\r` from each line so both
Unix and Windows endings work. A trailing newline does not produce a final
empty line.

```rust
import std.string

fun main() {
    for l in "x\r\ny".lines() {
        print(l) // x, then y
    }
}
```

## Parsing

### `parse_int(self) -> Option[int]`

Parse a base-10 integer with an optional leading `+` or `-`. The string is
trimmed first. Returns `None` when it is empty or holds any non-digit
character.

```rust
import std.string

fun main() {
    match "  -42 ".parse_int() {
        case Some(n) { print(n) } // -42
        case None { print("no int") }
    }
    match "12x".parse_int() {
        case Some(n) { print(n) }
        case None { print("no int") } // no int
    }
}
```

### `parse_float(self) -> Option[float]`

Parse a floating point number: an optional sign, an integer part, an optional
`.fraction`, and an optional `e`/`E` exponent. The string is trimmed first.
Returns `None` on any trailing or invalid character.

```rust
import std.string

fun main() {
    match "3.5e2".parse_float() {
        case Some(f) { print(f) } // 350
        case None { print("no float") }
    }
}
```

## Comparison and matching

`str` values compare lexicographically by bytes with `<`, `<=`, `>`, and
`>=`, and they support `==`. A string literal can also be a `match` pattern,
compared by content:

```rust
fun classify(word: str) -> int {
    return match word {
        case "yes" { 1 }
        case "no" { 0 }
        case _ { -1 }
    }
}
```

## Worked example: a tiny slug builder

```rust
import std.string

fun slug(title: str) -> str {
    return title.trim().to_lower().replace(" ", "-")
}

fun main() {
    print(slug("  Hello World  ")) // hello-world
}
```

## See also

- [std.fmt](fmt.md) for padding, joining, and number formatting.
- [std.regex](regex.md) for pattern matching beyond literal substrings.
- The [language reference](../language-reference.md) for string literals,
  interpolation, and block strings.
