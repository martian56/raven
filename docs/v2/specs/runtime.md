# Runtime Crate Spec

## Goal

`raven-runtime` is the small Rust crate that compiled v2 programs link
against at run time. The compiler back-end emits machine code that calls
into a fixed C ABI surface for heap allocation, panicking, and the few
intrinsic operations the back-end does not want to open-code (string
printing, eventually GC entry points). The crate is therefore the
boundary between the static Raven world and the host operating system.

This document specifies the scaffolding shape only. The crate is created
as a workspace member, exposes its symbols, and pins object header
constants the back-end and later GC work will both depend on. The real
allocator and the tracing collector arrive in follow-up issues (#64 and
#65), and trait object dispatch lands with #66.

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
| `raven_print_str` | `fn(ptr: *const u8, len: usize)` | Writes the UTF-8 slice to standard output without a trailing newline. |
| `raven_println_str` | `fn(ptr: *const u8, len: usize)` | Writes the UTF-8 slice to standard output followed by a single `\n`. |

All string-shaped entries take a raw pointer plus length so the codegen
back-end does not need to know any Rust slice layout. The bytes are
assumed valid UTF-8; the runtime does not re-validate them, mirroring
the type-checker invariant.

## Object header layout

Every heap-allocated Raven object the GC walks begins with a fixed
header:

```rust
#[repr(C)]
pub struct ObjectHeader {
    pub tag: u32,      // discriminator: which kind of object this is
    pub gc_bits: u32,  // mark / colour bits reserved for the future collector
    pub len: u32,      // logical length (string bytes, list elements, ...)
    pub cap: u32,      // capacity in elements; 0 when not applicable
}
```

Constraints:

* `size_of::<ObjectHeader>() == 16` on every supported target.
* `align_of::<ObjectHeader>() == 4`, but objects are allocated at
  `OBJECT_ALIGN = 8` so the payload after the header is aligned for any
  primitive Raven value.
* `tag` is read by `raven_panic` (when a debug build wants to print the
  object kind) and by the GC once it lands. The current scaffold writes
  it once at allocation time and never inspects it.
* `gc_bits` is reserved. The scaffold leaves it zero. Issue #64 defines
  the bit layout.

## Tag constants

| Constant | Value | Meaning |
|----------|-------|---------|
| `TAG_STRING` | `0x01` | UTF-8 string. `len` is byte length, `cap` is allocated bytes. |
| `TAG_LIST` | `0x02` | Boxed `List<T>`. `len` is element count, `cap` is allocation slots. |
| `TAG_MAP` | `0x03` | Boxed `Map<K, V>`. `len` is entry count, `cap` is bucket count. |
| `TAG_SET` | `0x04` | Boxed `Set<T>`. `len` is entry count, `cap` is bucket count. |
| `TAG_CLOSURE` | `0x05` | Closure object: function pointer plus a tail of captures. |
| `TAG_BOX` | `0x06` | Generic heap box. Reserved for trait-object payloads (#66). |

Constants live in `raven-runtime/src/object.rs` and are re-exported from
the crate root. Code generation reads them through the C ABI eventually,
but for now the compiler imports them directly because the back-end is
also Rust.

## Out of scope for this PR

* Real allocator behaviour. The current `raven_alloc` is `std::alloc`
  passthrough. A bump or slab allocator is issue #64.
* Garbage collection. `gc_bits` is reserved but unused.
* Full object layouts for `String`, `List`, `Map`, `Set`, `Closure`.
  This scaffold fixes only the shared header; the per-kind layouts land
  with issue #65 and are documented in `docs/v2/specs/object-layout.md`.
* Trait object dispatch tables. The `BOX` tag exists as a placeholder;
  the actual vtable shape lands in issue #66.
* Async, threading, FFI safety beyond the stated symbols.

## Test coverage

* Inline unit tests in `raven-runtime/src/lib.rs`:
  * `ObjectHeader` size is 16 bytes.
  * `ObjectHeader` alignment divides `OBJECT_ALIGN`.
  * Round-tripping an allocation through `raven_alloc` and
    `raven_dealloc` does not abort.
  * `raven_println_str` accepts an empty slice without panic.
* Integration test `raven-runtime/tests/abi.rs`:
  * Calls the four allocation-shaped entry points across an
    allocate-write-deallocate cycle.
  * Verifies that the staticlib actually links from a separate crate
    boundary (catches `crate-type` regressions).
