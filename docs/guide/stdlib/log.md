# std.log

Leveled, structured logging that a library and the application using it can
share. A `Logger` has a name, a minimum level, and fields that go into every
line. Lines go to stderr, as text or as JSON.

```rust
from std.log import Logger

fun main() {
    let log = Logger.from_env("shop")
    let req = log.with_field("request", "r-17")
    req.info("order placed")
    // 2026-11-23T09:30:00.123Z INFO shop: order placed request=r-17
}
```

## Making a logger

- `Logger.new(name)`: text lines at INFO.
- `Logger.from_env(name)`: `LOG_LEVEL` (`debug`, `info`, `warn`, `error`)
  sets the level, and `LOG_FORMAT=json` switches to JSON lines.
- `with_level(level)` and `with_json(on)` change it.
- `with_field(key, value)` returns a child logger with one more field. The
  parent is unchanged, so a handler can add a request id without affecting
  other requests.

## Writing

`debug(msg)`, `info(msg)`, `warn(msg)`, `error(msg)`, or `log(level, msg)`.
Levels are `DEBUG()`, `INFO()`, `WARN()`, and `ERROR()`; `parse_level(name)`
reads one from text. `enabled(level)` says whether a line at `level` would be
written, so expensive messages can be skipped.

A text line is `<RFC 3339 time> <LEVEL> <name>: <message> key=value ...`,
with values quoted when they contain spaces. A JSON line is one object with
`ts`, `level`, `logger`, `msg`, and the fields. `format_line(level, msg, ms)`
returns a line without writing it.
