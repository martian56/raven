# std.url

Parse and build URLs (RFC 3986), percent-encode their parts, and read and
write query strings.

```rust
import std.url

fun main() {
    let u = url.parse("https://api.example.com:8443/v1/items?page=2&tag=a&tag=b").unwrap()
    print(u.host, u.effective_port(), u.path) // api.example.com 8443 /v1/items
    let q = u.query_params()
    print(q.get("page").unwrap(), q.get_all("tag")) // 2 ["a", "b"]
}
```

## Parsing

`parse(text) -> Result[Url, Error]` reads an absolute URL. A `Url` has
`scheme` (lowercased), `username` and `password` (decoded), `host`
(lowercased, IPv6 without brackets), `port` (0 when absent), `path`, `query`,
and `fragment` (the last three kept encoded, without `?` and `#`). Input
without a scheme, an unclosed IPv6 bracket, or a port outside 1 to 65535 is
an error of kind `"url"`.

| Method | Returns |
|--------|---------|
| `effective_port()` | the port, or the scheme's default (`default_port(scheme)`: 80, 443, 5432, 3306, ...) |
| `host_port()` | `host:port` for connecting, IPv6 in brackets |
| `request_target()` | path and query, as an HTTP request line wants it |
| `query_params()` | the decoded query |
| `with_path(p)`, `with_query(params)` | a changed copy |
| `redacted()` | the URL text with the password replaced, safe to log |
| `to_string()` | the URL text |

```rust
import std.url

fun main() {
    let db = url.parse("postgres://app:secret@[::1]/shop").unwrap()
    print(db.host_port(), db.path) // [::1]:5432 /shop
    print(db.redacted()) // postgres://app:xxxxx@[::1]/shop
}
```

## Query strings

`parse_query(text)` decodes `a=1&b=two%20words` (with or without a leading
`?`; `+` decodes to a space). `query_params()` starts an empty one.
`QueryParams` keeps parameters in order and allows repeated keys:
`get(key) -> Option[str]`, `get_or(key, default)`, `get_all(key)`,
`has(key)`, `keys()`, `len()`, `add(key, value)`, `set(key, value)`,
`remove(key)`, and `encode()`.

```rust
import std.url

fun main() {
    let q = url.query_params().add("q", "a&b c").add("page", "2")
    print(q.encode()) // q=a%26b%20c&page=2
}
```

## Percent encoding

`encode_component(s)` escapes everything except letters, digits, and `-._~`,
so the result is safe in any part of a URL. `decode_component(s)` reverses it
and keeps a malformed escape as written.
