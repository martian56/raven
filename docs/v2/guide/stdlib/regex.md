# std/regex

Regular expressions: compile a pattern once into a reusable handle, then test
for a match, find the first match or every match, pull out capture groups,
replace matches, and split on a pattern.

```raven
import std/regex { compile }

fun main() {
    match compile("[0-9]+") {
        Ok(re) -> {
            print(re.is_match("order 42"))      // true
            re.free()
        }
        Err(e) -> print(e.message),
    }
}
```

## Importing

```raven
import std/regex { compile }
```

`compile` is the entry point. The methods on `Regex` arrive with the type, so
they need no separate selector.

## Compiling a pattern

A pattern is compiled once into a `Regex` handle, then reused for every match
operation. Compiling is the only fallible step: an invalid pattern is reported
here rather than at each match.

### `compile(pattern: String) -> Result<Regex, Error>`

Compile `pattern` into a reusable `Regex`. On success the result is
`Ok(Regex)`. An invalid pattern is `Err(Error)` from [std/error](error.md),
tagged with kind `"regex"`, its message the underlying syntax error.

```raven
import std/regex { compile }

fun main() {
    let result = compile("(")       // unbalanced group
    match result {
        Ok(re) -> re.free(),
        Err(e) -> print(e.message),     // a regex syntax error
    }
}
```

## Freeing a handle

A compiled `Regex` holds a runtime handle that lives outside the garbage
collector: the runtime keeps the compiled pattern in a process-wide registry
and hands Raven an opaque id. There is no destructor, so call `free()` when you
are done with a pattern. Not freeing leaks the registry entry for the life of
the process.

For a short program with a fixed set of patterns the leak is bounded and
harmless, but a long-running program that compiles patterns dynamically should
free each one once it is no longer needed.

### `free(self)`

Release the compiled pattern and drop its registry entry. Do not call any other
method on the handle afterward.

## Matching

### `is_match(self, text: String) -> Bool`

True when the pattern matches anywhere in `text`.

```raven
import std/regex { compile }

fun main() {
    match compile("^[a-z]+$") {
        Ok(re) -> {
            print(re.is_match("raven"))     // true
            print(re.is_match("R2D2"))      // false
            re.free()
        }
        Err(e) -> print(e.message),
    }
}
```

### `find(self, text: String) -> Option<String>`

The first match in `text`, or `None` when there is no match. A matched empty
string is `Some("")`, distinct from `None`.

```raven
import std/regex { compile }

fun main() {
    match compile("[0-9]+") {
        Ok(re) -> {
            match re.find("room 237, floor 4") {
                Some(s) -> print(s),        // 237
                None -> print("no digits"),
            }
            re.free()
        }
        Err(e) -> print(e.message),
    }
}
```

### `find_all(self, text: String) -> List<String>`

Every non-overlapping match in `text`, in order. No matches yields an empty
list.

```raven
import std/regex { compile }

fun main() {
    match compile("[0-9]+") {
        Ok(re) -> {
            for n in re.find_all("a1 b22 c333") {
                print(n)        // 1, then 22, then 333
            }
            re.free()
        }
        Err(e) -> print(e.message),
    }
}
```

## Capture groups

### `captures(self, text: String) -> List<String>`

The capture groups of the first match: group 0 (the whole match) first, then
groups 1, 2, and so on. An unmatched optional group is an empty string. No
match yields an empty list.

```raven
import std/regex { compile }

fun main() {
    match compile("([0-9]+)-([0-9]+)") {
        Ok(re) -> {
            let groups = re.captures("range 10-20 here")
            print(groups[0])        // 10-20  (whole match)
            print(groups[1])        // 10
            print(groups[2])        // 20
            re.free()
        }
        Err(e) -> print(e.message),
    }
}
```

## Replacing

### `replace_all(self, text: String, repl: String) -> String`

Replace every non-overlapping match in `text` with `repl`. Group references in
`repl` are honored: `$1` and `${1}` for numbered groups, `$name` for named
groups.

```raven
import std/regex { compile }

fun main() {
    match compile("([a-z]+)@([a-z]+)") {
        Ok(re) -> {
            let masked = re.replace_all("ada@host bob@host", "$1@***")
            print(masked)       // ada@*** bob@***
            re.free()
        }
        Err(e) -> print(e.message),
    }
}
```

## Splitting

### `split(self, text: String) -> List<String>`

Split `text` on the pattern, returning the pieces in order.

```raven
import std/regex { compile }

fun main() {
    match compile("[, ]+") {     // commas and spaces, run together
        Ok(re) -> {
            for part in re.split("a, b,  c   d") {
                print(part)     // a, b, c, d
            }
            re.free()
        }
        Err(e) -> print(e.message),
    }
}
```

## Supported syntax

The engine is RE2-style, with linear-time matching and **no** backreferences
and **no** lookaround (no `\1` inside the pattern, no `(?=...)` or `(?<=...)`).
The usual constructs are supported: character classes, anchors, quantifiers,
alternation, groups, named groups, and Unicode classes.

## A note on newlines in results

`find_all`, `captures`, and `split` return their pieces as a `List<String>`.
Internally the pieces are joined on `\n` to cross the runtime boundary, so a
matched substring or split piece that itself contains a literal newline reads
back as two list elements. A pattern that does not match newline content is
unaffected.

## Worked example: parsing key=value lines

```raven
import std/regex { compile }

fun main() {
    let lines = ["host=localhost", "port=8080", "debug=true"]

    // Match the value that follows `=` on each line.
    match compile("=([a-z0-9]+)") {
        Ok(re) -> {
            for line in lines {
                match re.find(line) {
                    Some(hit) -> print("${line} -> ${hit}"),    // ... -> =localhost, ...
                    None -> print("skip: ${line}"),
                }
            }
            re.free()
        }
        Err(e) -> print("bad pattern: ${e.message}"),
    }
}
```

Output (`find` returns the whole match, so the `=` is included):

```
host=localhost -> =localhost
port=8080 -> =8080
debug=true -> =true
```

## See also

- [std/string](string.md) for literal substring search and replace when you do
  not need a full pattern.
- [std/error](error.md) for the `Error` type that `compile` returns on failure.
