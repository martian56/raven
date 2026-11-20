# std/json

Parse and serialize JSON. `std/json` is a recursive descent parser
(`parse`) and a compact serializer (`stringify`) written in pure Raven, with
a `JsonValue` enum for the parsed value tree.

```rust
from std.json import parse, stringify

fun main() {
    match parse("{\"name\": \"Ada\", \"id\": 7}") {
        case Ok(v) { print(stringify(v)) } // {"name":"Ada","id":7}
        case Err(e) { print("bad json") }
    }
}
```

JSON string literals embedded in Raven source need escaped quotes, so the
text `{"name": "Ada"}` is written `"{\"name\": \"Ada\"}"`.

## Importing

```rust
from std.json import parse, stringify
```

Bring in just what you use. The accessor methods on `JsonValue` (`is_null`,
`as_bool`, `get`, ...) come along with the `JsonValue` type, so a selective
import of `parse` and `stringify` is enough for most code. Import the type
explicitly when you name it:

```rust
from std.json import parse, stringify, JsonValue
```

## The value type

A parsed JSON document is a `JsonValue`, a tagged union over the six JSON
shapes:

```rust
enum JsonValue {
    Null,
    Bool(bool),
    Number(float),
    Str(str),
    Array(list[JsonValue]),
    Object(dict[str, JsonValue]),
}
```

Construct variants with the qualified form (`JsonValue.Null`,
`JsonValue.Bool(true)`, `JsonValue.Number(1.0)`, `JsonValue.Str(s)`,
`JsonValue.Array(list)`, `JsonValue.Object(map)`). Match with the bare
variant names (`Null`, `Bool(b)`, `Number(n)`, ...).

`Object` holds a `std/collections` `dict[str, JsonValue]`. The serializer
emits members in insertion order.

### Numbers are Float

Every JSON number, integer or not, parses to `Number(float)`. Raven `float`
is an IEEE 754 double, so integers beyond the 53-bit mantissa (roughly
9.0e15) lose precision. There is no separate integer JSON type, so a
whole-number value like `7` comes back as `Number(7.0)` and serializes again
as `7`.

## Parsing

### `parse(text: str) -> Result[JsonValue, Error]`

Parse `text` as a single JSON value. The parser handles nested objects and
arrays (including empty `{}` and `[]`), strings, numbers, `true`, `false`,
`null`, and inter-token whitespace (space, tab, newline, carriage return).
Any non-whitespace content after the top-level value is rejected.

`parse` returns a `Result`, so handle both arms with `match`:

```rust
from std.json import parse

fun main() {
    match parse("[1, 2, 3]") {
        case Ok(v) { print("parsed ok") }
        case Err(e) { print(e.message) }
    }
}
```

A failure is an `std/error` `Error` tagged with `kind` `"json"`. The message
names roughly what failed and, for an unexpected or trailing byte, the byte
offset.

String escapes decode as in standard JSON: the two-character escapes
`\" \\ \/ \b \f \n \r \t`, and a `\uXXXX` escape for a code point (UTF-8
encoded into the result). A high surrogate followed by a low surrogate
decodes to the astral code point; an unpaired surrogate is a parse error.

## Navigating a parsed value

A `JsonValue` is a tree. Two methods step into containers and four extract a
scalar. The container steps return `Option[JsonValue]`, and the extractors
return an `Option` of the underlying type, so a wrong-kind or missing lookup
is a normal `None` rather than an error.

### `get(self, key: str) -> Option[JsonValue]`

The member of an object by key, or `None` when the value is not an object or
the key is absent.

### `at(self, i: int) -> Option[JsonValue]`

The element of an array by index, or `None` when out of range or not an
array.

### `is_null(self) -> bool`

True only for the `Null` variant.

### `as_bool(self) -> Option[bool]`

`Some(b)` for a `bool`, else `None`.

### `as_number(self) -> Option[float]`

`Some(n)` for a `Number`, else `None`. Remember every JSON number is a
`float`.

### `as_string(self) -> Option[str]`

`Some(s)` for a `Str`, else `None`.

### `as_int(self) -> Option[int]`

`Some(i)` for a `Number`, truncated toward zero, else `None`. Use this when
you want a whole-number field as an `int` rather than the raw `float`.

```rust
from std.json import parse

fun main() {
    match parse("{\"port\": 8080}") {
        case Ok(v) {
            match v.get("port") {
                case Some(field) {
                    match field.as_int() {
                        case Some(i) { print(i) } // 8080
                        case None { print("not a number") }
                    }
                }
                case None { print("no port") }
            }
        }
        case Err(e) { print("bad json") }
    }
}
```

### `keys(self) -> list[str]`

The object's member keys as a `list[str]`, in insertion order. An
empty list for any non-object value.

### `length(self) -> int`

The element count: the number of array elements or object members. `0` for
any scalar value.

```rust
from std.json import parse

fun main() {
    match parse("[10, 20, 30]") {
        case Ok(v) { print(v.length()) } // 3
        case Err(e) { print("bad json") }
    }
}
```

A typical read chains a container step and then an extractor, handling each
`Option` with `match`:

```rust
from std.json import parse

fun main() {
    match parse("{\"port\": 8080}") {
        case Ok(v) {
            match v.get("port") {
                case Some(field) {
                    match field.as_number() {
                        case Some(n) { print(n) } // 8080
                        case None { print("port is not a number") }
                    }
                }
                case None { print("no port field") }
            }
        }
        case Err(e) { print("bad json") }
    }
}
```

Reaching into nested data composes the same way: `v.get("user")` returns an
`Option[JsonValue]` you match on, then call `.get("name")` or `.at(0)` on the
inner value.

## Serializing

### `stringify(value: JsonValue) -> str`

Compact serialization with no spaces between tokens. Object members are
emitted in insertion order. String escaping is the reverse of parsing:
`"` and `\` are escaped, control bytes below `0x20` use the `\b \f \n \r \t`
shorthands where they exist and `\u00XX` otherwise, and every other byte
passes through unchanged.

A whole-number `float` renders the way the runtime prints it, so `1.0`
serializes as `1` and `0.15` serializes as `0.15`.

```rust
from std.json import parse, stringify

fun main() {
    match parse("{ \"a\" : 1 , \"b\" : [ true , null ] }") {
        case Ok(v) { print(stringify(v)) } // {"a":1,"b":[true,null]}
        case Err(e) { print("bad json") }
    }
}
```

### `stringify_pretty(value: JsonValue, indent: int) -> str`

Multi-line serialization with `indent` spaces of padding per nesting level.
Object members and array elements each go on their own line; empty arrays and
objects stay on one line as `[]` and `{}`. Members are emitted in the Map's
key order, as with `stringify`.

```rust
from std.json import parse, stringify_pretty

fun main() {
    match parse("{\"a\":1,\"b\":[true,null]}") {
        case Ok(v) { print(stringify_pretty(v, 2)) }
        case Err(e) { print("bad json") }
    }
}
```

## Building a value

Build a `JsonValue` tree directly with these free constructors instead of
parsing text. Select them from the module, and import `std/collections` when
you need a `Map` for `json_object`.

```rust
from std.json import json_null, json_bool, json_number, json_int, json_string, json_array, json_object
```

### `json_null() -> JsonValue`

The `Null` value.

### `json_bool(b: bool) -> JsonValue`

A `bool` value.

### `json_number(n: float) -> JsonValue`

A `Number` from a `float`.

### `json_int(n: int) -> JsonValue`

A `Number` from an `int`, widened to `float` (JSON has one number type).

### `json_string(s: str) -> JsonValue`

A `Str` value.

### `json_array(items: list[JsonValue]) -> JsonValue`

An `Array` wrapping a list of values.

### `json_object(map: dict[str, JsonValue]) -> JsonValue`

An `Object` wrapping a `std/collections` `Map`. A map literal `["k": v]`
serves as the argument.

```rust
import std.collections
from std.json import json_object, json_int, json_string, stringify_pretty

fun main() {
    let doc = json_object({"n": json_int(7), "name": json_string("ok")})
    print(stringify_pretty(doc, 2))
    print(doc.length()) // 2
}
```

## Derived conversions

`std/json` also defines two traits for converting between a Raven value and
its JSON form:

```rust
trait ToJson {
    fun to_json(self) -> JsonValue
}

trait FromJson {
    fun from_json(j: JsonValue) -> Result[Self, Error]
}
```

`to_json` is an ordinary `self` method; `from_json` is an associated
function called as `Point.from_json(j)`. Built-in impls cover `int`, `float`,
`bool`, `str`, `list[T]`, and `Option[T]` so field recursion bottoms out.
An `int` round-trips through `float` (JSON has one number type) and loses
precision beyond 2^53; a number decodes back to `int` by truncation toward
zero.

Annotate a user struct or enum with `@derive(ToJson, FromJson)` to get these
traits automatically: a struct serializes to an object keyed by field name,
an enum to a tagged object. See the [derive spec](../../specs/derive.md) for
the full encoding and the helper functions the derive emits.

## Worked example: read a config field

```rust
from std.json import parse

// Pull the "name" string out of a JSON object, with a fallback for any
// shape that does not match.
fun config_name(text: str) -> str {
    return match parse(text) {
        case Ok(v) {
            match v.get("name") {
                case Some(field) {
                    match field.as_string() {
                        case Some(name) { name }
                        case None { "unnamed" }
                    }
                }
                case None { "unnamed" }
            }
        }
        case Err(e) { "invalid" }
    }
}

fun main() {
    print(config_name("{\"name\": \"raven\", \"version\": 2}")) // raven
    print(config_name("{\"version\": 2}")) // unnamed
    print(config_name("not json")) // invalid
}
```

## See also

- [std/io](io.md) for reading the text you hand to `parse`.
- The [derive spec](../../specs/derive.md) for `@derive(ToJson, FromJson)`
  on your own types.
- The [language reference](../language-reference.md) for `match`, `Result`,
  and `Option`.
