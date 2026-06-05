# C FFI Spec

## Goal

Specify calling C functions from Raven. A program declares foreign
signatures in an `extern "C"` block, then calls them like ordinary
functions. The minimum to be useful is a small set of C ABI primitive
types, a C string literal `c"..."`, type checking of the foreign
signatures, and codegen that emits direct C-ABI calls to imported
symbols. `strlen(c"hello")` returns `5`.

## Pipeline position

```
Source -> Lexer -> Parser -> Resolver -> Tycheck -> HIR -> MIR -> Codegen -> Linker
```

* The lexer produces `CStringLit` for `c"..."` and `Extern` for the
  keyword.
* The parser produces an `Extern` declaration holding `ExternFn`
  signatures. This is already part of the grammar (`docs/v2/specs/parser.md`).
* The resolver records each foreign function as `Binding::Extern` and
  treats the FFI type names as built-in.
* The type checker assigns each FFI type name a distinct `Ty::Ffi(...)`
  and checks calls against the foreign signature.
* HIR carries the resolved extern signatures through as a passive item.
* MIR records the foreign functions in `MirProgram.externs` and lowers a
  call to a foreign name as a direct `Call`.
* Codegen declares each foreign function as an imported symbol and lowers
  the call to a direct C-ABI call. The linker satisfies the symbol.

## Syntax

```rust
extern "C" {
    fun strlen(s: CStr) -> CSize
    fun abs(x: CInt) -> CInt
    fun puts(s: CStr) -> CInt
}
```

* `extern` is followed by an ABI string literal. Only `"C"` is meaningful
  in v2.0; other ABI strings parse but are treated as C.
* The block holds zero or more `fun name(params) -> Ret` signatures with
  no bodies. A signature with no `-> Ret` returns the C `void`
  equivalent (`Unit`).
* Foreign functions are not generic.

A call uses the foreign name directly:

```rust
fun main() {
    let n = strlen(c"hello")
    print(n)
}
```

## FFI primitive types

These names are recognized by the type checker as C ABI primitives.
They are kept distinct from the native Raven types so a native value is
not silently passed where the C ABI expects a foreign one.

| Raven type | C type           | Cranelift ABI type     |
|------------|------------------|------------------------|
| `CInt`     | `int`            | `i32`                  |
| `CLong`    | `long`           | `i64`                  |
| `CSize`    | `size_t`         | pointer width (`i64`)  |
| `CStr`     | `const char *`   | pointer width (`i64`)  |
| `CFloat`   | `float`          | `f32`                  |
| `CDouble`  | `double`         | `f64`                  |
| `CPtr<T>`  | `T *` (opaque)   | pointer width (`i64`)  |

`CInt` is fixed at 32-bit, which matches the C `int` on every ABI Raven
targets. `CLong` and `CSize` are 64-bit on the 64-bit targets Raven
supports. `CStr`, `CSize`, and `CPtr<T>` are all pointer width. `CDouble`
is C `double`, the same `f64` representation a Raven `Float` already uses,
so a `Float` argument is accepted where a `CDouble` is expected with no
conversion at the call. `CFloat` is C `float` (`f32`). A `Float` argument
is accepted where a `CFloat` is expected: the back end narrows the f64 to
f32 at the call boundary (`fdemote`), and a `CFloat` return is widened back
to an f64 `Float` (`fpromote`) before use. The C numeric FFI types are now
complete: `CInt`, `CLong`, `CSize`, `CFloat`, `CDouble`, plus `CStr` and
`CPtr<T>` for pointers.

`CPtr<T>` is a typed but opaque pointer in v2.0: the pointee type is kept
for documentation and future conversions, but the back end treats the
value as a raw pointer-width integer. There is no dereference operator
for it yet.

`CString` is accepted as an alias for `CStr` so that older `extern`
signatures keep checking.

The `std/ffi` module adds runtime `String` to `CStr` conversion helpers
on top of these primitives. See `docs/v2/specs/std-ffi.md`.

## C string literals and `CStr`

A `c"..."` literal types as `CStr`. In codegen it lowers to the address
of a static, read-only, null-terminated byte buffer: the literal's bytes
followed by a single `\0`. No heap allocation and no runtime call occur,
so the pointer is handed straight to the C function as a `const char *`.
Identical literals share one symbol.

Passing a native Raven `String` where a `CStr` is expected is rejected.
A heap `String` (see `docs/v2/specs/object-layout.md` and
`raven-runtime/src/object/string.rs`) is length-prefixed and not
guaranteed to be null-terminated, so it is not a valid `const char *`.
To pass a runtime `String`, convert it through `std/ffi`'s `to_cstr`,
which copies the bytes into a null-terminated buffer (see
`docs/v2/specs/std-ffi.md`). A `c"..."` literal still produces a `CStr`
directly with no allocation.

An integer C parameter (`CInt`, `CLong`, `CSize`) accepts a native `Int`
argument, so a literal such as `abs(-7)` checks. The back end converts
the i64 `Int` to the parameter's machine width at the call (a reduce to
i32 for `CInt`, a pass-through for the i64-width types). This keeps a C
function with integer parameters callable without a cast.

## Type checking extern declarations

Each `extern "C"` function becomes a known signature (`Binding::Extern`)
with FFI parameter and return types. A call:

* checks arity against the signature,
* checks each argument against the parameter's FFI type, with the two
  coercions above (`c"..."` -> `CStr`, native `Int` -> integer FFI type),
* otherwise requires the argument type to unify with the parameter type.

A mismatch (for example a `c"..."` where a `CInt` is expected, or a
`String` where a `CStr` is expected) is a normal `TypeError` reported at
the argument's span.

The return type flows out as the call's type. An integer FFI return
(`CInt`, `CLong`, `CSize`) satisfies `ToString`: it widens to `Int` (a
narrower one is sign-extended) and renders through the `Int` to-string
path, so `print(n)` and `"${n}"` both observe a C call result directly.

## Codegen

Each foreign function in `MirProgram.externs` is declared as a Cranelift
external symbol with `Linkage::Import`, under the module's default
calling convention (the platform C ABI), using the ABI type of each
parameter and the return. The raw C name is the link-time symbol and the
key a call site resolves against. A call to a foreign name lowers to a
direct call to that imported symbol, the same mechanism the back end
already uses for runtime symbols such as `raven_println_str`.

## Linking boundary

On `x86_64-pc-windows-msvc` the link step already puts the C runtime on
the link line (`/defaultlib:msvcrt`), so CRT-provided symbols such as
`strlen`, `abs`, and `printf` resolve without any extra flags. The CRT is
the only library guaranteed to link in this release.

Symbols from any other library are out of scope here. Per-project link
flags (an `[ffi]` section in `rv.toml` passing `-l<lib>` or `.lib`
entries) arrive with `rvpm build`, tracked by issue #81. Until then a
non-CRT symbol surfaces as an unresolved-symbol error at link time.

## Out of scope

* Passing or returning structs by value larger than 16 bytes. `@repr(C)`
  structs up to 16 bytes (integer, float, and nested `@repr(C)` struct fields,
  in one or two registers, or by reference on Windows x64) do cross by value;
  see `docs/v2/specs/std-ffi.md`.
* Variadic C functions (for example `printf` with format arguments).
* Non-CRT libraries and their link flags (issue #81).
* Dereferencing or arithmetic on `CPtr<T>`.
