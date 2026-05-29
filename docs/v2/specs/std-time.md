# std/time Spec

Date and time over the raven-runtime C ABI, backed by `chrono`. Every
value is UTC. Timestamps are Unix time in seconds (or milliseconds for
`now_millis`); there is no local-timezone handling. The primitives bind
runtime symbols through `extern "C"`; the structs and the convenience
helpers are pure Raven on top of them.

## Import

```raven
import std/time { now, now_millis, from_timestamp, weekday, format_timestamp, parse_timestamp, sleep_millis }
```

## Structs

```raven
struct Date { year: Int, month: Int, day: Int }
struct Time { hour: Int, minute: Int, second: Int }
struct DateTime { date: Date, time: Time }
```

`month` is 1 through 12, `day` is 1 through 31, `hour` is 0 through 23,
`minute` and `second` are 0 through 59. `Date`, `Time`, and `DateTime`
each have a `ToString` impl producing ISO-like text: a `Date` renders as
`1970-01-01` (year zero-padded to four digits, month and day to two), a
`Time` as `00:00:00`, and a `DateTime` as the two joined by a space,
`1970-01-01 00:00:00`.

## Surface

### Current time

```raven
fun now() -> Int
fun now_millis() -> Int
```

`now` is the current Unix timestamp in whole seconds (UTC). `now_millis`
is the current Unix time in milliseconds (UTC). Both are
non-deterministic; tests assert only structural facts about them (for
example `now() > 1700000000`).

### Decomposition

```raven
fun from_timestamp(ts: Int) -> DateTime
fun weekday(ts: Int) -> Int
```

`from_timestamp` decomposes a Unix timestamp (seconds, UTC) into a
`DateTime`. A struct cannot cross the FFI boundary, so the runtime exposes
one scalar extractor per field (`raven_time_year`, `_month`, `_day`,
`_hour`, `_minute`, `_second`), and the `DateTime` is assembled in Raven.
A timestamp outside chrono's representable range falls back to the epoch.

`weekday` returns the weekday of `ts` as 0 for Sunday through 6 for
Saturday (chrono's `num_days_from_sunday`).

### Formatting and parsing

```raven
fun format_timestamp(ts: Int, pattern: String) -> String
fun parse_timestamp(text: String, pattern: String) -> Result<Int, Error>
```

`pattern` is a chrono strftime pattern, for example `%Y-%m-%d %H:%M:%S`.
`format_timestamp` renders the UTC datetime of `ts` with that pattern.
`parse_timestamp` parses `text` as a UTC datetime by the pattern and
returns the Unix timestamp in seconds.

Parsing is fallible. The runtime keeps a thread-local last-error string:
the parse wrapper runs `raven_time_parse`, then reads
`raven_time_last_error` and turns a non-empty value into an `Err`. The
error is an std/error `Error` tagged with kind `"time"`. The `Error` type
resolves across the bundled-module boundary, but a sibling module's free
functions do not (issue #178), so the wrapper builds the `Error` struct
literal directly rather than calling `error_kind`.

### Sleep

```raven
fun sleep_millis(ms: Int)
```

`sleep_millis` sleeps the current thread for `ms` milliseconds
(`std::thread::sleep`). A negative value is treated as zero. The Raven
return type is `Unit`.

## FFI path

This module uses `extern "C"` blocks binding raven-runtime symbols
directly, not compiler builtin intrinsics. A Raven `String` is a single GC
pointer at the ABI, so it crosses the boundary unchanged in both
directions, which lets `extern "C"` carry `String` arguments and returns
with no codegen change. Returning a struct across the FFI is not
supported, so the runtime returns scalar `i64` components and the `.rv`
wrapper assembles the `Date`, `Time`, and `DateTime` structs. The runtime
symbols (`raven_time_now`, `raven_time_now_millis`, `raven_time_year`,
`raven_time_month`, `raven_time_day`, `raven_time_hour`,
`raven_time_minute`, `raven_time_second`, `raven_time_weekday`,
`raven_time_format`, `raven_time_parse`, `raven_time_last_error`,
`raven_time_sleep_millis`) live in `raven-runtime/src/lib.rs` and are
backed by `chrono` in UTC. The runtime depends on `chrono` with
`default-features = false` and the `clock` and `std` features.
```
