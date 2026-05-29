# std/json Spec

A JSON parser and serializer written in pure Raven over the `__str_*` byte
intrinsics. `parse` is a recursive descent parser; `stringify` is a compact
serializer. The value tree is the `JsonValue` enum.

## Value type

```raven
enum JsonValue {
    Null,
    Bool(Bool),
    Number(Float),
    Str(String),
    Array(List<JsonValue>),
    Object(Map<String, JsonValue>),
}
```

Construct variants with the qualified form (`JsonValue.Null`,
`JsonValue.Bool(true)`, `JsonValue.Number(1.0)`, `JsonValue.Str(s)`,
`JsonValue.Array(list)`, `JsonValue.Object(map)`). Match with the bare
variant names (`Null`, `Bool(b)`, `Number(n)`, ...).

`Object` uses `std/collections` `Map<String, JsonValue>`. The Map stores
parallel `keys` and `values` lists, so members keep insertion order. The
serializer reads `map.keys` and `map.values` directly to emit members in
that order.

## Numbers are Float

Every JSON number, integer or not, parses to `Float`. Raven `Float` is an
IEEE 754 double, so integers beyond the 53-bit mantissa (roughly
9.0e15) lose precision. There is no separate integer JSON type.

## Import

```raven
import std/json { parse, stringify }

fun main() {
    match parse("{\"a\": 1, \"b\": [true, null]}") {
        Ok(v) -> print(stringify(v)),
        Err(e) -> print("bad json"),
    }
}
```

## Surface

| Item | Result | Notes |
|---|---|---|
| `parse(text: String)` | `Result<JsonValue, Error>` | full recursive descent parse |
| `stringify(value: JsonValue)` | `String` | compact (non-pretty) serialization |
| `JsonValue.is_null(self)` | `Bool` | true only for `Null` |
| `JsonValue.as_bool(self)` | `Option<Bool>` | `Some` only for `Bool` |
| `JsonValue.as_number(self)` | `Option<Float>` | `Some` only for `Number` |
| `JsonValue.as_string(self)` | `Option<String>` | `Some` only for `Str` |
| `JsonValue.get(self, key: String)` | `Option<JsonValue>` | object member, else `None` |
| `JsonValue.at(self, i: Int)` | `Option<JsonValue>` | array element, else `None` |

The accessors return `Option`, so a wrong-kind or missing lookup is a normal
`None` rather than an error.

## Parsing

`parse` handles nested objects and arrays (including empty `{}` and `[]`),
strings, numbers, `true`, `false`, `null`, and inter-token whitespace
(space, tab, newline, carriage return). It rejects any non-whitespace
content after the top-level value.

### String escapes

The two-character escapes `\" \\ \/ \b \f \n \r \t` decode to their usual
bytes. A `\uXXXX` escape decodes a BMP code point and is encoded to UTF-8
bytes in the result.

### Surrogate pairs

A high surrogate (`\uD800` to `\uDBFF`) immediately followed by a low
surrogate (`\uDC00` to `\uDFFF`) decodes to the astral code point and is
UTF-8 encoded. A high surrogate not followed by a low surrogate, or a lone
low surrogate, decodes to U+FFFD (the replacement character). Note that the
Raven lexer rejects surrogate `\u` escapes inside a source string literal,
so a surrogate input reaches `parse` only from data read at runtime.

### Numbers

A number is an optional minus, integer digits, an optional `.` fraction,
and an optional `e`/`E` exponent with an optional sign. Digits accumulate
into a `Float` (integer part built through the runtime's
`raven_int_to_float`, fraction divided by a power of ten, exponent applied
as a power-of-ten factor).

## Errors

A parse failure is an `std/error` `Error` with `kind` `"json"`. Raven has
no type alias and a bundled module cannot call another bundled module's
free functions, so the value is built directly as the `Error` struct
literal (the same workaround `std/fs` and others use). The message names
roughly what failed and, for unexpected or trailing bytes, the byte offset.

## Serialization

`stringify` produces compact output with no spaces between tokens. Object
members are emitted in the Map's key order. String escaping is the reverse
of parsing: `"` and `\` are escaped, control bytes below `0x20` use the
`\b \f \n \r \t` shorthands where they exist and `\u00XX` otherwise, and
every other byte (printable ASCII or a UTF-8 continuation byte) passes
through unchanged.

A whole-number `Float` renders the way the runtime's Float-to-string
produces it: `1.0` serializes as `1`, and `0.15` serializes as `0.15`.

## Accessor implementation note

The payload-reading accessor methods (`as_bool`, `as_number`, `as_string`,
`is_null`, `get`, `at`) forward to free functions that take the value as a
plain parameter. Matching a `self` receiver and then reading the bound
payload currently corrupts the extracted value in the back end, so every
accessor that inspects a payload goes through a free helper. The serializer
is already a free function for the same reason.

## Out of scope

Pretty printing (indentation), duplicate-key policy beyond last-wins (the
Map overwrites on a repeated key), and streaming or incremental parsing.
These can be added later without changing the existing surface.
