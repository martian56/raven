# std/http Spec

An HTTP/1.1 client: GET, POST, PUT, and DELETE returning a captured
response. The primitives bind the raven-runtime C ABI (backed by ureq);
the wrappers add the Result/Error model and the `HttpResponse` type in
pure Raven.

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
