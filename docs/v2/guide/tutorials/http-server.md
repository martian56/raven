# Tutorial: a guestbook HTTP server

This tutorial builds a small HTTP server with [std/http](../stdlib/http.md): a
guestbook that lists messages, accepts new ones over `POST`, and serves one
message by id. Along the way you meet routing, path captures, request bodies,
JSON and HTML responses, and a little in-memory state. Every step compiles and
runs.

The server is pure Raven on top of [std/net](../stdlib/net.md), so there is
nothing to install. You run the program, then talk to it from another terminal
with `curl` (or a browser).

## Step 1: hello, server

A server is a `Server` with a routing table. You register a handler for a path,
then `listen`. A handler is a function from a `Request` to a `Response`.

```rust
import std/http { Server, Request, Response }

fun home(req: Request) -> Response {
    return Response.text("Hello from Raven!")
}

fun main() {
    let app = Server.new()
    app.get("/", home)
    print("listening on http://127.0.0.1:8080")
    app.listen("127.0.0.1:8080")
}
```

Run it, then from another terminal:

```text
$ curl http://127.0.0.1:8080/
Hello from Raven!
```

`app.listen` binds the address and blocks, serving requests until you stop the
process (Ctrl-C). `Response.text` builds a `200 OK` with a `text/plain` body;
the server adds the `Content-Length` and `Connection` headers for you.

## Step 2: capturing part of the path

A path segment written `:name` matches any single segment and captures it. Read
it in the handler with `req.param("name")`:

```rust
fun greet(req: Request) -> Response {
    return Response.text("Hi, ${req.param("name")}!")
}

fun main() {
    let app = Server.new()
    app.get("/greet/:name", greet)
    app.listen("127.0.0.1:8080")
}
```

```text
$ curl http://127.0.0.1:8080/greet/ada
Hi, ada!
```

A request that matches no route gets a `404` automatically, so `/greet` with no
name (a different shape) does not reach `greet`.

## Step 3: choosing a response

`Response` has a constructor per common case, so you say what you mean:

```rust
Response.text("plain")          // 200, text/plain
Response.html("<h1>hi</h1>")    // 200, text/html
Response.json("{\"ok\":true}")  // 200, application/json
Response.created("made it")     // 201
Response.not_found()            // 404
Response.bad_request("why")     // 400
```

Builders refine a response and chain, because each returns the response:

```rust
fun teapot(req: Request) -> Response {
    return Response.text("short and stout")
        .status_code(418)
        .header("X-Brewing", "true")
}
```

## Step 4: reading the request body

For `POST` and `PUT`, the request carries a body in `req.body`. This handler
echoes whatever it receives back as JSON:

```rust
fun echo(req: Request) -> Response {
    if req.body.length() == 0 {
        return Response.bad_request("send a body")
    }
    return Response.json(req.body)
}

fun main() {
    let app = Server.new()
    app.post("/echo", echo)
    app.listen("127.0.0.1:8080")
}
```

```text
$ curl -X POST -d '{"hi":"there"}' http://127.0.0.1:8080/echo
{"hi":"there"}
```

## Step 5: keeping state

A guestbook needs to remember messages between requests. A module-level `let`
is shared by every handler, and a handler can push to it. Seed it with one
message:

```rust
let messages: List<String> = ["Welcome to the guestbook!"]

fun add_message(req: Request) -> Response {
    if req.body.length() == 0 {
        return Response.bad_request("message body is empty")
    }
    messages.push(req.body)
    return Response.created(req.body)
}
```

Each `POST` appends to the same `messages` list, so the data accumulates across
requests for as long as the server runs.

## Step 6: listing and fetching

To list the messages as a JSON array, build the array text by joining the
entries. (A real service would reach for [std/json](../stdlib/json.md) to
escape values; here we keep it to string building.)

```rust
fun messages_json() -> String {
    let out = "["
    let i = 0
    while i < messages.len() {
        if i > 0 {
            out = out.concat(",")
        }
        out = out.concat("\"").concat(messages[i]).concat("\"")
        i += 1
    }
    return out.concat("]")
}

fun list_messages(req: Request) -> Response {
    return Response.json(messages_json())
}
```

To fetch one message by its index, the captured `:id` arrives as a `String`, so
parse it. `parse_int` returns an `Option<Int>`, which a `match` turns into
either a lookup or a `400`:

```rust
fun message_at(i: Int) -> Response {
    if i >= 0 && i < messages.len() {
        return Response.json("\"".concat(messages[i]).concat("\""))
    }
    return Response.not_found()
}

fun get_message(req: Request) -> Response {
    return match req.param("id").parse_int() {
        Some(i) -> message_at(i),
        None -> Response.bad_request("id must be a number"),
    }
}
```

`get_message` returns the value of the `match`: each arm produces a `Response`,
so there is no early `return` inside the arms.

## The whole program

```rust
import std/http { Server, Request, Response }
import std/string

let messages: List<String> = ["Welcome to the guestbook!"]

fun messages_json() -> String {
    let out = "["
    let i = 0
    while i < messages.len() {
        if i > 0 {
            out = out.concat(",")
        }
        out = out.concat("\"").concat(messages[i]).concat("\"")
        i += 1
    }
    return out.concat("]")
}

fun home(req: Request) -> Response {
    let body = "<h1>Guestbook</h1><ul>"
    let i = 0
    while i < messages.len() {
        body = body.concat("<li>").concat(messages[i]).concat("</li>")
        i += 1
    }
    return Response.html(body.concat("</ul>"))
}

fun list_messages(req: Request) -> Response {
    return Response.json(messages_json())
}

fun add_message(req: Request) -> Response {
    if req.body.length() == 0 {
        return Response.bad_request("message body is empty")
    }
    messages.push(req.body)
    return Response.created(req.body).header("Location", "/messages")
}

fun message_at(i: Int) -> Response {
    if i >= 0 && i < messages.len() {
        return Response.json("\"".concat(messages[i]).concat("\""))
    }
    return Response.not_found()
}

fun get_message(req: Request) -> Response {
    return match req.param("id").parse_int() {
        Some(i) -> message_at(i),
        None -> Response.bad_request("id must be a number"),
    }
}

fun main() {
    let app = Server.new()
    app.get("/", home)
    app.get("/messages", list_messages)
    app.post("/messages", add_message)
    app.get("/messages/:id", get_message)
    print("Guestbook on http://127.0.0.1:8080")
    app.listen("127.0.0.1:8080")
}
```

## Trying it

Run the program, then in another terminal:

```text
$ curl http://127.0.0.1:8080/messages
["Welcome to the guestbook!"]

$ curl -X POST -d "Raven was here" http://127.0.0.1:8080/messages
Raven was here

$ curl http://127.0.0.1:8080/messages
["Welcome to the guestbook!","Raven was here"]

$ curl http://127.0.0.1:8080/messages/1
"Raven was here"

$ curl -i http://127.0.0.1:8080/messages/9
HTTP/1.1 404 Not Found
...
```

Open `http://127.0.0.1:8080/` in a browser to see the messages as a list.

## Where to go next

- The [std/http](../stdlib/http.md) reference lists every `Request` accessor,
  `Response` constructor, and `Server` method, including `query_value` for
  reading `?key=value` query strings.
- The server handles one connection at a time. A request that takes a while to
  answer holds up the clients behind it; per-connection concurrency is a planned
  improvement.
- [std/json](../stdlib/json.md) builds and parses JSON properly (with escaping),
  which you want once message text can contain quotes.
