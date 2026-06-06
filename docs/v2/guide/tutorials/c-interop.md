# Calling C from Raven

Raven talks to C through an `extern "C"` block: you declare the C function's
signature, then call it like an ordinary Raven function. The compiler emits a
direct C-ABI call and the linker resolves the symbol (the C runtime is always
on the link line; other libraries arrive with project link flags). This guide
walks through the FFI from the simplest call to structs by value, callbacks,
and variadics.

The runnable examples live under `examples/v2/` (`ffi_*.rv`); each is checked by
the golden suite, so the output shown here is exact.

## A first call

Declare the C function, then call it:

```rust
extern "C" {
    fun strlen(s: CStr) -> CSize
    fun abs(x: CInt) -> CInt
}

fun main() {
    print(strlen(c"hello")) // 5
    print(abs(-7)) // 7
}
```

`c"..."` is a C string literal: a static, null-terminated `const char *` with
no allocation. A native `Int` is accepted where an integer C type (`CInt`,
`CLong`, `CSize`) is expected, so `abs(-7)` checks without a cast.

## The C type set

| Raven | C | Notes |
|-------|---|-------|
| `CInt` | `int` | 32-bit |
| `CLong`, `CSize` | `long`, `size_t` | 64-bit |
| `CFloat`, `CDouble` | `float`, `double` | a native `Float` is accepted; `CFloat` narrows f64 to f32 |
| `CStr` | `const char *` | from a `c"..."` literal or `to_cstr` |
| `CPtr<T>` | `T *` | an opaque, typed pointer |
| `CFnPtr` | a function pointer | for callbacks |

## Passing a runtime String

A `c"..."` literal is fixed at compile time. To pass a `String` value, convert
it with `std/ffi`'s `to_cstr`, which copies into a fresh null-terminated
buffer. The buffer lives outside the garbage collector; release it with
`free_cstr` (or leave it for short-lived programs):

```rust
import std/ffi { to_cstr, free_cstr }

extern "C" {
    fun puts(s: CStr) -> CInt
}

fun main() {
    let msg = "hello, " .concat("world")
    let c = to_cstr(msg)
    let _ = puts(c)
    free_cstr(c)
}
```

## Structs by value

A struct marked `@repr(C)` crosses the C ABI by value, both as an argument and
as a return value. Its fields must be C scalars (or nested `@repr(C)` structs):

```rust
@repr(C)
struct Point {
    x: CDouble,
    y: CDouble,
}

extern "C" {
    fun hypot(x: CDouble, y: CDouble) -> CDouble
    fun length(p: Point) -> CDouble // a C function taking Point by value
}
```

The back end follows each platform's ABI: small structs travel in registers
(integer or SSE), and larger ones in memory or by reference, with a hidden
return pointer where the ABI calls for one. There is no size limit, and a field
may itself be a nested `@repr(C)` struct, whose bytes are inlined. A struct
literal accepts native `Int`/`Float` for its C-scalar fields:
`Point { x: 1.0, y: 2.0 }`. See `examples/v2/ffi_struct_*.rv`.

## Callbacks

A Raven function or closure can be passed where a C `CFnPtr` is expected, so a
C API can call back into Raven. A **top-level function** passes directly:

```rust
extern "C" {
    fun qsort(base: CPtr<CInt>, n: CSize, size: CSize, cmp: CFnPtr)
}

fun compare(a: CPtr<CInt>, b: CPtr<CInt>) -> CInt { ... }

// qsort(buf, 5, 4, compare)
```

A **capturing closure** also works, through a generated trampoline. Because the
closure carries an environment, you pass it to the C function's callback slot
*and* to its `userdata` slot (a `CPtr<Unit>`); C threads the closure back to the
trampoline, which invokes it. This is the userdata-last convention (for example
glibc `qsort_r`):

```rust
extern "C" {
    fun apply_cb(cb: CFnPtr, data: CPtr<Unit>, x: CLong) -> CLong
}

fun run(base: CLong) {
    let add = fun(x: CLong) -> CLong = x + base // captures `base`
    print(apply_cb(add, add, 5)) // add -> trampoline; add -> userdata
}
```

The callback's parameters and return must be C types. A callback that allocates
is safe: the collector traces the suspended Raven stack across the C call. A C
API whose userdata is not the last callback argument (or that has none) needs a
small C shim. See `examples/v2/ffi_callback_closure.rv`.

## Variadic functions

An `extern` signature ending in `...` is variadic; a call may pass extra
arguments after the fixed ones:

```rust
extern "C" {
    fun printf(fmt: CStr, ...) -> CInt
}

fun main() {
    let _ = printf(c"%d items, %s\n", 3, c"ok")
}
```

Each variadic argument must be a C integer or pointer type (or a native `Int`).
**Float variadic arguments are rejected at compile time**, the backend cannot
honor the platform's variadic float rules, so a `%f` format needs a fixed-arity
C shim.

## Raw pointers

`std/ffi` wraps manual allocation and unchecked pointer access for buffers a C
API reads or writes. The memory is outside the GC and is yours to `free`:

```rust
import std/ffi { alloc, store, load, free }

fun main() {
    let buf: CPtr<CInt> = alloc<CInt>(3)
    store(buf, 10)
    store(offset(buf, 1), 20)
    print(load(buf)) // 10
    free(buf)
}
```

This is unchecked, exactly like C: no bounds checks, no use-after-free
protection. Guard a possibly-null pointer with `is_null`.

## Platform support and linking

Raven ships for Linux and Windows x86_64. The C runtime (the CRT, including the
`printf` family) is always on the link line; symbols from other libraries
arrive through project link flags. See `docs/v2/specs/ffi.md` and
`docs/v2/specs/std-ffi.md` for the full ABI rules and out-of-scope notes.
