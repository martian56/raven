# std/http

A small HTTP/1.1 **client** and **server**, both built on [std/net](net.md).
The client sends `GET`, `POST`, `PUT`, `DELETE`, `PATCH`, and `HEAD` requests
and hands back a captured response. Every call returns
`Result<HttpResponse, Error>`, so transport failures (DNS, connect, timeout)
come back as an [std/error](error.md) `Error` you handle with `match`. The
server (`Server`, `Request`, `Response`, `Method`) lets you route requests to
handler functions; jump to [Server](#server) below.

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
import std/http { get, post, put, delete, patch, head, request }
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

### `patch(url: String, body: String) -> Result<HttpResponse, Error>`

Send `body` to `url` with a `PATCH`, for a partial update.

```rust
import std/http { patch }

fun main() {
    match patch("https://example.com/items/1", "{\"name\":\"raven\"}") {
        Ok(resp) -> print(resp.status_code),
        Err(e) -> print("request failed"),
    }
}
```

### `head(url: String) -> Result<HttpResponse, Error>`

Send a `HEAD` to `url`. The response carries the status and headers but no
body, so `resp.body` is empty. Use it to check status or read headers without
transferring the payload.

```rust
import std/http { head }

fun main() {
    match head("https://example.com") {
        Ok(resp) -> {
            print(resp.status_code)     // 200
            print(resp.headers)
        },
        Err(e) -> print("request failed"),
    }
}
```

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

## Server

The server side of `std/http` runs an HTTP/1.1 endpoint. You build a routing
table on a `Server`, then `listen`. A handler is a function from a `Request` to
a `Response`.

```rust
import std/http { Server, Request, Response }

fun hello(req: Request) -> Response {
    return Response.text("Hello, ${req.param("name")}!")
}

fun main() {
    let app = Server.new()
    app.get("/", fun(req: Request) -> Response = Response.html("<h1>Raven</h1>"))
    app.get("/hello/:name", hello)
    app.post("/echo", fun(req: Request) -> Response = Response.json_raw(req.body))
    app.listen("127.0.0.1:8080")
}
```

`listen` binds the address and blocks, accepting connections and serving each
in its own goroutine, so connections are handled concurrently.

### Access log

`with_access_log()` turns on a one-line-per-request log to stdout, returning the
server so it chains onto `Server.new()`. Each served request prints its method,
path, and status code:

```rust
fun main() {
    let app = Server.new().with_access_log()
    app.get("/", fun(req: Request) -> Response = Response.text("ok"))
    app.listen("127.0.0.1:8080")
}
```

```text
GET / 200
GET /missing 404
```

The log is written at the single point every request passes through, so it
covers every route without a line in each handler. It is off by default.

### Timeouts

A `Server` bounds how long one slow client can hold a connection, so an idle or
trickling client cannot pin a goroutine open indefinitely. Read and write
timeouts both default to **30 seconds** and are set in seconds:

```rust
let app = Server.new()
    .with_timeout(15)         // both read and write -> 15s
// or set them separately:
let app2 = Server.new()
    .with_read_timeout(10)    // waiting for the request
    .with_write_timeout(30)   // sending the response
```

`0` disables a timeout (the connection may then block forever on a slow
client). The read timeout bounds reading the request line, headers, and body;
the write timeout bounds writing the response. When a read times out the server
closes the connection instead of waiting.

### Keep-alive

Connections are kept alive by default: an HTTP/1.1 client can send several
requests over one connection instead of reconnecting each time, which is faster
and is what browsers and HTTP libraries expect. The server reuses the
connection unless the client sends `Connection: close` (an HTTP/1.0 client must
opt in with `Connection: keep-alive`). Each response carries the matching
`Connection` header, and an idle kept-alive connection is closed once the read
timeout elapses. HTTP pipelining is supported: bytes read past one request are
carried into the next parse, so a client may send several requests back to back
on one connection and read the responses in order.

### Graceful shutdown

`shutdown()` stops the server cleanly: it stops accepting new connections, lets
the requests already in flight finish, and then `listen` returns. Because
`listen` blocks, call `shutdown` from another goroutine:

```rust
import std/http { Server, Request, Response }
import std/sync { sleep_millis }

fun main() {
    let app = Server.new()
    app.get("/", fun(req: Request) -> Response = Response.text("ok"))
    // Stop cleanly from another goroutine (here after a delay; in a real
    // program on some condition):
    spawn(fun() -> Unit {
        sleep_millis(5000)
        app.shutdown()
    })
    app.listen("127.0.0.1:8080")       // returns once in-flight requests drain
    print("stopped")
}
```

`shutdown` is safe to call more than once. It is the clean way to stop a server
in a test (start it in a goroutine, exercise it, then shut it down) or to wire
up a controlled stop. To trigger it from inside a request handler, have the
handler call a named function that calls `app.shutdown()`, since a handler
closure is a single expression.

### Routing

Register a handler per method with `get`, `post`, `put`, `delete`, or `patch`
(or the general `route(method, pattern, handler)`). Routes are tried in the
order you register them; the first match wins, and a request that matches none
gets a `404`.

A path segment written `:name` is a **capture**: it matches any single segment
and binds it under that name, read in the handler with `req.param("name")`.

```rust
app.get("/users/:id", show_user)        // /users/42  -> param("id") == "42"
app.get("/users/:id/posts/:slug", show) // two captures
```

### Handlers

A handler is a value of type `fun(Request) -> Response`. Use a named function
for anything with more than one statement, or a single-expression closure for a
one-liner:

```rust
fun show_user(req: Request) -> Response {
    let id = req.param("id")
    return Response.text("user ${id}")
}

app.get("/users/:id", show_user)
app.get("/ping", fun(req: Request) -> Response = Response.text("pong"))
```

### `struct Request`

```rust
struct Request {
    method: Method,
    path: String,
    version: String,
    headers: Map<String, String>,   // keys lowercased
    params: Map<String, String>,    // captured :name segments
    query: Map<String, String>,     // decoded query string
    body: String,
}
```

The accessors return `""` when a key is absent, so you can use them without
unwrapping an `Option`:

| Method | Returns |
|--------|---------|
| `req.header(name)` | a request header (case-insensitive), `""` if absent |
| `req.param(name)` | a captured path segment, `""` if absent |
| `req.query_value(name)` | a query-string value, `""` if absent |

```rust
fun search(req: Request) -> Response {
    let q = req.query_value("q")       // /search?q=birds -> "birds"
    let auth = req.header("authorization")
    return Response.text("query: ${q}")
}
```

### `struct Response`

Build a response with a constructor, then refine it with chaining builders.

```rust
struct Response {
    status: Int,
    headers: Map<String, String>,
    body: String,
}
```

| Constructor | Status | Content-Type |
|-------------|--------|--------------|
| `Response.ok(body)` / `Response.text(body)` | 200 | `text/plain` |
| `Response.html(body)` | 200 | `text/html` |
| `Response.json(value)` | 200 | `application/json` (serializes a `ToJson` value, see below) |
| `Response.json_raw(body)` | 200 | `application/json` (body is already JSON text) |
| `Response.file(path)` | 200 / 404 | from the file extension (see below) |
| `Response.created(body)` | 201 | `text/plain` |
| `Response.no_content()` | 204 | none |
| `Response.not_found()` | 404 | `text/plain` |
| `Response.bad_request(msg)` | 400 | `text/plain` |
| `Response.server_error(msg)` | 500 | `text/plain` |
| `Response.redirect(location)` | 302 | sets `Location` |
| `Response.with_body(status, body)` | any | none |

The builders return the response, so they chain:

```rust
fun create(req: Request) -> Response {
    return Response.created(req.body)
        .header("X-Created-By", "raven")
}
```

| Builder | Effect |
|---------|--------|
| `resp.header(name, value)` | set a response header |
| `resp.content_type(value)` | set `Content-Type` |
| `resp.status_code(code)` | replace the status code |

`listen` adds `Content-Length` and `Connection: close` for you when it writes
the response.

### Working with JSON

`Response.json(value)` serializes any value whose type implements
[`ToJson`](json.md), so a `@derive(ToJson)` struct or enum, a `List`, an
`Option`, or a scalar, and sets `application/json`. No string-building, no manual
escaping:

```rust
@derive(ToJson, FromJson)
struct User { name: String, age: Int }

fun show_user(req: Request) -> Response {
    return Response.json(User { name: "Ada", age: 36 })   // {"name":"Ada","age":36}
}
```

To read a JSON request body into a struct, use `req.json<T>()`, which decodes
the body through the type's `FromJson` impl. Decoding failures (bad JSON, a
missing or mistyped field) come back as an `Error`:

```rust
fun create_user(req: Request) -> Response {
    return match req.json<User>() {
        Ok(user) -> Response.json(user).status_code(201),
        Err(e) -> Response.bad_request(e.message()),
    }
}
```

The type can also come from an annotation, `let user: User = req.json()?`, and
`decode<User>(req.body)` from [std/json](json.md) does the same thing without a
request. For ad-hoc access when a struct is overkill, `req.json_value()` returns
the body as a `JsonValue`.

### Serving files

`Response.file(path)` reads a file and serves it with a `Content-Type` chosen
from its extension (`html`, `css`, `js`, `json`, `svg`, `txt`; otherwise
`application/octet-stream`); a missing file is a 404.

`Server.static(prefix, dir)` mounts a directory: it serves
`GET <prefix>/<file>` from `<dir>/<file>`. The file name is a single path
segment, so a request cannot escape `dir` with a slash.

```rust
fun main() {
    let app = Server.new()
    app.static("/static", "public")     // GET /static/style.css -> public/style.css
    app.get("/", fun(req: Request) -> Response = Response.file("public/index.html"))
    app.listen("127.0.0.1:8080")
}
```

Paths are relative to the process's working directory.

### `enum Method`

```rust
enum Method { Get, Post, Put, Delete, Patch, Head, Options, Unknown }
```

`req.method` is one of these. `Unknown` stands for any verb the server does not
model and never matches a registered route. The enum derives `Eq` and
`ToString`, so you can compare it or print it.

### A goroutine per connection

`listen` hands each accepted connection to its own goroutine, so a slow handler
only delays its own client. The goroutines run in parallel across the worker
pool, and a connection waiting on a read or write does not hold up the others.

## See also

- [std/net](net.md) for the TCP sockets this client is built on.
- [std/json](json.md) for parsing and building request and response bodies.
- [std/error](error.md) for the `Error` type returned on transport failure
  (the errors here are tagged with kind `"http"`).
