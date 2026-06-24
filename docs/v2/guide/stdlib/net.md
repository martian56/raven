# std/net

TCP networking: connect a client stream, bind a listener and accept
connections, read and write bytes, resolve DNS names, and probe whether an
address is reachable. The handle types (`TcpStream`, `TcpListener`) and the
free functions all return values through the `Result<T, Error>` model.

```rust
import std/net { connect }

fun main() {
    match connect("example.com:80") {
        Ok(stream) -> {
            stream.close()
            print("connected")
        }
        Err(e) -> print(e.message),
    }
}
```

## Importing

```rust
import std/net { connect, listen, dns_lookup, reachable }
```

Select the free functions you use. The methods on `TcpStream` and
`TcpListener` arrive with the types, so they need no separate selector.

A socket cannot cross the FFI boundary, so the runtime keeps the real socket
in a process-wide registry and hands Raven an opaque integer id. `TcpStream`
and `TcpListener` are thin structs wrapping that id; every method looks the
socket up by id and acts on it.

```rust
struct TcpStream { id: Int }
struct TcpListener { id: Int }
```

You rarely touch `id` directly. Build the handles with `connect`, `listen`,
and `accept`, and use the methods below.

## The error model

Every fallible operation returns `Result<T, Error>` where `Error` is the
[std/error](error.md) type tagged with kind `"net"`. The message is a short
context prefix (the operation name) joined to the OS error text. Match on the
`Result`, or use the [std/error](error.md) combinators to thread it.

```rust
import std/net { connect }

fun main() {
    match connect("127.0.0.1:9999") {
        Ok(stream) -> stream.close(),
        Err(e) -> print(e.message),     // e.g. connect: ...
    }
}
```

The one exception is `reachable`, which never fails and returns a plain
`Bool` rather than a `Result`.

## Connecting a client

### `connect(addr: String) -> Result<TcpStream, Error>`

Open a TCP stream to `addr` in `"host:port"` form. On success you get an open
`TcpStream`; on failure an `Err` carrying the OS message.

```rust
import std/net { connect }

fun main() {
    match connect("example.com:80") {
        Ok(stream) -> {
            print("connected")
            stream.close()
        }
        Err(e) -> print(e.message),
    }
}
```

## Reading and writing a stream

The stream methods come in with the `TcpStream` type.

### `read(self, max: Int) -> Result<String, Error>`

Read up to `max` bytes and return them as a `String`. The payload is a raw
byte buffer carried in a `String`, not guaranteed to be text; the bytes are
preserved exactly, with no UTF-8 conversion, so binary data round-trips. A
clean end of stream returns `Ok("")`, so an empty result means EOF rather than
an error.

### `read_all(self) -> Result<String, Error>`

Read until end of stream, accumulating every byte into one `String`. It reads
in 4 KiB chunks and stops at a clean EOF. Like `read`, the payload is a byte
buffer carried in a `String`, not guaranteed text.

```rust
import std/net { connect }

fun main() {
    match connect("example.com:80") {
        Ok(stream) -> {
            stream.write("GET / HTTP/1.0\r\nHost: example.com\r\n\r\n")
            match stream.read_all() {
                Ok(body) -> print(body),
                Err(e) -> print(e.message),
            }
            stream.close()
        }
        Err(e) -> print(e.message),
    }
}
```

### `write(self, data: String) -> Result<Int, Error>`

Write all bytes of `data` and return the count written.

### `set_read_timeout_ms(self, ms: Int)`

Set the read timeout in milliseconds. A value of `0` means no timeout
(blocking reads); a positive value makes a stalled read fail rather than hang.
This method does not return a `Result`.

### `set_write_timeout_ms(self, ms: Int)`

Set the write timeout in milliseconds, the same way as the read timeout: `0`
means no timeout, and a positive value makes a stalled write fail rather than
hang. This method does not return a `Result`.

### `close(self)`

Close the stream, releasing the runtime-side socket. It does not return a
`Result`.

```rust
import std/net { connect }

fun main() {
    match connect("example.com:80") {
        Ok(stream) -> {
            stream.set_read_timeout_ms(2000)
            match stream.write("GET / HTTP/1.0\r\n\r\n") {
                Ok(n) -> print("sent ${n} bytes"),
                Err(e) -> print(e.message),
            }
            match stream.read(1024) {
                Ok(reply) -> print(reply),
                Err(e) -> print(e.message),
            }
            stream.close()
        }
        Err(e) -> print(e.message),
    }
}
```

## Binding a listener

### `listen(addr: String) -> Result<TcpListener, Error>`

Bind a TCP listener to `addr` in `"host:port"` form.

### `accept(self) -> Result<TcpStream, Error>`

A method on `TcpListener`. Block until a connection arrives and return the
accepted `TcpStream`. `accept` blocks the calling goroutine, but the other
goroutines keep running on the worker pool.

A server loops: bind once with `listen`, then call `accept` repeatedly. It can
serve each accepted stream before accepting the next, or `spawn` a goroutine
per stream to serve connections concurrently.

```rust
import std/net { listen }

fun serve() {
    match listen("127.0.0.1:7878") {
        Ok(server) -> {
            while true {
                match server.accept() {
                    Ok(client) -> {
                        match client.read(1024) {
                            Ok(req) -> {
                                client.write("hello\n")
                            }
                            Err(e) -> print(e.message),
                        }
                        client.close()
                    }
                    Err(e) -> print(e.message),
                }
            }
        }
        Err(e) -> print(e.message),
    }
}
```

A single program can act as both a client and a server at once: run the accept
loop in one goroutine and open client connections from another. Goroutines run
in parallel across the worker pool, so the server keeps accepting while the
client work proceeds.

### `close(self)`

A method on `TcpListener`. Stop listening and release the runtime-side socket,
freeing the bound port. It does not return a `Result`.

## Resolving and probing

### `dns_lookup(host: String) -> Result<List<String>, Error>`

Resolve `host` to its IP addresses as a `List<String>`. An empty result is an
empty list, not a one-element list of `""`.

```rust
import std/net { dns_lookup }

fun main() {
    match dns_lookup("example.com") {
        Ok(addrs) -> {
            let i = 0
            while i < addrs.len() {
                print(addrs[i])
                i = i + 1
            }
        }
        Err(e) -> print(e.message),
    }
}
```

### `reachable(addr: String) -> Bool`

Probe whether `addr` in `"host:port"` form accepts a TCP connection within a
short timeout. This is a non-failing check: it returns a plain `Bool` and
never sets an error.

```rust
import std/net { reachable }

fun main() {
    if reachable("127.0.0.1:7878") {
        print("up")
    } else {
        print("down")
    }
}
```

## Worked example: a one-shot client

Connect, send a request, read the reply, and close, handling the `Result` at
each step.

```rust
import std/net { connect }

fun fetch(addr: String, request: String) -> Result<String, Error> {
    let stream = connect(addr)?
    stream.set_read_timeout_ms(3000)
    stream.write(request)?
    let reply = stream.read(4096)?
    stream.close()
    return Ok(reply)
}

fun main() {
    match fetch("example.com:80", "GET / HTTP/1.0\r\n\r\n") {
        Ok(body) -> print(body),
        Err(e) -> print(e.message),
    }
}
```

## See also

- [std/http](http.md) builds request and response handling on top of TCP.
- [std/error](error.md) for the `Error` type, `error_kind`, and the `?`
  operator used above.
- [std/string](string.md) for slicing and scanning the byte buffers that
  `read` returns.
