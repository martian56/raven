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
`docs/v2/specs/ffi.md`; `CDouble` is added here.

| Raven type | C type         | Cranelift ABI type    |
|------------|----------------|-----------------------|
| `CInt`     | `int`          | `i32`                 |
| `CLong`    | `long`         | `i64`                 |
| `CSize`    | `size_t`       | pointer width (`i64`) |
| `CStr`     | `const char *` | pointer width (`i64`) |
| `CDouble`  | `double`       | `f64`                 |
| `CPtr<T>`  | `T *` (opaque) | pointer width (`i64`) |

`CDouble` is C `double`. It maps to `f64`, the exact representation a
Raven `Float` already crosses the C ABI as, so a `Float` argument is
accepted where a `CDouble` parameter is expected with no conversion at the
call boundary, and a `CDouble` return is an `f64`. A libm function such as
`sqrt(x: CDouble) -> CDouble` is callable with a `Float`.

`CString` is accepted as an alias for `CStr`.

### CFloat (deferred)

C `float` (`CFloat`, an f32) is not provided in this release. A Raven
`Float` is f64, so passing one to a C `float` parameter or reading a C
`float` return would require an `fdemote`/`fpromote` narrowing at the call
boundary that the codegen FFI path does not yet emit. Shipping the f32
type without that conversion would be a silently wrong ABI, so the variant
is deferred rather than half-wired. `CDouble` covers the common floating
case (libm and most C math take `double`).

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
    print_int(n)
    let eq = strcmp(to_cstr("abc"), to_cstr("abc"))
    print_int(eq)
    let lt = strcmp(to_cstr("abc"), to_cstr("abd"))
    print_int(lt)
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

The C integer results print through `print_int`, which accepts the integer
FFI types (`CInt`, `CLong`, `CSize`) and widens a narrower one to i64. The
surface has no `ToString` impl for the FFI integer types, so a `CSize` or
`CInt` cannot be interpolated into a `"${...}"` string or compared to a
native `Int` directly; route them through `print_int` to observe a value.

## Out of scope

* A `free`/`drop` for buffers from `to_cstr` (copy-and-leak semantics).
* C `float` (`CFloat`); deferred as noted above.
* Everything listed out of scope in `docs/v2/specs/ffi.md` (struct by
  value, variadics, callbacks into Raven, `CPtr<T>` dereference, non-CRT
  libraries).
```
