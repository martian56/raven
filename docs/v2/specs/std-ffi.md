# std/ffi Spec

Companion module for the C foreign-function interface. The C type set is
built into the type checker (see `docs/v2/specs/ffi.md`); this module adds
the runtime conversions between a Raven `String` and a C `CStr`, which the
compiler FFI layer deliberately left out. A `c"..."` literal already
yields a `CStr` at compile time, but a `String` value computed at runtime
needs a conversion, which is what `to_cstr` and `from_cstr` provide.

## Import

```rust
import std/ffi { to_cstr, from_cstr }
import std/ffi { alloc, free, load, store, offset, is_null, null_ptr }
```

## Surface

```rust
fun to_cstr(s: String) -> CStr
fun from_cstr(p: CStr) -> String
```

`to_cstr` copies the bytes of `s` into a fresh, null-terminated buffer and
returns a `CStr` pointing at the first byte. The result is a standalone
copy, not the String's own length-prefixed buffer, so it is a valid
`const char *`. `from_cstr` reads the null-terminated bytes at `p`,
stopping at the first NUL, and builds a Raven `String` from them (the
terminator is not included).

Both wrappers are pure Raven over two raven-runtime symbols bound through
`extern "C"`: `raven_string_to_cstr` and `raven_cstr_to_string`.

## Raw pointer and buffer access

`CPtr<T>` is a usable raw pointer. The following generic functions read and
write C memory through it and obtain or release a buffer:

```rust
fun alloc<T>(count: Int) -> CPtr<T>
fun free<T>(p: CPtr<T>)
fun load<T>(p: CPtr<T>) -> T
fun store<T>(p: CPtr<T>, value: T)
fun offset<T>(p: CPtr<T>, count: Int) -> CPtr<T>
fun is_null<T>(p: CPtr<T>) -> Bool
fun null_ptr<T>() -> CPtr<T>
```

* `alloc<T>(count)` reserves room for `count` elements of `T` and returns a
  `CPtr<T>` to the (uninitialized) buffer, or the null pointer on a zero
  count or allocation failure.
* `free<T>(p)` releases a buffer from `alloc`; a null pointer is a no-op.
* `load<T>(p)` reads the element of type `T` at `p`; `store<T>(p, value)`
  writes it. The pointee width sets the size of the single machine
  load/store the back end emits.
* `offset<T>(p, count)` advances `p` by `count` elements, scaling by the
  size of `T` (`p + count * sizeof(T)`). There is no `at`/indexed-load in
  this release; compose `load(offset(p, i))`.
* `is_null<T>(p)` is true exactly when `p` is the null pointer; `null_ptr<T>()`
  is that null `CPtr<T>`. Many C APIs return null on failure, so check before
  dereferencing.

### Supported pointee types

`T` must be a C scalar (`CInt`, `CLong`, `CSize`, `CFloat`, `CDouble`,
`CStr`) or a native `Int`/`Float`. The pointer load/store width is the
Cranelift machine type of `T`: `CInt` is a 4-byte i32, `CLong`/`CSize`/`CStr`
and `Int` are 8-byte i64, `CFloat` is a 4-byte f32, `CDouble`/`Float` an
8-byte f64. A native `Int` stored to a `CInt` pointee is narrowed to i32 at
the store, and a `Float` stored to a `CFloat` pointee is narrowed to f32,
the same boundary conversions an `extern` call applies. The pointee may also
be a generic parameter inside a generic function, which is how these
wrappers forward `T`; it is grounded to a concrete type per monomorphization
to pick the width.

### Lowering

The wrappers forward to compiler builtins (`__ptr_load`, `__ptr_store`,
`__ptr_offset`, `__ptr_is_null`, `__ptr_null`, `__ptr_alloc`, `__ptr_free`),
each parameterized by the pointee type `T` the same way the reflection
builtins carry their type argument. The type checker records `T`, HIR
carries it, and MIR grounds it under the monomorphization substitution so
codegen emits a Cranelift `load`/`store` of the right width and scales
`offset`/`alloc` by `sizeof(T)`. `alloc` and `free` call the raven-runtime
symbols `raven_ffi_alloc(bytes) -> ptr` and `raven_ffi_free(ptr)`, thin
`malloc`/`free` wrappers.

### Lifetime and safety

Memory from `alloc` lives **outside the garbage collector**. It is never
traced and never reclaimed automatically: the caller owns it and **must**
`free` it (a manual lifetime), consistent with the copy-and-leak note for
`to_cstr` above. This is unchecked raw memory access. There are no bounds
checks, no null checks, and no use-after-free or double-free protection: an
out-of-range `offset`, a `load`/`store` through a null or freed pointer, or
a width that does not match how C allocated the region is undefined
behavior, exactly as in C. Use `is_null` to guard pointers a C API may
return null, and keep the element type `T` consistent with the C side.

## Lifetime and ownership

`to_cstr` allocates the null-terminated buffer outside the GC heap and
does not reclaim it: the pointer is valid for the rest of the program run.
This copy semantics is intentional. A C callee may read the pointer after
the call returns or retain it, and the buffer must not move or be freed by
a later garbage collection, so it cannot be the String's own GC-managed
bytes. The cost is that each `to_cstr` call leaks one buffer; callers that
convert in a hot loop should hoist the conversion. There is no `free`
helper in this release.

`from_cstr` produces an ordinary GC-managed `String`, traced and reclaimed
like any other. Embedded NUL bytes in the source `String` are preserved by
`to_cstr` up to the first one, but a C reader stops at that first NUL, so a
round trip through `from_cstr` truncates at an embedded NUL. Plain UTF-8
text without interior NUL bytes round-trips exactly.

## C type set

The C ABI primitives recognized by the type checker, with their machine
mappings. `CInt`/`CLong`/`CSize`/`CStr`/`CPtr<T>` are specified in
`docs/v2/specs/ffi.md`; `CFloat` and `CDouble` are added here. The C
numeric FFI types are now complete.

| Raven type | C type         | Cranelift ABI type    |
|------------|----------------|-----------------------|
| `CInt`     | `int`          | `i32`                 |
| `CLong`    | `long`         | `i64`                 |
| `CSize`    | `size_t`       | pointer width (`i64`) |
| `CStr`     | `const char *` | pointer width (`i64`) |
| `CFloat`   | `float`        | `f32`                 |
| `CDouble`  | `double`       | `f64`                 |
| `CPtr<T>`  | `T *` (opaque) | pointer width (`i64`) |
| `CFnPtr`   | function ptr   | pointer width (`i64`) |

`CDouble` is C `double`. It maps to `f64`, the exact representation a
Raven `Float` already crosses the C ABI as, so a `Float` argument is
accepted where a `CDouble` parameter is expected with no conversion at the
call boundary, and a `CDouble` return is an `f64`. A libm function such as
`sqrt(x: CDouble) -> CDouble` is callable with a `Float`.

`CFloat` is C `float` (an f32). A Raven `Float` is f64, so it does not
share `CFloat`'s representation the way it does `CDouble`'s. A `Float`
argument is still accepted where a `CFloat` parameter is expected: the
back end narrows the f64 to f32 at the call boundary with `fdemote`, and a
`CFloat` return is widened back to an f64 `Float` with `fpromote` before it
is used. A single-precision CRT/libm function such as
`sqrtf(x: CFloat) -> CFloat` is callable with a `Float` and its result
prints and interpolates as a `Float`.

`CString` is accepted as an alias for `CStr`.

## Function-pointer callbacks

`CFnPtr` is an untyped C function pointer (pointer width). It lets a C
function that takes a callback call back into Raven. A non-capturing
top-level Raven function can be passed where a `CFnPtr` is expected by
naming it bare:

```rust
extern "C" {
    fun qsort(base: CPtr<CInt>, count: CSize, size: CSize, cmp: CFnPtr)
}

fun compare(a: CPtr<CInt>, b: CPtr<CInt>) -> CInt {
    return load<CInt>(a) - load<CInt>(b)
}

fun main() {
    // ... fill a CInt buffer ...
    qsort(buf, 5, 4, compare)
}
```

Rules:

* Only a bare name of a **non-capturing top-level function** is accepted.
  A closure value (a local of function type, which carries a capture
  environment C cannot supply) is rejected. Capturing closures as
  callbacks are a follow-up (they need a userdata/trampoline mechanism).
* The function's parameters and return must all be C-FFI types (`CInt`,
  `CLong`, `CSize`, `CFloat`, `CDouble`, `CStr`, `CPtr<T>`, `CFnPtr`, or
  `Unit` return) so the C ABI of the resulting pointer is well defined. A
  function with a native `Int`/`Float`/`String` parameter or return is
  rejected.
* `CFnPtr` is untyped: the type checker does not verify the function's
  signature matches what the C side expects. The signature match is the
  programmer's responsibility, exactly as in C.

### Call convention

Ordinary Raven functions are already compiled under the platform default
calling convention, which is the platform C ABI (WindowsFastcall on
x86_64-pc-windows-msvc, SystemV on x86_64 Linux). It is the same
convention `extern "C"` functions are called with. A Raven function whose
signature uses only C-FFI types is therefore directly callable by C with
no wrapper or thunk: passing a `CFnPtr` emits the function's address
(`func_addr`) and hands it to C.

### GC and allocation rule

A Raven function runs its normal prologue and epilogue when invoked,
including entering and leaving its GC shadow-stack frame, regardless of
who calls it. A callback that does not allocate (the comparator above only
loads and subtracts) never triggers a collection, so its frame is always
consistent. Callbacks should avoid allocating; a callback that allocates
is supported by the frame machinery but is outside what this slice
verifies.

## C string literals

A `c"..."` literal types as `CStr` and lowers to a static, read-only,
null-terminated buffer with no allocation and no runtime call. Use it for
compile-time-known strings; use `to_cstr` for a `String` value. See
`docs/v2/specs/ffi.md` for the literal's codegen.

## libc verification

`examples/v2/use_ffi.rv` converts runtime Strings and calls libc:

```rust
import std/ffi { to_cstr, from_cstr }
import std/io { println }

extern "C" {
    fun strlen(s: CStr) -> CSize
    fun strcmp(a: CStr, b: CStr) -> CInt
}

fun main() {
    let s = "hello"
    let n = strlen(to_cstr(s))
    print(n)
    let eq = strcmp(to_cstr("abc"), to_cstr("abc"))
    print(eq)
    let lt = strcmp(to_cstr("abc"), to_cstr("abd"))
    print(lt)
    println(from_cstr(to_cstr("roundtrip")))
}
```

Output:

```
5
0
-1
roundtrip
```

`strlen` counts the 5 bytes of the converted `String`. `strcmp` returns 0
for equal strings and a negative value when the first differing byte is
smaller (`-1` here for `'c'` against `'d'`). `from_cstr(to_cstr("roundtrip"))`
recovers the original text.

The C integer results print through `print`. The integer FFI types
(`CInt`, `CLong`, `CSize`) satisfy `ToString` by widening to `Int` (a
narrower one is sign-extended) and rendering through the `Int` to-string
path, so a `CSize` or `CInt` can be printed or interpolated into a
`"${...}"` string directly. `CSize` is treated as a signed `Int`, correct
for realistic sizes (below 2^63). The float FFI types (`CFloat`,
`CDouble`) satisfy `ToString` the same way through the `Float` to-string
path; a `CFloat` widens its f32 to f64 first.

## Raw pointer verification

`examples/v2/ffi_pointers.rv` allocates a buffer of `CInt`s, round-trips
values through raw `store`/`load` at successive offsets, checks `is_null` on
a null and a live pointer, and calls a C function
(`raven_ffi_fill_i32(p, n, val)`) that writes through the pointer Raven
passed, proving Raven and C share the same memory. It then frees the buffer:

```
10
20
30
40
true
false
7
7
```

The first four lines are the values stored and loaded back. `true`/`false`
are `is_null` on the null pointer and on the live buffer. The trailing `7`s
are read back after the C `raven_ffi_fill_i32` filled the buffer with `7`.

## Small C structs by value

A small C struct can cross the FFI by value, both as an argument and as a
return. The Raven side declares a struct with C memory layout by marking it
`@repr(C)`:

```rust
@repr(C)
struct Point {
    x: CInt
    y: CInt
}

extern "C" {
    fun raven_ffi_point_sum(p: Point) -> CInt
    fun raven_ffi_translate(p: Point, dx: CInt, dy: CInt) -> Point
}

fun main() {
    let p = Point { x: 3, y: 4 }
    print(raven_ffi_point_sum(p))       // 7
    let q = raven_ffi_translate(p, 1, 2) // {4, 6}
    print(q.x)
    print(q.y)
}
```

### Layout rule

A `@repr(C)` struct has C layout: fields in declaration order, each at its
naturally aligned offset, and the total size rounded up to the struct's
alignment. `CInt` is 4-byte aligned and 4 bytes; `CLong`, `CSize`, `CStr`,
`CPtr<T>`, and `CFnPtr` are 8-byte aligned and 8 bytes. So `Point` above is
`{ x at 0, y at 4 }`, total 8 bytes. The fields are still readable on the
Raven side (`q.x`), since the value remains an ordinary heap struct; only
the call boundary marshals it by value.

### Supported shape (this release)

* Every field must be an **integer-class C scalar**: `CInt`, `CLong`,
  `CSize`, `CStr`, `CPtr<T>`, or `CFnPtr`. A float field (`CFloat`,
  `CDouble`) is rejected, because System V AMD64 classifies a floating
  field as SSE and would pass it in a different register class.
* The total C size must be **at most 8 bytes** (one machine register). A
  larger struct is rejected with a clear error; pass a `CPtr<...>` to the
  struct instead.
* A native `Int` literal initializes a `CInt`/`CLong`/`CSize` field
  (`Point { x: 3, y: 4 }`), the same coercion a C call applies.
* Only a `@repr(C)` struct may be handed to a C function by value; a plain
  heap struct (a GC pointer) is rejected at the call.

### ABI and marshaling

Both System V AMD64 (Linux, macOS) and Windows x64 pass an aggregate of at
most eight bytes whose members are all integer-class in a **single integer
register**, and return it in `RAX`. The two ABIs agree on this case, so a
struct of this shape marshals identically: the back end packs the fields
into one i64 (each field reduced to its C width and placed at its C byte
offset) and passes that i64 where the extern signature has the struct
parameter; a returned struct arrives as one i64 and is unpacked into a
fresh heap struct. The i64's byte image equals the struct's C memory image
on little-endian x86-64.

Cranelift's `StructArgument` purpose was not used: in this version it
passes the whole aggregate on the stack rather than applying the ABI
register classification, which is wrong for the common small-struct case on
both ABIs. The manual single-register packing above is correct for the
supported shape on both.

### Out of scope (struct by value)

* Structs larger than 8 bytes (the System V two-register path and the
  Windows x64 hidden-pointer path for 3, 5, 6, 7, or >8 byte structs).
* Floating-point fields (the System V SSE classification).
* Nested struct fields and struct fields of struct type.

## Out of scope

* A `free`/`drop` for buffers from `to_cstr` (copy-and-leak semantics).
  Buffers from `alloc` do have `free`.
* Capturing closures as callbacks (a userdata/trampoline mechanism beyond
  the non-capturing top-level functions supported here).
* The rest of what `docs/v2/specs/ffi.md` lists out of scope (variadics,
  non-CRT libraries).
```
