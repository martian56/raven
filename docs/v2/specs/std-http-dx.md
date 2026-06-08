# std/http developer experience: JSON and static files

A `std/http` server today makes the application write boilerplate the library
should own: building JSON by hand with string concatenation and backslash
escaping, and reading static files with manual content-type guessing. This
design folds that work into the library so a handler is a few lines of intent.

## Goal

Take the notes example from this:

```rust
fun escape(s: String) -> String { /* backslash, quote, newline ... */ }
fun note_json(id, text) -> String { /* "{\"id\":\"" .concat ... */ }
fun notes_json(filter) -> String { /* loop building "[ ... ]" */ }
fun content_type_for(name) -> String { /* .html -> text/html ... */ }
fun serve_file(path, ctype) -> Response { /* fs.read, 404 ... */ }
```

to this:

```rust
@derive(ToJson, FromJson)
struct Note { id: String, text: String }

fun list_notes(req: Request) -> Response { return Response.json(all_notes()) }
fun add_note(req: Request) -> Response {
    return match decode<Note>(req.body) {
        Ok(note) -> { store(note); Response.json(note).status_code(201) },
        Err(e) -> Response.bad_request(e.message()),
    }
}
fun main() {
    let app = Server.new()
    app.static("/static", "public")    // serve a whole directory
    app.get("/notes", list_notes)
    app.post("/notes", add_note)
    app.listen("127.0.0.1:8080")
}
```

## What ships

`std/http` gains a dependency on `std/json`, `std/string`, and `std/fs`, and the
following surface. Every item below was verified to compile and run against the
current toolchain.

### JSON responses

- `Response.json(value)`: `fun json<T: ToJson>(value: T) -> Response`. Serializes
  `value` through `std/json`'s `ToJson` (so `@derive(ToJson)` structs/enums,
  lists, options, and scalars all work), sets `Content-Type: application/json`,
  status 200. Chain `.status_code(201)` for Created, etc. This replaces every
  hand-built JSON string and all manual escaping.

### JSON request bodies

- `decode<T: FromJson>(body: String) -> Result<T, Error>` in `std/json`: parse
  and decode a JSON string into a `T` in one call (`from_json_value(parse(body)?)`).
  A handler writes `match decode<Note>(req.body) { Ok(n) -> ..., Err(e) -> ... }`.
  Decoding errors (bad JSON, missing/mistyped fields) come back as an `Error`.
- `Request.json(self) -> Result<JsonValue, Error>`: parse the body to a
  `JsonValue` for ad-hoc access when a struct is overkill.

### Static and HTML file serving

- `Response.file(path: String) -> Response`: read a file from disk and serve it
  with a `Content-Type` chosen from its extension (html, css, js, json, txt,
  svg, png, ... ; `application/octet-stream` otherwise). A missing file is a 404.
- `Server.static(self, prefix: String, dir: String)`: mount a directory. Adds a
  `GET <prefix>/:file` route that serves `<dir>/<file>` via `Response.file`. The
  `:file` capture is a single path segment, so it cannot escape `dir` with a
  slash. (Nested subdirectories under one mount are a later addition.)
- An internal `content_type_for(path)` helper backs both.

## Prerequisite compiler fix (already implemented)

While verifying the generic JSON helpers, a pre-existing type-checker crash
surfaced: `finalize_types` resolved the *shared* type map with the current
body's inference context and panicked (`index out of bounds`) on a variable a
different body owned. It now resolves only the entries each body recorded, and
`find`/`resolve` are bounds-guarded. This turns the crash into ordinary
type-errors and is independent of the std/http work; it shipped as its own fix (#385, merged).

## Known limitation (filed separately)

The fully-typed method form `req.json<Note>()` is intentionally *not* part of
this design. Method-call generic instantiation does not propagate the expected
or explicit type to the method's type parameter, so `T` collapses to `Unit`
(`Unit$from_json`). That is a broader language gap, not specific to JSON; the
request decoder is the free function `decode<T>` until it is fixed. Tracked in #384.

## Testing app refactor

`testing/martian-testing` is rewritten on the new surface as the worked example:
`Note` derives `ToJson`/`FromJson`; handlers use `Response.json` and `app.static`
plus `Response.file`; and `escape`, `note_json`, `notes_json`, `content_type_for`,
`serve_file`, and `static_file` are deleted (roughly 60 lines removed).

## Verification

Each new function gets a golden example (a single end-to-end program that prints
a JSON round-trip and a file-serve result) plus the refactored server example,
and the lib/golden suites must stay green. The server is exercised by hand with
curl for the file and JSON routes.
