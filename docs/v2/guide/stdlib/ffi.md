# std/ffi

Bridges for the C foreign-function interface. `std/ffi` converts a Raven
`String` to and from a C `CStr`, and gives you a raw pointer API over
`CPtr<T>` for allocating, reading, and writing C memory. Use it alongside an
`extern "C"` block to call C functions with runtime values.

```rust
import std/ffi { to_cstr, from_cstr }

extern "C" {
    fun strlen(s: CStr) -> CSize
}

fun main() {
    let s = "hello"
    print(strlen(to_cstr(s)))     // 5
}
```

The C type set (`CInt`, `CLong`, `CSize`, `CStr`, `CPtr<T>`, `CFloat`,
`CDouble`) is built into the type checker. A `c"..."` literal already yields a
`CStr` at compile time; `to_cstr` covers the case of a `String` value computed
at runtime, which has no compile-time form.

## Importing

Bring in just the helpers you use:

```rust
import std/ffi { to_cstr, from_cstr }
import std/ffi { alloc, free, load, store, offset, is_null, null_ptr }
```

## Memory lives outside the GC

Read this before using `to_cstr` or `alloc`.

Both `to_cstr` and `alloc` hand back memory that the garbage collector does
**not** trace and will **never** reclaim. The lifetime is manual:

- `alloc<T>(...)` memory is yours to `free`. You **must** call `free` on it, or
  it leaks for the rest of the program run.
- `to_cstr(...)` copies into a fresh buffer that is valid for the rest of the
  run. There is no `free` helper for it in this release: each call leaks one
  buffer, so hoist the conversion out of hot loops.

The raw pointer access is unchecked, exactly like C. There are no bounds
checks and no use-after-free protection. An out-of-range `offset`, or a
`load`/`store` through a null or freed pointer, is undefined behavior. Use
`is_null` to guard a pointer a C API may return null, and keep the pointee
type `T` consistent with how the C side allocated the memory.

## String bridges

### `to_cstr(s: String) -> CStr`

Copy `s` into a fresh, null-terminated C string and return a `CStr` pointing
at the first byte. The result is a standalone copy (not the String's own
length-prefixed buffer), so it is a valid `const char *` that a C function may
read or retain after the call returns. Embedded NUL bytes are kept up to the
first one, at which a C reader stops.

### `from_cstr(p: CStr) -> String`

Read the null-terminated bytes at `p`, stopping at the first NUL, and build an
ordinary GC-managed Raven `String` (the terminator is dropped). The resulting
`String` is traced and reclaimed like any other.

```rust
import std/ffi { to_cstr, from_cstr }

fun main() {
    print(from_cstr(to_cstr("roundtrip")))    // roundtrip
}
```

Plain UTF-8 text without interior NUL bytes round-trips exactly. A `String`
with an embedded NUL truncates at that NUL on a round trip, since the C reader
stops there.

## Raw pointer and buffer access

`CPtr<T>` is a usable raw pointer. The following generic functions allocate,
release, and read or write C memory through it. The pointee type `T` must be a
C scalar (`CInt`, `CLong`, `CSize`, `CFloat`, `CDouble`, `CStr`) or a native
`Int`/`Float`; its width drives the load/store size and the element stride for
`offset`.

### `alloc<T>(count: Int) -> CPtr<T>`

Reserve room for `count` elements of type `T` and return a `CPtr<T>` to the
buffer. The memory is uninitialized. Returns the null pointer on a zero count
or allocation failure. The caller must `free` it.

### `free<T>(p: CPtr<T>)`

Release a buffer obtained from `alloc`. A null pointer is a no-op.

### `load<T>(p: CPtr<T>) -> T`

Read the element of type `T` at `p`.

### `store<T>(p: CPtr<T>, value: T)`

Write `value` to the element of type `T` at `p`.

### `offset<T>(p: CPtr<T>, count: Int) -> CPtr<T>`

Advance `p` by `count` elements, scaled by the size of `T`
(`p + count * sizeof(T)`). There is no indexed load in this release; compose
`load(offset(p, i))` to read element `i`.

### `is_null<T>(p: CPtr<T>) -> Bool`

True exactly when `p` is the null pointer. Many C APIs return null on failure,
so check before dereferencing.

### `null_ptr<T>() -> CPtr<T>`

The null `CPtr<T>`.

### Example: a buffer of `CInt`

```rust
import std/ffi { alloc, free, load, store, offset, is_null, null_ptr }

fun main() {
    let n = 3
    let buf = alloc<CInt>(n)
    if is_null<CInt>(buf) {
        return
    }

    // Store 10, 20, 30 at successive offsets.
    store<CInt>(buf, 10)
    store<CInt>(offset<CInt>(buf, 1), 20)
    store<CInt>(offset<CInt>(buf, 2), 30)

    // Read them back.
    print(load<CInt>(buf))                    // 10
    print(load<CInt>(offset<CInt>(buf, 1)))   // 20
    print(load<CInt>(offset<CInt>(buf, 2)))   // 30

    print(is_null<CInt>(null_ptr<CInt>()))    // true
    print(is_null<CInt>(buf))                 // false

    free<CInt>(buf)
}
```

## Calling a C function with a runtime String

Declare the C function in an `extern "C"` block, then pass a `to_cstr`
conversion of a runtime `String` where the signature expects a `CStr`:

```rust
import std/ffi { to_cstr, from_cstr }

extern "C" {
    fun strlen(s: CStr) -> CSize
    fun strcmp(a: CStr, b: CStr) -> CInt
}

fun main() {
    let s = "hello"
    print(strlen(to_cstr(s)))                     // 5
    print(strcmp(to_cstr("abc"), to_cstr("abc"))) // 0
    print(strcmp(to_cstr("abc"), to_cstr("abd"))) // -1
}
```

The integer FFI types print directly: `CInt`, `CLong`, and `CSize` satisfy
`ToString` by widening to `Int`, so a `CSize` or `CInt` result can be printed
or interpolated into a `"${...}"` string.

## See also

- The [language reference](../language-reference.md) for the FFI section:
  `extern "C"` blocks, the C type set, `c"..."` literals, `@repr(C)` structs by
  value, and `CFnPtr` callbacks.
- [std/string](string.md) for working with the `String` values you convert.
