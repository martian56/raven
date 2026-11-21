# std.tls

Client-side TLS streams over the raven-runtime TLS backend. Use `std.tls` when
you need a raw encrypted byte stream for a protocol such as Redis, Postgres,
MySQL, or SMTP after STARTTLS negotiation. For ordinary HTTPS requests, use
[std.http](http.md); its client already handles TLS.

## Importing

```rust
from std.tls import connect, connect_with, upgrade, upgrade_with, config
```

`TlsStream` and `TlsConfig` methods come in with their types.

## Connecting

### `connect(addr: str, server_name: str) -> Result[TlsStream, Error]`

Open a TCP connection to `addr` (`"host:port"`), start TLS immediately, send
`server_name` as SNI, and verify the peer against the bundled Mozilla root
store.

```rust
from std.tls import connect

fun main() {
    match connect("example.com:443", "example.com") {
        case Ok(stream) {
            let _ = stream.write("GET / HTTP/1.1\r\nHost: example.com\r\nConnection: close\r\n\r\n")
            print(stream.read_all()?)
            stream.close()
        }
        case Err(e) { print(e.message()) }
    }
}
```

### `connect_with(addr, server_name, cfg) -> Result[TlsStream, Error]`

Connect with a custom `TlsConfig`, for private CAs, client certificates, or a
development-only skip of certificate verification.

## Configuring Trust

### `config() -> TlsConfig`

Create a TLS client configuration. It starts with the bundled public roots.

### `add_ca_file(path: str) -> TlsConfig`

Trust PEM certificate(s) from `path` in addition to the bundled roots and return
the config for chaining.

### `add_ca_file_checked(path: str) -> Result[TlsConfig, Error]`

The checked form of `add_ca_file`. Missing, unreadable, or malformed CA files
return an `Err` immediately.

### `client_cert(cert_path: str, key_path: str) -> TlsConfig`

Configure a client certificate chain and private key for mutual TLS and return
the config for chaining.

### `client_cert_checked(cert_path: str, key_path: str) -> Result[TlsConfig, Error]`

The checked form of `client_cert`. Missing, unreadable, or malformed certificate
or key files return an `Err` immediately.

### `insecure_skip_verify() -> TlsConfig`

Disable certificate verification. The connection is encrypted but the peer is
not authenticated, so this is only for local development.

```rust
from std.tls import config, connect_with

fun main() {
    let cfg = config().add_ca_file_checked("dev-ca.pem")?
    match connect_with("db.internal:5432", "db.internal", cfg) {
        case Ok(stream) {
            // speak the protocol
            stream.close()
        }
        case Err(e) { print(e.message()) }
    }
    cfg.free()
}
```

## STARTTLS

### `upgrade(stream: TcpStream, server_name: str) -> Result[TlsStream, Error]`

Turn an already-connected `std.net` TCP stream into a TLS stream on the same
socket. This is for protocols that negotiate in plaintext and then switch to
TLS. On success the original TCP stream is consumed.

### `upgrade_with(stream, server_name, cfg) -> Result[TlsStream, Error]`

The configured form of `upgrade`.

## Stream Methods

### `read(max: int) -> Result[str, Error]`

Read up to `max` decrypted bytes. A clean close returns `Ok("")`.

### `read_all() -> Result[str, Error]`

Read decrypted bytes until the peer closes the connection.

### `write(data: str) -> Result[int, Error]`

Encrypt and send all bytes in `data`, returning the count written.

### `set_read_timeout_ms(ms: int)`, `set_write_timeout_ms(ms: int)`

Set per-stream read or write timeouts. `0` disables the timeout.

### `close()`

Send TLS close-notify and release the runtime-side connection.
