# std/http

A small HTTP/1.1 client built on [std/net](net.md). It sends `GET`, `POST`,
`PUT`, and `DELETE` requests and hands back a captured response. Every call
returns `Result<HttpResponse, Error>`, so transport failures (DNS, connect,
timeout) come back as an [std/error](error.md) `Error` you handle with
`match`.

```rust
import std/http { get }

fun main() {
    match get("https://example.com") {
        Ok(resp) -> print(resp.status_code),    // 200
        Err(e) -> print("request failed"),
    }
}
```

## Importing

```rust
import std/http { get, post, put, delete, request }
```

Select the functions you use. The `HttpResponse` type comes in with the
module and needs no separate selector.

## The response type

### `struct HttpResponse`

```rust
struct HttpResponse {
    status_code: Int,
    status_text: String,
    headers: String,
    body: String,
}
```

| Field | Meaning |
|-------|---------|
| `status_code` | The HTTP status as an `Int`, for example `200` or `404`. |
| `status_text` | The reason phrase, for example `"OK"` or `"Not Found"`. |
| `headers` | Response headers as `Key: Value` lines joined by `\n`. |
| `body` | The full response body as a `String`. |

Read the fields directly off the struct once you have unwrapped the `Ok`
case.

## A 404 is not an error

A non-2xx HTTP status (404, 500, and the like) is a successful request that
returned a response, so it takes the `Ok` path with `status_code` set to the
server's value. Only transport failures (DNS, connect, timeout, malformed
response) take the `Err` path. Inspect `status_code` yourself to tell a 200
from a 404:

```rust
import std/http { get }

fun main() {
    match get("https://example.com/missing") {
        Ok(resp) -> {
            if resp.status_code == 200 {
                print(resp.body)
            } else {
                print("server returned ${resp.status_code}")
            }
        },
        Err(e) -> print("could not reach server"),
    }
}
```

## Requests

### `get(url: String) -> Result<HttpResponse, Error>`

Send a `GET` to `url`. No request body.

```rust
import std/http { get }

fun main() {
    match get("https://example.com") {
        Ok(resp) -> {
            print(resp.status_code)     // 200
            print(resp.body)
        },
        Err(e) -> print("request failed"),
    }
}
```

### `post(url: String, body: String) -> Result<HttpResponse, Error>`

Send `body` to `url` with a `POST`.

```rust
import std/http { post }

fun main() {
    match post("https://example.com/items", "{\"name\":\"raven\"}") {
        Ok(resp) -> print(resp.status_code),
        Err(e) -> print("request failed"),
    }
}
```

### `put(url: String, body: String) -> Result<HttpResponse, Error>`

Send `body` to `url` with a `PUT`.

### `delete(url: String) -> Result<HttpResponse, Error>`

Send a `DELETE` to `url`. No request body.

### `request(method: String, url: String, body: String, headers: String) -> Result<HttpResponse, Error>`

The general form the others delegate to. `method` is the HTTP verb
(`"GET"`, `"POST"`, and so on), `body` is the request body (pass `""` for
none), and `headers` is a String of `Key: Value` lines separated by `\n`
(pass `""` for none). Use `request` when you need a custom method or
request headers.

```rust
import std/http { request }

fun main() {
    let headers = "Accept: application/json\nX-Token: secret"
    match request("GET", "https://example.com/api", "", headers) {
        Ok(resp) -> print(resp.body),
        Err(e) -> print("request failed"),
    }
}
```

`get`, `post`, `put`, and `delete` are thin wrappers: `get` and `delete`
pass `""` for both body and headers, while `post` and `put` pass your body
and `""` for headers.

## Worked example: fetch and report

This GETs a URL, reports the status, and prints the body only on a 200.

```rust
import std/http { get }

fun fetch(url: String) {
    match get(url) {
        Ok(resp) -> {
            print("status: ${resp.status_code} ${resp.status_text}")
            if resp.status_code == 200 {
                print(resp.body)
            }
        },
        Err(e) -> print("error reaching ${url}"),
    }
}

fun main() {
    fetch("https://example.com")
}
```

## See also

- [std/net](net.md) for the TCP sockets this client is built on.
- [std/json](json.md) for parsing and building request and response bodies.
- [std/error](error.md) for the `Error` type returned on transport failure
  (the errors here are tagged with kind `"http"`).
