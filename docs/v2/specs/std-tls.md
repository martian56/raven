# std/tls Spec

Client-side TLS: opening a verified TLS connection, reading and writing
encrypted bytes, and a configuration builder for private CAs, client
certificates (mutual TLS), and a development-only skip of verification. The
primitives bind the raven-runtime C ABI; the wrappers add the Result/Error
model and the handle types in pure Raven, mirroring `std/net`.

For outbound HTTPS the `std/http` client already speaks TLS (it is backed by
`ureq` with rustls), so `get("https://...")` needs nothing from this module.
`std/tls` is for raw TLS streams: speaking a binary protocol such as Redis,
Postgres, or MySQL over an encrypted socket.

## Import

```rust
import std/tls { connect, connect_with, config }
```

The methods on `TlsStream` and `TlsConfig` come in with the types and need no
separate selector.

## Handle registry model

A live TLS session cannot cross the FFI boundary, so the runtime keeps it in a
process-wide registry keyed by an incrementing `i64` id and hands Raven only
that id. `TlsStream { id: Int }` and `TlsConfig { id: Int }` wrap the id. Ids
start at 1; an id of 0 is the failure sentinel paired with a set last-error.
A TLS session is stateful and, unlike a raw socket fd, cannot be cloned per
read, so each connection is held behind its own mutex: one goroutine should own
a given `TlsStream` at a time.

## Trust and crypto

TLS is `rustls` with the `ring` crypto provider. The default configuration
verifies the server against the bundled Mozilla root store (`webpki-roots`) and
checks that the certificate matches `server_name`, which is also sent as SNI.

`TlsConfig` adjusts that:

- `add_ca_file(path)` trusts the PEM certificate(s) in `path` in addition to the
  bundled roots, for a server signed by a private CA.
- `client_cert(cert_path, key_path)` presents a client certificate chain and
  private key (PEM) for mutual TLS.
- `insecure_skip_verify()` accepts any certificate. The session is still
  encrypted, but the peer is not authenticated, so this is for local
  development only.

## STARTTLS upgrade

`connect` does its own TCP connect, so it is for TLS from the first byte (HTTPS,
Redis over TLS). Some protocols instead open a plaintext socket, exchange a
start-TLS step, and then turn that same socket into TLS: Postgres (SSLRequest),
MySQL, and SMTP STARTTLS. For those, connect with `std/net`, perform the
protocol's negotiation, then call `upgrade(stream, server_name)` (or
`upgrade_with` for a custom config) to run the handshake over the existing
socket and get back a `TlsStream`.

The `std/net` stream is consumed by an upgrade: on success its socket moves into
the returned `TlsStream`, and on failure the connection is closed. Do not use
the original `TcpStream` afterward. The config and SNI are validated before the
socket is taken, so an upgrade that fails on a bad config leaves the original
stream usable.

## Error model

Fallible operations return `Result<T, Error>`. The error is an std/error `Error`
tagged with kind `"tls"`, built as a struct literal, its message a short context
prefix (the operation name) joined to the runtime error text. As in `std/net`,
no error structs cross the FFI: the runtime keeps a thread-local last-error
string that a fallible op clears on success and sets on failure, and
`raven_tls_last_error()` returns it. An id of 0 from a connect, a negative count
from a write, or a non-empty last error becomes an `Err`.

## Bytes

Reads and writes carry bytes in a `String` buffer, the same convention `std/net`
uses today. `read(max)` returns up to `max` decrypted bytes (a clean close
yields `Ok("")`); `write(data)` encrypts and sends every byte and returns the
count. A dedicated bytes type is future work tracked with the database-client
libraries.

## Scheduler integration

The TCP connect, the TLS handshake, and every read and write run inside the
runtime's `gc::blocking` bracket, the same one `std/net` uses. A goroutine
waiting on TLS I/O therefore hands its worker back to the scheduler (which keeps
the worker pool at one runnable thread per core) and parks at a GC safepoint, so
many TLS connections can run across the worker pool at once.

## C ABI surface

```
raven_tls_last_error() -> String
raven_tls_config_new() -> i64
raven_tls_config_add_ca_file(cfg, path) -> i64
raven_tls_config_client_cert(cfg, cert_path, key_path) -> i64
raven_tls_config_insecure_skip_verify(cfg, on)
raven_tls_config_free(cfg)
raven_tls_connect(addr, server_name, cfg) -> i64
raven_tls_upgrade(net_stream_id, server_name, cfg) -> i64
raven_tls_read(stream_id, max) -> String
raven_tls_write(stream_id, data) -> i64
raven_tls_close(stream_id)
raven_tls_set_read_timeout_ms(stream_id, ms)
raven_tls_set_write_timeout_ms(stream_id, ms)
```

A `cfg` of 0 in `raven_tls_connect` selects the default verifying configuration,
so the common path needs no config handle.

## Example

```rust
import std/io { println }
import std/tls { connect }

fun main() {
    match connect("example.com:443", "example.com") {
        Ok(stream) -> {
            let _ = stream.write("GET / HTTP/1.1\r\nHost: example.com\r\nConnection: close\r\n\r\n")
            match stream.read(64) {
                Ok(data) -> println(data.trim()),
                Err(e) -> println("read: ${e.message()}"),
            }
            stream.close()
        },
        Err(e) -> println("connect: ${e.message()}"),
    }
}
```
