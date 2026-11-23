# std.time

Date and time over the raven-runtime C ABI, backed by `chrono`. Every value
is UTC, and there is no local-timezone handling. Timestamps are Unix time:
whole seconds, except `now_millis`, which is milliseconds. The functions are
brought into scope with a selective import.

```rust
from std.time import now, format_timestamp

fun main() {
    print(format_timestamp(now(), "%Y-%m-%d")) // today's UTC date
}
```

## Importing

Pull in the functions you use with a selective `{ ... }` list:

```rust
from std.time import now, now_millis, from_timestamp, weekday, format_timestamp, parse_timestamp, sleep_millis
```

## Timestamp unit

A timestamp is an `int` holding Unix time in UTC. Everywhere except
`now_millis` the unit is **whole seconds** since 1970-01-01 00:00:00 UTC.
`from_timestamp`, `weekday`, `format_timestamp`, and the value returned by
`parse_timestamp` all use seconds. `now_millis` returns **milliseconds**, and
`sleep_millis` takes a duration in **milliseconds**; divide a millisecond
value by 1000 before passing it to the seconds-based functions.

## Datetime classes

`from_timestamp` returns a `DateTime`, which nests a `Date` and a `Time`:

```rust
class Date {
    year: int
    month: int
    day: int
}
class Time {
    hour: int
    minute: int
    second: int
}
class DateTime {
    date: Date
    time: Time
}
```

`month` is 1 through 12, `day` is 1 through 31, `hour` is 0 through 23, and
`minute` and `second` are 0 through 59.

Each class has a `ToString` impl producing ISO-like text. A `Date` renders
as `1970-01-01` (year zero-padded to four digits, month and day to two), a
`Time` as `00:00:00`, and a `DateTime` as the two joined by a space,
`1970-01-01 00:00:00`.

```rust
from std.time import from_timestamp

fun main() {
    let dt = from_timestamp(0)
    print(dt.to_string()) // 1970-01-01 00:00:00
    print(dt.date.year) // 1970
    print(dt.time.hour) // 0
}
```

## Current time

### `now() -> int`

The current Unix timestamp in whole seconds (UTC).

### `now_millis() -> int`

The current Unix time in milliseconds (UTC).

```rust
from std.time import now, now_millis

fun main() {
    let secs = now()
    let millis = now_millis()
    print(millis / 1000 - secs) // 0 (same instant, different units)
}
```

## Decomposition

### `from_timestamp(ts: int) -> DateTime`

Decompose a Unix timestamp (seconds, UTC) into a `DateTime`. A timestamp
outside chrono's representable range falls back to the epoch.

### `weekday(ts: int) -> int`

The weekday of `ts` (UTC) as `0` for Sunday through `6` for Saturday.

```rust
from std.time import from_timestamp, weekday

fun main() {
    let dt = from_timestamp(1700000000)
    print(dt.to_string()) // 2023-11-14 22:13:20
    print(weekday(1700000000)) // 2 (Tuesday)
}
```

## Formatting and parsing

`pattern` is a chrono strftime pattern. Common tokens:

| Token | Meaning |
|-------|---------|
| `%Y`  | year, four digits |
| `%m`  | month, `01` through `12` |
| `%d`  | day of month, `01` through `31` |
| `%H`  | hour, `00` through `23` |
| `%M`  | minute, `00` through `59` |
| `%S`  | second, `00` through `59` |

### `format_timestamp(ts: int, pattern: str) -> str`

Render the UTC datetime of `ts` with the given chrono strftime pattern, for
example `"%Y-%m-%d %H:%M:%S"`.

```rust
from std.time import format_timestamp

fun main() {
    print(format_timestamp(1700000000, "%Y-%m-%d %H:%M:%S")) // 2023-11-14 22:13:20
    print(format_timestamp(1700000000, "%Y/%m/%d")) // 2023/11/14
}
```

### `parse_timestamp(text: str, pattern: str) -> Result[int, Error]`

Parse `text` as a UTC datetime by the given pattern and return the Unix
timestamp in seconds. Parsing is fallible: on failure it returns an `Err`
holding a `std.error` `Error` tagged with kind `"time"`. Handle the `Result`
with a `match` or the `?` operator.

```rust
from std.time import parse_timestamp

fun main() {
    let parsed = parse_timestamp("2023-11-14 22:13:20", "%Y-%m-%d %H:%M:%S")
    match parsed {
        case Ok(ts) { print(ts) } // 1700000000
        case Err(e) { print(e.to_string()) }
    }

    let bad = parse_timestamp("not a date", "%Y-%m-%d")
    match bad {
        case Ok(ts) { print(ts) }
        case Err(e) { print(e.kind()) } // time
    }
}
```

## Sleep

### `sleep_millis(ms: int)`

Sleep the current thread for `ms` milliseconds. A negative value is treated
as zero.

```rust
from std.time import sleep_millis

fun main() {
    sleep_millis(100) // pause for 100 ms
}
```

## RFC 3339 and UTC offsets

RFC 3339 (`2026-11-23T09:30:00Z`) is the timestamp format of JSON APIs and
databases.

| Function | Does |
|----------|------|
| `format_rfc3339(ts)` | Unix seconds as UTC text ending in `Z` |
| `format_rfc3339_millis(ms, offset_minutes)` | Unix milliseconds at an offset, such as `+04:00`; milliseconds appear when nonzero |
| `parse_rfc3339(text) -> Result[int, Error]` | Unix seconds from text at any offset; a space may stand in for `T` |
| `parse_rfc3339_millis(text) -> Result[int, Error]` | the same, in milliseconds |
| `local_offset_seconds(ts)` | the local time zone's offset from UTC at `ts`, which shifts with daylight saving time |

```rust
from std.time import format_rfc3339, format_rfc3339_millis, parse_rfc3339

fun main() {
    let ts = parse_rfc3339("2026-11-23T13:30:00+04:00").unwrap()
    print(format_rfc3339(ts)) // 2026-11-23T09:30:00Z
    print(format_rfc3339_millis(ts * 1000, 240)) // 2026-11-23T13:30:00+04:00
}
```

## Calendar arithmetic

`is_leap_year(year)`, `days_in_month(year, month)`,
`date_to_timestamp(year, month, day)` (midnight UTC), `start_of_day(ts)`,
`add_days(ts, n)`, and `add_months(ts, n)`. Adding months keeps the time of
day and clamps the day to the target month, so January 31 plus one month is
the last day of February. All of these work in UTC.

```rust
from std.time import add_months, date_to_timestamp, format_rfc3339

fun main() {
    let jan31 = date_to_timestamp(2024, 1, 31)
    print(format_rfc3339(add_months(jan31, 1))) // 2024-02-29T00:00:00Z
}
```

## Worked example: a formatted timestamp round trip

Format the current time, then parse the formatted text back to a timestamp:

```rust
from std.time import now, format_timestamp, parse_timestamp, from_timestamp, weekday

fun main() {
    let ts = now()
    let pattern = "%Y-%m-%d %H:%M:%S"
    let text = format_timestamp(ts, pattern)
    print(text) // for example 2026-06-05 09:41:00

    let dt = from_timestamp(ts)
    print(dt.date.year) // 2026
    print(weekday(ts)) // 0 (Sunday) through 6 (Saturday)

    match parse_timestamp(text, pattern) {
        case Ok(round_tripped) { print(round_tripped == ts) } // true
        case Err(e) { print(e.to_string()) }
    }
}
```

## See also

- [std.error](error.md) for the `Error` type that `parse_timestamp` returns,
  plus helpers like `is_ok`, `unwrap_or`, and `with_context`.
- [std.string](string.md) for building and inspecting the text you format and
  parse.
