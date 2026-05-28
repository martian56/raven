# Codegen Spec

## Goal

Translate a monomorphized `MirProgram` into a native object file, link it
with the `raven-runtime` staticlib through the host toolchain linker, and
produce an executable. The backend reuses Cranelift for instruction
selection and register allocation. The covered surface is primitives,
binary and unary operators, branches, switches, function calls with
static dispatch, returns, the `print` and `print_int` intrinsics, and
heap value construction: structs, enums (including `Option` and
`Result`), field access, enum dispatch, and non-capturing closures, all
with garbage collector root frames.

Trait object (`dyn Trait`) dispatch, calling and capturing closures,
`defer`, string interpolation, C FFI, and rich stdlib collection methods
are out of scope here and tracked by the follow up issues listed below.

## Pipeline position

```
Source -> Lexer -> Parser -> Resolver -> Tycheck -> HIR -> MIR -> Codegen -> object file
                                                                              |
                                                                              v
                                                                      link with raven-runtime
                                                                              |
                                                                              v
                                                                          executable
```

The codegen entry point consumes a fully monomorphized `MirProgram`. No
generic functions, no inference variables, no type parameters. Every
`MirType` is ground.

## Cranelift integration

Three Cranelift crates are pulled in at the workspace level:

* `cranelift-codegen`: instruction selection, register allocation, the
  `ir::Function` and `Signature` builders.
* `cranelift-frontend`: the `FunctionBuilder` helper that smooths the
  process of emitting Cranelift IR.
* `cranelift-module`: the `Module` trait that abstracts symbol
  declaration and linking, plus `DataDescription` for static data.
* `cranelift-object`: the `ObjectModule` that writes the final relocatable
  object file via the `object` crate.

The backend produces a single object file per `MirProgram`. Cross
function references resolve through `cranelift-module`'s `FuncId` and
`DataId` indices; symbol mangling is the responsibility of MIR (every
`MirFunction::name` is already the mangled monomorphic name).

## Value representation

Primitive Raven values map to fixed Cranelift types:

| MIR type | Cranelift type | Notes |
|----------|----------------|-------|
| `Unit`   | (none)         | Carried as the zero sized "no value". A `Unit` constant emits no Cranelift value; a function returning `Unit` has an empty `returns` list in its signature. |
| `Bool`   | `types::I8`    | `0` or `1`. Logical operators produce the same width so `if` over a `Bool` compares against zero. |
| `Int`    | `types::I64`   | Signed 64 bit integers. Overflow is wrap on add, sub, mul (matching Cranelift defaults). Division by zero traps via Cranelift's `sdiv` semantics on supported targets; an explicit zero check is inserted on targets where this is not guaranteed. |
| `Float`  | `types::F64`   | IEEE 754 doubles. |
| `Char`   | `types::I32`   | Reserved. The MVP does not exercise `Char` operations; ground work only. |
| `Str`    | pointer        | A heap object pointer. String literals reaching the `print` intrinsic still pull bytes from the static data table directly; a `Str` flowing through a local is a single traced GC pointer. |
| `Struct`, `Enum`, `Option`, `Result`, `List`, `Function` | pointer | A single GC pointer to a heap object. Struct and enum construction, field access, and closure allocation are lowered (see the heap value section below). `List` and `Map` rich methods, and trait object dispatch, remain tracked by issues #66 and the stdlib issues. |

The calling convention is whatever Cranelift picks for the host triple
(the system V AMD64 ABI on x86_64 Linux and Windows, ARM64 ABI on
aarch64). The backend never inspects the chosen convention. Parameters
are passed through the standard Cranelift `Signature`; aggregate
returns are not used in the MVP because every supported return type is
a single scalar or absent.

## Function lowering

Each `MirFunction` becomes one Cranelift `ir::Function`. The lowering
pass walks the MIR locals first to allocate `StackSlot`s for every
non parameter local and a Cranelift `Block` for every `MirBlockId`. The
mapping is direct: `MirLocal(i)` indexes the function's slot vector and
`MirBlockId(i)` indexes the function's block vector.

Parameters live in Cranelift block parameters on the entry block. The
backend reads each parameter once, stores it into the matching stack
slot, and from that point treats parameters and locals uniformly.

Per block: every `MirStatement::Assign` translates into a recipe that
reads its operand locals into Cranelift values, performs the operation,
and writes the result back to the destination slot. `StorageLive` and
`StorageDead` are no ops in the MVP (the future allocator may grow real
lifetimes); `Nop` is also a no op.

## Operand and rvalue lowering

| MIR construct | Cranelift expansion |
|---------------|---------------------|
| `Use(Copy(l))` | `stack_load` from `l`'s slot. |
| `Use(Const(Int n))` | `iconst.i64 n`. |
| `Use(Const(Float f))` | `f64const f`. |
| `Use(Const(Bool b))` | `iconst.i8 (if b { 1 } else { 0 })`. |
| `Use(Const(Unit))` | No value emitted. |
| `Use(Const(Str s))` | Allocates or reuses a static data symbol holding the bytes plus a length companion, emits `global_value` for the address. Currently only consumed by the `print` intrinsic. |
| `BinaryOp(op, lhs, rhs)` on `Int` | `iadd`, `isub`, `imul`, `sdiv`, `srem`, `icmp` with the matching condition code. |
| `BinaryOp(op, lhs, rhs)` on `Float` | `fadd`, `fsub`, `fmul`, `fdiv`, `fcmp`. The modulus operator is not defined on `Float` here. |
| `BinaryOp(And/Or)` on `Bool` | `band` / `bor` over `i8`. The frontend already short circuits to branches; what reaches MIR is value level. |
| `BinaryOp(BitAnd/BitOr/BitXor/Shl/Shr)` on `Int` | `band`, `bor`, `bxor`, `ishl`, `sshr`. |
| `UnaryOp(Neg)` on `Int` | `ineg`. |
| `UnaryOp(Neg)` on `Float` | `fneg`. |
| `UnaryOp(Not)` on `Bool` | `bxor` with the constant `1`. |
| `UnaryOp(Ref)` | Deferred; emits `Unreachable`. |
| `Call { callee, args }` | Loads each argument from its slot, declares the callee through `cranelift-module`, emits `call` with the resulting `FuncRef`, stores the return value. |
| `StructCreate`, `EnumCreate`, `FieldAccess`, `ClosureCreate` | Lowered. See the heap value section below. |
| `Cast` | Integer and float conversions through `sextend`, `ireduce`, `fcvt_from_sint`, `fcvt_to_sint_sat`. |
| `IndexAccess`, `ArrayLit` | Deferred to the stdlib collection issues. |

## Terminator lowering

| MIR terminator | Cranelift expansion |
|----------------|---------------------|
| `Goto(b)` | `jump b`. |
| `SwitchInt { discr, targets, otherwise }` | A linear cascade of `icmp` plus `brif` against each `(value, block)` pair; the final fall through is `jump otherwise`. A dense fan out could later switch to `br_table`; the MVP keeps the cascade for simplicity. |
| `SwitchEnum { discr, targets, otherwise }` | Loads the enum value's discriminant slot (slot 0) and emits an `icmp` plus `brif` cascade against each `(variant, block)` pair. With an explicit otherwise it is the default; without one the last target is the default, since a well typed match is exhaustive. |
| `Return(op)` | Reads the operand, then `return value` if the function has a non Unit return type, otherwise a bare `return`. |
| `Unreachable` | `trap unreachable`. |

## Stack slot layout

Every non parameter local with a sized primitive type gets an
`ExplicitSlot` sized to the primitive's byte width and aligned to the
same. `Unit` locals occupy a zero sized slot that is never loaded or
stored. Parameter locals also live in stack slots; the entry block
spills the incoming Cranelift block parameter into the slot before any
other code runs so subsequent reads can use the uniform slot path.

## Print intrinsic

The MIR call site `Call { callee: MirFnRef { mangled: "print", .. } }`
is recognized as an intrinsic. The codegen:

1. Pulls the single argument operand. If it is `Const(Str s)`, the bytes
   are interned into the per module string table, producing a pair of
   constants: the address of the bytes and the length in bytes.
2. Declares `raven_println_str` with signature `fn(ptr: i64, len: i64)`.
   The pointer width is whatever `module.target_config().pointer_type()`
   reports; the description here uses i64 because every supported host
   is 64 bit.
3. Emits a `call` with the two constants as arguments. The runtime
   prints the slice and writes a trailing newline.

The string table is a `BTreeMap<Vec<u8>, DataId>` keyed by the bytes, so
identical literals share a single allocation in the object file. Each
data symbol is named `__raven_str_<n>` for stable diagnostic output.

When the print argument is a non literal `Str` value, the codegen emits
an `Unreachable` and the driver reports a `print: non literal string
not supported` diagnostic. Literal printing is enough to get hello
world running; the full string type lands with issue #65.

### `print_int` intrinsic

`print_int(n: Int)` is the integer companion of `print`. It is a built
in free function recognized by the resolver allowlist and the type
checker the same way `print` is, with the signature `fn(Int) -> Unit`.
The codegen recognizes the mangled name `print_int`, loads its single
`Int` operand, and emits a `call` to the runtime symbol
`raven_println_int(value: i64)`, which formats the value in base ten and
writes it followed by a newline. This gives programs a way to observe a
computed integer without a string conversion, which is what the
struct and enum smoke programs use to print their results.

The decision to add a dedicated `print_int` rather than a general
`Int.to_string()` keeps the heap value work focused: `to_string`
implies a `String` builder and the full string object machinery, while
`print_int` is one runtime symbol and one intrinsic arm. A richer
stringification path lands with the string and stdlib issues.

## Heap value layout and lowering

A struct or enum value is a heap object: the standard 16-byte
`ObjectHeader` followed by a sequence of uniform 8-byte field slots, one
per field, in declaration order. Using a single slot width means the
back end never computes per-field offsets from alignment, every
primitive (`Int`, `Float`) fits a slot exactly, a smaller scalar
(`Bool`, `Char`) is widened into one, and a heap value is a single
pointer. The slots begin at offset 16; field `i` is at offset
`16 + 8 * i`; the total size is `16 + 8 * field_count`, already 8-byte
aligned. See `docs/v2/specs/object-layout.md`.

### Struct construction and field access

`StructCreate { ty, fields, field_tys }`:

1. Compute the struct's GC pointer bitmask from `field_tys`: bit `i` is
   set when field slot `i` holds a GC pointer (any heap value).
2. Intern a descriptor id for the struct type (keyed by the mangled type
   name) and record the bitmask. The id is registered with the collector
   once, from the program entry shim (see below).
3. Evaluate each field operand into a register, widening scalars to the
   8-byte slot width.
4. Call `raven_struct_new(field_count, type_id)` to allocate the body,
   then `raven_struct_fields(obj)` to get the field base pointer, then
   `store` each field at `16 + 8 * i`.

`FieldAccess { base, index }` loads the base pointer, calls
`raven_struct_fields`, and `load`s the pointer-width slot at
`16 + 8 * index`. The destination local's machine type narrows the
loaded value: a `Float` field's bits are reinterpreted with `bitcast`,
a narrow scalar is reduced. The MIR lowering resolves `index` to the
field's position in declaration order, so creation and access agree.

### Enum construction and dispatch

An enum value reuses the struct value layout. Slot 0 holds the variant
discriminant (a pointer-width integer); slots `1..` hold the active
variant's payload. `EnumCreate { ty, variant, payload, payload_tys }`
allocates `1 + payload.len()` slots, stores the discriminant in slot 0,
and the payload in slots `1..`. The GC pointer bitmask shifts each
payload pointer to slot `i + 1`. Because each variant is constructed
independently, the enum type's descriptor is the union of every
variant's payload pointer mask; an inactive variant's slots are zero and
trace harmlessly.

`SwitchEnum` loads slot 0 and branches on it. A `match` arm that binds a
payload reads slot `index + 1`, since slot 0 is the discriminant.

This makes `Option`, `Result`, and user enums runnable. The built in
constructors `Some(x)`, `Ok(x)`, `Err(x)`, and `None` are recognized
during HIR lowering and become `EnumCreate`, rather than calls to
undefined symbols.

### Closures

`ClosureCreate { fn_name, captures }` allocates a `Closure` object
through `raven_closure_new(fn_ptr, size, align, count, ptr_count)`. The
MVP supports the non-capturing shape the front end emits today: the
captures list is empty, the capture buffer is zero sized, and the
function pointer is the lifted body's address when `fn_name` resolves to
a defined function (or null while lambda body lifting is pending). A
non-empty captures list returns a `CodegenError::Unsupported`.

Calling a closure value (indirect dispatch through the stored function
pointer) and capturing closures both require front-end lambda body
lifting and capture analysis. They are tracked separately; the closure
smoke program builds a closure value and prints a sentinel to show the
allocation and GC root frame run.

## GC root frames

The collector finds its roots through a shadow stack the back end
maintains (see `docs/v2/specs/gc.md`). For every function, the codegen:

1. Identifies the locals whose type is a GC pointer (`Str`, `Struct`,
   `Enum`, `Option`, `Result`, `List`, closure `Function`).
2. Allocates one contiguous root array stack slot holding one
   pointer-sized entry per GC local.
3. At entry, after spilling parameters: zeroes every non-parameter GC
   local (so a collection before the body assigns it never follows a
   stale pointer), writes each GC local's slot address into the root
   array, and calls `raven_gc_enter_frame(array_addr, count)`.
4. Before every `Return`, calls `raven_gc_leave_frame()`. The return
   value is evaluated before leaving the frame so a collection during
   evaluation still sees the locals rooted. Frames nest in strict
   last-in-first-out order.

A function with no GC pointer locals sets up no frame at all.

### Worked example

For `fun add_coords(p: Point) -> Int = p.x + p.y`, the parameter `p` is
a GC pointer and any temporaries for the field reads are scalars. The
entry block emits, in order:

```
;; spill the incoming parameter pointer into p's slot
v0 = stack_addr.i64 p_slot          ; address of p's slot
;; p is a parameter, so it is not re-zeroed
stack_store v0, root_array+0        ; root_array[0] = &p
v1 = stack_addr.i64 root_array
v2 = iconst.i64 1                   ; one GC local
call raven_gc_enter_frame(v1, v2)
;; ... body: load p, read fields at +16 and +24, add ...
call raven_gc_leave_frame()
return vsum
```

The root array holds the *address* of `p`'s slot, not `p` itself, so a
collection reads `*(&p)` and observes whatever pointer the slot currently
holds. This matches the ABI example in `docs/v2/specs/gc.md`.

### Struct descriptor registration

The collector traces a struct or enum by tag plus a per-type GC pointer
bitmask. The back end registers every interned descriptor with
`raven_struct_register(type_id, ptr_mask)` from the `int main(void)`
entry shim, before calling the Raven `main`, so any value the program
builds is traceable from its first allocation. The struct value stores
its `type_id` in `header.cap` and its field count in `header.len`.

## Driver

A new `build` subcommand on the `raven` binary owns the end to end
pipeline:

```
raven build path/to/program.rv -o out
```

The driver runs lexing, parsing, resolution, type checking, HIR
lowering, MIR lowering, and monomorphization in order, then hands the
`MirProgram` to `codegen::compile_to_object`. The returned `Vec<u8>` is
written to a temporary `.o` file. The driver then links the object with
the `raven-runtime` staticlib (built by the same Cargo invocation as
the compiler) into the requested output.

`compile_to_object(program, target_isa)` is also exposed for in process
use by tests, returning the raw object bytes so a test can call the
linker itself or inspect the object with `goblin`.

### Entry point

The exported program entry is an `int main(void)` shim, not the Raven
`main` directly. Cranelift lowers the Raven `main` under an internal
symbol that returns whatever the function returns (unit for a typical
program, which is no machine value at all). A C runtime starting that
symbol as `int main()` would read an uninitialized register as the
process exit code. The shim calls the Raven body and returns a literal
`0`, so a successful program exits with status `0` on every host.

### Linker selection

Cranelift emits an object in the host's native format: an MSVC-flavor
COFF on windows-msvc, ELF on Linux, Mach-O on macOS. The link step is
therefore toolchain aware and keyed on the host target triple
(`target_lexicon::Triple::host()`):

* `*-windows-msvc`: the MSVC `link.exe`, located through the Windows
  registry with `cc::windows_registry::find_tool`. The `cc` crate hands
  back a `Command` preloaded with the SDK and CRT `LIB`, `INCLUDE`, and
  `PATH` environment, which is why this path is preferred. The link line
  is `/NOLOGO /OUT:<output> <object> <runtime.lib> <native system libs>
  /SUBSYSTEM:CONSOLE`. If the registry lookup fails, the driver falls
  back (best effort) to the Rust toolchain's bundled `rust-lld` in
  `lld-link` flavor, located under `rustc --print sysroot`. The fallback
  needs the SDK `LIB` paths already present in the environment because
  `rust-lld` does not supply them.
* `*-windows-gnu`: a `cc`/`gcc` driver, which must be a 64-bit
  MinGW-w64. A 32-bit MinGW.org `gcc` cannot read the 64-bit object, so
  a link failure surfaces a hint about the architecture mismatch.
* Linux, macOS, and other Unix: the system `cc` driver, which brings the
  system linker and its default library search paths, plus the Rust std
  system libraries the runtime depends on (`-lpthread`, `-ldl`, `-lm`,
  and so on).

### Rust std native system libraries on MSVC

The Rust staticlib references system import libraries that must appear
on the MSVC link line, because the staticlib does not carry them. The
exact set is captured with:

```
cargo rustc -p raven-runtime --crate-type staticlib -- --print native-static-libs
```

The `note: native-static-libs: ...` line it prints is hardcoded as a
`const` in `linker.rs` (currently `kernel32.lib advapi32.lib ntdll.lib
userenv.lib ws2_32.lib dbghelp.lib /defaultlib:msvcrt`, captured against
rustc 1.85.0). The driver does not shell out to cargo at link time.
Refresh the list the same way if the runtime crate gains native
dependencies.

If no linker is available for the host at all, the smoke test that
compiles and runs a hello world program prints a diagnostic and short
circuits with a successful exit. On a correctly configured host it links
and runs the program for real and asserts the output. The unit tests
that only consult MIR lowering do not depend on a linker.

## Out of scope (tracked by follow up issues)

* ~~Trait object (`dyn Trait`) dispatch through vtables (issue #66).~~
  Implemented: a trait object is a boxed fat pointer, vtables are emitted
  as read-only data, and a `dyn` method call dispatches through the
  vtable. See `docs/v2/specs/dyn-trait.md`. Static trait method calls are
  monomorphized by MIR into direct calls.
* Calling closure values and capturing closures (lambda body lifting and
  capture analysis); non-capturing closure allocation is supported.
* `defer` ordering and runtime hooks (issue #68).
* String interpolation (issue #69) and C FFI (issue #70).
* Rich stdlib collection methods (`push`, `get`, and so on) beyond
  constructing values (issues #71 onwards).
* Cross platform installer packaging (issue #92).

## Test coverage

* Unit tests on `compile_to_object` covering: a function returning a
  constant `Int`, an arithmetic `+` lowering, an `if` with a value, a
  call between two functions, and the `print("hello")` lowering.
* Layout unit tests on the field offset, GC pointer classification, and
  struct and enum pointer masks.
* End to end smoke tests that compile, link, run, and check the stdout of
  `examples/v2/hello.rv` (`Hello, Raven!`), `point.rv` (a struct,
  prints `7`), `option_sum.rv` (Option construction and matching, prints
  `104`), and `closure_value.rv` (a non-capturing closure allocation,
  prints `42`). Each gates on linker and runtime staticlib presence and
  emits an `eprintln!` plus a successful exit only when one is missing;
  on a correctly configured host it links and runs the program for real.

## Crate layout

```
src/codegen/
  mod.rs                Entry point: compile_to_object, compile_program
  context.rs            Per module Cranelift context, function table, string table, struct descriptors
  function.rs           Per function lowering, heap rvalues, GC root frames
  layout.rs             Struct and enum heap layout: field offsets and GC pointer masks
  intrinsics.rs         Recognized intrinsic mangled names and runtime ABI symbols
  linker.rs             per-platform linker selection by target triple
src/main.rs             build subcommand wired through minimal arg parsing
```
