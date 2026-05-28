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
| `ExprKind::For { pat, iter, body }` | Lowered to `Loop` containing a `Match` on iterator `next` |
| `ExprKind::Try(inner)`            | Lowered to `Match` on `Ok/Err` (or `Some/None`)         |
| `ExprKind::Str(s)` with `${...}`  | `Interpolate { parts: Vec<InterpolPart> }`              |
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

```
for pat in iter { body }
```

becomes a `loop` over an iterator handle:

```
let __iter = IterNew(iter);
loop {
    match IterNext(__iter) {
        Some(pat) => body,
        None      => break,
    }
}
```

`IterNew` and `IterNext` are HIR built-ins. The type checker already
accepts `for` only over `List<T>`. For `List<T>` they map to a hidden
index counter; for `Range`/`RangeInclusive` (which the type checker
currently shapes as `List<Int>`) they map to integer increments. User
defined iterators are a follow up: a future PR adds the `Iterator`
trait and threads it through the lowering.

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

The lexer keeps `${...}` verbatim inside a `StringLit`. HIR lowering
splits the raw string into a `Vec<InterpolPart>` where each part is
either a `Text(String)` chunk or an `Expr(HirExpr)` placeholder. The
embedded expressions are re-lexed and re-parsed by the lowering pass
so the resulting HIR expression mirrors what the user wrote.

Concatenation is left to MIR/codegen so HIR retains the structured
form. When no `${...}` segment is present, the literal stays as a
plain `HirExpr::Str(s)`.

### Range expressions

```
a..b       becomes RangeNew { start: a, end: b, inclusive: false }
a..=b      becomes RangeNew { start: a, end: b, inclusive: true  }
```

`Range` is treated as a built-in iterable in HIR. Its element type is
`Int` (the type checker enforces integer bounds).

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

* User-defined `Iterator` trait protocol. Built-in iteration over
  `List<T>` and ranges only.
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
