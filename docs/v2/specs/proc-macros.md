# Procedural Macros Spec (design)

Design for procedural macros / compile-time functions (#224), the broader
procedural side of the metaprogramming epic (#214/#215). Declarative
(`macro_rules`-style) macros already ship (`docs/v2/specs/macros.md`); this
document specifies the procedural side and, crucially, **chooses the
compile-time execution model**, the one large new subsystem the feature needs.

## Goal

Let a program define a function that runs **at compile time**, takes the tokens
(or source text) of a `name!(...)` call as input, and returns tokens to splice
in its place. Unlike a declarative macro (pattern-and-template token rewriting),
a procedural macro is arbitrary code: it can loop, branch, parse, and build
output programmatically, the general case that `@derive` and declarative macros
are fixed instances of.

## Why, and what already exists

The high-value metaprogramming is already delivered: `@derive` (Eq/Hash/
ToString/Debug), compile-time reflection, and declarative macros with hygiene
and nested repetition. Procedural macros are the **general escape hatch** for
code generation that the fixed mechanisms cannot express (a custom `derive`, a
DSL, a router table from annotations). They are the lowest-priority, highest-cost
item in the epic: every common need is already met, so this is about
completeness, not unblocking a stuck use case.

## The hard part: a compile-time execution model

The v2 compiler is a pure ahead-of-time pipeline (Source -> Lexer -> Macros ->
Parser -> Resolver -> Tycheck -> HIR -> MIR -> Cranelift). Nothing in it runs
Raven code; v1's tree-walking interpreter was deleted at the v2 cutover. A
procedural macro must run user code during compilation, so the feature's core is
a new way to **execute Raven at compile time**. Three approaches were considered.

### Approach A: a compile-time interpreter

Build a tree-walking interpreter for (a subset of) the HIR and run the macro
function over a `TokenStream` value at expand time.

- **Pro:** macros live in the same file; no subprocess; no separate build step;
  works under cross-compilation (the macro runs in the host compiler).
- **Con:** it is a second, parallel implementation of the language's semantics
  (expression/statement evaluation, the value model, every builtin and method
  the macro body uses) that must be built and kept in step with the compiled
  semantics forever. Even a useful subset is large, and a macro can only use the
  subset the interpreter implements. This is the single biggest and most
  open-ended piece of work in the whole v2 effort.

### Approach B: staged compilation (recommended)

Compile the procedural macro with the existing compiler to a native **macro
host** executable, then run it at expand time, passing the call's input and
splicing its output. This is how Rust does it (a proc-macro crate), minus the
dynamic-library loading: Raven communicates with the host over a stdin/stdout
protocol instead.

- **Pro:** reuses the entire existing compiler and runtime, no second semantics
  to build or maintain; a macro is written in the **full** language and runs as
  fast native code; conceptually small new surface (host generation + a
  protocol + wiring).
- **Con:** a procedural macro must live in its own module (so the module can be
  compiled without the unexpanded calls that use it, breaking the circularity);
  a build-time cost to compile the host once per build; and it cannot run when
  cross-compiling to a target the host machine cannot execute (the same
  limitation Rust has, addressed later by building the host for the host triple).

### Approach C: built-in compile-time intrinsics only

Ship a fixed set of compiler-implemented compile-time functions (`stringify!`,
`concat!`, `include_str!`, `env!`, `line!`, `file!`) and no user-defined
procedural macros.

- **Pro:** small, no execution model at all.
- **Con:** does not satisfy #224 (no user-defined macros, no token-building API).
  Useful as a complementary convenience, but not the feature.

### Decision

**Approach B (staged compilation).** It is the only one that does not commit the
project to building and maintaining a second implementation of the language's
semantics, and it gives macros the full language for free. The separate-module
requirement is a reasonable, Rust-like constraint. Approach C's intrinsics may
ship alongside as conveniences but are not the feature.

## Model (staged compilation)

### Defining a procedural macro

A procedural macro is a function in a **procedural-macro module** (a file
declared as such), annotated `@macro`, with the signature `fun(TokenStream) ->
TokenStream` (the first slice may use `String -> String`, raw source text in and
out, deferring a structured `TokenStream` API):

```
// in greet_macros.rv, a procedural-macro module
@macro
fun repeat(input: TokenStream) -> TokenStream {
    // build and return tokens
}
```

### Using it

The consuming program declares the dependency and the macro flows in by name:

```
import macro greet_macros { repeat }

fun main() {
    print(repeat!(3, "hi"))   // expanded by running the compiled macro host
}
```

`name!(...)` keeps the existing call shape; the expander routes the call to a
declarative rule if `name` is a declarative macro, or to the macro host if
`name` is a procedural macro.

### Compilation and expansion flow

1. The driver sees a procedural-macro module is imported. It compiles that module
   with the ordinary pipeline, replacing its entry point with a synthesized
   **dispatch main** (read a macro name plus input from stdin, call the matching
   `@macro` function, write its output to stdout), links it to a host executable,
   and caches it keyed by the module's content hash.
2. During token expansion of the consuming file (the same pass that runs
   declarative macros), each `name!(...)` whose `name` is a procedural macro is
   expanded by sending `(name, input)` to the host and reading back the output,
   which is lexed and spliced in place. Expansion continues to a fixpoint under
   the existing limit, so a procedural macro may emit further macro calls.
3. The expanded token stream is parsed and compiled as usual.

Synthetic spans, the expansion limit, and the no-op guarantee for macro-free
programs all carry over from the declarative expander.

## Implementation slices (sub-issues to file)

1. **Macro host compilation + protocol.** Recognize a procedural-macro module
   and its `@macro` functions, synthesize the dispatch main, compile/link/cache
   the host, and define the stdin/stdout protocol. `String -> String` macros.
2. **Expander wiring.** Route `name!(...)` for a procedural macro to the host and
   splice the lexed result, integrated with the declarative expander, the
   fixpoint loop, spans, and errors (a macro that fails compilation, panics, or
   times out is a spanned diagnostic at the call site).
3. **A `TokenStream` API.** A structured token type and builder/inspection
   methods (and optionally a `quote`-style template) so macros manipulate tokens
   rather than raw strings.
4. **Cross-compilation + built-in intrinsics (optional).** Build the host for the
   host triple when cross-compiling; ship `stringify!`/`concat!`/`include_str!`
   etc. as conveniences.

## Deferred

* Attribute and derive procedural macros (`@my_derive` running user code); this
  spec covers function-like `name!(...)` macros first.
* A structured `TokenStream` API (slice 3) if the first slice ships `String ->
  String`.
* Cross-compilation host building.
