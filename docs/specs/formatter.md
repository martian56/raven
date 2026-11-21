# Formatter

Raven ships a single canonical source formatter, exposed through `rvpm
fmt`. There are no style options. Given any source that parses, the
formatter produces one canonical rendering. The library entry point is
`raven::format::format_source(src: &str) -> Result<String, FormatError>`.

## Guarantees

The formatter is built around two invariants, both enforced by tests:

* Idempotency: `format(format(x)) == format(x)`. Formatting an already
  formatted file is a no-op.
* Semantic preservation: `parse(format(x))` succeeds and yields an AST
  equal to `parse(x)` (ignoring spans). Formatting never changes meaning.

The only failure mode is input that does not lex or parse:
`format_source` returns `FormatError::Parse` with the underlying
diagnostic. The formatter never emits source that fails to re-parse.

## Style rules

### Indentation

Four spaces per level. Tabs are never emitted. Block bodies (functions,
`if`, `while`, `loop`, `for`, `match` arm blocks, lambda blocks, struct
and enum and trait and impl and extern bodies) indent their contents one
level deeper than the opening line.

### Braces

K&R / gofmt style: the opening brace sits on the same line as the
construct that introduces it (`fun`, `if`, `while`, `struct`, `enum`,
`trait`, `impl`, `extern`, `match`, lambda). The closing brace sits on
its own line at the construct's indentation.

An empty body renders inline as `{}` (for example `fun noop() {}`,
`struct Empty {}`, `impl T for U {}`).

### Statements

One statement per line. Raven has no statement terminators, so none are
emitted. `continue`, `break`, and `return` render bare or with their
optional value.

### Blank lines

* Runs of two or more blank lines collapse to a single blank line.
* A blank line that the author placed between two top level items, or
  between two statements, or before a comment that precedes an item, is
  preserved (collapsed to one).
* Leading and trailing blank lines inside a block are removed.
* Members of a `trait` or `impl` are separated by one blank line.
* The file ends with exactly one newline and no trailing blank lines.

### Spacing

* One space around binary operators (`a + b`, `x == y`, `n << 2`).
* No space between a unary operator and its operand (`-x`, `!ok`, `&v`).
* One space after a comma; none before it.
* One space after the `:` in a type annotation and in struct fields
  (`x: Int`, `name: String`); none before it.
* No space before the `(` of a call or the `[` of an index.
* One space around `=` in `let`, `const`, assignment, and single
  expression function and lambda bodies.
* One space around `->` in return types and `=>`-free arrow forms, and
  around `->` in `match` arms.
* Generic argument and parameter lists use `<A, B>` with no inner
  padding. Trait bounds render as `T: Bound1 + Bound2`.

### Trailing whitespace

Removed from every line.

### Constructs

* Functions: `fun name<G>(params) -> Ret { ... }`, or `fun name(...) =
  expr` for single expression bodies. Trait members with no body render
  as the signature alone.
* `struct` and `enum` and trait and extern members render one per line.
  Struct fields and enum variants carry a trailing comma. Enum struct
  payloads use the parenthesized field syntax `Variant(name: Type, ...)`
  that the parser accepts.
* Imports: `import std/io`, `import std/collections { Map, Set }`,
  `import "github.com/user/repo" as alias`. Selector lists render with
  one space of inner padding and comma separation.
* `match` arms render one per line, each ending in a comma:
  `pattern if guard -> body,`.
* Struct literals: shorthand `{ name }` is used when a field's value is
  the identifier of the same name (so `Point { x: x }` becomes
  `Point { x }`). A struct literal that spanned multiple lines in the
  source renders multi line with a trailing comma per field; one that fit
  on a single line renders inline as `Name { a: 1, b: 2 }`.
* Lambdas: the shorthand `{ x -> body }` form renders inline when the
  body is a single expression, and multi line when it contains
  statements. The `fun(params) -> Ret = expr` and `fun(params) { ... }`
  forms render with the same spacing as named functions.
* Ranges render as `start..end` and `start..=end` with no surrounding
  spaces.
* String literals are re-escaped canonically. A `$` immediately before
  `{` inside a plain (non interpolated) string is escaped to `\$` so the
  result does not re-parse as an interpolation. Interpolated strings
  render their embedded expressions with `${ ... }` and the formatter
  recurses into them.
* Float literals always show a decimal point (`3` written where a float
  is expected renders as `3.0`).
* Macros. A `macro name { (matcher) => { template } ... }` definition and a
  `name!(...)` / `name![...]` / `name!{...}` invocation are parsed into
  dedicated AST nodes (the formatter parses un-expanded source, unlike the
  compile pipeline, which expands macros at the token level before parsing).
  A definition with one rule renders on a single line; with several rules
  each gets its own indented line. The matcher, template, and call arguments
  are rendered token by token with canonical spacing: metavariables
  (`$x`, `$x:expr`) and the repetition sigil `$(` stay tight, brackets and
  punctuation follow the same rules as expressions. The result re-lexes to
  the same tokens, so macro formatting is idempotent. Because the rendering
  is token-level (a macro body is not a normal expression), a few constructs
  such as generic angle brackets inside a template may keep surrounding
  spaces; the output stays parseable and stable.

There is no line length limit and no expression wrapping: an expression
renders on one line unless it contains a block bearing sub expression
(`if`, `match`, a block, or a multi line struct literal). This keeps the
formatter simple and idempotent. Authors who want a long call broken
across lines should refactor into intermediate `let` bindings.

## Comments

The lexer discards comments, so the formatter recovers them by rescanning
the source for `//` line comments and `/* ... */` block comments, skipping
string and char literals so a `//` inside a string is not mistaken for a
comment. Each comment is classified as own line (only whitespace precedes
it on its line) or trailing (it follows code).

Comment handling:

* Own line comments are kept on their own line, attached to the item or
  statement that follows them, at that item's indentation. A blank line
  the author left before such a comment is preserved.
* Trailing comments are kept on the same line as the code they follow,
  separated by one space (`let x = 1 // count`).
* Comments inside nested blocks (for example inside an `if` or `while`
  body) are placed at the correct position because block rendering draws
  from the same position ordered comment stream.

Fidelity achieved: full preservation of line and block comments by source
position, for comments that sit at statement or item boundaries or trail a
line. Known limitation: a comment placed in the interior of a single
expression that the formatter renders on one line (for example between two
arguments of a call written across several source lines) is reattached to
the nearest preceding boundary rather than kept mid expression, because
the formatter collapses such expressions onto one line. No comment is ever
dropped.

## `rvpm fmt`

```
rvpm fmt [--check] [paths...]
```

* With no paths, formats every `.rv` file under the project `src/`
  directory, recursively, in place.
* With paths, formats each named file; a directory argument is walked
  recursively for `.rv` files.
* `rvpm fmt` rewrites changed files in place and prints `formatted
  <path>` for each. Files already canonical are left untouched.
* `rvpm fmt --check` writes nothing. It lists every file that is not
  canonically formatted and exits non-zero if any are; it exits zero when
  all inputs are already formatted.
* A file that fails to parse is reported on stderr and causes a non-zero
  exit, without modifying the file.

## Tests

* `src/format/tests.rs`: per construct unit tests, each asserting
  idempotency and AST equality of the formatted output.
* `tests/fmt_golden.rs`: a golden corpus under `tests/fmt_corpus/`
  (input `.rv`, expected `.rv.fmt`) covering every construct, plus an
  idempotency and semantic preservation check over the whole corpus and
  every bundled `stdlib/std/*.rv` file. Refresh baselines with
  `RAVEN_UPDATE_FMT=1 cargo test --test fmt_golden`.
* `tests/rvpm_fmt.rs`: drives the `rvpm` binary for the in place and
  `--check` behaviors.

The bundled standard library parses, formats idempotently, and preserves
its AST under these rules. It is not byte for byte canonical yet (the
formatter canonicalizes struct literal field shorthand, removes leading
blank lines inside blocks, and adds trailing commas after block bodied
match arms). Reformatting the stdlib is left as a separate change to keep
this one focused on the formatter.
