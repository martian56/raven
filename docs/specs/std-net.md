# std/net Spec

TCP networking: connecting a client stream, binding a listener and
accepting connections, reading and writing bytes, DNS resolution, and a
reachability probe. The primitives bind the raven-runtime C ABI; the
wrappers add the Result/Error model and the handle types in pure Raven.

## Import

```rust
import std/net { connect, listen, dns_lookup, reachable }
```

The methods on `TcpStream` and `TcpListener` come in with the types and need
no separate selector.

## Handle registry model

A `TcpStream` or `TcpListener` cannot cross the FFI boundary, so the runtime
keeps the real socket in a process-wide registry keyed by an incrementing
`i64` id and hands Raven only that id. The Raven `TcpStream { id: Int }` and
`TcpListener { id: Int }` structs wrap the id. Every operation looks the
socket up by id, acts on it, and returns scalars or a String. Ids start at
1; an id of 0 (or any non-positive value) is the failure sentinel paired
with a set last-error. `close` removes the id from the registry, and
dropping the socket closes it.

## Error model

Fallible operations return `Result<T, Error>`. The error is an std/error
`Error` tagged with kind `"net"`, built directly as a struct literal (a
bundled module cannot call another bundled module's free functions, but its
types resolve). The message is a short context prefix (the operation name)
joined to the runtime error text.

There are no error structs across the FFI. The runtime keeps a thread-local
last-error string that a fallible op clears on success and sets to the OS
message on failure; `raven_net_last_error()` returns it, and the Raven
wrapper turns a non-empty last error (or, for the id-returning ops, an id of
0) into an `Err`.

## Surface

```rust
struct TcpStream { id: Int }
struct TcpListener { id: Int }

fun connect(addr: String) -> Result<TcpStream, Error>
fun listen(addr: String) -> Result<TcpListener, Error>

impl TcpListener {
    fun accept(self) -> Result<TcpStream, Error>
}

impl TcpStream {
    fun read(self, max: Int) -> Result<String, Error>
    fun write(self, data: String) -> Result<Int, Error>
    fun set_read_timeout_ms(self, ms: Int)
    fun close(self)
}

fun dns_lookup(host: String) -> Result<List<String>, Error>
fun reachable(addr: String) -> Bool
```

`connect` opens a stream to `addr` in "host:port" form. `listen` binds a
listener to `addr`. `accept` blocks until a connection arrives and returns
the accepted stream (blocking accept is intentional).

`read` reads up to `max` bytes and returns them as a `String`. The payload is
a raw byte buffer carried in a String, preserved exactly with no UTF-8
conversion, not guaranteed text; a negative `max` is an error. `read_all`
reads to the end of the stream, accumulating every byte into one String. A
clean EOF returns `Ok("")`: the runtime clears the last error on a successful
read of zero bytes so the caller can tell EOF from an error. `write` writes all
bytes of `data` and returns the count written.

`set_read_timeout_ms` and `set_write_timeout_ms` set the stream read and write
timeouts. A value of 0 means no timeout (blocking); a positive value makes a
stalled read or write fail rather than hang. `close` on a `TcpStream` releases
the runtime-side socket, and `close` on a `TcpListener` stops listening and
frees the bound port.

`dns_lookup` resolves `host` to its IP addresses. The runtime joins them
with `\n` into one String and the wrapper splits that into a
`List<String>`; an empty result is an empty list, not a one-element list of
`""`.

`reachable` is a non-failing boolean probe: it attempts a short
connect_timeout to `addr` and returns whether it succeeded. It never sets an
error and never returns a Result.

## Client and server in one program

Goroutines run in parallel across a worker pool, so a single Raven program can
be both a server (a goroutine blocked on `accept`, spawning a goroutine per
connection) and a client at the same time. The end-to-end test still runs a
loopback echo server on a background thread on the test side with the compiled
Raven program as the client, because the harness drives a single process.

## FFI path

This module uses `extern "C"` blocks binding raven-runtime symbols
directly, not compiler builtin intrinsics. A Raven `String` is a single GC
pointer at the ABI, so it crosses the boundary unchanged in both
directions; `Bool` maps to Rust `bool` and `Int` to `i64`. The runtime
symbols (`raven_net_connect`, `raven_net_listen`, `raven_net_accept`,
`raven_net_read`, `raven_net_write`, `raven_net_close`,
`raven_net_set_read_timeout_ms`, `raven_dns_lookup`, `raven_net_reachable`,
`raven_net_last_error`) live in `raven-runtime/src/lib.rs`. The socket
registry there is an `OnceLock<Mutex<HashMap<i64, Socket>>>` with an
`AtomicI64` issuing ids.
