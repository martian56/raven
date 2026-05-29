# Raven v2 Standard Library Charter

This document is the authoritative list of standard-library modules for Raven v2, with the rationale behind the selection. It governs which functionality ships in `std` versus which lives in rvpm packages.

## Philosophy

Raven ships packages directly from GitHub through rvpm, so the standard library can stay curated and production-grade rather than sprawling. The model is Go and Rust: a focused core that covers the needs of most programs, with niche, heavy, security-sensitive, or fast-moving functionality delivered as packages.

Concretely:

- **Curated, not exhaustive.** If most programs need it, it goes in `std`. Otherwise it is a package. This avoids the dead-battery problem that accreted in some older standard libraries.
- **Method-first OOP.** Operations are methods on their receiver type (`s.trim()`, `list.map(f)`), defined with `impl` blocks, including `impl` on built-in types. Free functions are used only where there is no natural receiver.
- **Lazy, zero-cost iterators.** Collection pipelines (`list.iter().map(f).filter(g).collect()`) run in a single pass with no intermediate allocation, monomorphized to a tight loop.
- **Trait-polymorphic.** A small prelude of core traits (`ToString`, `Eq`, `Ord`, `Hash`, `Iterator`, `Clone`, `Default`) makes the library generic and consistent.
- **Result-based.** Fallible operations return `Result<T, E>`. No exceptions, no sentinel values, no nulls.

## What the parent languages contribute

| Language | Adopted | Rejected |
|---|---|---|
| Go | Curated production-grade modules, `net/http` in std, `time` design, one obvious way | nothing significant |
| Rust | Small sharp core, `Iterator`, `Result`, `Option`, traits, package-first for niche | an std too minimal to feel batteries-ready |
| Python | Breadth of common needs (json, csv, datetime, random, regex) | dead-battery sprawl, dynamic typing |
| Java | `java.time` date/time design, streams, strong collections | verbosity, checked exceptions |
| C++ | Container and algorithm quality, `chrono`, `<format>` | header model, manual memory |

## Prelude: `std/core` (auto-imported)

Every module implicitly imports the prelude.

- Types: `Option<T>`, `Result<T, E>`
- Traits: `ToString`, `Eq`, `Ord`, `Hash`, `Iterator`, `Clone`, `Default`
- Functions: `panic`, `assert`, `assert_eq`

## Core modules (v2.0)

| Module | Contents | Primary inspiration |
|---|---|---|
| `std/io` | stdin, stdout, stderr, `print`/`println` over `ToString`, `read_line`, `eprintln`, flush | Go fmt, Rust io |
| `std/string` | `String` methods, `StringBuilder`, parse and format, split and join | Go strings and strconv, Python str |
| `std/collections` | `List<T>`, `Map<K, V>`, `Set<T>`, `Deque<T>`; ordered variants later | Rust collections, Java util |
| `std/iter` | `Iterator` trait, lazy adapters (map, filter, fold, take, skip, zip, enumerate, chain), ranges | Rust iter |
| `std/math` | constants, abs, min, max, clamp, sqrt, pow, exp, log, trig, rounding | all |
| `std/cmp` | ordering helpers, sorting a `List` by `Ord` or comparator, min, max | Go sort, Rust cmp |
| `std/fmt` | format strings, debug formatting, `ToString` derivation | Rust fmt, C++ format |
| `std/error` | `Error` trait, context, chaining, `?` ergonomics | Rust error, Go errors |
| `std/test` | assertions and the test harness | Go testing, Rust test |

## System and data modules (v2.0)

| Module | Contents | Primary inspiration |
|---|---|---|
| `std/time` | UTC timestamps, `Date`, `Time`, `DateTime`, strftime format and parse, sleep | Java java.time, Go time |
| `std/fs` | files, directories, metadata, read and write, all `Result` | Go os, Rust fs |
| `std/path` | join, split, normalize, extension | Go path/filepath, Python pathlib |
| `std/env` | args, environment variables, exit, platform info | Go os, Rust env |
| `std/process` | spawn subprocesses, pipes, exit codes | Go os/exec, Rust process |
| `std/random` | RNG, ranges, shuffle, choice | Python random, Go math/rand |
| `std/json` | parse and serialize, `JsonValue` sum type, trait-based ser and de | Go encoding/json |
| `std/encoding` | base64, hex, utf8, csv | Go encoding |
| `std/hash` | non-crypto hashing (FNV, xxHash) backing the `Hash` trait, checksums | Rust hash, Go hash |
| `std/net` | TCP, UDP, addresses, DNS | Go net, Rust net |
| `std/http` | HTTP client plus a minimal server | Go net/http |
| `std/regex` | regular expressions | Go regexp, Python re, Java regex |
| `std/ffi` | `CStr`, `CInt`, `CPtr<T>`, conversions | C interop |

## Deferred to v2.x

Concurrency is its own milestone; v2.0 is single-threaded.

- `std/sync` (Mutex, RwLock, Once)
- `std/thread` (spawn, join)
- `std/channel` (message passing)
- `std/atomic` (atomic primitives)

A lazy/async runtime, if added, is also a v2.x discussion.

## Deliberately packages, not std

Delivered through rvpm rather than the standard library, because they are security-sensitive, fast-moving, opinionated, or niche:

- Cryptography and TLS signing (sha2, hmac, ed25519, and so on)
- Compression (gzip, zstd, brotli)
- Serialization beyond JSON (YAML, TOML, msgpack, protobuf)
- Database drivers
- Web frameworks, routers, template engines, ORMs
- GUI, graphics, audio
- Async runtimes (until v2.x concurrency lands)

A frictionless GitHub-direct package system is exactly the right home for these.

## Compiler foundations required first

The method-first, trait-based, lazy-iterator design depends on compiler capabilities that are built before the modules that use them:

1. `impl` blocks on built-in types (so the stdlib can attach methods to `String`, `List<T>`, and the primitives).
2. Capturing closures and closure-value invocation (so iterator adapters can store and call closures).
3. The `std/core` trait prelude (so the library is polymorphic).
4. The lazy iterator pipeline (`Iterator` trait plus adapters).

Implementation proceeds in that dependency order, then the modules above, then the system and data modules.

## Build order summary

1. Compiler foundations: methods on built-ins, capturing closures, core trait prelude, lazy iterators.
2. Convert the already-shipped `std/io` and `std/string` to the method-first API.
3. Core modules: collections, math, cmp, fmt, error, test.
4. System and data modules: time, fs, path, env, process, random, json, encoding, hash, net, http, regex.
5. v2.x: concurrency.
