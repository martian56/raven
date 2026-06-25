# Changelog

All notable changes to Raven are documented in this file.

## [2.18.213] - 2026-06-25

### Fixed

- `std/fs.list_dir` preserves entry names that contain a newline and non-UTF-8 Unix names. The runtime joined names by a newline and decoded them lossily, so a filename with a newline was split into two entries and a non-UTF-8 name was mangled into a different path that could not be passed back to `exists`/`read`/`remove_file`. Names are now joined by a NUL byte (which cannot occur in a filename) using their raw bytes, so each entry round-trips (#655, #656).

## [2.18.212] - 2026-06-25

### Fixed

- `rvpm update`/`install` records the authoritative content hash in `rv.lock`, so a freshly written lock validates. Lock generation reused a cached hash trusted from a cheap metadata signature (file count, total bytes, newest mtime), which a same-size, same-mtime edit to a cached dependency left unchanged; the lock then carried a stale hash that the validation path (which always re-hashes content) immediately rejected. Lock generation now hashes the full tree content like validation does (#715).

## [2.18.211] - 2026-06-25

### Fixed

- rvpm's conflicting-version check compares dependency sources case-insensitively. A GitHub owner and repository path is case-insensitive, so two casing variants (`github.com/Acme/Demo` and `github.com/acme/demo`) named the same repository twice and slipped past the check, letting the lock pin one repository at two different refs. The check now groups by the lowercased source (#724).

## [2.18.210] - 2026-06-25

### Fixed

- The `rv.toml` manifest rejects a dependency key with a subpath (`github.com/<user>/<repo>/<sub>`). A dependency identifies a whole repository, but a subpath key was accepted and recorded in the lock as the source, while import resolution looks up the bare repository identity, so the lock entry could never be matched. A dependency key must now be a bare `github.com/<user>/<repo>` (#718).

## [2.18.209] - 2026-06-25

### Fixed

- `Cargo.lock` is kept in sync with the workspace version, so `cargo build --locked` (and `cargo install --locked`) works against a clean checkout. The committed lock had drifted behind the manifest version because routine builds rewrite it locally but CI never committed it; the lock now records the current version and is updated alongside future version bumps (#764).

## [2.18.208] - 2026-06-25

### Fixed

- The `rv.toml` manifest rejects `[fmt]` widths outside their documented bounds instead of accepting any value. `indent_width` must be 1..=16 and `wrap_width` must be 40..=200 (the ranges the docs state); a value outside the range is now an `invalid value for [fmt]....` error rather than being silently used (#721).

## [2.18.207] - 2026-06-25

### Fixed

- Constant folding matches the runtime instead of rejecting expressions with defined behavior. Integer `+`/`-`/`*` and unary `-` fold with wrapping (two's complement) arithmetic, and `<<`/`>>` fold with the same saturation the back end emits (a left shift by 64 or more is 0, a right shift by 64 or more collapses to the sign). A `const` like `9223372036854775807 + 1` or `1 << 64` now compiles to the value the equivalent runtime expression produces; only a divide or modulo by zero (a runtime trap) stays unfolded (#736).

## [2.18.206] - 2026-06-25

### Fixed

- `String.parse_float` and JSON number parsing preserve the sign of negative zero. Both negated a negative input by subtracting from `0.0`, and `0.0 - 0.0` is `+0.0`, so `-0.0` (and JSON `-0`) lost its sign; they now negate by multiplying by `-1.0`, which keeps `-0.0` (#740).

## [2.18.205] - 2026-06-25

### Fixed

- The base64 and base32 decoders reject malformed padding rather than accepting it. `base64_decode` now rejects a `=` before the final group and a third-position pad without a fourth (`X=Z=`); `base32_decode` now requires a length that is a multiple of 8 (a partial group was silently dropped) and rejects data after a padding byte (#434).

## [2.18.204] - 2026-06-25

### Fixed

- Unary `&` (address-of) is rejected at type-check instead of behaving as a silent identity operator. Code generation returned the operand unchanged, so `&x` compiled to `x` while looking like it took a reference; Raven has no reference or pointer type for it to produce, so the type checker now reports it as unsupported (#720).

## [2.18.203] - 2026-06-25

### Fixed

- `Rng.gen_range_float(lo, hi)` no longer returns the excluded upper endpoint `hi`. The convex-combination interpolation could round onto `hi`, especially when `lo` and `hi` are adjacent floats; it now re-draws until the result falls below `hi` (bounded, falling back to `lo`), keeping the interval half-open while staying finite for very wide ranges (#738).

## [2.18.202] - 2026-06-25

### Fixed

- `Rng.weighted_choice` ignores non-positive weights instead of summing them. A negative weight was added to the total and the cumulative sum, which could shrink the total to zero or below (making every draw return `None`) or leave a later positive item unreachable; only positive weights now contribute, so a non-positive weight is simply never drawn (#737).

## [2.18.201] - 2026-06-25

### Fixed

- The HTTP server honors a `close` token in a multi-token `Connection` header. It compared the whole header value as one string, so an HTTP/1.1 `Connection: keep-alive, close` was treated as keep-alive and the connection was retained against the client's request; the value is now split into comma-separated tokens and each is checked, so a `close` beside any other option closes the connection (#744).

## [2.18.200] - 2026-06-25

### Fixed

- The HTTP response builder rejects an invalid field name and strips control bytes from a field value, instead of sending them unchanged. `Response.header` removed only CR and LF, so a handler could set a name with a space or other forbidden byte, or a value with a NUL or other control byte, and produce a malformed response clients may reject or parse inconsistently. A name that is not a valid HTTP token is now dropped, and a value's C0 controls and DEL (except HTAB) are stripped (#742).

## [2.18.199] - 2026-06-25

### Fixed

- The HTTP server replaces an out-of-range response status code with 500 instead of writing it into the status line verbatim. `Response.status_code` accepts any `Int`, so a handler could emit a malformed status line like `HTTP/1.1 -1 OK`; the server now frames the wire status from a value clamped to the valid 100..599 range (#741).

## [2.18.198] - 2026-06-25

### Fixed

- `String.substring` clamps negative bounds to 0 instead of converting them to a huge unsigned index. The runtime took the bounds as unsigned, so a negative `start` or `end` became a pointer-sized value that clamped to the string length and gave the wrong slice; the bounds are now signed and clamped into `0..length` as documented (#556).

## [2.18.197] - 2026-06-25

### Fixed

- The HTTP server no longer sends a message body or a Content-Length header for a status that forbids one. A 1xx, 204, or 304 response previously carried `resp.body` and a `Content-Length`, putting bytes on the wire that corrupt framing for clients and persistent connections; the server now omits the body for those statuses and the Content-Length for 1xx and 204. The server also strips any handler-set `Content-Length` or `Transfer-Encoding` (case-insensitively), since it always frames responses itself, and the HTTP client no longer reports a 1xx/204/304 or HEAD response as a truncated transfer when its Content-Length advertises a body it correctly does not carry (#745).

## [2.18.196] - 2026-06-25

### Fixed

- Writes to stdout now ignore `SIGPIPE` on Unix, so a program that keeps printing after its pipe reader exits (`./prog | head -n 1`) is no longer killed by the signal; the broken-pipe write returns an ignored `EPIPE` and the program runs to completion. The runtime already did this for TCP and child-stdin writes but not for `print`/`println` (#766).

## [2.18.195] - 2026-06-25

### Fixed

- Macro hygiene renames bindings scope-aware, so a binding in a nested block no longer renames a free use of the same name in an outer scope. The hygiene built one rename map by spelling for the whole template, so an inner local (`let helper` inside a nested block) renamed every occurrence of that spelling, breaking an unrelated outer `helper()` call. Renaming is now a scope-aware pass over the instantiated tokens: each block is its own scope and a binding only covers its own scope and nested ones (#664).

## [2.18.194] - 2026-06-25

### Fixed

- Macro hygiene recognizes function and closure parameters and match-pattern bindings, not only `let`/`const`/`for` targets. A parameter or pattern variable a template introduced was treated as a free definition-site name, so the resolver could not find it; a macro can now generate a function, a closure, or a match whose body uses names the template binds (#653).

## [2.18.193] - 2026-06-24

### Fixed

- The HTTP server decodes a `Transfer-Encoding: chunked` request body instead of discarding it. It ignored the chunked framing, so the handler received an empty body and the unread chunk bytes desynchronized a keep-alive connection. The server now de-chunks the body, delivers it to the handler, and rejects `Content-Length` combined with chunked framing as a smuggling vector (#617).

## [2.18.192] - 2026-06-24

### Fixed

- `Server.shutdown()` wakes a server bound to an IPv6 wildcard address. The wakeup helper split the address on the first colon, which mangles a bracketed IPv6 address (`[::]:8080`), so the throwaway self-connection went to an unreachable address and `listen` blocked forever. It now parses a bracketed host and rewrites the IPv6 wildcard `[::]` to the loopback `[::1]`, which the wildcard listener accepts (#643).

## [2.18.191] - 2026-06-24

### Fixed

- The HTTP server rejects a malformed request line or header instead of parsing it loosely. A request line must be exactly `METHOD TARGET VERSION` with an `HTTP/` version, and every header must be `name: value` with a non-empty name and no obsolete line folding; anything else now answers 400 (#641).
- The HTTP server rejects two Content-Length headers with different values rather than silently keeping the last, closing a request-smuggling vector (#618).

## [2.18.190] - 2026-06-24

### Fixed

- A large HTTP server timeout no longer silently disables timeouts. `with_timeout`, `with_read_timeout`, and `with_write_timeout` computed `seconds * 1000`, which overflowed for a large `seconds` and wrapped to a non-positive value the server reads as "disabled". The conversion now saturates to a large positive bound, so a big timeout stays in effect; a normal value is exact and a non-positive value disables the timeout (#683).

## [2.18.189] - 2026-06-24

### Fixed

- The HTTP server returns a correct reason phrase for any status code. `reason_phrase` defaulted to `OK`, so a `503` was written as `503 OK`; it now covers the common codes and falls back to a class phrase (`Server Error` for 5xx, and so on) for an unknown code (#642).
- The HTTP server omits the body of a HEAD response, which previously carried the full GET body. The response still reports the `Content-Length` a GET would return, but writes no body (#619).

## [2.18.188] - 2026-06-24

### Fixed

- The website deploy script reports failure when triggering the deployment fails, instead of warning and exiting `0` so the workflow looked successful (#626).
- The website hero no longer keeps applying parallax on mobile: the scroll listener now skips the transform on small viewports, matching the mobile setup that clears it (#627).
- The responsive example-tab rule under `@media (max-width: 480px)` targets the real `.tab-btn` class, so the full-width mobile tab layout applies (#628).
- The mobile navigation toggle is keyboard operable: it carries a button role, focus, an accessible name, and `aria-expanded`, and responds to Enter and Space (#629).
- The double-tap zoom guard only cancels when both taps hit the same element, so quickly tapping two different controls no longer loses the second tap (#630).
- The website Basics example imports `std/collections`, which its map literal requires, so copying it compiles (#631).

## [2.18.187] - 2026-06-24

### Fixed

- The VS Code grammar's FFI type list matches the compiler. It highlighted `CChar` and `CVoid`, which the type checker rejects, and omitted the supported `CString` alias. The grammar and the completion list now cover exactly `CInt`, `CLong`, `CSize`, `CStr`, `CString`, `CPtr`, `CFloat`, `CDouble`, and `CFnPtr` (#624).

## [2.18.186] - 2026-06-24

### Fixed

- The VS Code "Run Raven File" command no longer silently overwrites an unrelated sibling file. It builds the executable next to the source as `<basename>` (or `<basename>.exe`), so running `demo.rv` could clobber an existing `demo`/`demo.exe`. It now confirms before overwriting a file it did not build this session (#625).

## [2.18.185] - 2026-06-24

### Fixed

- The VS Code "Run Raven File" command no longer passes the source path through a shell. It built `raven build "<path>"` as a command string and invoked the binary with a double-quoted path, so a workspace file name containing `$(...)` or backticks executed that command on Run (PowerShell expands `$(...)` even inside double quotes). The build now runs `raven` with `execFile` and an argument array (no shell), and the produced binary is invoked through a single-quoted path so the shell treats it as literal text (#622).

## [2.18.184] - 2026-06-24

### Fixed

- The release workflow no longer interpolates the manual `version` dispatch input directly into shell and PowerShell source. The input is passed through the environment and validated against an allowed character set before use, and every later staging step reads the resolved version from the environment too, so a value with a quote or command separator can no longer run as code on the release runners (#632).

## [2.18.183] - 2026-06-24

### Fixed

- A duplicated `[[package]]` block in `rv.lock` no longer compiles a dependency's FFI sources twice. Loading a lock now drops exact-duplicate entries, so a repeated identical block is collapsed to one and a dependency with an `[ffi]` source is not gathered twice (which previously made the linker fail with duplicate definitions). A genuine conflict (one source pinned to two versions) is still rejected (#649).

## [2.18.182] - 2026-06-24

### Fixed

- `rvpm` commands reject unexpected arguments instead of silently ignoring them. `new`, `init`, `fetch`, `lock`, `add`, and `test` did not validate their full argument list, and `build`/`install`/`update`/`doc` rejected an unknown flag but still ignored extra positional arguments, so a typo like `rvpm new app extr` or `rvpm lock --typo` looked like a success. Each command now rejects an unknown flag and any positional beyond what it accepts (#647).

## [2.18.181] - 2026-06-24

### Fixed

- The dependency tree hash is lossless and unambiguous, closing two ways a changed cache tree could pass lockfile validation. Path components and symlink targets were hashed through a lossy Unicode conversion, so distinct non-Unicode names collapsed to the same bytes (#659), and a symlink target was written with no length boundary, so its bytes could run into the next entry's path and two different trees could share a hash (#660). The hash now absorbs each path component and symlink target length-prefixed, using UTF-8 bytes for valid Unicode (identical across platforms) and native OS bytes otherwise, with distinct file and symlink markers.

## [2.18.180] - 2026-06-24

### Fixed

- The package manager validates a dependency's user, repo, and version before using them to build a cache path. Each becomes a directory name under the cache, so a single shared check (`is_safe_cache_component`) now rejects an empty, `.`/`..`, separator-, drive-colon-, or control-character-bearing component in `GithubPath::parse` (user/repo) and the lock's version resolution, and the fetch and clone paths refuse a cache destination that still contains a `..` component, so a malicious transitive dependency cannot steer directory creation outside the cache root (#431).

## [2.18.179] - 2026-06-24

### Fixed

- `rvpm add` no longer leaves `rv.toml` modified when resolution fails. The command wrote the edited manifest before resolving or fetching the dependency, so a failed resolve (a missing repo, an invalid constraint) left the dependency in `rv.toml` with no matching `rv.lock`. The manifest and lock are now resolved first and written only after both succeed, so a failed add leaves project state untouched (#648).

## [2.18.178] - 2026-06-24

### Fixed

- `rvpm new` rejects an absolute or drive/root-prefixed target, not just a `..` component. An absolute path (`/path/app`, `C:\path\app`) has no `..` component, so it bypassed the outside-tree guard and scaffolded a package anywhere the process could write; the target must now be a relative path inside the current directory (#646).

## [2.18.177] - 2026-06-24

### Fixed

- `rvpm` no longer follows directory symlinks out of the package when walking sources. `rvpm doc`, `rvpm fmt`, and `rvpm test` descended into a symlinked directory pointing outside the project, so they could read, rewrite, or compile and run code from another tree. The source walks now skip symlinked entries (#678, #679, #680).

## [2.18.176] - 2026-06-24

### Fixed

- A declarative macro repetition can be followed by another matcher item. `($($x:expr),* ; $tail:expr)` failed to match because the repetition's last element ran past the `;`; the last element of a repetition now stops at the token that follows the repetition, so a trailing matcher item after a `*`/`+` group matches (#661).

## [2.18.175] - 2026-06-24

### Fixed

- A literal token in a macro matcher matches by value, not just by token kind. `(foo)` matched every identifier and `(1)` every integer, so rules differing only by a literal value were unreachable; a literal matcher token now has to appear verbatim (#651).

## [2.18.174] - 2026-06-24

### Fixed

- A compiled Raven program no longer dies from SIGPIPE when it writes to a closed pipe or socket. Writing the rest of a child's stdin after the child exits, or to a network peer that has closed the connection, raised SIGPIPE on Unix, whose default disposition terminated the program; the runtime now ignores SIGPIPE so the write surfaces as an ordinary handled error instead.

## [2.18.173] - 2026-06-24

### Fixed

- A declarative macro call inside an imported local module is expanded. Macro expansion ran only on the entry file, so a macro used inside an imported module reached HIR lowering unexpanded and failed with an internal error; imported modules now get the same macro pre-pass, and the free identifiers their macros introduce flow to the resolver (#650).

## [2.18.172] - 2026-06-24

### Changed

- The macro token-limit test asserts the token-limit-specific error wording (`produced over`) rather than the phrase the pass-limit error also uses, so it cannot pass without exercising the size cap.

## [2.18.171] - 2026-06-24

### Fixed

- Declarative macro expansion caps the generated token count, not just the pass count. A macro that expands one call into several (`boom!() => boom!() + boom!()`) doubled the token stream every pass and exhausted memory before the 128-pass limit; it now reports the likely-recursive diagnostic once the stream grows past the size cap (#652).

## [2.18.170] - 2026-06-24

### Fixed

- The language reference describes the current FFI surface. A capturing closure can be used as a C callback through a userdata pointer (pass it to both the `CFnPtr` and the userdata `CPtr`; the compiler emits a trampoline), and a `@repr(C)` struct passed by value may have float fields, nested struct fields, and any size (the back end classifies it for the platform ABI), correcting the old claims that capturing closures are rejected and that by-value structs must be integer-class and at most 8 bytes (#640).

## [2.18.169] - 2026-06-24

### Fixed

- `CLong` uses the correct ABI width per platform. C `long` is 32-bit under LLP64 (Windows) and 64-bit under LP64 (Linux); the back end lowered it as 64-bit everywhere, so a Windows call truncated a 64-bit argument and read a 32-bit return without sign extension (`atol("-1")` came back as 4294967295). `CLong` is now 32-bit on Windows, so arguments pass and returns sign-extend at the right width (#639).

## [2.18.168] - 2026-06-24

### Fixed

- The `std/net` guide, spec, and module header describe current behavior: `TcpStream.read` returns raw bytes with no lossy UTF-8 conversion, goroutines let one program be both a client and a server, and the documented surface now includes `read_all`, `set_write_timeout_ms`, and `TcpListener.close` (#634).

## [2.18.167] - 2026-06-24

### Fixed

- The `std/http` guide describes the current server: `listen` serves each connection in its own goroutine (connections are handled concurrently), and HTTP pipelining is supported (bytes past one request are carried into the next parse), correcting the previous claims that connections are served one at a time and that pipelining is unsupported (#644).

## [2.18.166] - 2026-06-24

### Fixed

- The concurrency documentation describes the current parallel scheduler. The language reference and the `std/sync` guide no longer claim that exactly one goroutine runs at a time on a single OS thread or that multicore parallelism and `select` are future work; the M:N scheduler runs goroutines in parallel across a worker pool, and the `std/sync` guide now documents `sleep_millis`, the mutex, the wait group, `select_recv`, and channel `free` (#638).

## [2.18.165] - 2026-06-24

### Fixed

- Removed stray `</content>` and `</invoke>` wrapper tags accidentally left at the end of `docs/v2/guide/stdlib/io.md`, `docs/v2/guide/stdlib/net.md`, and `docs/v2/migration.md`, which MkDocs copied verbatim into the published HTML (#658).

## [2.18.164] - 2026-06-24

### Fixed

- A derived enum `FromJson` validates that the `values` payload is an array with exactly the variant's arity. A unit variant now rejects a non-array or non-empty payload, and a tuple variant rejects too few or too many elements, instead of decoding malformed data as `Ok` (#681).

## [2.18.163] - 2026-06-24

### Fixed

- A user declaration that uses the `raven_derive_` prefix reserved for the generated JSON derive helpers is rejected with a clear message at the declaration, instead of failing later with a confusing `declared multiple times` error pointing at the synthetic helper source (#682).

## [2.18.162] - 2026-06-24

### Fixed

- A local module's globals initialize before an importer that reads them at load time. Modules are now merged in dependency order (a module after every module it imports), so a top-level `let` that calls an imported function during initialization sees the imported module's initialized globals instead of zero (#669).

## [2.18.161] - 2026-06-24

### Fixed

- Two imported modules can each declare a module-level `let` or `const` with the same name. Module globals are now namespaced by their module path the way functions and types already are, so unrelated modules no longer collide during resolution with a `declared multiple times` error (#667).

## [2.18.160] - 2026-06-24

### Fixed

- The HTTP client reports a truncated response body as an error instead of `Ok`. If the server advertised a larger `Content-Length` and closed the connection early, the partial bytes were returned with the original success status; the client now treats a body shorter than `Content-Length` (or a read error) as a failed request (#615).

## [2.18.159] - 2026-06-24

### Fixed

- The HTTP client keeps every value of a repeated response header. A response with two `Set-Cookie` lines recorded the first value twice and lost the second, because it read only the first value per header name. It now emits one line per value for each header, so all values survive (#620).
## [2.18.158] - 2026-06-24

### Fixed

- `std/http` sends a request body of arbitrary bytes. The body was run through a UTF-8-only helper and a request was rejected as soon as the body held a byte such as `0xFF`; it is now sent as raw bytes, so a binary payload is delivered intact. The method, URL, and header lines are still text (#616).

## [2.18.157] - 2026-06-24

### Fixed

- `std/regex` operates on raw bytes, so a Raven `String` that is not valid UTF-8 is matched as-is instead of silently failing or being discarded. Matching, capture, replacement, and split now run on the byte-oriented engine, so non-UTF-8 bytes around a match are preserved in the result. The pattern itself is still text (#621).

## [2.18.156] - 2026-06-24

### Fixed

- The `std/process` and `std/http` response accessors no longer hold their registry lock while allocating the returned `String`. The allocation can trigger a stop-the-world collection, so holding the lock across it could deadlock the collector against another worker blocked on the same lock. Each accessor now copies the value out and releases the lock before allocating (#670).
## [2.18.155] - 2026-06-24

### Fixed

- `TcpStream.read` validates its size limit. A negative limit is now an error instead of a zero-length `Ok("")` that looked like a clean EOF, and a limit too large to allocate fails gracefully instead of aborting the process on the buffer allocation (#671).

## [2.18.154] - 2026-06-24

### Fixed

- `std/process.run` passes a single empty command-line argument through to the child. The args were NUL-joined, so an empty argument list and a one-element list `[""]` both encoded to the empty String and the runtime rebuilt zero arguments for each. Each argument is now NUL-prefixed, so the two cases stay distinct (#607).
## [2.18.153] - 2026-06-24

### Fixed

- `std/process` carries arbitrary bytes through a child. A child's stdout and stderr are captured as raw bytes instead of being lossily decoded as UTF-8 (a non-UTF-8 byte was replaced with U+FFFD), and `run_with_input` feeds non-UTF-8 stdin instead of rejecting it before spawning. Only the program path and its args still need to be valid UTF-8 (#608).

## [2.18.152] - 2026-06-24

### Fixed

- `std/io.read_line` preserves a line that contains non-UTF-8 bytes. It read the line as UTF-8 text and silently returned an empty string on a bad byte; it now reads the raw bytes up to the newline, so the line round-trips through the byte-buffer `String` (#657).
## [2.18.151] - 2026-06-24

### Fixed

- `std/fs.read` returns the file's raw bytes instead of requiring valid UTF-8. A Raven `String` is a byte buffer and `fs.write` already writes arbitrary bytes, so a binary file written through the API now reads back byte for byte instead of failing with a UTF-8 error (#609).

## [2.18.150] - 2026-06-24

### Fixed

- `WaitGroup.add` saturates its counter instead of doing an unchecked signed add. A delta that would overflow i64 no longer aborts the debug runtime, nor wraps negative and falsely completes the group so `wait()` returns without the work being done (#674).
## [2.18.149] - 2026-06-24

### Fixed

- `format_timestamp` returns the empty string for an invalid strftime pattern instead of aborting the process. chrono reports a bad directive through its formatter's `Display` error, and the runtime's `to_string()` panicked on it, which cannot unwind across the C ABI; the runtime now formats fallibly (#673).

## [2.18.148] - 2026-06-24

### Fixed

- A `ty` macro fragment balances angle brackets, so a comma inside generic arguments stays part of the type. `$t:ty` matching `Pair<Int, String>` now captures the whole type instead of stopping at the first comma. `<`/`>` in an `expr` or `pat` fragment are still treated as comparison operators (#662).
## [2.18.147] - 2026-06-24

### Fixed

- A macro matcher that binds the same metavariable name twice is rejected. `macro choose { ($x:expr, $x:expr) => { $x } }` reported no error and the second capture silently overwrote the first; it now reports `metavariable $x is bound more than once` (#665).

## [2.18.146] - 2026-06-24

### Fixed

- `std/ffi.alloc<T>(count)` returns null when `count * sizeof(T)` overflows the pointer width instead of wrapping to a small byte count and handing back a buffer far smaller than requested. The back end checks the high half of the size multiply and forces an overflowing (or negative) request to fail, so the documented null-on-failure check holds (#668).

## [2.18.145] - 2026-06-24

### Fixed

- A macro template that splices an undefined metavariable is rejected at the definition instead of silently dropping it. `macro keep { ($x:expr) => { $missing $x } }` now reports `template uses undefined metavariable $missing` rather than expanding to just `$x` (#663).

## [2.18.143] - 2026-06-24

### Fixed

- The `Eq` and `Ord` trait impls for `Float` are total in the presence of NaN. `Eq` is now reflexive (a NaN equals itself, so a `Float` map key or a derived struct compares to itself), and `Ord.compare` gives NaN one fixed position (two NaN are equal, a NaN sorts after every number) instead of comparing equal to everything. The `==`/`<` operators keep their IEEE meaning (#610).

## [2.18.142] - 2026-06-24

### Fixed

- The `std/encoding` guide documents the current decoder return types. `hex_decode`, `base64_decode`, and `base32_decode` return `Result<String, Error>` and reject malformed input, but the guide still described them as returning `String` and mapping bad bytes to zero. Updated the prose, signatures, and examples to match (#677).
## [2.18.141] - 2026-06-24

### Fixed

- The `std/fmt` spec documents `format_float`, `from_radix`, and `from_hex`. `format_float` was marked deferred and the radix parsers were missing from the surface table, though all three ship in `stdlib/std/fmt.rv` (#636).
## [2.18.140] - 2026-06-24

### Fixed

- The `std/path` spec documents the operations the module ships. `normalize`, `components`, `is_relative`, and `with_extension` were either marked deferred or missing from the surface table even though they are implemented; the spec now lists them and no longer calls path normalization out of scope (#635).

## [2.18.139] - 2026-06-24

### Fixed

- An `extern "C"` function that is never called or address-taken no longer creates a link dependency. The back end declared every foreign symbol as an import, so an unused `extern` declaration left an undefined external that failed the link; it now declares only the foreign functions the program actually references (#623).
## [2.18.138] - 2026-06-24

### Fixed

- `raven build` refuses to write the executable over its input source. The compiler reads the source first and the linker writes last, so `-o` pointing at the source would silently replace it with the binary; a typo there could destroy the only copy of a file. It now errors instead, leaving the source untouched (#612).

## [2.18.137] - 2026-06-24

### Fixed

- Compound assignment to a module-level global (`g += v`, `g -= v`, ...) works from any function. The desugar read the global through a local slot that did not exist, so the load came back as a Unit value and codegen aborted; it now loads and stores through the global symbol, the same way a plain read and write already did (#685).

## [2.18.136] - 2026-06-23

### Fixed

- A `for` loop can destructure its element with a pattern. `for Point { x, y } in points` binds the struct fields, and a refutable pattern like `for Some(x) in values` skips the elements that do not match. The loop previously kept only a single binding name and dropped the rest of the pattern, so the destructured variables were unbound and codegen aborted with `binop lhs used a Unit value` (#686).

## [2.18.135] - 2026-06-23

### Fixed

- A `match` arm's pattern bindings are scoped to that arm. They were bound in the enclosing scope, so `match value { Some(x) -> ... }` overwrote an outer `x` for the rest of the function and a non-matching arm left the outer variable reading a stale slot. Each arm now binds in its own scope, so an outer variable keeps its value and nested matches shadow correctly (#688).

## [2.18.134] - 2026-06-23

### Fixed

- The `std/fs` guide and spec document `is_symlink(path: String) -> Bool`, the shipped query that reports whether a path is a symbolic link without following it. The boolean-check sections previously stopped at `is_dir` (#637).

## [2.18.133] - 2026-06-23

### Fixed

- The `std/math` guide and spec no longer claim `pow_int` ignores overflow. They describe the current behavior: `pow_int` aborts with `pow_int overflow`, `abs_int` aborts on `i64::MIN`, and `lcm` aborts on an overflowing product (#633).

## [2.18.132] - 2026-06-23

### Fixed

- A struct destructured in a `match` arm now binds its fields. The fallback match lowering used by struct scrutinees only bound plain name patterns, so a `Point { x, y }` arm reached its body with the fields unbound and codegen aborted with `binop lhs used a Unit value`. The field type and slot are also resolved from the struct declaration, so a pattern that lists fields out of order reads the right ones (#687).

## [2.18.131] - 2026-06-23

### Fixed

- The JSON number parser caps an oversized exponent while it reads the digits, so an exponent such as `1e` followed by dozens of nines no longer overflows the `Int` accumulator and wraps into a small or negative step count. Every out-of-range positive exponent now parses as infinity (#611).

## [2.18.130] - 2026-06-23

### Fixed

- `pad_left`, `pad_right`, and `center` in `std/fmt` repeat only enough whole copies of a multi-byte fill to span the requested width, so the result overshoots by at most `fill.len() - 1` bytes instead of growing with the amount of padding (#613).
- The same padding helpers compare the width against the string length before subtracting, so an `Int::MIN` width no longer wraps to a huge positive count that spins the fill loop (#676).

## [2.18.129] - 2026-06-23

### Fixed

- `Date.to_string` and `DateTime.to_string` keep the minus sign in front of the zero-padded year, so a negative year renders as `-0001` rather than `000-1` (#684).

## [2.18.128] - 2026-06-23

### Fixed

- `Rng.gen_range_float` interpolates between the endpoints instead of computing `hi - lo` first, so a very wide finite range no longer overflows to infinity and the result stays within `[lo, hi)` (#672).

## [2.18.127] - 2026-06-23

### Fixed

- `List.get` rejects an index outside the `u32` range as out of bounds instead of truncating it to 32 bits, so a positive or negative multiple of 2^32 no longer aliases a real element (#689).

## [2.18.126] - 2026-06-23

### Fixed

- `Rng.weighted_choice` counts only the weights that have a matching item, so extra trailing weights no longer leave a slice of the draw range that returns `None` for a valid list (#604).
- `Rng.weighted_choice` saturates its running total, so a set of large positive weights no longer wraps past `Int` into a non-positive total and loses the whole draw (#605).

## [2.18.125] - 2026-06-23

### Fixed

- `String.matches_at` returns `false` for a negative offset instead of reporting a match for an empty needle at a position that does not exist (#654).

## [2.18.124] - 2026-06-23

### Fixed

- `std/fmt.from_radix` accumulates toward the sign's own end of the range with a per-step overflow check, so an out-of-range value returns `None` instead of wrapping, and the most negative `i64` now parses (#429).

## [2.18.123] - 2026-06-23

### Fixed

- `std/path.with_extension` treats a trailing dot as the extension separator, the same way `stem` does, so it neither appends a second dot (`report..txt`) nor leaves a stray one behind when removing the extension (#606).

## [2.18.122] - 2026-06-23

### Fixed

- `std/iter.nth` returns `None` for a negative index without consuming the iterator, instead of draining it to the end on a lookup that can never match (#614).

## [2.18.121] - 2026-06-16

### Fixed

- The garbage collector traces an enum value by its active variant rather than the union of every variant's pointer slots. An enum whose variants store a scalar (for example a `Float`) and a GC pointer (for example a `String`) in the same slot, such as a JSON value, previously had the scalar's bits followed as a pointer during a collection, corrupting the heap and crashing the program (a `SIGSEGV`, or a scheduler assertion under concurrency). The back end now registers a per-variant pointer mask and the collector selects it by the discriminant (#601).

## [2.18.120] - 2026-06-16

Continuing the codex-review fix batch (one patch per issue).

### Fixed

- The formatter honors `[fmt].wrap_width`: an argument list, collection literal, or function signature whose single-line form would exceed the wrap width is broken onto multiple lines (one element or parameter per line). `rvpm fmt` reads the width from the manifest. Previously `wrap_width` was parsed but ignored (#581).

## [2.18.119] - 2026-06-16

Continuing the codex-review fix batch (one patch per issue).

### Fixed

- The dependency resolver rejects a graph that pins one package source to two different versions instead of silently collapsing to one (which compiled a dependent against the wrong code); `rvpm install`/`build`/`update` report the conflict (#573).
- A selective `rvpm update` prunes packages no longer reachable from the manifest graph, so a transitive dependency dropped by the new version no longer lingers in `rv.lock` (#575).
- `rvpm install` verifies the lock contains the complete transitive graph, not just the direct dependencies, and re-resolves a fresh lock when a transitive package is missing (#576).

## [2.18.116] - 2026-06-16

Continuing the codex-review fix batch (one patch per issue).

### Fixed

- The doc extractor counts only structural braces when capturing a `struct`/`enum`/`trait` block, skipping braces inside comments, strings, and char literals so a brace in a doc comment no longer truncates the declaration (#577).
- A quoted import path is re-escaped when formatted, so a `"` or `\` in the path produces valid, round-tripping output (#578).
- The formatter weaves interior comments into struct literals, map literals, and `match` arms instead of dropping or relocating them (#579).
- Comment scanning skips `${ ... }` interpolations as a unit, so a string nested in an interpolation no longer ends the outer literal early and mis-scans a following comment (#580).

## [2.18.112] - 2026-06-15

Continuing the codex-review fix batch (one patch per issue).

### Fixed

- Bundled `[ffi]` C is compiled with the dynamic CRT (`/MD`) on windows-msvc, matching the Rust/Raven objects, so the linker no longer reports an `LNK4098` defaultlib conflict (#568).
- The bundled C object cache keys on a content hash, so a source edited with its modification time preserved is recompiled instead of skipped (#569).
- `rvpm run` forwards `--help` to the program (only a leading `--help`, or one before a `--` separator, is rvpm's) (#570).
- `rvpm build`/`install`/`update`/`doc`/`fmt` reject an unknown `-` option instead of silently ignoring it (#571).
- Package names are validated as a restricted identifier at manifest parse and in `rvpm init`/`new`, and `rvpm new` rejects a `..` in the target path, closing a path-traversal vector (#572).
- `rvpm test` writes its generated dispatcher to a per-process unique file, so it never overwrites or deletes a user file named `.rvpm-test-main.rv` (#574).

## [2.18.106] - 2026-06-15

Continuing the codex-review fix batch (one patch per issue).

### Fixed

- A positive integer literal above Int's maximum (`9223372036854775808`) is now a parse error instead of silently compiling as `Int::MIN`; `-9223372036854775808` still parses as `Int::MIN` (#543).
- Match redundancy checking now reports a duplicate literal or duplicate variant arm as unreachable (#544).
- Generic type name mangling is injective, so `Box<Int>` no longer collides with a struct literally named `Box_Int` in the monomorphization cache (#552).
- A struct or enum with more than 64 fields no longer aborts the collector; a garbage-collected field beyond the 64th (which the descriptor mask cannot track) is rejected at compile time (#553).
- `String.substring` clamps a negative bound to 0 instead of sign-extending it to a huge index (#556).

## [2.18.101] - 2026-06-15

Continuing the codex-review fix batch (one patch per issue).

### Fixed

- `std/fs` write/append now write the raw bytes of a Raven String, so binary or non-UTF-8 content is no longer rejected (#561).
- The HTTP client keeps the raw response body bytes instead of a lossily UTF-8-decoded string, so a binary or non-UTF-8 response is not corrupted (#562).
- `WaitGroup` saturates its counter at zero, so a `done` past zero cannot drive it negative and let a later `wait` return early (#563).
- `std/http` `Server` releases the listening socket after a graceful shutdown so the port is freed (#564); `parse_query` percent-decodes keys/values and turns `+` into a space (#565); keep-alive carries bytes read past one request's body so a pipelined request is not lost (#566); a non-numeric `Content-Length` is answered with 400 rather than treated as an empty body (#567).

## [2.18.94] - 2026-06-15

A batch of fixes from a full-codebase review (codex). The version is bumped one
patch per issue fixed across this and the two preceding batches.

### Fixed

- Frontend diagnostics instead of compiler panics / Cranelift crashes / silent miscompiles: `break`/`continue` outside a loop (#547), `break` with a value outside a `loop` (#551), a named-field variant matched with `{ ... }` (#545), a constructor pattern on a non-enum value (#546), duplicate enum variants (#548), duplicate impl/trait methods (#550), and duplicate parameter names (#549).
- `std/fmt`: `format_float` no longer hangs on infinity or formats NaN as zero (#555); `pad_int` renders `Int::MIN` without a doubled minus sign (#554).
- `String.parse_float` caps its exponent so `"1e999999999"` no longer spins billions of iterations (#427).
- `std/random` `gen_range` covers a full-width range instead of returning only `lo` when the span overflows (#557).
- `std/json` rejects invalid input it used to accept: a leading zero in a number, a raw control byte in a string, and an unpaired/invalid `\u` surrogate (#558); and `stringify` no longer emits raw non-UTF-8 bytes, replacing an invalid byte with the U+FFFD escape (#559).
- `std/test` `assert_eq_float` no longer treats a NaN operand as equal (#560).
- Docs, website, and the VS Code extension: build the saved buffer (#582), broader macro/interpolation grammar (#583), v2-accurate extension docs (#584), `rvpm new` for the create-a-directory flow (#585), copy button excludes its own icon (#586), no macOS package claims (#587), no nonexistent `raven run` subcommand (#589), and `mkdocs build --strict` passes (#590).


## [2.18.72] - 2026-06-14

### Added

- `std/http` `Server` now keeps connections alive: an HTTP/1.1 connection is reused for subsequent requests instead of being closed after one, unless the client sends `Connection: close` (HTTP/1.0 stays close-by-default unless it sends `Connection: keep-alive`). Each response carries the matching `Connection` header. The read timeout bounds an idle kept-alive connection. (HTTP pipelining is not supported; clients send requests sequentially.)
- `std/http` `Server.shutdown()` performs a graceful shutdown: it stops accepting new connections, lets in-flight requests finish, and then `listen` returns. Call it from another goroutine or a handler, since `listen` blocks. Useful for clean teardown in tests and controlled stops.

## [2.18.71] - 2026-06-14

### Added

- `std/http` `Server` now applies read and write timeouts so a slow or idle client cannot hold a connection's goroutine open forever. Both default to 30 seconds and are configurable: `with_timeout(seconds)` sets both, `with_read_timeout(seconds)` and `with_write_timeout(seconds)` set one, and `0` disables a timeout.
- `std/net` `TcpStream` gains `set_write_timeout_ms`, the write-side counterpart to `set_read_timeout_ms`.

## [2.18.70] - 2026-06-14

### Added

- `std/http` `Server` gains an opt-in access log: `Server.new().with_access_log()` writes one line per request to stdout (method, path, and status code, for example `GET /tasks 200`). Logging happens at the single point every request passes through, so it covers every route with no per-handler code.

## [2.18.69] - 2026-06-14

### Fixed

- A multi-file project that uses `@derive(ToJson)` or `@derive(FromJson)` in more than one module no longer fails to compile with "the name `raven_derive_json_decode` is declared multiple times". The shared JSON helper functions the derived `from_json` bodies call are now emitted once for the whole program instead of once per module.
- A type used only as a method-call type argument is now namespaced when its module is merged, so `req.json<NewTask>()` resolves a selectively imported local or external type. Previously only types in signatures and struct literals were rewritten, so the type argument stayed unresolved.
- Generated `@derive` code from different modules no longer collides on identifier resolution. Each generated source is lexed under a distinct pseudo-file, so use-sites keyed by `(file, byte range)` stay unique across modules.

## [2.18.68] - 2026-06-14

### Changed

- When a package with bundled C (`[ffi]`) `sources` is built and no C compiler is found, the error now says what to install instead of a raw "program not found". On a windows-msvc host it points to the Visual Studio C++ Build Tools (with the `winget` command) and notes that a MinGW `gcc` cannot be used for the MSVC-targeted release; elsewhere it points at installing `gcc`/`clang` or setting `CC`. The compiler is located through the Windows registry the same way the linker's `link.exe` is, so an installed toolchain is found with no Developer Command Prompt.
- `rvpm` no longer leaks the C compiler's `cargo:warning=...` detection chatter to the console.

### Docs

- The rvpm guide documents the C-compiler prerequisite for building `[ffi]` sources, including the Windows Build Tools install command.

## [2.18.67] - 2026-06-14

### Fixed

- A `match` over integer ranges (`90..=100 -> ...`) now checks each arm's bounds. The range arms lowered through the single-value integer switch, so every range collapsed to the wildcard arm and produced the wrong result.
- The `?` operator now requires the enclosing function to return a compatible `Result`/`Option`. Using `?` in a function that returns a plain value (or a `Result` with an incompatible error type) is reported as a type error instead of silently dropping the propagated error.
- A counter `for i in start..end` now evaluates `start` before `end`. The lowering emitted the `end` binding first, so a range whose bounds have side effects ran them out of order.
- Signed integer division now guards the `MIN / -1` overflow for every width, including a 32-bit `CInt`, not just 64-bit `Int`. The narrow case could trap with no diagnostic.
- The compiler no longer panics on a `-9223372036854775808` (i64 minimum) literal pattern in a debug build; the magnitude now wraps to `i64::MIN` to match expression handling.
- The formatter keeps a comment that sits between an opening `[`/`(` and the first element. Such a comment fell outside the multi-line decision window, so the construct collapsed to one line and the comment was lost or relocated.
- Lock validation re-hashes a dependency's tree content rather than trusting the cached metadata signature, so a tampered cache entry whose sidecar was rewritten to reassert the old hash is now caught.
- An interrupted package fetch no longer leaves a partial cache directory that a later build treats as complete. A download is staged in a sibling directory and promoted into place with a single atomic rename only once it finishes.

## [2.18.66] - 2026-06-13

### Changed

- `rvpm build` no longer recompiles an unchanged `[ffi]` C source on every build. Each compiled object is reused when it is at least as new as its source, so a large bundled source (a 9 MB `sqlite3.c`) is compiled once and skipped on rebuilds.

## [2.18.65] - 2026-06-13

### Fixed

- `rvpm test` now links the package's (and its dependencies') `[ffi]` native code, so a package can test its own FFI bindings. Previously only `rvpm build`/`run` applied `[ffi]`, so a test that called into bundled C failed to link.

## [2.18.64] - 2026-06-13

### Added

- The `[ffi]` section of `rv.toml` is now wired into `rvpm build`. A package can list bundled C `sources` (relative to the package root) that are compiled and linked into the final program, plus `libs` to link (`-l<name>`) and raw `link_args`. The `[ffi]` of every dependency is collected, so a package can ship its own C (for example a bundled SQLite) and a consumer needs nothing installed. The C compiler is detected the same way the linker is (cl.exe on windows-msvc, `cc`/`gcc` elsewhere), and its output is discarded so it never reaches a program's stdout under `rvpm run`.

## [2.18.63] - 2026-06-13

### Changed

- rvpm fetches package dependencies faster and shows live progress. A new version is downloaded as a gzip tarball through codeload (one HTTP GET, no git history) instead of a clone, falling back to `git clone` when `curl`/`tar` are unavailable. The dependencies of one install are fetched concurrently, and each fetched tree hash is recorded in a sidecar file so a warm install reuses it instead of re-reading every file (a cheap metadata signature still catches an edited cache entry). Commands that fetch (`install`, `build`, `run`, `add`, `update`) now print a line per package (downloaded or cached) and a one-line summary. `rvpm cache dir`, `list`, and `clean` account for the new sidecar files.

## [2.18.62] - 2026-06-12

### Added

- Type namespacing at merge: a local or external module's types (struct, enum, trait) are now namespaced the same way its functions already were, so two packages can both export a type of the same name (for example a `Table`) without colliding. Combined with import renames, both can be used in one program: `import "...a" { Table as ATable }` and `import "...b" { Table }`. The merge rewrites a module's own type references and the type names it imports from other packages; `@derive` on a namespaced type is expanded before namespacing so its generated impls target the renamed type.

## [2.18.61] - 2026-06-11

### Added

- Import selector renames: `import "..." { name as local }` binds the import under `local`, so two packages that export the same free function (for example several config parsers that each expose `parse`) can be used in one file without wrapper modules. Works for functions and types, across std, local, and external imports.
- Module-alias qualified calls: `import "..." as m` then `m.func(...)` now resolves for local and external packages, not just std modules. The call targets the same namespaced function a selective import would bind. Type access through an alias (`m.Type`) is not included; import types with a selector (optionally renamed) instead.

## [2.18.60] - 2026-06-11

### Fixed

- A local module that imports a free function from a `github.com/...` dependency (`import "github.com/user/repo" { f }`, then `f(...)`) now resolves the call. The expander rewrites a merged local module's calls to its own functions, but did not rewrite calls to functions it imported from an external package (only the entry file and external-package modules did), so the call was left unresolved under `rvpm test` or when the importing file was reached through another module. A struct or method from the same import already worked (#517).

## [2.18.59] - 2026-06-11

### Fixed

- A `match` whose arm is itself a `match` returning `Option`/`Result` no longer fails to infer an inner `None`/`Err` when the outer match also has one. The match scrutinee type is now resolved before the arms are bound and checked, so a nested match whose scrutinee is an inference variable (the inner `v` of `match opt { Some(v) -> match v { ... } }`) binds its payloads correctly and is not wrongly flagged as having an unreachable, shadowed arm (#515).

## [2.18.58] - 2026-06-11

### Fixed

- A `match` arm that binds an enum payload (`Ok(x)`, `Err(e)`, `Some(x)`) is now typed by its variant rather than the positional index. Previously `Err(e)` was typed as the `Ok` payload, which only worked when both shared the i64 slot; once they differed (for example `Result<Bool, MyError>` or `Result<Float, MyError>` with a struct error) the payload was narrowed to the wrong machine type, producing a Cranelift verifier error or a runtime segfault. This unblocks the idiomatic `fun f() -> Result<T, MyError>` pattern with `?` (#513).

## [2.18.57] - 2026-06-11

### Fixed

- The formatter no longer inserts a blank line between a doc comment and a following `@derive`/`@repr` attribute, so a comment stays attached to the item it documents (and `rvpm doc` keeps picking it up). The blank-line gap calculation now accounts for the attribute lines, which are not part of the item's span (#511).
- `rvpm fmt` with no arguments now formats every `.rv` file in the package (skipping the build output and hidden directories) instead of assuming a `src/` directory, so it works for a library (`lib.rv` at the root) as well as an application (#511).

## [2.18.56] - 2026-06-11

### Added

- `rvpm doc` generates Markdown API documentation from the package sources into `target/doc/<name>.md`. For each `.rv` file it lists the top-level `fun`, `struct`, `enum`, `trait`, and `const` items with their signatures and the `//` comment block above each (skipping any `@derive(...)` attribute line between the comment and the item). Items whose name begins with `_` are treated as internal and omitted (#509).

## [2.18.55] - 2026-06-11

### Added

- `rvpm test` runs a package's tests: zero-argument `test_*` functions in `*_test.rv` files. Each test runs in its own process (so a failed assertion's panic fails only that test), and the command prints a per-test `ok`/`FAIL` summary and exits non-zero if any test fails. Works for libraries too. The tests assert with `std/test` (#507).

## [2.18.54] - 2026-06-11

### Added

- `rvpm build` is now library-aware: a package with a `lib.rv` and no `src/main.rv` is type-checked (no binary), so a package author can verify the library compiles. `rvpm run` reports that a library has no executable instead of erroring on a missing entry.
- `rvpm new <name>` scaffolds a package into a fresh `<name>/` directory (with `--lib` for a library), complementing `rvpm init`.
- `rvpm --version` / `-V` / `version` prints the rvpm version (#505).

## [2.18.53] - 2026-06-11

### Added

- `rvpm init` now writes a `.gitignore` (ignoring the generated `/target/`) alongside the manifest and entry file, leaving an existing `.gitignore` untouched.
- `rvpm init --lib` scaffolds a library: `lib.rv` at the repo root (the entry point other projects import via `import "github.com/<user>/<repo>"`) instead of `src/main.rv`.
- `rvpm cache` to inspect and clear the shared package cache: `cache dir` prints the cache root, `cache list` lists cached packages, and `cache clean [github.com/<user>/<repo>]` removes the whole cache or one package's cached versions (#503).

## [2.18.52] - 2026-06-11

### Added

- `@derive(Ord)` for structs and enums. It synthesizes `compare(self, other) -> Int`: a struct compares its fields in declaration order (first non-zero wins), and an enum compares variant order first, then payload slots for the same variant. A derived type can be sorted with `std/cmp` (`sort`, `sorted_by`) without a hand-written comparator (#499).

## [2.18.51] - 2026-06-11

### Fixed

- A module-level `let` with an unannotated, non-literal initializer (for example `let lock = mutex()`) now reports a clear type error pointing at the binding, asking for a type annotation, instead of crashing in codegen with `unresolved callee: Unit$<method>`. An annotated global (`let lock: Mutex = mutex()`) works as before, and literal globals are still inferred (#497).

## [2.18.50] - 2026-06-11

### Fixed

- A module-level `let x: List<T> = []` now adopts its annotated element type instead of rejecting the empty array with "empty array literals require a context type". The top-level initializer check now threads the declared `List<T>` element type as the array hint, the same as a local `let` (#498).

## [2.18.49] - 2026-06-10

### Fixed

- The formatter no longer relocates a comment that sits inside a bracketed expression to the end of the file. A call, array, or tuple that contains an interior comment is now laid out multi-line with each comment kept beside its element; a comment-free bracketed expression still collapses to one line (#426).

## [2.18.48] - 2026-06-10

### Fixed

- `select` no longer strands a receiver by stealing its wakeup. When a select consumes one channel's value, it now re-issues the wakeup to a receiver parked on any other channel it was registered on that still holds a value. A receiver (plain `recv` or `select`) that wakes a blocked sender as it commits to block now does so under the scheduler lock, closing a window where both briefly sat in the blocked set and a concurrent deadlock check reported a false deadlock (#403).

## [2.18.47] - 2026-06-10

### Fixed

- An unbuffered channel is now a true rendezvous. A send modeled the channel as a one-slot buffer, so it completed without a receiver present; it now blocks until a receiver is waiting to take the value directly. `select_recv` also wakes a blocked sender on each channel it registers on, so a select can rendezvous with an unbuffered sender instead of deadlocking (#405).

## [2.18.46] - 2026-06-10

### Fixed

- `defer` now works correctly across goroutines. The defer-frame stacks were thread-local, so a goroutine that suspended and resumed on a different worker thread lost or mixed up its deferred closures, and a deferred closure on a non-collecting worker could be freed before it ran. Defer frames now live in each goroutine's root context, so they travel with the goroutine and are marked by the collector through the same per-thread registry as the shadow-stack roots (#400).

## [2.18.45] - 2026-06-10

### Fixed

- A spawned goroutine's closure is now rooted for the goroutine's whole life. It was referenced only as a raw integer address, so a collection between the spawn and the goroutine running, or during its run, could free the closure and the values it captured (#399).

## [2.18.44] - 2026-06-10

### Fixed

- The collector's extra-roots hook, which marks the roots of every parked goroutine, is now process-global instead of thread-local. It was installed only on the thread that first spawned a goroutine, so a collection on any other worker thread skipped parked goroutines and freed their live roots (#398).

## [2.18.43] - 2026-06-10

### Fixed

- Runtime reflection now works inside a goroutine. The type metadata table was thread-local and registered only on the main thread, so `get_field`/`set_field`/`type_name_of` on a worker thread saw an empty table; it is now process-global. The field-name list is also rooted while it is built, so a collection mid-construction no longer frees it (#401).

## [2.18.42] - 2026-06-10

### Fixed

- Struct GC descriptors are now process-global instead of thread-local. They were registered only on the main thread, so a garbage collection on a worker goroutine saw an empty table and left struct pointer fields untraced, freeing them while still live (#397).

## [2.18.41] - 2026-06-10

### Fixed

- `net.close` on a stream now shuts the underlying socket down, so a goroutine parked in a blocking read returns instead of hanging forever (the read had cloned the handle, so dropping the registry copy alone did not close the socket).

### Added

- `Channel.free()` and `WaitGroup.free()` release a channel's or wait group's runtime registry entry, which otherwise leaked for the life of the process (#407).

## [2.18.40] - 2026-06-10

### Fixed

- Blocking runtime calls (`time.sleep_millis`, DNS resolution, the filesystem remove/create/list/size operations, and `net.reachable`'s resolve) now leave the collector's running set while they block, so a garbage collection on another goroutine is no longer stalled until they return (#402).

## [2.18.39] - 2026-06-10

### Fixed

- The garbage collector no longer treats buffered channel `Int` values as object pointers. It was handing each channel queue slot to the collector as a root, so an integer payload was dereferenced as a pointer and GC bits were written into arbitrary memory, corrupting the heap or crashing (#396).

## [2.18.38] - 2026-06-10

### Fixed

- A send, receive, or select on a channel id that does not exist now reports a clear panic instead of busy-spinning forever (receive/select) or silently dropping the value (send) (#406).

## [2.18.37] - 2026-06-10

### Fixed

- `process.run_with_input` no longer deadlocks when a child produces a lot of output while still reading stdin. Stdin is now fed on a separate thread while the parent drains stdout and stderr (#404).

## [2.18.36] - 2026-06-10

### Fixed

- A nested constructor pattern (`Some(Ok(x))`, `Some(0)`) is now rejected with a clear error instead of being accepted and mis-bound. Only one level of constructor binding is lowered today; nesting an inner constructor, literal, or range silently miscompiled (#409).

## [2.18.35] - 2026-06-10

### Fixed

- `match` arm guards now work. They were parsed and type-checked but silently dropped at MIR lowering, so a guarded arm ran unconditionally and the wrong arm could execute. A guarded match now lowers to a sequential fall-through, and exhaustiveness no longer counts a guarded arm as covering its pattern (#408).

## [2.18.34] - 2026-06-10

### Changed

- `std/encoding`'s `hex_decode`, `base64_decode`, and `base32_decode` now return `Result<String, Error>` and reject malformed input (an odd or wrong length, or a non-alphabet byte) instead of silently dropping or zeroing it (#434).

## [2.18.33] - 2026-06-10

### Fixed

- `regex` `find_all`, `captures`, and `split` no longer corrupt a result that contains a newline. List results now cross the runtime boundary length-prefixed instead of newline-joined, which also distinguishes an empty list from a one-element list holding the empty string (#430).

## [2.18.32] - 2026-06-10

### Fixed

- The full 64-bit integer range is now writable. A hex or binary literal reinterprets its bits, so `0xFFFFFFFFFFFFFFFF` is `-1` and `0x8000000000000000` is `i64::MIN`, and the decimal `-9223372036854775808` is accepted as `i64::MIN`. A value beyond 64 bits still errors (#420).

## [2.18.31] - 2026-06-10

### Fixed

- `a < b >> c` is no longer wrongly rejected as a chained comparison. A failed attempt to read `<...>` as type arguments used to leave the `>>` token split into a single `>`; that rewrite is now undone on backtrack. Nested generics like `Map<K, Map<K, V>>` still close on `>>` (#413).

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
