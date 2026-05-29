# std/regex Spec

Regular expressions: compiling a pattern once into a reusable handle, then
testing for a match, finding the first match or all matches, extracting
capture groups, replacing matches, and splitting on a pattern. The
primitives bind the raven-runtime C ABI; the wrappers add the Result/Error
model, the `Option`/`List` return shapes, and the `Regex` handle type in
pure Raven.

## Import

```raven
import std/regex { compile }
```

The methods on `Regex` come in with the type and need no separate selector.

## Backing and syntax flavor

The runtime is backed by the Rust `regex` crate. Its syntax is RE2-style:
linear-time matching with no backreferences and no lookaround (no `\1`
within the pattern, no `(?=...)`/`(?<=...)`). Standard constructs are
supported: character classes, anchors, quantifiers, alternation, groups,
named groups, and Unicode classes. See the regex crate documentation for
the full grammar.

## Handle registry model

A compiled pattern (`regex::Regex`) cannot cross the FFI boundary, so the
runtime keeps it in a process-wide registry keyed by an incrementing `i64`
id and hands Raven only that id. The Raven `Regex { id: Int }` struct wraps
the id. Every match operation looks the pattern up by id and acts on it.
Ids start at 1; an id of 0 is the failure sentinel paired with a set
last-error.

Compiling once and reusing the handle avoids recompiling the pattern per
call and lets a syntax error be reported cleanly at compile time rather
than at each match.

There is no destructor. A caller should call `free(self)` when done with a
pattern to drop its registry entry. Not freeing leaks the entry for the
life of the process; for a short program with a fixed set of patterns the
leak is bounded and harmless, but a long-running program that compiles
patterns dynamically should free them.

## Error model

`compile` returns `Result<Regex, Error>`. The error is an std/error `Error`
tagged with kind `"regex"`, built directly as a struct literal, its message
the underlying regex syntax error. The runtime keeps a thread-local
last-error string set on a failed compile; `raven_regex_compile` returns id
0 on failure, and the wrapper turns an id of 0 into an `Err` carrying
`raven_regex_last_error()`. The match operations are infallible on a valid
handle and do not use the Result model.

## List representation across the FFI

No collections cross the FFI. `find_all`, `captures`, and `split` return
their results as a single String with the pieces joined by `\n`, which the
Raven wrapper splits back into a `List<String>`. An empty runtime result
maps to an empty list (not a one-element list of `""`).

This `\n`-join is ambiguous when a matched substring or a split piece itself
contains a literal newline: such a piece would be read as two list
elements. This is an accepted limitation for v2.0. A pattern that does not
match newline content is unaffected.

## No-match vs matched empty string

`raven_regex_find` returns the first match substring, or `""` when there is
no match. Because a pattern can also match the empty string, the empty
return is ambiguous on its own. The runtime additionally exposes
`raven_regex_has_match(id, text) -> bool`, and `find` uses it to decide:
`Some(s)` when there is a match (`s` may be `""`), `None` when there is no
match. `captures` uses the same flag to return an empty list on no match.

## Capture-group conventions

`captures` returns the groups of the first match, group 0 (the whole match)
first, then groups 1, 2, and so on, joined by `\n`. An unmatched optional
group becomes an empty line (an empty string in the resulting list). When
there is no match the result is an empty list.

## Replacement references

`replace_all(text, repl)` replaces every non-overlapping match with `repl`.
The regex crate honors group references in `repl`: `$1` and `${1}` for
numbered groups and `$name` for named groups. To insert a literal `$`,
escape it per the regex crate's replacement rules.

## Runtime symbols

| Symbol | Returns | Role |
|--------|---------|------|
| `raven_regex_last_error()` | String | Last compile error, empty on success |
| `raven_regex_compile(pattern)` | Int | Compile, return id or 0 on failure |
| `raven_regex_is_match(id, text)` | Bool | Whether the pattern matches anywhere |
| `raven_regex_has_match(id, text)` | Bool | Match flag distinguishing no-match from empty match |
| `raven_regex_find(id, text)` | String | First match, `""` when none |
| `raven_regex_find_all(id, text)` | String | All matches, `\n`-joined |
| `raven_regex_captures(id, text)` | String | Groups of the first match, `\n`-joined |
| `raven_regex_replace_all(id, text, repl)` | String | Replace all matches (honors `$1`/`$name`) |
| `raven_regex_split(id, text)` | String | Split on the pattern, `\n`-joined |
| `raven_regex_free(id)` | Unit | Drop the compiled pattern |

The `regex` crate is added to `raven-runtime`. It is pure Rust with no
system library dependency and builds on Linux, macOS, and Windows.
