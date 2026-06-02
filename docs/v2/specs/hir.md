# HIR (High-Level IR) Spec

## Goal

Lower the surface AST into a smaller, desugared tree that downstream
passes (MIR, codegen) can consume without re-implementing surface
syntax. The HIR keeps the program's meaning intact; it strips sugar.

Given a `TypedFile` (output of `src/tycheck`), `hir::lower_file` returns
a `HirProgram` that pairs each top-level item with its lowered body and
preserves enough type information for MIR to operate.

## Pipeline position

```
Source -> Lexer -> Parser -> Resolver -> TypeChecker -> HIR -> (MIR -> codegen)
```

The HIR pass runs after type checking and consumes the `TypeMap`. It
performs no I/O and reports no errors of its own beyond an internal
`HirError` for shapes the type checker should already have rejected.

## What HIR is

A simplified expression and statement tree where:

* Sugar (`for`, `?`, interpolation, ranges, compound assignment, single
  expression function bodies) is rewritten into core constructs.
* Blocks always evaluate to their last expression, or to unit.
* `if` and `match` in value position are lowered the same way as in
  statement position; the difference disappears at HIR.
* Patterns are simplified: nested-or patterns are not yet supported and
  literal range patterns are kept as-is.
* Every node still carries the original source `Span` so diagnostics
  in later passes anchor to the user's code.

HIR reuses `tycheck::Ty` directly as its type representation: lowering
copies the inferred type out of the `TypeMap` per expression and stores
it on the HIR node. Introducing a separate HIR-level type would cost a
translation pass that buys nothing for the desugarings in scope.

## HIR shape vs AST

| AST node                          | HIR equivalent                                          |
|-----------------------------------|---------------------------------------------------------|
| `ExprKind::For { pat, iter, body }` | Lowered to a counter `Loop` over a range, or an index `Loop` over a `List<T>` |
| `ExprKind::Try(inner)`            | Lowered to `Match` on `Ok/Err` (or `Some/None`)         |
| `ExprKind::InterpolatedString(fragments)` | `Interpolate { parts: Vec<InterpolPart> }`      |
| `ExprKind::Range { ... }`         | `RangeNew { start, end, inclusive }`                    |
| `StmtKind::Assign { op != = }`    | Lowered to `Assign` with a synthesized RHS              |
| `ExprKind::If`/`Match` in value position | Same node; trailing expression is the value         |
| `Block` with trailing expr        | Always: `HirBlock { stmts, tail }`                      |
| Single-expression function body   | A block with one trailing expression                    |
| `T?` (Optional sugar)             | Already resolved to `Option<T>` by the type checker     |

## Desugaring rules

Each rule is stated as a textual rewrite. Spans on synthesized nodes
clone the originating span so error messages still point at the user's
source range.

### `for x in iter` loops

The type checker accepts `for` only over a `List<T>` value or an integer
range (which it shapes as `List<Int>`). HIR lowers each of these two
built-in iterable forms directly to a concrete counter loop, so no
iterator object, `RangeNew`, or `IterNext` reaches MIR or codegen.

A range loop over `start..end` (exclusive) or `start..=end` (inclusive)
lowers to a counter over the integer interval. The endpoints are each
evaluated once into a local:

```
for x in start..end { body }
```

becomes:

```
let __end = end;
let __i = start;
let __first = true;
loop {
    if __first { __first = false } else { __i = __i + 1 }
    if __i >= __end { break }   // `>` for the inclusive form
    let x = __i;
    body
}
```

A list loop over a `List<T>` value lowers to an index loop. The list
expression is evaluated once into a local, and the per-iteration binding
reads `__list[__i]` using the built-in list length and indexing:

```
for x in list { body }
```

becomes:

```
let __list = list;
let __end = __list.len();
let __i = 0;
let __first = true;
loop {
    if __first { __first = false } else { __i = __i + 1 }
    if __i >= __end { break }
    let x = __list[__i];
    body
}
```

The increment sits at the top of the loop body, guarded by a `__first`
flag that skips it on the first pass so the counter starts at the lower
bound. This placement is what makes `break` and `continue` behave: a
user `break` exits to the loop continuation as usual, and a user
`continue` re-enters the loop header, which is the increment-and-test
step, so it always advances the counter before the next iteration rather
than skipping it (which would loop forever). The body is the trailing
statement of the loop, after the binding.

Only a plain binding pattern (`for x in ...`) is bound by name. Richer
destructuring patterns are type-checked upstream and reuse the same
`let pat = element` machinery; a future change can expand them in place.

Iteration over any other type is rejected by the type checker with a
clear diagnostic before lowering runs, so no codegen-time failure is
possible. User-defined iterators (`for x in <any Iterator>`) are out of
scope here and are generalized by the `Iterator` trait work in issue
#119, which will thread a trait-based protocol through this lowering.

### `?` operator

```
expr?      where expr: Result<T, E>
```

becomes:

```
match expr {
    Ok(__v)  => __v,
    Err(__e) => return Err(__e),
}
```

For `Option<T>`, the desugaring uses `Some/None`. HIR lowering reads
the type of `expr` from the `TypeMap` and selects the right enum.

### String interpolation

The parser splits a `"...${expr}..."` literal into an
`ExprKind::InterpolatedString(Vec<StrFragment>)` where each fragment is
either literal `Text` or a fully parsed embedded `Expr`. HIR lowering
walks those fragments and produces a `Interpolate { parts:
Vec<InterpolPart> }` node: literal fragments become `Text(String)`
chunks and embedded fragments become `Expr(HirExpr)` (lowered like any
other expression, carrying their static type).

Concatenation is left to MIR so HIR retains the structured form. MIR
folds the parts into a chain of runtime string-concat calls, inserting
a per-type to-string conversion for each non-String part. A literal
with no real `${...}` (or only an escaped `\$`) stays a plain
`HirExpr::Str(s)`. See `docs/v2/specs/interpolation.md` for the full
pipeline.

### Range expressions

```
a..b       becomes RangeNew { start: a, end: b, inclusive: false }
a..=b      becomes RangeNew { start: a, end: b, inclusive: true  }
```

A bare range expression (one not in the `iter` position of a `for`)
lowers to `RangeNew`. Its element type is `Int` (the type checker
enforces integer bounds). A range used directly as a `for` loop's
iterable never produces a `RangeNew`: the for-loop lowering reads the
range endpoints straight off the AST and emits the counter loop above,
so `RangeNew` carries no iteration responsibility.

### Enum variant construction

A user enum variant is built in expression position with a qualified
name:

```
EnumName.Variant          unit variant
EnumName.Variant(args)    payload variant
```

Both forms lower to `EnumCreate { variant, args }`, where `variant` is
the variant's index in declaration order (the same index patterns and
codegen use) and the node's `ty` is the enum type with its concrete type
arguments. The built-in `Some`, `Ok`, `Err`, and `None` keep their own
constructor nodes (`SomeCtor`, `OkCtor`, `ErrCtor`, `NoneCtor`), which
MIR lowers to the same `EnumCreate` rvalue.

Bare-name construction (`Red`, `Circle(2.0)`) is not supported yet; it
needs expected-type disambiguation and is a follow-up. Match patterns
are unchanged and continue to use bare variant names (`match c { Red ->
... }`).

### Compound assignment

```
x += y     becomes x = x + y
```

For non-identifier targets, the LHS is evaluated once:

```
obj.field += y
```

becomes:

```
let __recv = obj;
__recv.field = __recv.field + y;
```

And for indexed targets:

```
arr[i] += y
```

becomes:

```
let __arr = arr;
let __idx = i;
__arr[__idx] = __arr[__idx] + y;
```

The same rewrite covers `-=`, `*=`, `/=`, `%=`, `&=`, `|=`, `^=`,
`<<=`, `>>=`.

### `if` as expression

```
let r = if cond { a } else { b };
```

is identical in HIR to the statement form. The HIR `If` node always
has both branches as blocks ending in a trailing expression when used
in value position; the type checker has already verified the branches
agree.

### `match` as expression

Same approach as `if`: the trailing-expression form carries the value.
Arms are simplified to a flat list. Or-patterns and nested guards stay
out of scope for this pass and are rejected upstream by the type
checker today.

### Single-expression function bodies

```
fun add(a: Int, b: Int) -> Int = a + b
```

is lowered to:

```
fun add(a: Int, b: Int) -> Int { a + b }
```

so MIR sees only block bodies.

### `T?` optional sugar

The type checker already resolves `T?` into `Option<T>`. HIR preserves
the result type; the surface form does not appear at HIR level.

## What HIR preserves from the AST

* Original spans on every node.
* Inferred types, looked up from the `TypeMap` and stored on each
  expression.
* Item ordering (functions, structs, traits, impls).
* Identifier bindings, looked up through the resolver as needed.

## Out of scope

* User-defined `Iterator` trait protocol (issue #119). Built-in
  iteration over `List<T>` and integer ranges only, lowered to concrete
  counter and index loops.
* Closure capture analysis (defer to MIR).
* C FFI lowering.
* `dyn Trait` virtual dispatch.
* Or-patterns and nested or-pattern lowering.

## Crate layout

```
src/hir/
  mod.rs          # HirProgram, lower_file entry point
  ty.rs           # Re-export of tycheck::Ty for convenience
  expr.rs         # HirExpr enum
  stmt.rs         # HirStmt enum
  pattern.rs      # HirPattern
  decl.rs         # HirItem and friends
  pretty.rs       # Stable text dump for snapshot testing
  lower/          # The lowering pass itself
    mod.rs
    expr.rs
    stmt.rs
    pattern.rs
    sugar.rs      # Desugaring helpers
  tests.rs
```
