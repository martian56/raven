# C FFI Extensions Design

## Goal

Extend the C FFI from the minimal v2.0 surface (scalars, strings, raw
`CPtr<T>`, non-capturing callbacks, and small one-register structs by value)
to the shapes real native libraries use, so that practical bindings to
SQLite, SDL, GTK, OpenGL, libpq, and similar become writable as thin Raven
wrappers. This is the design doc that issue #213 asks for. It covers the ABI
rules for the remaining slices and breaks them into implementable sub-issues.

This doc is design only. It does not change behavior. Each slice lands as its
own PR after its sub-issue is filed.

## Current state (already implemented)

Recorded here so the design builds on what exists rather than restating it.
See `docs/v2/specs/ffi.md` and `docs/v2/specs/std-ffi.md` for the shipped
behavior.

* Scalar `extern "C"` calls and the FFI scalar types `CInt`, `CLong`,
  `CSize`, `CFloat` (f32, with `fdemote`/`fpromote` at the call boundary),
  `CDouble`, and `CStr`.
* `String` <-> `CStr` conversion (`to_cstr` / `from_cstr`).
* Raw pointers `CPtr<T>`: `alloc`, `free`, `load`, `store`, `offset`,
  `null_ptr`, `is_null` (the `__ptr_*` intrinsics).
* Non-capturing top-level Raven functions passed as a C function pointer
  (`CFnPtr`), validated by the type checker (`check_callback_arg`).
* Small `@repr(C)` structs by value, argument and return, limited to **one
  register**: total size at most 8 bytes, integer-class fields only, no
  floating-point, nested, or padded-past-8 fields. The call boundary packs
  the fields into a single `i64` (`marshal_struct_to_reg` in
  `src/codegen/function.rs`) using the recorded `ReprCLayout`
  (`src/codegen/context.rs`).

## What remains

| Slice | Sub-issue | ABI surface |
|-------|-----------|-------------|
| Structs > 8 bytes by value | A | SysV two-eightbyte + memory; Win64 size-class + by-reference |
| Floating-point fields in structs | B | SysV SSE classification; Win64 integer-register |
| Nested / struct-of-struct fields | C | recursive layout and classification |
| Capturing closures as callbacks | #234 (D) | userdata trampoline + GC rooting |
| Variadic C functions | E (low priority) | platform varargs registers |

The slices are ordered by leverage. A is the gate for most real structs
(`SDL_Rect` is 16 bytes); B and C build on A; D is independent and builds on
the closure work from #317. E is optional.

---

## Slice A: structs larger than one register by value

The existing path handles a struct that fits in one 8-byte register. Real C
structs are bigger, so the call boundary must classify a struct and either
split it across registers or pass it through memory, per the platform ABI.
The two supported platforms classify differently, so the codegen branches on
the target.

### System V AMD64 (Linux, macOS x86_64)

Classify the struct into **eightbytes** (8-byte chunks, the last padded):

* A struct larger than 16 bytes (more than two eightbytes), or one with an
  unaligned field, is **MEMORY class**: the caller copies it to the stack and
  passes a pointer; a MEMORY return uses a hidden first pointer argument
  (sret) that the callee writes through and returns in `rax`.
* A struct of 1 or 2 eightbytes has each eightbyte classified INTEGER or SSE
  (SSE is slice B; integer-only here). Each INTEGER eightbyte is passed in the
  next integer argument register (`rdi, rsi, rdx, rcx, r8, r9`), then on the
  stack when the registers are exhausted. A 1-or-2 eightbyte return comes back
  in `rax` (and `rdx` for the second eightbyte).

Codegen: extend `marshal_struct_to_reg` to produce a `Vec<Value>` of one
`i64` per eightbyte (pack fields by C bit offset within each eightbyte), and
declare the call signature with one integer `AbiParam` per eightbyte. For the
MEMORY case, store the struct to a stack slot and pass its address; for a
MEMORY return, allocate the result slot, pass its address as the hidden
argument, and read the struct back after the call.

### Windows x64

Windows does not split structs across registers. By size:

* Size **1, 2, 4, or 8 bytes**: passed by value in a single integer register
  (the existing one-register path already covers this; size 4 and 8 are the
  common cases). Returned in `rax`.
* Size **3, 5, 6, 7, or > 8 bytes**: passed **by reference**. The caller makes
  a temporary copy and passes a pointer; the callee receives a pointer. A
  return of these sizes uses a hidden sret pointer in `rcx`.

Codegen: for the by-reference case, store the struct to a stack slot and pass
the pointer; for the by-reference return, allocate the result slot, pass its
address as the hidden argument, and read it back.

### Shared mechanics

* `ReprCLayout` already records each field's C offset and width. Extend the
  classifier to compute the eightbyte split (SysV) or the size class (Win64)
  once per struct and cache it on the layout.
* A by-value struct argument is read from its Raven heap object (fields live
  in pointer-width heap slots) into the register/memory image, exactly as the
  one-register path does, just for N eightbytes.
* A by-value struct return is reconstructed into a fresh Raven heap object
  (the existing aggregate constructor) from the returned registers or the
  sret slot.

### Status

Done for structs up to 16 bytes (one or two integer registers on System V
and AArch64; one register or by reference on Windows x64), argument and
return. Structs larger than 16 bytes (the System V in-memory class) are still
rejected and remain a follow-up.

### Out of scope for A

Floating-point fields (slice B) and nested struct fields (slice C); a struct
that mixes them is rejected until those slices land.

---

## Slice B: floating-point fields in structs

### System V AMD64

An eightbyte made up entirely of `float`/`double` fields is **SSE class** and
travels in the next SSE register (`xmm0..xmm7`); an eightbyte that mixes a
float field with an integer field merges to INTEGER. Classification rule: per
eightbyte, INTEGER wins over SSE when both appear; two SSE stay SSE. An SSE
return eightbyte comes back in `xmm0` (and `xmm1` for the second).

Codegen: the classifier from slice A gains a per-eightbyte INTEGER/SSE result;
an SSE eightbyte is marshalled as an `f64` (or two `f32` packed, for a
`{float, float}` eightbyte) and passed in an SSE `AbiParam`.

### Windows x64

Windows has no homogeneous-float-aggregate rule for normal (non-vararg)
calls: a struct with float fields follows the same size-class rule as slice A
(in an integer register when 1/2/4/8 bytes, by reference otherwise). So B on
Windows is mostly a matter of allowing float fields through the existing
size-class path rather than rejecting them.

### Status

Done. `CFloat` and `CDouble` fields are supported, argument and return, up to
16 bytes. The back end builds a per-register plan (`RegPlan`) from the struct
layout and the target convention: System V classifies each eightbyte
INTEGER (i64) or SSE (f64); AArch64 detects a homogeneous float aggregate and
gives each member its own SIMD register (f32/f64), falling back to general
registers otherwise; Windows x64 uses one integer register or by reference.
A `CFloat` field is narrowed from f64 to f32 (and widened back) at the
boundary. Windows verified locally, System V in CI, AArch64 in the release
smoke test.

---

## Slice C: nested / struct-of-struct fields

A `@repr(C)` struct field whose type is itself a `@repr(C)` struct flattens
into the parent's layout at the field's offset. Classification (SysV
eightbytes, Win64 size) runs over the fully flattened field list. The layout
builder becomes recursive: a nested struct contributes its fields at
`parent_offset + nested_offset`. No new ABI rule beyond A and B, only a
recursive layout pass and the matching recursive read/reconstruct of the
Raven heap objects.

---

## Slice D: capturing closures as callbacks (#234)

The non-capturing path hands C the raw address of a top-level function. A
capturing closure carries an environment a bare C function pointer cannot
supply, so it needs a **userdata trampoline**:

* Generate a C-ABI **trampoline** whose signature matches the C callback type.
  The trampoline recovers the closure environment from a `void *user_data`
  pointer the C API threads through, then invokes the Raven closure body with
  that environment (the closure shim from #317 already has the env-leading
  calling convention to call into).
* Bind the closure environment to the specific C API's userdata slot. Common
  shapes: `qsort_r`/`qsort_s` (comparator + context), pthread-style start
  routines, GLFW/SDL user-pointer setters. The binding is per-API, so the
  surface is a typed callback that pairs the function pointer with the
  userdata pointer the caller passes to the C function.
* **GC rooting**: the captured environment must be a GC root for as long as
  the C side may hold the callback, otherwise a collection frees it under C.
  Register the environment as a root when handed to C and unroot it when the C
  side is done (API-dependent; for a scoped call like `qsort_r` the root lives
  for the duration of the call).

Out of scope for D: C callback APIs with **no** userdata channel (there is
nowhere to thread the environment); those still require a non-capturing
function. A foreign thread invoking the callback interacts with the GC and the
concurrency model (#212); D targets same-thread callbacks first and notes the
dependency.

---

## Slice E: variadic C functions (low priority)

Calling a variadic C function (`printf`-style) needs the platform vararg
convention (SysV passes the number of SSE registers used in `al`; Win64
passes floating args in both the integer and SSE register). Low priority: most
library entry points used in bindings are non-variadic, and a binding can wrap
a variadic call in a non-variadic C shim. Tracked but not scheduled.

---

## Testing strategy

* **Unit and golden**: each slice adds golden examples that declare an
  `extern "C"` against a C function the platform CRT or a tiny bundled C shim
  provides, then compile and run, asserting the round-tripped value. A struct
  round-trip (pass a struct in, return one out) is the core check.
* **Cross-platform**: struct-by-value ABI differs by platform, so the Windows
  x64 path is verified locally and the System V path through CI on the Linux
  and macOS runners. A golden example that is ABI-correct prints the same
  output on every target.
* **Proof binding** (acceptance for #213): once A through C land, a real
  binding (for example `SDL_Rect` construction, or `sqlite3_open` /
  `sqlite3_exec` / `sqlite3_close`) compiles and runs as a thin Raven wrapper.

## Out of scope (whole epic)

* A `free`/`drop` for buffers from `to_cstr` (copy-and-leak today; buffers
  from `alloc` already have `free`).
* Non-CRT libraries and their link flags (#81).
* Bitfields, packed/`#pragma pack` structs, unions, and C++ ABIs.
* A `bindgen`-style header-to-extern generator (its own follow-up).

## Sub-issue breakdown

Filed against #213:

* **A** Structs larger than one register by value (SysV two-eightbyte +
  memory; Win64 size-class + by-reference).
* **B** Floating-point fields in structs (SysV SSE classification).
* **C** Nested / struct-of-struct fields by value.
* **D** Capturing closures as callbacks via a userdata trampoline (#234,
  already filed).
* **E** Variadic C functions (low priority).
