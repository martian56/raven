# Changelog

All notable changes to Raven are documented in this file.

## [2.18.30] - 2026-06-10

### Fixed

- A char literal inside a `${...}` interpolation can now hold a brace or a quote (`"${f('}')}"`). The lexer and the interpolation splitter both skip a char literal, so its contents are no longer read as interpolation structure (#419).

## [2.18.29] - 2026-06-10

### Fixed

- A heap return value is now kept rooted across an allocating `defer`. A function that returned a heap value (a String, list, struct, ...) and had a `defer` that allocated could have its return value collected by the defer's allocation, a use-after-free at the return (#438).

## [2.18.28] - 2026-06-10

### Fixed

- `rvpm` rejects a dependency path or version that contains `..`, a path separator, or a `.` segment, closing a cache path-traversal gap before any directory is created (#431).

## [2.18.27] - 2026-06-10

### Fixed

- `rvpm init` no longer overwrites an existing `src/main.rv`; it adds the manifest and keeps the source. The lock file's tree hash now also covers symlinks (a regular-file tree hashes the same as before, so existing lock files stay valid) (#435).

## [2.18.26] - 2026-06-10

### Fixed

- `rvpm fmt` now honors `[fmt].indent_width` from `rv.toml` (it previously loaded no manifest and always used four spaces). `wrap_width` is still read but has no effect, because the formatter does not reflow long lines yet (#432).

## [2.18.25] - 2026-06-10

### Fixed

- An escaped `\$` now stays a plain `$` in a c-string and in a string match pattern. The internal escape sentinel was leaking into both because neither runs through the interpolation splitter (#418).

## [2.18.24] - 2026-06-10

### Fixed

- A generic instantiation (`List<Int>`, `Box<Int>`) used where a trait bound requires a trait its constructor never implements is now a clear type error, instead of slipping through to code generation as an unresolved callee. A type whose constructor does have an impl is still left to the call site, so valid generic code is unaffected (#411).

## [2.18.23] - 2026-06-10

### Fixed

- A trait with a method that returns `Self` is now correctly rejected as not object-safe when used as a `dyn` type, with a clear error, instead of being accepted and miscompiling (#412).

## [2.18.22] - 2026-06-10

### Fixed

- CRLF line endings no longer corrupt string literals. A CRLF inside a single-line string is now reported as unterminated (it was folded to a stray carriage return), and a block string normalizes CRLF to LF so its content is the same on any platform (#415).

## [2.18.21] - 2026-06-10

### Fixed

- A numeric literal with a digit invalid for its base or a trailing letter (`0b12`, `0o78`, `123abc`) is now a single malformed-literal error rather than being silently split into two tokens (#421).

## [2.18.20] - 2026-06-10

### Fixed

- A method chain can now break across lines with the dot leading the next line (`s\n    .trim()\n    .to_lower()`). The postfix parser continues across a newline when a `.` or `?` follows it (#416).

## [2.18.19] - 2026-06-10

### Fixed

- A line that begins with `+` or `-` is now a new statement rather than a silent continuation of the line above, so `foo()` followed by `-1` on the next line no longer parses as `foo() - 1`. An operator at the end of a line still continues the expression (#414).

## [2.18.18] - 2026-06-10

### Fixed

- A struct literal is now accepted inside a call argument within an `if`/`while`/`for`/`match` head (`if check(Point { x: 1 }) {`). The no-struct-literal rule that keeps the condition unambiguous is lifted inside the call's parentheses (#417).

## [2.18.17] - 2026-06-10

### Fixed

- A leading UTF-8 byte-order mark is ignored instead of failing with an "unexpected character" error, so a source file saved with a BOM compiles (#422).

## [2.18.16] - 2026-06-10

### Fixed

- `std/fs.walk` no longer follows symlinked directories, so a link that points back up the tree can no longer send it into endless recursion. Adds `fs.is_symlink` (#436).

## [2.18.15] - 2026-06-10

### Fixed

- `std/json` no longer emits `inf` or `NaN` (which are not valid JSON); a non-finite number is written as `null`. A huge exponent like `1e999999999` is now applied in bounded time instead of looping a billion times (#427).

## [2.18.14] - 2026-06-10

### Fixed

- The `std/json` parser bounds nesting depth (256 levels) and returns an error instead of overflowing the stack on deeply nested untrusted input (#425).

## [2.18.13] - 2026-06-10

### Fixed

- The `std/http` static file server rejects a requested file name that contains a path separator (including a Windows backslash), a parent reference, or a drive marker, closing a path-traversal hole that could escape the served directory (#423).

## [2.18.12] - 2026-06-10

### Fixed

- The `std/http` server caps the request body at 10 MiB and rejects a negative or oversized `Content-Length` with a 400 before reading, so a client can no longer make it buffer unbounded memory (#428).

## [2.18.11] - 2026-06-10

### Fixed

- `std/http` strips CR and LF from response header names and values, closing a header-injection / response-splitting hole when a handler sets a header from untrusted input (#424).

## [2.18.10] - 2026-06-10

### Fixed

- `std/math` integer helpers no longer return a silently wrong value on overflow. `abs_int(i64::MIN)`, and an overflowing `pow_int` or `lcm`, now panic with a message instead of wrapping (#433).

## [2.18.9] - 2026-06-10

### Fixed

- `String.parse_int` reports `None` on overflow instead of wrapping to a wrong value. It accumulates in the sign's own direction, so it still parses the whole i64 range including `i64::MIN` (#429).

## [2.18.8] - 2026-06-10

### Fixed

- A shift count at or beyond the bit width is now well defined. `1 << 65` gave `2` because the hardware masked the count to its low bits; a left shift past the width now yields `0` and an arithmetic right shift collapses to the sign (#439).

## [2.18.7] - 2026-06-10

### Fixed

- A `match` arm now tells a constructor from a binding by the scrutinee's real variant names rather than by letter case. A lowercase enum variant is treated as a constructor (so omitting one is a non-exhaustive match instead of a silently accepted wrong dispatch), and an uppercase binding stays a binding. The int/bool/char match path also now binds a binding arm to the scrutinee value, which previously read a garbage slot (#410).

## [2.18.6] - 2026-06-10

### Fixed

- Integer divide and modulo by zero, and the `i64::MIN / -1` overflow, now raise a Raven panic with a message instead of aborting with a raw hardware exception (#437).

## [2.18.5] - 2026-06-10

### Fixed

- The monomorphizer no longer hangs on a generic function that instantiates itself at an ever larger type (`fun f<T>(x: T) { f(wrap(x)) }`). It now stops at a nesting depth of 128 and reports a clear error instead of looping forever (#443).

## [2.18.4] - 2026-06-10

### Fixed

- An inclusive range loop (`for i in a..=b`) to `i64::MAX` no longer runs forever. The counter was incremented past `end`, which wrapped to `i64::MIN`; the loop now stops once it reaches `end` (#444).

## [2.18.3] - 2026-06-10

### Fixed

- A closure, `defer`, or `spawn` body inside a method can now refer to `self`. It was not captured, so the lifted body had no `self` and the build failed (#440).

## [2.18.2] - 2026-06-10

### Fixed

- A binding arm in a `match` over a `String`, `Float`, or struct scrutinee no longer loses the value. The bound name was stored in a Unit slot, so it came back empty inside the arm; it now keeps the scrutinee's real type (#442).

## [2.18.1] - 2026-06-10

### Fixed

- `&&` and `||` now short-circuit. The right operand is only evaluated when the left does not already decide the result, so a guard like `i < xs.len() && xs[i] == x` no longer runs the index when the bounds check fails (#441).

## [2.18.0] - 2026-06-08

### Added

- `Request.json<T: FromJson>()` in `std/http` decodes a request body straight into a struct: `req.json<User>()` or `let u: User = req.json()?`. It is enabled by the return-only generic method inference from 2.17.1 and is equivalent to the free `decode<User>(req.body)`.

### Changed

- `Request.json()` is now the typed decoder above. The ad-hoc form that parses the body into a `JsonValue` is now `Request.json_value()`. (The previous `Request.json() -> Result<JsonValue, Error>` shipped in 2.17.0.)

## [2.17.1] - 2026-06-08

### Fixed

- A generic method whose type parameter appears only in the return type (for example `fun decode<T: FromJson>(self) -> Result<T, Error>`) can now be called. The explicit form `recv.decode<T>()` was ignoring the type argument, and the annotated form `let n: T = recv.decode()` was monomorphizing the parameter to `Unit`. The type checker now applies a method call's explicit type arguments, and MIR lowering matches the declared return type against the call's resolved result type so the parameter is recovered for monomorphization (#384).

## [2.17.0] - 2026-06-08

### Added

- JSON and static file serving in `std/http`. `Response.json(value)` serializes any value whose type implements `ToJson` (a `@derive(ToJson)` struct or enum, a `List`, an `Option`, a scalar) with `Content-Type: application/json`, replacing hand-built JSON strings and manual escaping; `Response.json_raw(body)` keeps the pre-rendered-JSON form. `std/json.decode<T: FromJson>(body)` parses and decodes a request body into a struct in one call, and `Request.json()` returns the body as a `JsonValue`. `Response.file(path)` serves a file with a `Content-Type` chosen from its extension (404 if missing), and `Server.static(prefix, dir)` mounts a directory of files (#386).

### Fixed

- The type checker no longer panics when it finalizes a function body after an earlier body left an unresolved inference variable. It shared one type map across all bodies but resolved the whole map with the current body's inference context, crashing on a variable another body owned. It now resolves only each body's own entries, so a generic call whose type argument cannot be inferred reports a clear error instead.
- `rvpm build` and `rvpm run` no longer overflow the stack on deeply recursive compilation, for example a program that uses `@derive(ToJson)`. rvpm runs the compiler on a worker thread with a generous stack, matching the `raven` CLI (#388).

## [2.16.0] - 2026-06-08

### Fixed

- Generic bounds are now enforced on a fully inferred type, not only on a type written in a declaration. A binding such as `let m = Map.new()` later used with a key type that lacks `Eq`/`Hash` is rejected at type-check, instead of reaching the back end as an unresolved `K$hash` callee. The inference context verifies a pending bound the moment its variable resolves to a simple concrete type. Fixing this also corrected a latent bound-attachment bug: the associated-function and method candidate loops unified each impl's self type before checking the impl provided the called method, so a rejected impl such as `impl Eq for Map<K, V: Eq>` (which has no `new`/`set`) leaked an `Eq` requirement onto a map's value type (#375).
- The runtime no longer holds the global network registry lock across a blocking socket syscall. `accept`, `read`, and `write` now clone the socket handle under the lock and run the blocking call without it, so goroutines doing network IO on different sockets run concurrently instead of serializing behind a connection that has not yet arrived. As a result, `std/http`'s `Server` handles each connection on its own goroutine again: a slow handler no longer holds up the clients behind it, and single, spaced, and parallel requests are all served reliably (#377).

## [2.15.0] - 2026-06-08

### Added

- HTTP server in `std/http` (#378), written entirely in Raven on top of `std/net` (TCP), so it needs no new runtime code. Build a routing table on a `Server` and call `listen`. The surface is typed and ergonomic: a `Method` enum; a `Request` with the method, path, lowercased headers, captured `:name` path params, a decoded query map, and the body, plus `header`/`param`/`query_value` accessors; a `Response` with constructors (`ok`, `text`, `html`, `json`, `created`, `no_content`, `not_found`, `bad_request`, `server_error`, `redirect`) and chaining builders (`header`, `content_type`, `status_code`); and a `Server` with `get`/`post`/`put`/`delete`/`patch`/`route` registration. Handlers are `fun(Request) -> Response` values. Routes match in registration order, `:name` segments capture, and a non-match falls through to a 404. Connections are served one at a time; per-connection concurrency waits on a scheduler fix (#377).

### Fixed

- A method that uses `self` but does not declare it as a parameter now reports a clear resolve error pointing at the use, with a help to add `self`, instead of the opaque back-end error `field base used a Unit value` (#372).
- Generic bounds are now enforced on types written in declarations. A `Map<K, V>` whose key type lacks `Eq`/`Hash` (or any generic instantiation that violates its declaration's bounds) is rejected at type-check with a `does not implement` error, instead of surfacing as an unresolved `K$hash` callee at codegen. Covers struct fields, enum payloads, function parameters and returns, impl methods, and explicit `let`/`const` annotations (#374).

## [2.14.0] - 2026-06-07

### Added

- Synchronization primitives in `std/sync`, completing the concurrency toolkit over the parallel scheduler (#212). `Mutex` (`mutex()`, `lock`/`unlock`) guards shared mutable state across goroutines, built on a one-slot channel token. `WaitGroup` (`wait_group()`, `add`/`done`/`wait`) waits for a set of goroutines to finish. `sleep_millis(ms)` parks a goroutine for a duration. The wait-group and sleep leave the collector's running set while parked, so a concurrent collection is never blocked on them (#241).
- `select_recv(channels)` in `std/sync` receives from whichever of several channels has a value first, returning a `SelectResult { index, value }` that names the channel (by position) and the value. The common recv-select for fan-in or waiting on data versus a cancellation signal; ties go to the lowest index (#239).
- Blocking IO no longer stalls a collection. A goroutine parked in a blocking runtime call (`std/net` connect/accept/read/write, `std/http` request, `std/process` wait, `std/fs` read/write/append, or a stdin read) now runs the syscall outside the collector's running set, so a stop-the-world collection another goroutine triggers proceeds instead of freezing every goroutine for the length of the call. The M:N pool already kept other goroutines running on other workers during the call (#240).

## [2.13.0] - 2026-06-07

### Added

- **Goroutines now run in parallel across CPU cores.** The scheduler is M:N: goroutines (stackful coroutines) are multiplexed onto a pool of worker OS threads, one per available core, so a spawned goroutine makes progress concurrently with the code that spawned it rather than only when that code yields. `main` runs on its own thread and parks when it blocks on a channel; a spawned goroutine suspends its coroutine so its worker runs another. Channel `send`/`recv` keep their blocking semantics and are coordinated without lost wakeups; the all-goroutines-blocked deadlock detector works across workers. The language surface (`spawn`, `channel`/`channel_buffered`, `yield_now`) is unchanged, so existing concurrent programs gain parallelism without edits (#212, #237).
- A **shared-heap stop-the-world garbage collector** that runs alongside parallel goroutines. Each OS thread sweeps its own heap; a cross-thread root registry surfaces every thread's live roots so an object one thread holds survives a collection another triggers; the object header's mark field is reinterpreted as a per-collection epoch. Compiled code reaches safepoints at allocations and loop back-edges, where a stop-the-world collection parks it with a complete shadow stack; threads blocked or not running compiled Raven are scanned without being waited for. A 100-run-per-build concurrency soak test exercises the scheduler and collector under constant collection.

## [2.12.0] - 2026-06-06

### Added

- Declarative macros gained the two remaining features of their design: **nested repetition** (a `$( ... )` group inside another, binding a metavariable to a sequence of sequences and splicing each level in turn) and **full referential hygiene** (a free identifier a macro template names, such as a function it calls, resolves at the macro's definition site, the module scope, so a call-site local of the same name cannot capture it). The expander marks each free template identifier's span and the resolver resolves marked identifiers against the module scope, skipping call-site locals; a new HIR `FnRef` node keeps a resolved function callee a direct call so the binding is not re-shadowed during lowering. This completes the declarative macro system; procedural macros remain a follow-up (#215).

## [2.11.0] - 2026-06-06

### Added

- FFI: `std/ffi` gains `free_cstr(p: CStr)`, which releases a buffer returned by `to_cstr`. The `to_cstr` buffer is now `malloc`-allocated so it can be freed (a null pointer is a no-op); previously it could only leak for the program's lifetime. A new "Calling C from Raven" tutorial walks through the whole FFI: extern declarations, the C type set, runtime-String conversion, `@repr(C)` structs by value, callbacks (including capturing closures), variadics, and raw pointers (#213).
- `Eq` for the built-in generic and collection types, so `==`/`!=` compare them by value: `Option<T>`, `Result<T, E>`, and `List<T>` (in std/core, always available), and `Set<T>` and `Map<K, V>` (in std/collections; Set and Map compare order-independently). An element type must itself implement `Eq`, which the bound requires. A follow-up to the 2.10.2 operator fix (#340): previously these compared by object identity, so `Some(1) == Some(1)` and `[1, 2] == [1, 2]` were `false`; they are now `true` (#342).

## [2.10.2] - 2026-06-06

### Fixed

- `==` and `!=` on a struct or enum that implements `Eq` (including via `@derive(Eq)`) now compare by value instead of by object identity. Previously the operators compared the operands' heap pointers, so two equal values (for example `Status.Doing == Status.Doing`, or two structs with equal fields) compared unequal even though the derived `equals` method itself was correct. HIR lowering now rewrites the operator to a call to the type's `equals` method (the same way `print` routes a value through `to_string`); a primitive keeps the native compare, a `String` keeps its byte-equality path, and a type with no `Eq` impl is unchanged.

## [2.10.1] - 2026-06-06

### Changed

- Dropped macOS as a release target. Raven now ships for Linux x86_64 and Windows x86_64. The macOS arm64 build was removed because the Apple arm64 ABI passes variadic arguments on the stack, which the Cranelift backend cannot express, so a variadic C call (added in 2.10.0) crashed on that platform. On the remaining x86_64 targets variadic arguments go in registers and work correctly. The AArch64 code paths stay in the compiler but are no longer built, shipped, or supported.

## [2.10.0] - 2026-06-06

### Added

- FFI: variadic C functions can now be called. A signature ending in `...` (`fun printf(fmt: CStr, ...) -> CInt`) accepts extra arguments after the fixed parameters; each must be a C-FFI integer or pointer type (or a native `Int`). The back end builds a signature from the actual arguments at each call site and dispatches through `call_indirect`. Float variadic arguments are rejected at compile time, because the Cranelift backend cannot set the System V `al` register or apply the Windows x64 float-shadow rule; a `%f` format needs a fixed-arity C shim. On windows-msvc the `printf` family now links via `legacy_stdio_definitions.lib` (#330).

## [2.9.0] - 2026-06-06

### Added

- FFI: `@repr(C)` structs of any size now cross the C ABI by value, removing the previous 16-byte limit. Up to 16 bytes a struct still crosses in registers; a larger one crosses in memory on the stack on System V AMD64 (the MEMORY class, via Cranelift's `StructArgument` with the size rounded up to 8 bytes) or by reference on Windows x64 and AArch64, and is returned through a hidden `sret` pointer on every target (#327).

## [2.8.0] - 2026-06-06

### Added

- FFI: a capturing Raven closure (a lambda or a captured local) can now be passed as a C callback, not just a non-capturing top-level function. The closure is given to the C function's callback-pointer parameter, where the compiler emits a generated trampoline whose last argument is a `userdata` pointer, and to the function's userdata parameter (a `CPtr`), which is the closure object C threads back to the trampoline. This follows the userdata-last convention (for example glibc `qsort_r`); a userdata-first or no-userdata API needs a small C shim. Because the GC shadow stack persists across a C call, a callback that allocates is traced correctly, the golden suite exercises one allocating on every call across a thousand C-driven invocations (#234).

## [2.7.0] - 2026-06-06

### Added

- FFI: a `@repr(C)` struct field may now itself be a nested `@repr(C)` struct, crossing the C ABI by value. The nested struct's bytes are inlined into the parent's C image (the back end follows the heap pointer recursively), and on a return the nested object is rebuilt with the parent's GC descriptor marking the nested-field slot as a pointer so the collector traces it. Register classification runs over the flattened field list, so a nested struct passes exactly like its fields inlined, and the 16-byte cap applies to the flattened total. A non-`@repr(C)` struct field and a struct that contains itself are rejected (#329).

## [2.6.0] - 2026-06-06

### Added

- FFI: `@repr(C)` structs with floating-point fields (`CFloat`, `CDouble`) now cross the C ABI by value, up to 16 bytes, as arguments and return values. The back end builds a per-register plan from the struct layout and the target convention: System V classifies each eightbyte INTEGER (i64) or SSE (f64), AArch64 passes a homogeneous float aggregate in SIMD registers (one per field) and other structs in general registers, and Windows x64 uses one integer register or by-reference. A `CFloat` field is narrowed from f64 to f32 (and widened back) at the boundary, and a struct literal accepts a native `Float` for a float field. Nested structs and structs larger than 16 bytes remain follow-ups (#328).

## [2.5.0] - 2026-06-06

### Added

- FFI: `@repr(C)` structs up to 16 bytes now cross the C ABI by value, both as arguments and as return values, beyond the previous 8-byte (one-register) limit. The back end classifies each struct from its size and the target ABI: one or two integer registers on System V AMD64 and AArch64, and one register or a by-reference copy (with a hidden-pointer `sret` for those returns) on Windows x64. This unblocks binding C structs like `SDL_Rect`. Float fields, nested structs, and structs larger than 16 bytes remain follow-ups (#327).

## [2.4.4] - 2026-06-05

### Fixed

- The Linux release binary is now built on Ubuntu 22.04 instead of the latest runner image (24.04), so it links against an older glibc and runs on both Ubuntu 22.04 and 24.04. A binary built on 24.04 could fail to start on 22.04 with a `GLIBC_2.x not found` error, because glibc is backward compatible but not forward compatible. No source changes.

## [2.4.3] - 2026-06-05

### Added

- Standard library enrichment across text, data structures, numbers, and I/O, including two new modules: **std/list** (generic list utilities `contains`, `index_of`, `reverse`, `slice`, `concat`, `flatten`, `first`, `last`, `insert`, `remove_at`, `repeat`, `range`) and **std/option** (`is_some`, `is_none`, `unwrap_or`, `map`, `and_then`, `filter`, `or_else`). Existing modules gained:
  - **std/string**: `split`, `split_whitespace`, `lines`, `parse_int`, `parse_float`, `trim_start`, `trim_end`, `reverse`, `count`, `last_index_of`, `byte_at` (#305).
  - **std/collections**: `Set.to_list`, `union`, `intersection`, `difference`, `is_subset`; `Map.get_or`, `entries`, `clear` (#307).
  - **std/error**: Result combinators `map_ok`, `map_err`, `unwrap_or_else` (#308).
  - **std/math**: `fmod`, `atan2`, `asin`, `acos`, `atan`, `log2`, `cbrt`, `hypot`, `sinh`, `cosh`, `tanh`, `gcd`, `lcm`, `sign`, `sign_int`, `is_nan`, `is_inf`, `infinity`, `nan`, `to_radians`, `to_degrees` (#309).
  - **std/iter**: `sum`, `product`, `min`, `max`, `position`, `nth`, `last` (#310).
  - **std/test**: generic `assert_eq` / `assert_ne`, `assert_eq_float`, and `assert_some` / `assert_none` / `assert_ok` / `assert_err` (#311).
  - **std/fmt**: `format_float`, `from_radix`, `from_hex` (#312).
  - **std/json**: `stringify_pretty`, `JsonValue.as_int` / `keys` / `length`, and value constructors `json_null` / `json_bool` / `json_number` / `json_int` / `json_string` / `json_array` / `json_object` (#313).
  - **std/encoding**: `url_encode` / `url_decode`, `base32_encode` / `base32_decode` (#314).
  - **std/path**: `normalize`, `components`, `with_extension`, `is_relative` (#315).
  - **std/fs**: `create_dir_all`, `read_lines`, `copy`, recursive `walk` (#315).
  - **std/random**: `gen_range_float`, `sample`, `weighted_choice`; **std/http**: `patch`, `head`; **std/net**: `TcpStream.read_all`; **std/hash**: `crc32` (#316).
- Reference documentation for every new stdlib API, with a compile-verified example per function, and new pages for std/list and std/option.

### Fixed

- A named top-level function used as a first-class value (passed to a higher-order function, bound to a variable, returned, or given to a stdlib combinator like `option.map`) crashed at runtime with a misaligned pointer dereference. Such a function was lowered to its raw C address, but a Raven `fun(T) -> U` value is a closure object, so the call site dereferenced the code pointer as a closure. A named function value now lowers to a zero-capture closure that forwards to the function, the same representation a lambda has. A C-FFI callback passed where a `CFnPtr` is expected still lowers to the raw address (#317).
- Importing two stdlib modules that declare the same C extern symbol (for example std/json and std/random, which both bind `raven_int_to_float`) no longer fails with a duplicate-declaration error. Redeclaring an extern name is now treated as the same linker symbol.

## [2.3.1] - 2026-06-05

### Added

- Mutable module-level globals. A `let` at file scope is now a real mutable global with runtime storage: any function can read and reassign it, its initializer may be any expression (not only a constant) and runs before `main` in declaration order so a later global can read an earlier one, and a heap-valued global (`String`, `List`, struct, and so on) is registered as a permanent GC root so the collector keeps it alive for the whole program. A `const` is unchanged (an inlined compile-time constant). This completes #278.
- `const` inside a function body now parses and works as an immutable local binding (previously a parse error). Unlike a module-level `const`, a local `const` has stack storage, so its initializer may be any runtime expression (a function call, for example), but reassigning or compound-assigning it (`c = ...`, `c += ...`) is a compile error. `let` stays mutable (#278).
- Module-level `const` and `let` initializers may now be constant expressions, not only literals: arithmetic, comparison, bitwise, and boolean combinations of literals are folded at compile time and inlined at each use site, so `const SECS_PER_HOUR: Int = 60 * 60` and `const ENABLED: Bool = true && (1 < 2)` work. A non-constant initializer (for example a function call) is now a clear "must be a constant expression" error instead of a code-generation failure (#278).
- The parser now recovers at item and statement boundaries, so one compile reports several syntax errors instead of only the first. On a failed top-level item the parser skips to the next item-starting keyword (`fun`, `struct`, `enum`, `trait`, `impl`, `extern`, `import`, or `@`); on a failed statement inside a block it skips to the next statement boundary and keeps parsing the body, so more than one error per function is reported too. Both track bracket depth to step over nested groups, and a compile with parse errors reports them all (de-duplicated) and stops before resolve and type checking. Recovery is opt-in, so a valid program parses unchanged (#294).
- The type checker now reports multiple errors per compile instead of stopping at the first. The body pass recovers at item and statement boundaries: an error in one function, impl method, `const`, or `let` no longer hides errors in the next, and each statement in a block reports independently. Recovery binds a failed `let` to its annotated type (or `Ty::Error`) so later references do not cascade into spurious follow-on errors, and identical diagnostics are de-duplicated. Each error is rendered with the rich source-pointer format from 2.1.0, separated by a blank line. Parser-level recovery (multiple syntax errors per compile) remains a follow-up (#284).

### Fixed

- `rvpm fmt` now formats files that declare or use macros instead of leaving them untouched. A `macro name { (matcher) => { template } ... }` definition and a `name!(...)` invocation are parsed into dedicated AST nodes (the formatter parses un-expanded source) and rendered canonically: one rule per line for multi-rule macros, with metavariables (`$x:expr`) and the repetition sigil kept tight. The surrounding code in a macro-using file is now formatted normally rather than passed through verbatim (#261).

## [2.1.6] - 2026-06-04

### Added

- Documented and locked in macro calls in statement position and item (top-level) position, not only expression position. The token-level pre-pass already splices a template wherever the call appears, so a template that parses as one or more items or statements (a `struct`/`fun` declaration, a `let` binding) is valid there. Added tests and a golden example covering both positions (#221).
- Macro calls now expand inside `"${...}"` string-interpolation fragments. A fragment such as `"${twice!(n + 1)}"` previously failed with "interpolation must contain a single expression" because fragments are parsed after the main macro pre-pass; the file's macro table is now carried into fragment parsing so the call expands like anywhere else in expression position (#226).
- Compile-time enum reflection `variant_names<T>()` and `variant_field_types<T>()`: `variant_names` returns an enum's variant names in declaration order as a `List<String>`; `variant_field_types` returns a `List<List<String>>` with one inner list per variant of its payload field type names (empty for a unit variant, so the inner length is the payload arity). For a generic enum each payload type renders at its concrete instantiation. A non-enum type argument is rejected (#228).
- Compile-time reflection `field_types<T>()`: the positional counterpart to `field_names<T>()`, returning each struct field's type name in declaration order as a `List<String>`. For a generic struct each field renders its concrete type per instantiation, so `field_types<Box<Int>>()` yields `["Int"]`. A non-struct type argument is rejected, matching `field_names` (#227).
- Macro fragment specifiers beyond `expr` and `ident`: `$x:ty` and `$x:pat` capture a balanced token run (a type such as `List<Int>`, a pattern such as `Some(n)`), `$x:literal` matches exactly one literal token and rejects anything else, and `$x:block` captures a brace-delimited `{ ... }` group. The names document call-site intent and, for `literal` and `block`, constrain what a rule accepts (#223).
- Runtime reflection `set_field(a, name, value)`: write a struct field by name through an `Any`, the counterpart to `get_field`. A write whose value type does not match the field is ignored (#230).
- Rich, colorful compiler error messages. `raven build` now prints a friendly headline, a box-drawing source pointer with an inline label at the offending span, and `help:`/`note:` lines, instead of a single terse line. Type mismatches, unknown types, missing match arms, undefined methods, and parse errors all get hand-written wording and suggestions. Color is automatic on a terminal and disabled under `NO_COLOR` or when output is piped (#283).

## [2.0.10] - 2026-06-04

### Added

- Module-level constants with literal initializers now work and can be used from any function: `const MAX: Int = 100`, `let GREETING = "hello"`, `let NEG = -7`. Each reference is inlined to its literal, and an unannotated `let` infers its type from the literal. References previously mis-typed as `Unit` and failed in code generation. Mutable or computed module-level bindings remain unsupported (#278).
- Module-qualified calls: after `import std/fs` you can call `fs.write(...)`, and after `import std/io` call `io.println(...)`, alongside the existing name-import form (`import std/fs { write }` then `write(...)`). The qualified call resolves to the module's namespaced function (#264).
- Lexicographic ordering on `String` with the `<`, `<=`, `>`, `>=` operators, backed by a new `raven_string_cmp` runtime function. Previously these were a type error (#267).

### Fixed

- `rvpm fmt` and `rvpm fmt --check` no longer fail on a file that declares or uses a macro. Such a file is left unchanged for now (macro definitions and `name!(...)` invocations have no AST node to format); reformatting them is a follow-up (#261).
- String interpolation may now contain a nested string literal, including in a call argument: `"hello ${shout("world")}"`. The inner `"` previously ended the outer string early. Macro invocations inside `${...}` remain unsupported (tracked by #226) (#262).
- The formatter now writes `spawn(...)` as a call, without the stray space it used to insert before the parenthesis (#263).
- `match` on `String` literal patterns now compares by content. Arms like `"yes" -> ...` previously compared heap-pointer identity, so they never matched and silently fell through to the wildcard (#265).
- `String.len()` and `String.is_empty()` now work without an import, spelled the same as `List`/`Map`/`Set`. They previously type-checked but failed in code generation with `unresolved callee: Str$len`. A user `impl String` of either name still takes precedence (#266).

## [2.0.2] - 2026-06-02

### Fixed

- A GitHub package consumed as a dependency can now use free functions it imports from the standard library. Previously `import std/time { now_millis }` (and similar) inside a dependency left the function unresolved in the consumer, even though types and methods from the same package resolved. The resolver now rewrites those call sites to the `std` namespace when the package is merged.

## [2.0.1] - 2026-06-02

### Fixed

- `Rng.from_entropy()` (and the runtime `raven_random_entropy` seed source) now returns a distinct value on every call within a process. A process-global counter is mixed into the seed, so code that seeds a fresh `Rng` per call, such as per-call UUID generation, no longer risks repeats when calls land in the same clock tick.

## [2.0.0] - 2026-06-02

Raven 2.0 is a complete rewrite. Version 1 was a tree-walking interpreter; version 2 is a compiled language that emits native binaries through a Cranelift backend, with a new syntax and a real type system. This is a breaking change with no automated migration. Version 1 stays on the `v1.x-maintenance` branch, and the [migration guide](https://martian56.github.io/raven/v2/migration/) maps the differences.

### Language

- Native compilation through Cranelift to a single static binary. A tracing garbage collector manages the heap, so there are no manual frees and no borrow checker.
- A static type system with generics and trait bounds, traits for polymorphism, sum types (`enum` with payloads), exhaustive `match`, and local type inference.
- `Result<T, E>` and `Option<T>` with the `?` operator. There is no `null`.
- String interpolation (`"${expr}"`), closures, range-based `for`, and `defer` with function-scoped semantics.
- Enum construction with `EnumName.Variant(args)`, set literals `{a, b}`, and map literals `["k": v]`.
- Concurrency: lightweight goroutines with `spawn` and channels in `std/sync` (cooperative single-thread scheduler in this release).
- C FFI: `extern "C"` blocks, the numeric types `CInt`/`CLong`/`CSize`/`CFloat`/`CDouble`, `CStr` and `CPtr<T>`, raw pointer load/store/alloc, function-pointer callbacks (`CFnPtr`), and small `@repr(C)` structs passed by value.
- Metaprogramming: `@derive` for `Eq`, `Hash`, `ToString`, `Debug`, `ToJson`, and `FromJson`; declarative macros with repetition and hygiene; and compile-time (`type_name`, `field_names`) plus runtime (`to_any`, `get_field`, `cast`) reflection.

### Standard library

- Bundled `std` modules: `io`, `string`, `collections` (hash-backed `Map`/`Set`), `math`, `cmp`, `hash`, `iter`, `fmt`, `encoding`, `random`, `env`, `fs`, `time`, `net`, `http`, `json`, `regex`, `process`, `ffi`, `error`, `path`, `test`, `sync`, and the always-in-scope `core` traits.

### Tooling

- `rvpm`: GitHub-direct packages with `rv.toml` and a content-hashed `rv.lock`, a shared cache, and `init`, `add`, `install`, `update`, `build`, `run`, and `fmt`.
- One canonical formatter (`rvpm fmt`), an updated VS Code extension, and refreshed documentation that defaults to v2.
- Cross-platform installers built by the release workflow: `.deb`/`.rpm`/`.tar.gz` for Linux, `.msi`/`.zip` for Windows, and `.pkg`/`.tar.gz` for macOS (Intel and Apple Silicon).

### Changed

- New syntax: PascalCase type names (`Int`, `String`, `Bool`, `Float`, `Char`, `Unit`), no semicolons, and programs that run from `fun main()` with no top-level statements. The `export` keyword is gone; every top-level item is importable.

### Removed

- The version 1 tree-walking interpreter and its REPL, and the version 1 syntax. Use the `v1.x-maintenance` branch for version 1 code.

## [1.4.0] - 2026-03-21

### Added
- `json` standard library module added and included in release packaging artifacts.
- Community health files added for repository standards:
  - `CODE_OF_CONDUCT.md`
  - `CONTRIBUTING.md`
  - `SECURITY.md`
  - Issue templates and pull request template under `.github/`.

### Changed
- Language type spelling standardized to lowercase `string` in Raven source/docs/examples/editor assets.
- CLI/package version bumped to `1.4.0`.

### Fixed
- Module method-call type inference now resolves imported module function signatures:
  - `import "json"; let content: string = json.load("test.json");` now type-checks correctly.
- Windows/Linux release manifests updated to include all standard library modules.

## [1.3.0] - 2025-10-04

### Added
- Professional Python-style CLI interface (`raven file.rv`, `raven`)
- Complete enum support with string-to-enum conversion (`enum_from_string()`)
- Advanced type system with custom types in function signatures
- Complex assignment targets (`object.field[index] = value`)
- Method chaining support (`object.method1().method2()`)
- Field access with array indexing (`object.field[index]`)
- Comprehensive standard library (math, collections, string, time, filesystem, network, testing)
- WiX installer for Windows (401KB MSI)

### Changed
- CLI interface now matches Python/Node.js behavior
- Enhanced error messages and type checking
- Improved module loading and resolution

### Fixed
- Release build compilation issues (STATUS_ACCESS_VIOLATION)
- Complex assignment parsing
- Method calls on struct fields
- Type checking for custom types
- Module path resolution
- Compiler warnings cleanup

### Removed
- Unnecessary `-f` and `--repl` flags
- Unprofessional startup messages
- Dead code and unused imports

## [1.0.0] - 2025-10-03

### Added
- Initial public release of the Raven language and toolchain.
- Tokenizer/lexer with comments, strings, numbers, and identifiers.
- Parser with support for core language constructs.
- AST generation, type checking, and interpreter execution.
- CLI tool and interactive REPL.
- Variables and types (`int`, `float`, `string`, `bool`, arrays).
- Control flow (`if`/`else`, `while`, `for`).
- Functions with parameters, return types, and recursion.
- String operations, array operations, and built-in functions.
- File I/O and module system (`import`/`export`).
- Error reporting with line/column information.
