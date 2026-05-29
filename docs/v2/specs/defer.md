# defer Spec

## Goal

Specify the `defer` statement: `defer expr` schedules `expr` to run when
the enclosing function returns. Deferred work runs in reverse declaration
order (LIFO), runs on every normal return path the deferred statement was
reached through, and runs before the function hands control back to its
caller. This is the v2.0 counterpart to Go's `defer`, scoped to fit a
control-flow-graph back end that does not unwind.

## Pipeline position

```
Source -> Lexer -> Parser -> Resolver -> Tycheck -> HIR -> MIR -> Codegen
```

* The parser already accepts `defer expr` as `StmtKind::Defer(Expr)`.
* The type checker checks the deferred expression like any expression and
  discards its type (the result is evaluated for side effects only).
* HIR keeps the statement as `HirStmtKind::Defer(HirExpr)`.
* MIR lowering performs the expansion: it records each deferred expression
  and re-emits the pending set, in reverse order, at every block exit and
  at every `return`.
* Codegen needs no defer-specific logic. The deferred calls are ordinary
  MIR statements that land in the block before its `Return` terminator,
  so they are emitted before the GC leave-frame epilogue automatically.

## Semantics

* **LIFO.** Within a return path, deferred expressions run in the reverse
  of the order they were declared. `defer A; defer B; return` runs `B`
  then `A`.
* **Reached-only.** A `defer` schedules its expression only if control
  actually reached the `defer` statement at run time. A `defer` placed
  after an early `return` that was taken never runs, because that line was
  never executed.
* **Return value first.** On `return e`, the return value `e` is evaluated
  first, then the deferred expressions run, then the function returns.
  This matches Go: defers observe the already-computed result, not a
  partially evaluated one.
* **Block-scoped, function-scoped in practice.** A defer runs when the
  block that registered it exits, by normal fall-through or by a `return`
  escaping through it. A defer written at the function-body level (the
  common case, and where Go programs put defers) therefore runs at every
  function return, indistinguishable from Go's function scope. A defer
  nested inside an inner block runs at that inner block's exit. See
  Scope below for the precise rule and the rationale.
* **No run on panic.** Raven `panic` aborts the process in v2.0; there is
  no stack unwinding. Deferred expressions do not run on a panic, exactly
  as a process that calls `abort()` runs no cleanup. This is intentional
  and documented, not a gap to be patched with unwinding.
* **Loops.** A `defer` inside a loop body runs when the body block exits
  (each iteration), not at loop teardown, because the body is its
  enclosing block. A `break` or `continue` is a control-flow edge out of
  the body block and triggers the body's pending defers the same way a
  fall-through does. Defers do not accumulate across iterations.

## Lowering strategy

The chosen strategy is **compile-time expansion at MIR lowering** with a
per-function pending-defer stack. No runtime defer stack, no heap thunks,
no closures-as-defer-bodies. The alternative, a runtime per-frame stack of
`(fn_ptr, env)` thunks, was rejected for v2.0: it needs runtime support
and closure capture (issue #62), and the compile-time expansion already
covers every required control-flow case correctly.

### The pending stack

`mir::lower::LowerCx` carries `defers: Vec<HirExpr>`, the deferred
expressions registered so far in declaration order.

* Lowering `HirStmtKind::Defer(e)` pushes a clone of `e` onto `defers`.
  Nothing is emitted at the `defer` site.
* Lowering a block records `mark = defers.len()` on entry. On the block's
  normal exit (after its tail expression, when control has not diverged)
  it emits `defers[mark..]` in reverse order as ordinary side-effecting
  expressions, then truncates `defers` back to `mark` so an enclosing
  block does not re-run them.
* Lowering `HirExprKind::Return(value)` lowers `value` first, then emits
  the entire pending `defers` stack in reverse order (a `return` escapes
  all enclosing blocks at once), then closes the block with the `Return`
  terminator. The stack is cloned, not consumed, so a return that escapes
  several blocks emits each pending defer exactly once on its own path.
* When a block has already diverged (a `return`, `break`, or `continue`
  closed it and rolled a fresh dead block), the normal-exit flush is
  skipped: the escaping statement already emitted what it needed, and the
  dead block must stay empty.

### Worked example

```
fun f(early: Bool) -> Int {
    defer print(9)      // push 9         -> defers = [9]
    if early {
        return 1            // emit [9] reversed -> print 9; return 1
    }
    defer print(8)      // push 8         -> defers = [9, 8]
    return 2                // emit [9, 8] reversed -> print 8, print 9; return 2
}
```

The MIR has two return-bearing blocks. The early-return block contains
`print(9); return 1`. The fall-through block contains
`print(8); print(9); return 2`. So `f(true)` prints `9` and
`f(false)` prints `8` then `9`. The corresponding executable confirms
this on a real run.

For the classic ordering case:

```
fun demo() -> Int {
    defer print(1)      // defers = [1]
    defer print(2)      // defers = [1, 2]
    return 0                // emit reversed: print 2, print 1; return 0
}
```

The single return block is `print(2); print(1); return 0`, so the
program prints `2` then `1`.

## Interaction with `?`

The `?` operator is already desugared in HIR to a `match` whose error arm
is `return Err(__e)` (or `return None` for `Option`). That synthesized
`return` is an ordinary `HirExprKind::Return`, so it flushes the pending
defers exactly like a written `return`. A defer declared before a `?` runs
when the `?` takes its early-return arm; a defer declared after it does
not, because the `?` return was reached first. No `?`-specific code is
needed.

## Interaction with the GC leave-frame

Codegen maintains a GC root frame for any function with GC pointer locals
and emits `raven_gc_leave_frame` inside the `Return` terminator, after
evaluating the return operand. Deferred expressions are lowered as MIR
statements that precede the `Return` terminator, so codegen emits them
before `raven_gc_leave_frame`. Deferred code may therefore read and write
GC-managed locals safely: the locals are still rooted when the deferred
calls run. This ordering is verified by an example whose deferred call
reads a heap-allocated struct local and prints its field.

## Scope rationale

True Go function-scope with reached-only semantics for defers nested in
conditional sub-blocks needs runtime state, because a compile-time pass
cannot know at the merge point whether a conditional branch executed. The
block-scoped rule sidesteps this: every defer has a statically known exit
point (the end of its block, plus any `return` escaping it), so reached-
only is exact without runtime bookkeeping. For function-body-level defers,
the enclosing block is the function body, so the rule coincides with Go's
function scope. This keeps the required cases correct and the
implementation runtime-free.

## Out of scope

* Running defers on panic or any form of unwinding. v2.0 panic aborts.
* A runtime defer stack and deferred closures with captured environments.
* Deferring a `return` value rewrite (Go's named-return mutation from a
  defer). Deferred expressions are evaluated for side effects only.
* Defer inside generic functions interacts with monomorphization only
  through ordinary expression lowering; no defer-specific specialization
  is needed.
```
