# Runtime Crate Spec

## Goal

`raven-runtime` is the small Rust crate that compiled v2 programs link
against at run time. The compiler back-end emits machine code that calls
into a fixed C ABI surface for heap allocation, panicking, and the few
intrinsic operations the back-end does not want to open-code (strings,
collections, I/O, process and network services, scheduling, and GC entry
points). The crate is therefore the
boundary between the static Raven world and the host operating system.

The crate exposes a stable C ABI, owns the heap object layouts and tracing
collector, runs the M:N goroutine scheduler, and provides the host-backed
operations used by the bundled standard library. The compiler links its
static library into every Raven executable.

## Pipeline position

```
Source -> Lexer -> Parser -> Resolver -> TypeChecker -> HIR -> MIR -> codegen -> object file
                                                                                    |
                                                                                    v
                                                                          link with raven-runtime
                                                                                    |
                                                                                    v
                                                                                executable
```

The runtime is invisible to every stage before codegen. It is also the
only crate other than the compiler itself that ships in the workspace.

## Crate layout

```
raven-runtime/
  Cargo.toml          rlib + staticlib, no dependencies
  src/
    lib.rs            C ABI exports, unit tests for header layout
    object.rs         ObjectHeader struct, tag constants, OBJECT_ALIGN
  tests/
    abi.rs            integration test that links the staticlib surface
```

The top-level `Cargo.toml` becomes a virtual workspace with two members:
the existing `raven` crate (compiler) at the root, and `raven-runtime`
under `raven-runtime/`. `[workspace.package]` carries the shared edition
and license; each member keeps its own `[package]`.

## C ABI surface

Every exported symbol is `#[no_mangle] pub extern "C"`. The list is
intentionally short. New symbols are added as later issues need them.

| Symbol | Signature | Contract |
|--------|-----------|----------|
| `raven_alloc` | `fn(size: usize, align: usize) -> *mut u8` | Returns a fresh allocation of `size` bytes aligned to `align`. Returns null on allocation failure. The current implementation forwards to `std::alloc::alloc` with a `Layout` built from the arguments. |
| `raven_dealloc` | `fn(ptr: *mut u8, size: usize, align: usize)` | Frees an allocation previously returned by `raven_alloc` with the same `size` and `align`. Passing a null pointer is a no-op. |
| `raven_panic` | `fn(msg_ptr: *const u8, msg_len: usize) -> !` | Writes the UTF-8 slice `msg_ptr[..msg_len]` to standard error with a `raven panic: ` prefix and a trailing newline, then exits the process with status 101 (Rust panic code). Does not return. |
| `raven_print_str` | `fn(ptr: *const u8, len: usize)` | Writes the byte slice to standard output without a trailing newline. |
| `raven_println_str` | `fn(ptr: *const u8, len: usize)` | Writes the byte slice to standard output followed by a single `\n`. |
| `raven_string_from_bytes` | `fn(ptr: *const u8, len: usize) -> *mut String` | Allocates a GC-managed `String` and copies `len` bytes into it. A zero `len` or null `ptr` yields an empty string. The back-end promotes static string literals into heap String values with this. |
| `raven_string_concat` | `fn(a: *const String, b: *const String) -> *mut String` | Allocates a fresh GC `String` whose bytes are the concatenation of `a` then `b`. Either input may be null (treated as empty). The interpolation concat chain folds through this. |
| `raven_int_to_string` | `fn(value: i64) -> *mut String` | Allocates a GC `String` with the base-ten rendering of `value`; negatives carry a leading `-`, zero renders `0`. |
| `raven_bool_to_string` | `fn(value: i8) -> *mut String` | Allocates a GC `String` of `true` or `false`; any nonzero `value` is `true`. |
| `raven_float_to_string` | `fn(value: f64) -> *mut String` | Allocates a GC `String` with the default `{}` rendering of `value` (so `7.0` renders `7`). |
| `raven_char_to_string` | `fn(value: u32) -> *mut String` | Allocates a GC `String` holding the single Unicode scalar `value`; an invalid code point renders the replacement character `U+FFFD`. |

All string-shaped entries take a raw pointer plus length so the codegen
back-end does not need to know any Rust slice layout. Raven `String` is a byte
buffer: only APIs that interpret text require UTF-8. The `raven_*_to_string` and
`raven_string_concat` constructors return GC pointers to `String`
objects (tag `TAG_STRING`) allocated through `raven_gc_alloc`, so the
collector traces and frees them like any other heap value.

## Object header layout

Every heap-allocated Raven object the GC walks begins with a fixed
header:

```rust
#[repr(C)]
pub struct ObjectHeader {
    pub tag: u32,      // discriminator: which kind of object this is
    pub gc_bits: u32,  // current GC mark epoch
    pub len: u32,      // logical length (string bytes, list elements, ...)
    pub cap: u32,      // capacity in elements; 0 when not applicable
}
```

Constraints:

* `size_of::<ObjectHeader>() == 16` on every supported target.
* `align_of::<ObjectHeader>() == 4`, but objects are allocated at
  `OBJECT_ALIGN = 8` so the payload after the header is aligned for any
  primitive Raven value.
* `tag` is written at allocation and read by diagnostics and by the collector
  to choose the object's tracing and buffer-freeing layout.
* `gc_bits` carries the non-zero mark epoch of the most recent collection that
  reached the object. Each collection uses a fresh process-wide epoch, avoiding
  a separate clear-marks pass. See `docs/v2/specs/gc.md`.

## Tag constants

| Constant | Value | Meaning |
|----------|-------|---------|
| `TAG_STRING` | `0x01` | Byte string. `len` is byte length, `cap` is allocated bytes. |
| `TAG_LIST` | `0x02` | Boxed `List<T>`. `len` is element count, `cap` is allocation slots. |
| `TAG_MAP` | `0x03` | Boxed `Map<K, V>`. `len` is entry count, `cap` is bucket count. |
| `TAG_SET` | `0x04` | Boxed `Set<T>`. `len` is entry count, `cap` is bucket count. |
| `TAG_CLOSURE` | `0x05` | Closure object: function pointer plus a tail of captures. |
| `TAG_BOX` | `0x06` | Generic heap box used by trait objects, reflection, and other boxed payloads. |

Constants live in `raven-runtime/src/object/mod.rs` and are re-exported from the
crate root. The Rust back end imports them directly so compiler and runtime
agree on every object tag.

## Out of scope

* Moving, generational, incremental, or concurrently marking collection.
* Async/await and an event-loop runtime; Raven's concurrency model is
  goroutines scheduled over an OS-thread pool.
* Sandboxing native calls. `extern "C"`, raw pointers, and host I/O carry the
  same trust and safety obligations as their underlying platform APIs.

## Test coverage

* Inline unit tests cover allocation, every object layout, tracing and
  cross-thread roots, stop-the-world coordination, scheduler/channel behavior,
  reflection, TLS configuration, and host-service registries.
* Integration tests under `raven-runtime/tests/` pin the exported ABI and
  object layouts and exercise collection through the crate boundary.
