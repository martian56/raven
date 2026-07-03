# std/http Spec

An HTTP/1.1 client and a small HTTP/1.1 server. The client (GET, POST, PUT,
DELETE, ...) binds the raven-runtime C ABI (backed by ureq) and adds the
Result/Error model and the `HttpResponse` type in pure Raven. The server
(`Server`, `Request`, `Response`, `Method`) is written entirely in Raven on top
of `std/net` (TCP), so it needs no runtime support of its own.

## Import

```rust
import std/http { get, post, put, delete, request }
```

The `HttpResponse` type comes in with the module and needs no separate
selector.

## Surface

```rust
struct HttpResponse {
    status_code: Int,
    status_text: String,
    headers: String,
    body: String,
}

fun get(url: String) -> Result<HttpResponse, Error>
fun post(url: String, body: String) -> Result<HttpResponse, Error>
fun put(url: String, body: String) -> Result<HttpResponse, Error>
fun delete(url: String) -> Result<HttpResponse, Error>
fun request(method: String, url: String, body: String, headers: String) -> Result<HttpResponse, Error>
```

`get` and `delete` send no request body. `post` and `put` send their
`body` argument. `request` is the general form the others delegate to: its
`headers` argument is a String of `Key: Value` lines separated by `\n`
(empty for none), and `get`/`post`/`put`/`delete` pass `""` for it. The
response `headers` field is likewise the response headers as `Key: Value`
lines joined by `\n`.

## Response handle registry model

A `ureq::Response` is consumed when its body is read, so it cannot be
handed back across calls. The whole request runs in one runtime call
(`raven_http_request`) that eagerly reads the status code, reason phrase,
all response headers, and the body into an owned struct, stores that in a
process-wide registry keyed by an incrementing `i64` id, and returns the
id. The extractors (`raven_http_status`, `raven_http_status_text`,
`raven_http_body`, `raven_http_header`, `raven_http_headers`) read the
stored struct by id, and `raven_http_free` drops the entry. The Raven
`request` wrapper pulls the four fields, frees the id, and returns
`Ok(HttpResponse { ... })`. Ids start at 1; an id of 0 is the failure
sentinel paired with a set last-error. The registry is an
`OnceLock<Mutex<HashMap<i64, HttpResp>>>` with an `AtomicI64` issuing ids.

## Non-2xx is a successful response, not an Err

A non-2xx HTTP status (for example 404 or 500) is a successful request
that yielded a response, not a transport failure. ureq surfaces a 4xx/5xx
as `Err(ureq::Error::Status(code, resp))`; the runtime treats that as a
normal response, captures its status and body, and stores a normal
registry entry. The Raven caller therefore gets `Ok(HttpResponse)` with
`status_code` set to 404 (or whatever the server returned) and inspects
the code itself. Only transport errors (DNS, connect, timeout, malformed
response) become id 0 plus a last-error, and only those take the `Err`
path.

## Error model

Fallible requests return `Result<HttpResponse, Error>`. On a transport
failure the error is an std/error `Error` tagged with kind `"http"`, built
directly as a struct literal (a bundled module cannot call another bundled
module's free functions, but its types resolve). The message is a short
context prefix (the HTTP method) joined to the runtime error text.

There are no error structs across the FFI. The runtime keeps a
thread-local last-error string that `raven_http_request` clears on success
and sets to the transport error text on failure; `raven_http_last_error()`
returns it, and the Raven wrapper turns a non-empty last error (or an id of
0) into an `Err`. A non-2xx status never sets this slot.

## TLS backend

TLS is provided by ureq's default `tls` feature: rustls with the
`webpki-roots` bundled Mozilla root certificate set. This needs no OpenSSL
or system-TLS `-sys` package, so the build is portable across the Linux CI
host and Windows without extra system libraries. The v1 carryover note
about "TLS via system roots" is intentionally not followed here: build
portability across CI takes precedence, so the roots are bundled rather
than read from the OS trust store. On windows-msvc, rustls pulls in
`bcrypt.lib` (BCryptGenRandom), which the linker adds to the native system
library list.

## Timeouts

The runtime builds the ureq agent with fixed timeouts so a hung server
cannot wedge a program or a test: 5 seconds to connect, 10 seconds to read,
and 10 seconds to write.

Endpoints that legitimately take longer (model inference APIs, slow report
generators) use `request_with_timeout(method, url, body, headers,
timeout_ms)`: a positive `timeout_ms` is an overall deadline covering the
connect, the send, and reading the whole body, and a deadline overrun
surfaces as an `Err` like any other transport failure. `timeout_ms <= 0`
keeps the defaults, so `request_with_timeout(m, u, b, h, 0)` behaves
exactly like `request(m, u, b, h)`.

## Streaming

`stream(method, url, body, headers, timeout_ms)` opens a request without
buffering the body: the returned `HttpStream` carries the `status_code`,
`status_text`, and `headers` captured at open, and `read_chunk()` pulls the
body as the server sends it, at most 8 KiB per call, returning `Ok("")`
when the stream ends. This is how to consume server-sent events, chunked
transfers, and bodies too large or too slow to hold whole. `close()`
releases the stream and its connection.

Streaming semantics differ from the buffered path in three ways:

* The timeout is per operation, not overall: a positive `timeout_ms` bounds
  the connect and each individual read, because a healthy stream may stay
  open for minutes. A read that outlives it is an `Err` from `read_chunk`.
* A non-2xx status opens normally (the error body is often worth reading);
  only a transport failure is an `Err` from `stream`.
* One read at a time per stream: the runtime hands the reader to the read
  in flight, and a concurrent `read_chunk` on the same stream observes end
  of stream rather than blocking.

Chunk boundaries follow socket reads and carry no meaning; a consumer
reassembles or splits on its own framing (for SSE, blank-line-delimited
events).

The runtime side registers the open response in a stream registry keyed by
the same id space as buffered responses (`raven_http_stream_open`,
`raven_http_stream_read`, `raven_http_stream_status`,
`raven_http_stream_status_text`, `raven_http_stream_headers`,
`raven_http_stream_close`). The registry lock is never held across a
blocking read: the reader is taken out of the entry for the read's
duration and put back after.

## FFI path

This module uses `extern "C"` blocks binding raven-runtime symbols
directly, not compiler builtin intrinsics. A Raven `String` is a single GC
pointer at the ABI, so it crosses the boundary unchanged in both
directions, and `Int` maps to `i64`. The runtime symbols
(`raven_http_request`, `raven_http_status`, `raven_http_status_text`,
`raven_http_body`, `raven_http_header`, `raven_http_headers`,
`raven_http_free`, `raven_http_last_error`) live in
`raven-runtime/src/lib.rs`.

## Testing without external network

CI has no external network and Raven v2 has no threads, so a single program
cannot be both server and client. The end-to-end smoke test binds a
`std::net::TcpListener` on `127.0.0.1:0`, spawns a background thread that
accepts one connection, reads the request headers, and writes a fixed
minimal HTTP/1.1 response (`HTTP/1.1 200 OK` with `Content-Length` and
`Connection: close`). The compiled Raven program is the client: it GETs the
loopback URL (passed through the `RAVEN_HTTP_URL` env var) and prints the
status code then the body, asserted to be exactly `200` then `hello`. Read
timeouts on both ends keep a failure from hanging CI. No test hits a real
external URL.

## Server

The server side is pure Raven over `std/net`, no new runtime code. You build a
routing table on a `Server` and call `listen`:

```rust
import std/http { Server, Request, Response }

fun greet(req: Request) -> Response {
    return Response.text("Hello, ${req.param("name")}!")
}

fun main() {
    let app = Server.new()
    app.get("/", fun(req: Request) -> Response = Response.html("<h1>hi</h1>"))
    app.get("/greet/:name", greet)
    app.post("/echo", fun(req: Request) -> Response = Response.json(req.body))
    app.listen("127.0.0.1:8080")
}
```

### Surface

```rust
enum Method { Get, Post, Put, Delete, Patch, Head, Options, Unknown }

struct Request {
    method: Method, path: String, version: String,
    headers: Map<String, String>,   // keys lowercased
    params: Map<String, String>,    // captured `:name` path segments
    query: Map<String, String>,     // decoded query string
    body: String,
}
fun Request.header(name) -> String          // "" if absent
fun Request.param(name) -> String           // path capture, "" if absent
fun Request.query_value(name) -> String     // "" if absent
fun Request.json<T: FromJson>(self) -> Result<T, Error>  // decode the body into a struct
fun Request.json_value(self) -> Result<JsonValue, Error> // parse the body to a JsonValue

struct Response { status: Int, headers: Map<String, String>, body: String }
// constructors
fun Response.json<T: ToJson>(value) -> Response       // serialize, application/json
fun Response.json_raw(body) -> Response               // body is already JSON text
fun Response.file(path) -> Response                   // serve a file, 404 if missing
Response.text/ok/html/created/no_content/not_found/bad_request/server_error/redirect/with_body
// chaining builders (return self)
fun Response.header(name, value) -> Response
fun Response.content_type(value) -> Response
fun Response.status_code(code) -> Response

// decode a request body into a struct (std/json)
fun decode<T: FromJson>(body: String) -> Result<T, Error>

struct Server { routes: List<Route> }
fun Server.new() -> Server
fun Server.route(method, pattern, handler)            // handler: fun(Request) -> Response
fun Server.get/post/put/delete/patch(pattern, handler)
fun Server.static(prefix, dir)                        // mount a directory
fun Server.listen(addr)                               // binds and blocks
```

### Handlers

A handler is a value of type `fun(Request) -> Response`, a named function or a
single-expression closure (`fun(req: Request) -> Response = ...`). Multi-statement
handlers are named functions, since a block-bodied closure cannot yet return a
value. Routes are tried in registration order; `:name` segments capture into
`params` and a non-match falls through to the next route, then to a 404.

### Request and response framing

`listen` binds with `std/net`, then for each connection reads the header block up
to the first blank line, parses the request line and headers, reads the body up
to `Content-Length`, routes, and writes the response framed with `Content-Length`
and `Connection: close`. A malformed request is answered `400`.

### A goroutine per connection

`listen` accepts in a loop and hands each connection to its own goroutine, so a
slow handler only delays its own client, not the ones behind it. The goroutines
run in parallel across the worker pool, and a connection blocked in a read or
write releases the shared net registry while it waits, so other connections are
never serialized behind it.
