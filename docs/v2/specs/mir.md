# MIR (Mid-Level IR) Spec

## Goal

Lower the HIR into an explicit control flow graph of basic blocks with
named locals and a flat list of monomorphic functions. The MIR is the
last form the codegen back-end sees: nested expressions are gone, every
operation reads from a local and writes to a local, every basic block
ends with one terminator, and every generic function appears once per
concrete type argument tuple reachable from the program roots.

Given a `HirProgram` (output of `src/hir`), `mir::lower_program`
returns a `MirProgram` that contains every monomorphic instance of every
reachable function.

## Pipeline position

```
Source -> Lexer -> Parser -> Resolver -> TypeChecker -> HIR -> MIR -> (codegen)
```

MIR runs after HIR. It needs the type checker's `Ty` information that
HIR copies onto every expression so it can specialize generic call
sites. It performs no I/O and reports no errors of its own beyond a
small `MirError` for shapes that earlier passes should already have
rejected.

## MIR types

### Top level

```text
MirProgram { functions: Vec<MirFunction> }

MirFunction {
    name: String,         // mangled monomorphic name
    origin: String,       // source function name, for diagnostics
    params: Vec<MirLocal>,
    ret_ty: MirType,
    locals: Vec<MirLocalDecl>,
    blocks: Vec<MirBlock>,
    entry: MirBlockId,
}

MirBlock {
    id: MirBlockId,
    statements: Vec<MirStatement>,
    terminator: MirTerminator,
}
```

`MirLocal` is a `u32` index into `MirFunction::locals`. `MirBlockId` is
a `u32` index into `MirFunction::blocks`. Both are dense and stable.

### Statements and terminators

```text
MirStatement:
    Assign  { dst: MirLocal, rvalue: MirRvalue }
    StoreField { base: MirOperand, index: usize, value: MirOperand }
    StoreIndex { base: MirOperand, index: MirOperand, value: MirOperand }
    StorageLive(MirLocal)
    StorageDead(MirLocal)
    Nop

MirRvalue:
    Use(MirOperand)
    BinaryOp(BinOp, MirOperand, MirOperand)
    UnaryOp(UnOp, MirOperand)
    Call { callee: FnRef, args: Vec<MirOperand> }
    StructCreate { ty: MirType, fields: Vec<MirOperand> }
    EnumCreate { ty: MirType, variant: usize, payload: Vec<MirOperand> }
    FieldAccess { base: MirOperand, index: usize }
    IndexAccess { base: MirOperand, index: MirOperand }
    ArrayLit { ty: MirType, elements: Vec<MirOperand> }
    Cast { operand: MirOperand, target: MirType }
    ClosureCreate { fn_name: String, captures: Vec<MirOperand> }

MirTerminator:
    Goto(MirBlockId)
    SwitchInt {
        discriminant: MirOperand,
        targets: Vec<(i64, MirBlockId)>,
        otherwise: MirBlockId,
    }
    SwitchEnum {
        discriminant: MirOperand,
        targets: Vec<(usize, MirBlockId)>,
        otherwise: Option<MirBlockId>,
    }
    Return(MirOperand)
    Unreachable

MirOperand:
    Copy(MirLocal)
    Const(MirConstant)

MirConstant:
    Int(i64) | Float(f64) | Bool(bool)
    Str(String) | Char(char) | Unit
```

`FnRef` carries the mangled monomorphic name plus the original
declaration id so duplicate references collapse. `BinOp` and `UnOp`
are simple Copy enums that mirror the HIR operator vocabulary.

### MIR types

`MirType` is a concretized companion to `tycheck::Ty`. After
monomorphization there are no generic parameters left, so the variants
are a strict subset:

```text
MirType:
    Unit | Bool | Int | Float | Char | Str
    Struct { id, args }
    Enum   { id, args }
    Option(Box<MirType>)
    Result(Box<MirType>, Box<MirType>)
    List(Box<MirType>)
    Function { params: Vec<MirType>, ret: Box<MirType> }
```

`Ty::Param` and `Ty::Var` cannot appear in MIR. `Ty::Error` is
flattened to `MirType::Unit` so partial programs still survive lowering
for diagnostics, but a `MirError::UnresolvedType` is emitted on the
side.

## Lowering rules

The CFG builder threads a current block id through every expression.
Calling `emit_statement` appends to the current block; calling
`finish_with(terminator)` closes it and starts a fresh block.

| HIR construct | MIR lowering |
|---------------|--------------|
| Literal       | `Assign { tmp, Use(Const) }` |
| Binary / Unary | Evaluate children into locals, then `Assign { tmp, BinaryOp(...) }` |
| `Ident`       | `Assign { tmp, Use(Copy(local)) }` |
| `Call`        | Lower callee and args to operands, emit `Assign { tmp, Call { callee, args } }` |
| `MethodCall`  | Receiver becomes the first arg, otherwise identical to `Call` |
| `Field`       | `FieldAccess { base, index }` (index resolved from struct decl) |
| `Index`       | `IndexAccess { base, index }` |
| `StructLit`   | Lower fields in declaration order, then `StructCreate` |
| `EnumCreate`-like ctors (`Some`, `None`, `Ok`, `Err`) | `EnumCreate { ty, variant, payload }` |
| `Array`       | Lower elements, then `ArrayLit` |
| `Block`       | Lower stmts in order; the optional tail expression's local is the block's result |
| `If`          | Cond into a local, `SwitchInt` to two arm blocks, each writes to a join local, both `Goto` the continuation block |
| `Match`       | Discriminant into a local, then `SwitchEnum` for enum scrutinees or chained `SwitchInt` for integer literals; arm blocks bind patterns and `Goto` the continuation |
| `Loop`        | Header block contains the body; `break` desugars to `Goto continuation`, `continue` to `Goto header` |
| `While`       | Header block tests cond with `SwitchInt`; true branch is the body and tail-jumps to header; false branch is the continuation |
| `Return(v)`   | Lower `v`, then close the current block with `Return(local)` |
| `Break(v)`    | Optional value written to the enclosing loop's result local, then `Goto continuation` |
| `Continue`    | `Goto header` of the enclosing loop |
| `Interpolate` | Left-folded chain of `__raven_str_concat` calls; each non-String part is first converted with its per-type to-string intrinsic (`__raven_int_to_string`, `__raven_bool_to_string`, `__raven_float_to_string`, `__raven_char_to_string`). Literal-text parts are string constants the back-end promotes to heap Strings. An empty interpolation yields an empty String. |
| `RangeNew`    | `StructCreate` of the built-in `Range` struct (or a runtime call when codegen needs it) |
| `IterNew`     | `Call { callee: __iter_new, args: [source] }` |
| `IterNext`    | `Call { callee: __iter_next, args: [iter] }` |
| `Lambda`      | `ClosureCreate { fn_name, captures }`; the body is lowered as a separate `MirFunction` whose first parameter is the capture struct (capture analysis itself is stubbed to "no captures" until issue #62) |

Pattern binding: in a match arm, the pattern walker emits `Assign`
statements at the start of the arm block that pull payload fields out
of the discriminant via `EnumCreate` projections (or struct field
projections), one per pattern binding name.

`defer e` does not emit at its own site. The lowering records `e` on a
per-function pending stack and re-emits the pending set, in reverse
(LIFO) order, at each block's normal exit and before every `return`. A
`return` escapes all enclosing blocks and flushes the whole stack; a
block's normal exit flushes only the defers registered inside it. See
`docs/v2/specs/defer.md` for the full algorithm and worked examples.

`?` propagation is already desugared to a match in HIR, so MIR has no
special rule for it. Because the desugaring produces an ordinary
`return`, defers flush on the `?` early-return path with no extra work.

## Monomorphization

### Algorithm (pseudocode)

```
roots = [non-generic top-level functions, plus `main` if present]
worklist = roots.map(f -> (f.id, []))
seen = {}
output = []

while let Some(item) = worklist.pop():
    if seen.contains(item): continue
    seen.insert(item)

    let (fn_id, type_args) = item
    let hir_fn = lookup_hir(fn_id)
    let subst = substitution_from(hir_fn.generic_params, type_args)
    let mir_fn = lower_function(hir_fn, &subst)
    output.push(mir_fn)

    // Every call site inside lowered_fn that targets a generic
    // function appends (callee_id, callee_concrete_args) to the
    // worklist if not already seen.

return MirProgram { functions: output }
```

Generic methods on `impl` blocks are looked up by `(self_type, method_name)`
in a table built once at the start of the pass; the lookup result is
the same generic `HirFn`, which the worklist then specializes with the
caller's concrete type arguments.

### Caching

`seen` is a `HashMap<(DeclId, Vec<Ty>), MirFnId>`. Specialization keys
use the canonicalized substituted types so `Foo<Int>` and `Foo<Int>`
collapse regardless of which call site introduced them.

### Naming

Mangled names follow the pattern

```
<source_name>$<typearg_1>$<typearg_2>$...
```

with each type argument rendered through a deterministic
`mangle_ty(Ty)` helper that produces text safe for an identifier
(spaces and punctuation are replaced with `_`). Non-generic functions
keep their source name verbatim. Examples:

```
add                         (non-generic)
identity$Int                (identity::<Int>)
swap$Int$String             (swap::<Int, String>)
Vec_push$Int                (impl Vec<Int>::push)
```

The PR's golden tests pin the exact spelling so future changes are
visible in diffs.

## Out of scope

* Closure capture analysis. Lambdas lower into `ClosureCreate` with an
  empty capture list. Full capture rewriting is tracked by issue #62.
* `dyn Trait` virtual dispatch. Trait method calls are resolved to
  concrete functions during type checking; `dyn` lowering is issue #66.
* Async, generators, drop tracking, borrow analysis.
* Cross-file monomorphization: the v2 pipeline still operates per file.

## Test coverage

* Unit tests in `src/mir/tests.rs` covering: arithmetic, branching,
  loops, return, monomorphization of one generic function with two
  distinct instantiations, struct construction, and enum lowering.
* Golden snapshots in `tests/mir_corpus/*.rv` paired with `.rv.mir`
  baselines. The corpus includes a branchy function, a loop, a
  monomorphic instance of a generic function, and a trait method
  monomorphization.

## Crate layout

```
src/mir/
  mod.rs              MirProgram + lower_program entry
  ty.rs               MirType
  ir.rs               MirFunction, MirBlock, MirStatement, MirTerminator, MirRvalue
  builder.rs          CFG builder used by lowering
  lower/              HIR -> MIR lowering
    mod.rs
    expr.rs
    stmt.rs
    pattern.rs
  mono.rs             Monomorphization driver
  pretty.rs           Stable text dump for snapshot tests
  tests.rs            Inline unit tests
```
