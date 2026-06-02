# defer Spec

## Goal

Specify the `defer` statement: `defer expr` schedules `expr` to run when
the enclosing function returns. Deferred work runs in reverse declaration
order (LIFO), runs only when its `defer` statement was actually reached at
run time, and runs before the function hands control back to its caller.
This is the v2.0 counterpart to Go's `defer`: function-scoped, not
block-scoped. A `defer` written inside a nested block runs at the
enclosing function's return, not at the inner block's exit.

## Pipeline position

```
Source -> Lexer -> Parser -> Resolver -> Tycheck -> HIR -> MIR -> Codegen
```

* The parser accepts `defer expr` as `StmtKind::Defer(Expr)`.
* The type checker checks the deferred expression like any expression and
  discards its type (the result is evaluated for side effects only).
* HIR keeps the statement as `HirStmtKind::Defer(HirExpr)`.
* MIR lowering turns each `defer expr` into a runtime registration: it
  lifts `expr` into a zero-argument thunk closure and emits a
  `__defer_push(thunk)` call at the point the `defer` statement executes.
  The enclosing `MirFunction` is flagged `has_defer`.
* Codegen opens a per-call defer frame on entry (for a `has_defer`
  function), lowers `__defer_push` to `raven_defer_push`, and runs the
  frame's parked thunks at every return path before leaving the GC frame.

## Semantics

* **Function-scoped.** A deferred expression runs when the enclosing
  function returns, regardless of how deeply the `defer` is nested in
  blocks, conditionals, or loops. There is no block-exit firing.
* **LIFO.** Across all reached defers in the function, deferred
  expressions run in the reverse of the order their `defer` statements
  executed. `defer A; defer B; return` runs `B` then `A`, and a defer in
  a nested block that ran after an earlier body-level defer still runs
  before it.
* **Reached-only.** A `defer` schedules its expression only if control
  actually reached the `defer` statement at run time. A `defer` in an
  `if` branch that was not taken never runs; one after an early `return`
  that was taken never runs, because that line was never executed. This
  is dynamic: the thunk is pushed when the statement runs, not when the
  block is compiled.
* **Return value first.** On `return e`, the return value `e` is evaluated
  first, then the deferred thunks run, then the function returns. This
  matches Go: defers observe the already-computed result.
* **Cannot alter the return value.** Deferred expressions are Unit-typed
  and run for their side effects only. v2.0 has no named-return mutation,
  so a defer cannot change what the function returns.
* **Loops.** A `defer` inside a loop body that runs on several iterations
  schedules one thunk per iteration it was reached on; all of them run at
  the function's return, in LIFO order. They do not fire at the end of
  each iteration.

## Lowering strategy

The chosen strategy is a **per-frame runtime defer list**. Compile-time
block expansion cannot express "only the dynamically reached defers, run
at function return" for arbitrary control flow, because a static pass
cannot know at a merge point whether a conditional branch executed. The
runtime list makes both reached-only and ordering exact and dynamic.

### The thunk closure

Lowering `defer expr` wraps `expr` as the single statement of a
zero-parameter lambda `fun() -> Unit { expr }`. The existing closure
machinery (capture analysis and body lifting, issue #62) lifts the body
into a standalone function and emits a `ClosureCreate` that captures the
free variables `expr` reads. The deferred expression therefore observes
the values its captures held when the `defer` statement ran, copied into
the closure environment exactly like any other closure capture.

### Registration and the runtime list

The runtime maintains a thread-local stack of defer frames, one inner
vector per open call frame:

* `raven_defer_enter_frame()` pushes a fresh, empty frame. Codegen emits
  it at the entry of every `has_defer` function, right after the GC root
  frame is set up.
* `raven_defer_push(closure)` appends a thunk closure to the current
  frame. Codegen lowers each `__defer_push` MIR call to it. Pushing at
  the point the `defer` statement executes is what makes the scheme
  reached-only and the order dynamic.
* `raven_defer_run_frame()` pops the current frame and invokes its parked
  thunks in LIFO order, then discards it. Codegen emits it at every
  return path before leaving the GC frame. A thunk that itself registers
  a `defer` appends to the same frame and runs before the frame is
  dropped, matching Go's behaviour for defers scheduled during a deferred
  call.

The runtime calls each thunk through a fixed ABI, `extern "C"
fn(env: *mut u8)`: it reads the closure's lifted-body function pointer and
its capture buffer and calls the body with the buffer as the environment
argument. The lifted thunk body has signature `(env) -> ()`, so this is
the closure ABI codegen already uses for an indirect closure call with no
user arguments.

### Which return paths run the defers

Codegen runs the frame at every `MirTerminator::Return`. Because the
front end lowers all exits to a `Return` terminator, this covers:

* a written `return e`,
* the implicit fall-through return at the end of the body,
* the `?` operator's error arm, which HIR desugars to `return Err(__e)`
  (or `return None`), an ordinary `Return`. A defer declared before a `?`
  runs when the `?` takes its early-return arm; a defer after it does not,
  because the `?` return was reached first.

Each call enters exactly one defer frame and, at run time, takes exactly
one return path, so the single `raven_defer_run_frame` on that path
balances the single `raven_defer_enter_frame` at entry.

## Interaction with the GC

Parked thunks are heap closure objects, and a collection can occur between
registration and execution. The collector roots them: `mark` visits every
closure pointer in every open defer frame, so a parked thunk (and every
value it captures) stays alive until it runs. The defer frame's lifetime
is tied to the call frame, so the roots are released as soon as the frame
runs.

Defers run before `raven_gc_leave_frame`, after the return operand is
evaluated. The return value is computed while the locals are still rooted,
and the deferred thunks may still read and write GC-managed locals because
the GC root frame is not torn down until they finish.

## Panic

Raven `panic` aborts the process in v2.0; there is no stack unwinding.
Deferred expressions do not run on a panic, exactly as a process that
calls `abort()` runs no cleanup. This is intentional and documented, not a
gap to be patched with unwinding. Defers run on every normal return path,
including the `?` operator's error return, which is an ordinary return and
not a panic.

## Out of scope

* Running defers on panic or any form of unwinding. v2.0 panic aborts.
* Named-return mutation from a defer (Go's ability to rewrite the return
  value). Deferred expressions are Unit-typed and run for side effects.
* Defer inside generic functions interacts with monomorphization only
  through ordinary closure and expression lowering; no defer-specific
  specialization is needed.
