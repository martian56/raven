# Declarative Macros Spec

## Goal

Specify declarative (macro_rules-style) macros for Raven: a `macro`
definition lists one or more rules, each a token matcher and a token
template, and a `name!(...)` invocation in expression position is rewritten
by matching the argument tokens against a rule and splicing the captured
token runs into a copy of the template. Expansion happens at the token
level before the main parse, so a macro can stand in for any expression its
template parses to.

This spec describes the full intended design and marks what the first slice
implements versus what is deferred to follow-ups.

## Pipeline position

```
Source -> Lexer -> [Macro expansion] -> Parser -> Resolver -> Tycheck -> HIR -> MIR -> Codegen
```

Macro expansion is a pass over the token stream produced by the lexer
(`src/lexer/`), run before `parser::parse`. It is a pure function
`expand_tokens(&[Token]) -> Result<Vec<Token>, RavenError>` in
`src/macros/`, exposed as `raven::macros`. The driver
(`src/driver/compile_to_object`) calls it between `tokenize` and `parse`,
and the test harnesses that build programs directly
(`tests/golden.rs`, `tests/codegen_smoke.rs`) call it in the same spot, so
every build path expands macros identically.

`macro` definitions never reach the AST. The expander strips them while
collecting their rules, exactly as imported stdlib modules are merged and
consumed before resolution.

### No-op guarantee

When the source defines no macros, `expand_tokens` returns the input token
stream unchanged (a clone). The pass detects a definition only by the
`macro <ident> {` shape, so a program that contains neither a `macro`
definition nor any `name!(...)` call is byte-for-byte identical before and
after the pass. Non-macro programs are therefore completely unaffected. The
unit test `no_macros_is_a_noop` asserts token equality, and the golden suite
re-runs every existing example through the pass with no baseline change.

## Syntax

### Definition

```
macro <name> {
    (<matcher>) => { <template> }
    (<matcher>) => { <template> }
}
```

`macro` is a contextual identifier, not a reserved keyword: it is treated as
the definition keyword only when immediately followed by `<ident> {`.
Existing programs that use `macro` as an ordinary name elsewhere are
unaffected.

A definition lists one or more rules. Each rule is `(<matcher>) => {
<template> }`. Rules may be separated by newlines, semicolons, or commas.
When a call is expanded, rules are tried in source order and the first one
whose matcher matches wins.

### Invocation

```
<name>!(<tokens>)
```

The `!` distinguishes a macro call from a function call. A macro call is
recognized by the token shape `<ident> ! (`. In ordinary Raven `!` is only a
prefix operator (logical not), so this three-token shape never occurs in a
valid non-macro program, which keeps detection unambiguous.

Macro calls are valid in expression position (including inside a `"${...}"`
string-interpolation fragment), in statement position, and in item position
(top level). Because expansion is a token-level pre-pass that runs before
parsing, a call's template is spliced wherever the call appears, so a
template that parses as one or more items or statements is valid in those
positions:

```raven
macro def_point { () => { struct Point { x: Int, y: Int } } }
macro emit { ($x:expr) => { print($x) } }

def_point!()                 // item position: declares `Point`

fun main() {
    emit!(Point { x: 1, y: 2 }.x)   // statement position
}
```

Hygiene renames a template's own `let` / `const` / `for` binding sites, but
not `fun`, `struct`, or `enum` names, so an item-position macro's
declarations keep the names the template wrote and stay referenceable.

### Metavariables and fragments

A matcher is a sequence of literal tokens and metavariable bindings. A
metavariable is written `$name:fragment`. The supported fragments are:

* `$x:expr` captures a balanced token run up to the next literal token in
  the matcher (the delimiter), or to the end of the argument tokens when no
  delimiter follows. "Balanced" means `()`, `[]`, and `{}` nest, so a
  top-level delimiter inside parentheses does not end the capture.
* `$x:ty` and `$x:pat` capture a balanced token run exactly like `expr`. The
  separate names document intent at the call site (a type such as
  `List<Int>`, or a pattern such as `Some(n)`); the matcher does not parse
  the capture, so any balanced run is accepted.
* `$x:literal` captures exactly one literal token: an integer, float, string,
  block string, character, C string, or `true`/`false`. A non-literal token
  (for example an identifier) fails the rule.
* `$x:block` captures a brace-delimited group `{ ... }`, braces included. The
  capture must start with `{`; the matching `}` ends it.
* `$x:ident` captures exactly one identifier token.

In a template, `$name` splices the captured token run for that
metavariable. All other template tokens are copied verbatim.

Because an `expr` capture is spliced verbatim, a template should wrap each
splice in parentheses where precedence matters, for example `($x) + ($x)`,
so that `twice!(n + 1)` expands to `((n + 1)) + ((n + 1))` rather than
`(n + 1) + (n + 1)` with the wrong grouping intent. This is the caller's
responsibility in this slice; there is no automatic parenthesization.

### Repetition

A repetition group matches and expands a sub-pattern a variable number of
times. The forms are:

```
$( <sub> )<sep>*      zero or more
$( <sub> )<sep>+      one or more
```

`<sep>` is an optional single separator token between repetitions, commonly
`,`. It sits between the closing `)` and the `*` or `+` marker. Omitting it
means the repetitions are adjacent with no separator.

In a matcher, a repetition matches `<sub>` as many times as it can, requiring
the separator between consecutive matches. Every metavariable declared inside
the repetition then binds to a sequence, one captured run per match. With `*`
zero matches are allowed; with `+` at least one match is required, and a call
with too few arguments falls through to the next rule (or, if none matches, is
the usual "no rule matches" error).

In a template, a repetition expands `<sub>` once per captured repetition,
splicing `<sep>` between consecutive expansions. A metavariable used inside a
template repetition must have been captured under a matcher repetition; the
expansion count is taken from that sequence's length.

The binding rule: a metavariable captured under a matcher repetition is a
sequence and may only be spliced inside a template repetition (where it is
viewed one element at a time). A non-repeated metavariable is a single run and
is spliced directly. Using a sequence metavariable outside a template
repetition splices nothing.

Example, a variadic sum that needs no stdlib:

```
macro sum_all { ($($x:expr),*) => { (0 $(+ ($x))*) } }
```

`sum_all!(1, 2, 3)` expands to `(0 + (1) + (2) + (3))` (6), `sum_all!(10)` to
`(0 + (10))` (10), and `sum_all!()` to `(0)` (0). With `+` instead of `*` the
zero-argument call is rejected.

One level of repetition is supported. Nested repetition (a repetition inside a
repetition) is a follow-up.

## Expansion model

1. Scan the token stream for `macro <ident> { ... }` definitions. Record
   each definition's rules and remove the definition tokens from the stream.
   A second definition of the same name is an error.
2. Find each outermost `name!(...)` call. Collect the argument tokens
   between the balanced parentheses. Match them against the rules of `name`
   in order. On the first matching rule, bind each metavariable to its
   captured token run and instantiate the template, splicing the bound runs.
   Replace the call tokens with the instantiated tokens.
3. Repeat step 2 until no `name!(...)` calls remain, so macros that expand to
   other macro calls (or to themselves through another macro) are driven to a
   fixpoint.
4. Parse the resulting token stream normally.

### Spans of expanded tokens

Every token produced by instantiation is given a fresh, unique synthetic
byte range that sits above all real source offsets, while keeping the
line and column of the call site for diagnostics. This matters because the
resolver keys identifier use sites by `(file, start, end)`: without unique
ranges, two spliced uses of the same captured identifier (for example the
two `n`s in `twice!`'s expansion) would collide. Diagnostics on expanded
code therefore point at the call site line, which is the best available
location without span tracking through expansion.

### Expansion limit

Expansion is bounded at 128 passes. Each pass rewrites every outermost call
once, so the bound also caps the depth of macros that expand into further
calls. A macro that expands to a call of itself (for example `macro loopy {
($x:expr) => { loopy!($x) } }`) never reaches a fixpoint and is reported as
an error once the limit is exceeded, rather than looping forever.

## Errors

All macro errors surface during the pre-parse pass and carry a span. The
expander reports:

* an unknown macro: a `name!(...)` call whose `name` has no definition,
* no matching rule: a call whose arguments match none of the macro's rules
  (this covers arity and shape mismatches),
* a malformed definition (missing `=>`, unclosed matcher or template,
  unsupported fragment, no rules),
* a duplicate definition of the same macro name,
* exceeding the expansion limit.

## Hygiene

Basic hygiene protects identifiers that a template introduces at a binding
site. During each expansion, an identifier that immediately follows `let`,
`const`, or `for` in the template (and is a verbatim template token, not a
metavariable splice) is renamed to a fresh, unique name, and every verbatim
use of the same spelling in that template is renamed to match. The fresh name
contains a `$`, which the lexer can never produce in a source identifier, so a
template temporary can never collide with or capture a caller name of the same
spelling.

Example:

```
macro doubled { ($x:expr) => { { let tmp = ($x); tmp + tmp } } }
```

Calling `doubled!(tmp)` where the caller already has a `tmp` in scope is safe:
the captured `$x` keeps the caller's spelling `tmp` (it refers to the caller's
binding), while the template's own `let tmp` becomes a fresh name. The result
reads the caller's `tmp` and the caller's `tmp` is left untouched.

### Boundary

The guarantee is limited to template-introduced binding sites at `let`,
`const`, and `for`. Metavariable-captured tokens always keep their original
identity, so they continue to refer to call-site bindings. This slice does not
provide full referential hygiene: a free identifier the template names (for
example a function it calls) is still resolved at the call site, not at the
macro's definition site, so it can be shadowed by a caller binding of the same
name. Definition-site resolution of free identifiers is a follow-up.

## What is supported

* `macro <name> { (<matcher>) => { <template> } ... }` with one or more
  rules, first matching rule wins.
* `name!(...)` invocation in expression, statement, and item (top-level)
  position.
* `$x:expr`, `$x:ty`, `$x:pat` (balanced capture up to the next matcher
  delimiter), `$x:literal` (single literal token), `$x:block` (a `{ ... }`
  group), and `$x:ident` (single identifier) metavariables.
* Repetition `$( <sub> )<sep>*` and `$( <sub> )<sep>+` (one level) in both
  matchers and templates, with an optional single separator.
* Basic hygiene: template-introduced binding-site identifiers (`let`, `const`,
  `for`) are renamed to fresh names so they cannot capture or be captured.
* Nested and composed macro calls, expanded to a fixpoint under the limit.
* Macro calls inside a `"${...}"` string-interpolation fragment. The fragment
  is lexed during parsing, after the file's main token pre-pass has run, so
  the file's collected macro table is carried into fragment parsing and the
  call expands there the same as anywhere else in expression position.
* Clear, spanned errors for the cases listed above.
* A strict no-op for programs that define no macros.

## Deferred to follow-ups

* Nested repetition (a repetition group inside another).
* Full referential hygiene (definition-site resolution of free identifiers).
* Procedural macros / compile-time functions (the broader procedural side).
```
