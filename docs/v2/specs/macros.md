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

This slice supports invocation in expression position only.

### Metavariables and fragments

A matcher is a sequence of literal tokens and metavariable bindings. A
metavariable is written `$name:fragment`. This slice supports two fragments:

* `$x:expr` captures a balanced token run up to the next literal token in
  the matcher (the delimiter), or to the end of the argument tokens when no
  delimiter follows. "Balanced" means `()`, `[]`, and `{}` nest, so a
  top-level delimiter inside parentheses does not end the capture.
* `$x:ident` captures exactly one identifier token.

In a template, `$name` splices the captured token run for that
metavariable. All other template tokens are copied verbatim.

Because an `expr` capture is spliced verbatim, a template should wrap each
splice in parentheses where precedence matters, for example `($x) + ($x)`,
so that `twice!(n + 1)` expands to `((n + 1)) + ((n + 1))` rather than
`(n + 1) + (n + 1)` with the wrong grouping intent. This is the caller's
responsibility in this slice; there is no automatic parenthesization.

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

This slice performs plain token substitution and is not hygienic. A name
introduced by a template (for example a `let` binding) and a name in a
captured argument live in the same flat namespace after expansion, so a
template binding can accidentally capture or shadow a caller's name, and a
caller's name can shadow a template's. Macros in this slice should avoid
introducing bindings whose names could collide with caller code. Full
hygiene (renaming template-introduced identifiers so they cannot capture)
is a follow-up.

## What this slice supports

* `macro <name> { (<matcher>) => { <template> } ... }` with one or more
  fixed-arity rules, first matching rule wins.
* `name!(...)` invocation in expression position.
* `$x:expr` (balanced capture up to the next matcher delimiter) and
  `$x:ident` (single identifier) metavariables.
* Nested and composed macro calls, expanded to a fixpoint under the limit.
* Clear, spanned errors for the cases listed above.
* A strict no-op for programs that define no macros.

## Deferred to follow-ups

* Repetition in matchers and templates (`$(...)sep*`).
* Item-position and statement-position invocations.
* Hygiene (avoiding accidental capture).
* Additional fragment kinds (`ty`, `literal`, `pat`, `block`).
* Procedural macros / compile-time functions (the broader procedural side).
```
