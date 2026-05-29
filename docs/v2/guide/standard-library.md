# Standard library (v2)

The standard library ships as bundled `std/...` modules. Import a module
with a selective import for its free functions, or a bare import for a
module that adds methods or constructors. The core traits are always in
scope.

Two import gotchas to keep in mind:

- To use `String` methods such as `concat` or `to_upper`, a module must
  `import std/string`, which merges the `impl String` block.
- `import std/collections` whole (not `{ Map }`), so the `Map.new()` and
  `Set.new()` constructors resolve.

## Core traits

`std/core` defines `ToString`, `Eq`, `Ord`, `Hash`, and `Iterator<T>`,
and implements them for the primitive types. These are auto imported, so
no `import std/core` line is needed.

```raven
fun describe<T: ToString>(x: T) -> String = x.to_string()
```

## std/io

Console input and output.

- `print(s: String)`: write without a trailing newline.
- `println(s: String)`: write with a trailing newline.
- `input(prompt: String) -> String`: print the prompt, read one line.
- `read_line() -> String`: read one line from standard input.

```raven
import std/io { println }

fun main() {
    println("${40 + 2}")        // 42
}
```

A global `print` builtin is also always available and accepts any
`ToString` value (it appends a newline).

## std/string

Methods on `String`, merged by a bare import.

`length`, `is_empty`, `char_at`, `substring(start, end)`, `concat`,
`to_upper`, `to_lower`, `trim`, `is_blank`, `repeat(n)`, `index_of`,
`contains`, `starts_with`, `ends_with`, `replace(from, to)`.

```raven
import std/string

fun main() {
    print("hello".to_upper())            // HELLO
    print("a-b".replace("-", "+"))       // a+b
    print("raven".substring(1, 4))       // ave
}
```

## std/collections

Generic hash-backed `Set<T: Eq + Hash>` and `Map<K: Eq + Hash, V>`, built
with `Set.new()` and `Map.new()`. Operations are O(1) average. Import the
whole module. Keys must implement `Eq + Hash` (`Int`, `Bool`, `String`, or
a user type with both impls; `Char` and `Float` are not yet hashable).

- `Set`: `add`, `remove`, `contains`, `len`, `is_empty`.
- `Map`: `set(k, v)`, `get(k) -> Option<V>`, `has(k)`, `remove(k)`,
  `keys()`, `values()`, `len`, `is_empty`.

```raven
import std/collections

fun main() {
    let m = Map.new()
    m.set("a", 10)
    match m.get("a") {
        Some(v) -> print(v),
        None -> print(0),
    }
}
```

## std/math

Float and integer math.

- Integer: `abs_int`, `min_int`, `max_int`, `clamp_int`, `pow_int`.
- Float: `sqrt`, `pow`, `exp`, `ln`, `abs`, `min`, `max`, `clamp`,
  `floor`, `ceil`, `round`, `trunc`, `sin`, `cos`.
- Constants: `pi()`, `e()`, `tau()`.

```raven
import std/math { sqrt, pow_int }

fun main() {
    print(sqrt(16.0))           // 4
    print(pow_int(2, 10))       // 1024
}
```

## std/cmp

Comparison helpers generic over `T: Ord`.

- `min(a, b)`, `max(a, b)`, `clamp(x, lo, hi)`.
- `sort(xs) -> List<T>`, `sorted_by(xs, cmp)`.
- `max_of(xs) -> Option<T>`, `min_of(xs) -> Option<T>`.

```raven
import std/cmp { sort, max_of }

fun main() {
    let s = sort([5, 2, 8, 1])
    print(s[0])                 // 1
}
```

## std/hash

Non cryptographic hashing.

- `fnv1a(s)`, `djb2(s)`, `checksum(s)` over strings.
- `hash_int(n)`, `combine(seed, value)` for mixing.

```raven
import std/hash { fnv1a }

fun main() {
    print(fnv1a("abc") == fnv1a("abc"))     // true
}
```

## std/iter

Lazy iterators. `xs.iter()` bridges a `List<T>` into an iterator. The
adapters `map`, `filter`, `take`, `skip`, and `enumerate` are lazy. The
consumers `collect`, `count`, `fold`, `any`, `all`, `find`, and
`for_each` drive the pipeline.

```raven
import std/iter { collect, fold, count }

fun main() {
    let xs = [1, 2, 3, 4, 5, 6]
    let kept = collect(xs.iter().map(fun(x: Int) -> Int = x * 10).filter(fun(y: Int) -> Bool = y > 20))
    print(kept.len())
    print(count(xs.iter().filter(fun(y: Int) -> Bool = y > 3)))
}
```

## std/fmt

String formatting helpers and a `Debug` trait.

- `repeat`, `pad_left`, `pad_right`, `center`, `join(parts, sep)`.
- `to_binary`, `to_octal`, `to_hex`, `to_radix(n, base)`, `pad_int`.
- `Debug` for the primitives, used through `.debug()`.

```raven
import std/fmt { pad_left, to_hex, join }

fun main() {
    print(pad_left("7", 3, "0"))            // 007
    print(to_hex(255))                      // ff
    print(join(["a", "b", "c"], ", "))      // a, b, c
}
```

## std/encoding

Hex and base64.

- `hex_encode(s)`, `hex_decode(s)`.
- `base64_encode(s)`, `base64_decode(s)`.

```raven
import std/encoding { base64_encode, base64_decode }

fun main() {
    print(base64_decode(base64_encode("hi")) == "hi")   // true
}
```

## std/random

A seeded random number generator `Rng`, built with `Rng.new(seed)` or
`Rng.from_entropy()`.

- `next_int`, `gen_range(lo, hi)`, `next_float`, `next_bool`.
- `choice(xs) -> Option<T>`, `shuffle(xs)`.

```raven
import std/random

fun main() {
    let r = Rng.new(42)
    print(r.gen_range(0, 10) < 10)          // true
}
```

## std/env

Process environment, arguments, and platform.

- `get_env(name)`, `get_env_or(name, default)`, `has_env(name)`.
- `arg_count`, `arg_at(i)`, `args() -> List<String>`.
- `exit(code)`, `os_name()`, `arch()`.

```raven
import std/env { os_name, get_env_or }

fun main() {
    print(os_name())
    print(get_env_or("HOME", "unknown"))
}
```

## std/fs

Filesystem access. The IO operations return `Result<_, Error>`.

- `read(path)`, `write(path, contents)`, `append(path, contents)`.
- `remove_file`, `create_dir`, `remove_dir`, `list_dir`, `size`.
- `exists`, `is_file`, `is_dir` return `Bool` directly.
- Path helpers: `join`, `basename`, `dirname`, `split`.

```raven
import std/fs { write, read }

fun main() {
    match write("note.txt", "hello") {
        Ok(_) -> {},
        Err(e) -> print("write failed"),
    }
    match read("note.txt") {
        Ok(s) -> print(s),
        Err(e) -> print("read failed"),
    }
}
```

## std/time

Timestamps and calendar conversions.

- `now() -> Int`, `now_millis()`, `sleep_millis(ms)`.
- `from_timestamp(ts) -> DateTime`, `weekday(ts)`.
- `format_timestamp(ts, pattern)`, `parse_timestamp(text, pattern) -> Result<Int, Error>`.

```raven
import std/time { format_timestamp, now }

fun main() {
    print(format_timestamp(0, "%Y-%m-%d"))
    print(now() > 0)
}
```

## std/net

TCP sockets.

- `connect(addr) -> Result<TcpStream, Error>`, `listen(addr) -> Result<TcpListener, Error>`.
- `TcpListener.accept`, `TcpStream` `read(max)`, `write(data)`,
  `set_read_timeout_ms(ms)`, `close`.
- `dns_lookup(host)`, `reachable(addr)`.

```raven
import std/net { connect }

fun main() {
    match connect("example.com:80") {
        Ok(s) -> s.close(),
        Err(e) -> print("connect failed"),
    }
}
```

## std/http

HTTP client built on `std/net`.

- `get(url)`, `post(url, body)`, `put(url, body)`, `delete(url)`.
- `request(method, url, body, headers)`.
- All return `Result<HttpResponse, Error>`; the response carries
  `status_code` and `body`.

```raven
import std/http { get }

fun main() {
    match get("http://example.com") {
        Ok(resp) -> print(resp.status_code),
        Err(e) -> print("request failed"),
    }
}
```

## std/json

JSON parsing and serialization.

- `parse(text) -> Result<JsonValue, Error>`, `stringify(value) -> String`.
- `JsonValue` accessors: `is_null`, `as_bool`, `as_number`, `as_string`,
  `get(key)`, `at(i)`.

```raven
import std/json { parse, stringify }

fun main() {
    match parse("{\"a\": 1}") {
        Ok(v) -> print(stringify(v)),
        Err(e) -> print("parse failed"),
    }
}
```

## std/regex

Regular expressions. `compile` returns `Result<Regex, Error>`. Free a
compiled pattern with `free` when done.

- `is_match(text)`, `find(text) -> Option<String>`, `find_all(text)`.
- `captures(text)`, `replace_all(text, repl)`, `split(text)`.

```raven
import std/regex { compile }

fun main() {
    match compile("a+") {
        Ok(re) -> {
            print(re.is_match("xaaay"))     // true
            re.free()
        },
        Err(e) -> print("compile failed"),
    }
}
```

## std/process

Run external programs.

- `run(program, args) -> Result<Output, Error>`.
- `run_with_input(program, args, input)`.
- `Output` carries `code`, `stdout`, `stderr`, and `success()`.

```raven
import std/process { run }

fun main() {
    let no_args: List<String> = []
    match run("echo", no_args) {
        Ok(out) -> print(out.code),
        Err(e) -> print("run failed"),
    }
}
```

## std/ffi

Bridge runtime `String` values to C strings for FFI calls.

- `to_cstr(s) -> CStr`, `from_cstr(p) -> String`.

```raven
import std/ffi { to_cstr, from_cstr }

extern "C" {
    fun strlen(s: CStr) -> CSize
}

fun main() {
    print(strlen(to_cstr("hello")))         // 5
}
```

## std/error

The `Error` type and `Result` helpers.

- `error(msg) -> Error`, `error_kind(kind, msg)`.
- `Error` methods: `message`, `kind`, `with_context(ctx)`, `to_string`.
- `is_ok(r)`, `is_err(r)`, `unwrap_or(r, default)`, `ok(r) -> Option<T>`,
  `err(r) -> Option<E>`.

```raven
import std/error { error, unwrap_or }

fun divide(a: Int, b: Int) -> Result<Int, Error> {
    if b == 0 {
        return Err(error("divide by zero"))
    }
    return Ok(a / b)
}

fun main() {
    print(unwrap_or(divide(1, 0), -1))      // -1
}
```

## std/path

Pure path string manipulation (no filesystem access).

- `join(a, b)`, `basename(p)`, `dirname(p)`, `extension(p)`, `stem(p)`,
  `is_absolute(p)`.

```raven
import std/path { join, basename, extension }

fun main() {
    print(join("a/b", "c.txt"))             // a/b/c.txt
    print(extension("a/b/c.txt"))           // txt
}
```

## std/test

Assertions for test programs. A failing assertion panics and aborts with
a non-zero exit.

- `assert(cond)`, `assert_msg(cond, msg)`.
- `assert_true`, `assert_false`.
- `assert_eq_int(a, b)`, `assert_eq_str(a, b)`.

```raven
import std/test { assert, assert_eq_int }

fun main() {
    assert(1 + 1 == 2)
    assert_eq_int(6 * 7, 42)
}
```
