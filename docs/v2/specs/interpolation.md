# String Interpolation Spec

## Goal

Specify string interpolation: a `"..."` literal may embed expressions
with `${expr}`, and the result is a `String` built by converting each
embedded value to text and concatenating the pieces. `print("sum is
${a + b}")` prints `sum is 7` when `a + b` is `7`. This carries
interpolation end to end, from the lexer through the runtime, for the
value types that have a known text rendering.

## Pipeline position

```
Source -> Lexer -> Parser -> Resolver -> Tycheck -> HIR -> MIR -> Codegen -> Runtime
```

* The lexer decodes escapes inside the literal and marks an escaped `\$`
  so the splitter can tell it apart from a real `${`.
* The parser splits the decoded literal into fragments, re-parsing each
  `${expr}` as a real expression.
* The resolver and type checker treat each embedded expression normally.
* HIR keeps the structured `Interpolate` node carrying typed fragments.
* MIR desugars the node into a concat chain with per-type conversions.
* Codegen lowers those to runtime calls returning a heap `String`.

## Syntax

```
"text ${expr} more text"
```

* `${` opens an interpolation; the matching `}` closes it. Braces nest,
  so `${ f({ a: 1 }) }` finds the correct closing brace by depth.
* The text between `${` and `}` is parsed as an ordinary Raven
  expression. Exactly one expression is allowed per `${...}`.
* A literal with no `${...}` is an ordinary string and keeps the plain
  `Str` representation.

### Escaping

* `\$` is a literal dollar sign. `"\${x}"` is the five-character text
  `${x}`, not an interpolation.
* All other string escapes (`\n`, `\t`, `\"`, `\\`, `\0`, `\'`) decode as
  usual before splitting.
* **Block strings are never interpolated.** A triple-quoted `"""..."""`
  literal is raw text; a `${...}` inside it is literal characters.

## AST

The parser produces:

```rust
enum ExprKind {
    // ...
    InterpolatedString(Vec<StrFragment>),
}

enum StrFragment {
    Literal(String),    // a run of literal text, escapes already decoded
    Expr(Box<Expr>),    // one embedded expression, fully parsed
}
```

A literal that turns out to have no real `${...}` (for example only an
escaped `\$`) collapses back to a plain `ExprKind::Str`, so
`InterpolatedString` always carries at least one embedded expression.

## Lexer and parser split

The lexer decodes the literal into a single `StringLit` token. An
escaped `\$` is decoded to a private-use sentinel (`U+E000`) followed by
`$`, so the parser can distinguish an escaped dollar from a real `${`
even though both look like `$` after decoding.

The parser walks the decoded text:

* A sentinel-plus-`$` becomes a literal `$` in the current text run (the
  sentinel is dropped).
* A real `${` starts a fragment. The parser scans to the depth-matched
  `}`, takes the inner snippet, and re-lexes and re-parses it as a
  standalone expression. The snippet is parsed against a synthetic
  per-fragment source path so that the resolver, type checker, and
  lowering passes (all keyed on file plus byte range) give each
  fragment's spans a private keyspace that never collides with the
  surrounding source or with another fragment.
* Anything else accumulates into the current literal text run.

An unterminated `${` or an empty `${}` is a parse error.

## Type rules

An interpolated string has type `String`. Each embedded expression must
have a type with a text rendering. The built-in scalars render through a
dedicated per-type conversion:

| Embedded type | Conversion |
|---------------|------------|
| `String`      | identity (used as-is) |
| `Int`         | base-ten, with a leading `-` for negatives |
| `Bool`        | `true` or `false` |
| `Float`       | default `{}` rendering (so `7.0` renders `7`) |
| `Char`        | the single Unicode scalar value |

Any other embedded type renders through its `ToString` impl. This covers
a user type that implements `ToString` (`${point}`) and a generic
parameter bounded by `ToString` (`fun show<T: ToString>(x: T) = "${x}"`),
where the bound supplies `to_string` and each monomorphization picks the
concrete impl. A type that is neither a built-in scalar nor a `ToString`
implementor is a type error with a hint to implement `ToString` or
convert to a `String` first. The type checker records each fragment
expression's resolved type so the lowering pass can pick the right
conversion.

## HIR desugaring

HIR lowering walks the AST fragments and produces a structured
`Interpolate { parts: Vec<InterpolPart> }` node. A `Literal` fragment
becomes `Text(String)`; an `Expr` fragment is lowered like any other
expression into `Expr(HirExpr)`, carrying its static type. A part whose
static type is a built-in scalar (or `String`) is kept as-is for the
per-type fast path. A part of any other type (a generic `T: ToString` or
a user type with a `ToString` impl) is rewritten into a `to_string()`
method call during HIR lowering, so it arrives at MIR already typed
`String`; that call routes through the ordinary bound/method dispatch and
monomorphizes per call site. HIR keeps the structured form so the concat
strategy stays a lowering concern.

MIR lowering performs the desugaring. It turns each part into a heap
`String` operand and folds them left to right:

```
["sum is ", a + b, "!"]
  becomes
concat(concat("sum is ", int_to_string(a + b)), "!")
```

A `String` part needs no conversion. An `Int`, `Bool`, `Float`, or
`Char` part is routed through its per-type to-string intrinsic. A part of
any other type is already a `to_string()` call result (a `String`) from
HIR lowering, so it too needs no conversion here. Literal text parts are
string constants the back-end promotes to heap Strings.
An empty interpolation yields an empty `String`; a single part binds
straight to a result temporary.

The intrinsic mangled names are `__raven_str_concat`,
`__raven_int_to_string`, `__raven_bool_to_string`,
`__raven_float_to_string`, and `__raven_char_to_string`. They live in
`mir::intrinsics` so the lowering and the back-end share one source of
truth.

## Runtime

The runtime exposes C-ABI functions that allocate GC-managed `String`
objects through the existing `raven_string_new` constructor (tag
`TAG_STRING`), so every result is a real heap value the collector
traces:

* `raven_string_from_bytes(ptr, len) -> *mut String`: copy a byte slice
  into a fresh `String`. Used to promote a literal into a heap value.
* `raven_string_concat(a, b) -> *mut String`: allocate a `String` whose
  bytes are `a` then `b`. Either input may be null (treated as empty).
* `raven_int_to_string(i64)`, `raven_bool_to_string(i8)`,
  `raven_float_to_string(f64)`, `raven_char_to_string(u32)`: render a
  scalar into a fresh `String`.

## Codegen and generalized print

A string constant used as a value (assigned, passed, concatenated,
interpolated) is promoted to a heap `String` via
`raven_string_from_bytes`, so every `Str`-typed local is a GC pointer
the root frame traces. The interpolation intrinsics are recognized by
mangled name and routed to their runtime symbols.

The `print` intrinsic accepts both shapes of string. A bare string
literal keeps the allocation-free static fast path: it passes the
interned bytes and the compile-time length straight to
`raven_println_str`. Any other `String` value (a `let`-bound string, a
returned string, an interpolation result) is a heap `String` pointer, so
the codegen reads the byte buffer pointer and length through
`raven_string_bytes` and `raven_string_len` before calling
`raven_println_str`. Both `print("literal")` and `print("x is ${x}")`
work.

## Out of scope

* Interpolating an enum, list, map, or function value that does not
  implement `ToString`. A type with a `ToString` impl interpolates; one
  without still raises a type error.
* Format specifiers or alignment (`${x:04}` style). The `${...}` body is
  a plain expression with no formatting mini-language.
* Reusing one parsed snippet across passes by source byte offset. The
  synthetic per-fragment span keyspace is internal and is not surfaced
  in diagnostics; errors inside a snippet are re-anchored to the
  enclosing string literal.
