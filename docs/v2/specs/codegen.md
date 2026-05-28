# Codegen Spec

## Goal

Translate a monomorphized `MirProgram` into a native object file, link it
with the `raven-runtime` staticlib through the system `cc` driver, and
produce an executable. The backend reuses Cranelift for instruction
selection and register allocation. The first cut covers primitives,
binary and unary operators, branches, switches, function calls with
static dispatch, returns, and the `print` intrinsic.

Heap allocated values (strings as first class objects, lists, maps,
sets), trait object dispatch, closure captures, and `defer` are
intentionally out of scope here. They land alongside the runtime object
layout work tracked by the follow up issues listed below.

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
| `Str`    | (deferred)     | String values are not first class in the MVP. The only place `Str` appears is the argument to the `print` intrinsic, where the codegen pulls the bytes out of the static data table directly. |
| `Struct`, `Enum`, `Option`, `Result`, `List`, `Function` | (deferred) | Tracked by issues #65, #66, #67. Encountering them in MIR currently emits an `Unreachable` and a panic in the driver. |

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
| `StructCreate`, `EnumCreate`, `FieldAccess`, `IndexAccess`, `ArrayLit`, `Cast`, `ClosureCreate` | Deferred. |

## Terminator lowering

| MIR terminator | Cranelift expansion |
|----------------|---------------------|
| `Goto(b)` | `jump b`. |
| `SwitchInt { discr, targets, otherwise }` | A linear cascade of `icmp` plus `brif` against each `(value, block)` pair; the final fall through is `jump otherwise`. A dense fan out could later switch to `br_table`; the MVP keeps the cascade for simplicity. |
| `SwitchEnum { discr, targets, otherwise }` | Deferred until enum layouts land. The codegen emits `trap` for any function that hits this terminator and logs a warning. |
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

* String, list, map, set object layouts and reference counted handles
  (issue #65).
* Trait object dispatch (issue #66).
* Closure captures (issue #67).
* `defer` ordering and runtime hooks (issue #68).
* Stdlib functions beyond `print` (issue #71 onwards).
* Cross platform installer packaging (issue #92).

## Test coverage

* Unit tests on `compile_to_object` covering: a function returning a
  constant `Int`, an arithmetic `+` lowering, an `if` with a value, a
  call between two functions, and the `print("hello")` lowering.
* An end to end smoke test that compiles `examples/v2/hello.rv` with
  the driver, links it with the host toolchain, runs the resulting
  binary, and asserts the stdout matches `Hello, Raven!\n` and the exit
  status is success. The test gates itself with a presence check on the
  host linker and emits an `eprintln!` plus a successful exit only when
  no linker is available at all; on a correctly configured host it links
  and runs the program rather than skipping.

## Crate layout

```
src/codegen/
  mod.rs                Entry point: compile_to_object, compile_program
  context.rs            Per module Cranelift context, function table, string table
  function.rs           Per function lowering
  intrinsics.rs         Recognized intrinsic mangled names (print, future stdlib)
  linker.rs             per-platform linker selection by target triple
src/main.rs             build subcommand wired through clap minimal parsing
```
