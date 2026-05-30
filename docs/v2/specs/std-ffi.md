# std/ffi Spec

Companion module for the C foreign-function interface. The C type set is
built into the type checker (see `docs/v2/specs/ffi.md`); this module adds
the runtime conversions between a Raven `String` and a C `CStr`, which the
compiler FFI layer deliberately left out. A `c"..."` literal already
yields a `CStr` at compile time, but a `String` value computed at runtime
needs a conversion, which is what `to_cstr` and `from_cstr` provide.

## Import

```raven
import std/ffi { to_cstr, from_cstr }
```

## Surface

```raven
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

## C string literals

A `c"..."` literal types as `CStr` and lowers to a static, read-only,
null-terminated buffer with no allocation and no runtime call. Use it for
compile-time-known strings; use `to_cstr` for a `String` value. See
`docs/v2/specs/ffi.md` for the literal's codegen.

## libc verification

`examples/v2/use_ffi.rv` converts runtime Strings and calls libc:

```raven
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

## Out of scope

* A `free`/`drop` for buffers from `to_cstr` (copy-and-leak semantics).
* Everything listed out of scope in `docs/v2/specs/ffi.md` (struct by
  value, variadics, callbacks into Raven, `CPtr<T>` dereference, non-CRT
  libraries).
```
