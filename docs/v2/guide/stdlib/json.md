# std/json

Parse and serialize JSON. `std/json` is a recursive descent parser
(`parse`) and a compact serializer (`stringify`) written in pure Raven, with
a `JsonValue` enum for the parsed value tree.

```rust
import std/json { parse, stringify }

fun main() {
    match parse("{\"name\": \"Ada\", \"id\": 7}") {
        Ok(v) -> print(stringify(v)),     // {"name":"Ada","id":7}
        Err(e) -> print("bad json"),
    }
}
```

JSON string literals embedded in Raven source need escaped quotes, so the
text `{"name": "Ada"}` is written `"{\"name\": \"Ada\"}"`.

## Importing

```rust
import std/json { parse, stringify }
```

Bring in just what you use. The accessor methods on `JsonValue` (`is_null`,
`as_bool`, `get`, ...) come along with the `JsonValue` type, so a selective
import of `parse` and `stringify` is enough for most code. Import the type
explicitly when you name it:

```rust
import std/json { parse, stringify, JsonValue }
```

## The value type

A parsed JSON document is a `JsonValue`, a tagged union over the six JSON
shapes:

```rust
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

`Object` holds a `std/collections` `Map<String, JsonValue>`. The serializer
emits members in the Map's key order (the hash-bucket layout), not insertion
order.

### Numbers are Float

Every JSON number, integer or not, parses to `Number(Float)`. Raven `Float`
is an IEEE 754 double, so integers beyond the 53-bit mantissa (roughly
9.0e15) lose precision. There is no separate integer JSON type, so a
whole-number value like `7` comes back as `Number(7.0)` and serializes again
as `7`.

## Parsing

### `parse(text: String) -> Result<JsonValue, Error>`

Parse `text` as a single JSON value. The parser handles nested objects and
arrays (including empty `{}` and `[]`), strings, numbers, `true`, `false`,
`null`, and inter-token whitespace (space, tab, newline, carriage return).
Any non-whitespace content after the top-level value is rejected.

`parse` returns a `Result`, so handle both arms with `match`:

```rust
import std/json { parse }

fun main() {
    match parse("[1, 2, 3]") {
        Ok(v) -> print("parsed ok"),
        Err(e) -> print(e.message),
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
scalar. The container steps return `Option<JsonValue>`, and the extractors
return an `Option` of the underlying type, so a wrong-kind or missing lookup
is a normal `None` rather than an error.

### `get(self, key: String) -> Option<JsonValue>`

The member of an object by key, or `None` when the value is not an object or
the key is absent.

### `at(self, i: Int) -> Option<JsonValue>`

The element of an array by index, or `None` when out of range or not an
array.

### `is_null(self) -> Bool`

True only for the `Null` variant.

### `as_bool(self) -> Option<Bool>`

`Some(b)` for a `Bool`, else `None`.

### `as_number(self) -> Option<Float>`

`Some(n)` for a `Number`, else `None`. Remember every JSON number is a
`Float`.

### `as_string(self) -> Option<String>`

`Some(s)` for a `Str`, else `None`.

### `as_int(self) -> Option<Int>`

`Some(i)` for a `Number`, truncated toward zero, else `None`. Use this when
you want a whole-number field as an `Int` rather than the raw `Float`.

```rust
import std/json { parse }

fun main() {
    match parse("{\"port\": 8080}") {
        Ok(v) -> {
            match v.get("port") {
                Some(field) -> {
                    match field.as_int() {
                        Some(i) -> print(i),     // 8080
                        None -> print("not a number"),
                    }
                }
                None -> print("no port"),
            }
        }
        Err(e) -> print("bad json"),
    }
}
```

### `keys(self) -> List<String>`

The object's member keys as a `List<String>`, in the Map's key order. An
empty list for any non-object value.

### `length(self) -> Int`

The element count: the number of array elements or object members. `0` for
any scalar value.

```rust
import std/json { parse }

fun main() {
    match parse("[10, 20, 30]") {
        Ok(v) -> print(v.length()),      // 3
        Err(e) -> print("bad json"),
    }
}
```

A typical read chains a container step and then an extractor, handling each
`Option` with `match`:

```rust
import std/json { parse }

fun main() {
    match parse("{\"port\": 8080}") {
        Ok(v) -> {
            match v.get("port") {
                Some(field) -> {
                    match field.as_number() {
                        Some(n) -> print(n),     // 8080
                        None -> print("port is not a number"),
                    }
                }
                None -> print("no port field"),
            }
        }
        Err(e) -> print("bad json"),
    }
}
```

Reaching into nested data composes the same way: `v.get("user")` returns an
`Option<JsonValue>` you match on, then call `.get("name")` or `.at(0)` on the
inner value.

## Serializing

### `stringify(value: JsonValue) -> String`

Compact serialization with no spaces between tokens. Object members are
emitted in the Map's key order. String escaping is the reverse of parsing:
`"` and `\` are escaped, control bytes below `0x20` use the `\b \f \n \r \t`
shorthands where they exist and `\u00XX` otherwise, and every other byte
passes through unchanged.

A whole-number `Float` renders the way the runtime prints it, so `1.0`
serializes as `1` and `0.15` serializes as `0.15`.

```rust
import std/json { parse, stringify }

fun main() {
    match parse("{ \"a\" : 1 , \"b\" : [ true , null ] }") {
        Ok(v) -> print(stringify(v)),     // {"a":1,"b":[true,null]}
        Err(e) -> print("bad json"),
    }
}
```

### `stringify_pretty(value: JsonValue, indent: Int) -> String`

Multi-line serialization with `indent` spaces of padding per nesting level.
Object members and array elements each go on their own line; empty arrays and
objects stay on one line as `[]` and `{}`. Members are emitted in the Map's
key order, as with `stringify`.

```rust
import std/json { parse, stringify_pretty }

fun main() {
    match parse("{\"a\":1,\"b\":[true,null]}") {
        Ok(v) -> print(stringify_pretty(v, 2)),
        Err(e) -> print("bad json"),
    }
}
```

## Building a value

Build a `JsonValue` tree directly with these free constructors instead of
parsing text. Select them from the module, and import `std/collections` when
you need a `Map` for `json_object`.

```rust
import std/json { json_null, json_bool, json_number, json_int, json_string, json_array, json_object }
```

### `json_null() -> JsonValue`

The `Null` value.

### `json_bool(b: Bool) -> JsonValue`

A `Bool` value.

### `json_number(n: Float) -> JsonValue`

A `Number` from a `Float`.

### `json_int(n: Int) -> JsonValue`

A `Number` from an `Int`, widened to `Float` (JSON has one number type).

### `json_string(s: String) -> JsonValue`

A `Str` value.

### `json_array(items: List<JsonValue>) -> JsonValue`

An `Array` wrapping a list of values.

### `json_object(map: Map<String, JsonValue>) -> JsonValue`

An `Object` wrapping a `std/collections` `Map`. A map literal `["k": v]`
serves as the argument.

```rust
import std/collections
import std/json { json_object, json_int, json_string, stringify_pretty }

fun main() {
    let doc = json_object(["n": json_int(7), "name": json_string("ok")])
    print(stringify_pretty(doc, 2))
    print(doc.length())     // 2
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
    fun from_json(j: JsonValue) -> Result<Self, Error>
}
```

`to_json` is an ordinary `self` method; `from_json` is an associated
function called as `Point.from_json(j)`. Built-in impls cover `Int`, `Float`,
`Bool`, `String`, `List<T>`, and `Option<T>` so field recursion bottoms out.
An `Int` round-trips through `Float` (JSON has one number type) and loses
precision beyond 2^53; a number decodes back to `Int` by truncation toward
zero.

Annotate a user struct or enum with `@derive(ToJson, FromJson)` to get these
traits automatically: a struct serializes to an object keyed by field name,
an enum to a tagged object. See the [derive spec](../../specs/derive.md) for
the full encoding and the helper functions the derive emits.

## Worked example: read a config field

```rust
import std/json { parse }

// Pull the "name" string out of a JSON object, with a fallback for any
// shape that does not match.
fun config_name(text: String) -> String {
    return match parse(text) {
        Ok(v) -> {
            match v.get("name") {
                Some(field) -> {
                    match field.as_string() {
                        Some(name) -> name,
                        None -> "unnamed",
                    }
                }
                None -> "unnamed",
            }
        }
        Err(e) -> "invalid",
    }
}

fun main() {
    print(config_name("{\"name\": \"raven\", \"version\": 2}"))   // raven
    print(config_name("{\"version\": 2}"))                        // unnamed
    print(config_name("not json"))                                // invalid
}
```

## See also

- [std/io](io.md) for reading the text you hand to `parse`.
- The [derive spec](../../specs/derive.md) for `@derive(ToJson, FromJson)`
  on your own types.
- The [language reference](../language-reference.md) for `match`, `Result`,
  and `Option`.
