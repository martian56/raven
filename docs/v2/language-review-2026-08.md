# Raven language review — gap analysis (August 2026)

Reviewed at `898efbe`, version 2.26.1. Roughly 56k lines of Rust across
lexer → parser → resolver → type checker → HIR → MIR → Cranelift, plus a
runtime with a tracing GC and an M:N scheduler, a package manager, a
formatter, and a VS Code extension.

Every claim below was verified against the source or by compiling and
running a probe program with the freshly built compiler. Findings are
ordered by how much they block real adoption.

---

## 0. What is already strong

Worth stating plainly, because the rest of this document is a list of
gaps and would otherwise read as a verdict on the whole project.

- The pipeline is complete and genuinely compiles to native code. Not a
  toy: monomorphized generics, trait objects with vtables, a precise
  tracing GC with shadow stacks, a work-stealing M:N scheduler, C FFI
  with real ABI struct classification, and `@repr(C)`.
- **Diagnostics are better than most hobby languages and competitive
  with mainstream ones.** Rustc-style rendering with box drawing,
  inline caret labels, and `help:`/`note:` lines. The type checker
  recovers and reports *multiple* errors per run rather than bailing at
  the first. Messages are written in plain language ("this should be
  `Int`, but it's `String`") with actionable hints (`did you mean to
  call .to_int()?`).
- Exhaustiveness checking on `match` works and names the missing
  variants.
- 211 golden end-to-end examples plus golden corpora at every IR stage
  (parser, HIR, MIR, tycheck, resolver, fmt). This is a real
  regression net.
- Formatter, doc generator, workspaces, lock file with content hashing,
  reproducible-ish builds, `.deb`/`.rpm`/`.msi` packaging, signed
  release pipeline. The *project* engineering is more mature than the
  *language* engineering.

The gaps below are what separates this from a language someone would
choose for production work.

---

## 1. Correctness and soundness bugs

These are defects, not missing features. They should be fixed before
anything else on this list.

### 1.1 String indexing is memory-unsafe (segfault)

```rust
fun main() {
    let t = "abc"
    let c = t[0]        // type checks as Char
    print("got")
}
```

Compiles cleanly, then segfaults.

The type checker explicitly supports it — `src/tycheck/expr.rs:3019`
returns `Ty::Char` for `Ty::Str` — but codegen has no `String` case.
`lower_index_access` (`src/codegen/function.rs:1474`) unconditionally
calls `call_list_len` and reads element slots at `ELEMENT_SLOT` stride,
so a `String` header is reinterpreted as a `List` header. The "length"
and "elements pointer" it reads are arbitrary bytes from the string
object, and the subsequent load dereferences them.

This is arbitrary memory read through completely ordinary, safe-looking
code in a language whose front page claims "safety of Rust". List
indexing is correctly bounds-checked (`emit_bounds_check`), which makes
this look like an oversight rather than a design choice.

Fix: either lower `Str` indexing to a bounds-checked runtime call
returning a `Char`, or remove the `Ty::Str => Ok(Ty::Char)` arm and make
it a type error directing users to `char_at`. The first is better —
see §3.9, string ergonomics are weak.

### 1.2 Corrupt diagnostic on binary operator type errors

```rust
struct V { x: Int }
fun main() {
    let a = V{x:1}
    let b = V{x:2}
    print((a + b).x)
}
```

```
error: this should be `V and V`, but it's `V and V`
```

The two halves are identical, so the message conveys nothing. The real
error is "`+` is not defined for `V`" — the operand pair is being
formatted into a slot meant for a single expected/actual type. Given
that operator overloading does not exist (§2.4), this is the message
every user hits the first time they try to add two values of a custom
type.

### 1.3 Integer overflow wraps silently, in every build mode

```rust
let big = 9223372036854775807
print(big + 1)              // -9223372036854775808
```

No check, no panic, no warning, and no debug/release distinction to
enable one. Rust panics in debug; Go documents wrapping as the semantic
and provides `math/bits`. Raven does neither — the behavior is
undocumented in the language reference and there is no
`checked_add`/`wrapping_add`/`saturating_add` surface to express intent.
Silent wrapping is a real source of security bugs in size and index
computations.

Note this interacts with §2.5: with only a 64-bit signed `Int`, code
that needs modular arithmetic (`std/hash`, `std/random`) *relies* on the
wrapping, so simply turning on checks would break the stdlib. A
`wrapping_*` method family needs to land first.

### 1.4 Data races are unprevented and undetectable

```rust
let counter: Int = 0
// 8 goroutines each doing counter = counter + 1, 50_000 times
```

Two runs: `290879` and `256109` out of an expected `400000`. No compile
error, no warning, no runtime diagnostic.

Goroutines run in true parallel across OS threads (the README advertises
this), every `let` is mutable, module-level `let` is shared global
state, and there is no `Send`/`Sync` analysis, no ownership discipline,
and no race detector. Go has the same hole in the type system but ships
`go build -race`, which is how the ecosystem survives it. Raven has
neither the prevention nor the detection.

This is the sharpest contradiction with the marketing. "Safety of Rust"
is claimed on the grounds that GC + static types give memory safety, but
Rust's headline safety property is *freedom from data races*, and that
is exactly what is missing.

At minimum: document the hazard prominently, and ship a race detector
(a ThreadSanitizer-style shadow-memory pass in the runtime, or
instrumented loads/stores behind a `--race` flag).

---

## 2. Missing language features

### 2.1 No visibility control — zero encapsulation

There is no `pub`, no `private`, no module-level export list. Verified:

```rust
// helpers.rv
struct Account { balance: Int, secret_pin: Int }
fun internal_only(x: Int) -> Int = x * 2

// main.rv
import "./helpers" { Account, internal_only }
fun main() {
    let a = Account { balance: 10, secret_pin: 1234 }
    a.secret_pin = 0            // allowed
    print(internal_only(a.balance))   // allowed
}
```

Every declaration and every struct field is importable and mutable from
any module and any dependent package. Consequences:

- A library cannot define a stable API surface. Every internal helper is
  public API, so every refactor is a breaking change.
- No type can maintain an invariant. A `NonEmptyList`, a validated
  `Email`, a `Mutex`-guarded value — none can be built, because a caller
  can always write the field directly.
- Encapsulation is a stated pillar ("structure of Java") and is entirely
  absent.

This is the most consequential *design* gap in the language. It should
be fixed before the package ecosystem grows, because adding `pub` later
makes every existing package's API surface shrink — a breaking change
for everyone.

### 2.2 No tuples, no multiple return values

`(Int, Int)` does not parse. `Ty` has no tuple variant. Enum variants
have *tuple payloads*, but the tuple is not a first-class type.

Every function that wants to return two values must declare a named
struct. There is no `(a, b) = f()` destructuring, no `zip`, no map
iteration yielding key/value pairs without a wrapper type, no
`divmod`-style API. This ripples through the whole stdlib design.

### 2.3 Trait system is missing its expressive core

Verified absent:

| Feature | Status |
|---|---|
| Associated types (`type Item`) | parse error |
| Supertraits (`trait B: A`) | parse error |
| `where` clauses | no `where` token in the lexer |
| `impl Trait` return position | parse error |
| Associated constants | absent |
| Generic associated types | absent |

`GenericParam` is `{ name, bounds: Vec<TypePath> }` — inline bounds
only.

Without associated types, `Iterator` cannot be generic over its item
type in the normal way, and any trait with a "related type" must take it
as an extra type parameter, which infects every use site. Without
supertraits you cannot express `Ord: Eq`. Without `where` clauses,
bounds on anything other than a bare parameter (`where T::Item: Display`)
are inexpressible. These four together are what makes a trait system a
*system* rather than a set of interfaces.

Blanket impls (`impl<T: ToString> Show for T`) do parse, which is a good
sign the resolver has the machinery to grow into this.

### 2.4 No operator overloading

`a + b` on user types is a type error (with the corrupt message from
§1.2). There is no `Add`/`Sub`/`Mul`/`Neg`/`Index` trait. Numeric types
(vectors, matrices, complex numbers, decimals, big integers, durations,
money) cannot be given natural syntax. Combined with the absence of
`Ord` derive, user types also can't be sorted without hand-written
comparators.

Notably `==` *does* work on any type, and `<` works on the builtins, so
the comparison operators are special-cased rather than trait-driven —
the inconsistency is user-visible.

### 2.5 Only two numeric types

`Int` is i64, `Float` is f64. That is the entire numeric tower. There
is no `Int8/16/32`, no unsigned type of any width, no `i128`, no
`Decimal`, no big integer, and no `Byte`/`Bytes`/`[u8]` type.

Practical consequences:

- **Binary data has no representation.** Everything goes through
  `String`, which the stdlib describes as a "heap-allocated byte
  string". `std/fs` reads bytes into `String`s; `std/regex` smuggles
  lists across the FFI as length-prefixed `String`s. Any binary format
  — images, protocol buffers, compression, checksums, crypto — is
  awkward or impossible to write cleanly.
- No compact numeric arrays. A `List<Int>` of a million bytes costs 8 MB
  plus per-element slot overhead.
- Unsigned arithmetic must be emulated. `std/random` hand-rolls a
  `ushr` function to fake a logical right shift, and `std/hash` writes
  the FNV basis as "its i64 two's-complement value". The stdlib is
  visibly fighting the type system.
- The FFI has a full C type ladder (`CInt`, `CLong`, `CSize`,
  `CFloat`...) that native code cannot mirror, so every boundary
  crossing is a widening conversion.

### 2.6 Channels carry only `Int`

```rust
let ch = channel()
ch.send("hello")    // error: this should be `Int`, but it's `String`
```

Documented ("Channels carry `Int` values in this release") but this
guts the concurrency story. The CSP model is "share memory by
communicating"; a channel that can only carry a machine word forces
every real program back onto shared mutable globals — which are exactly
what §1.4 shows is unsafe. The two gaps compound: the safe path is
unavailable, so users are pushed onto the racy one.

Generic channels are the single highest-value concurrency fix.

### 2.7 Closures capture by value and cannot mutate

```rust
let n = 0
let f = fun() -> Unit { n = n + 1 }
f(); f()
print(n)        // 0
```

The assignment inside the closure silently mutates a private copy. This
compiles without a warning, which makes it a trap rather than a
limitation — the code reads as if it accumulates.

There is no by-reference capture, no `&mut`, no `Cell`/`Ref` equivalent.
Accumulator closures, callback-based APIs that update state, memoizing
closures, and event handlers are all unwritable in the natural style.
At minimum this should be a hard error or a warning, not silent
divergence from what the code appears to say.

### 2.8 Modules are flat files, no `mod`

`mod name { ... }` does not parse. Module structure is one-to-one with
the filesystem, one level deep, no nesting, no re-exports, no
`prelude`-style curation. Combined with §2.1 (no visibility), a package
has no way to organize a large API into a hierarchy.

### 2.9 Mutability defaults are inverted

`let` is mutable; `const` is immutable. Modern languages default the
other way (Rust `let`/`let mut`, Swift `let`/`var`, Kotlin `val`/`var`)
because immutable-by-default catches accidental mutation and enables
optimization. Raven's `const` at module level is also a *compile-time*
constant with restricted initializers, so it isn't a general
"immutable binding" — there is no way to declare an immutable local of
arbitrary initializer at module scope.

Changing this now would be a large breaking change; worth deciding
before 3.0 rather than after.

### 2.10 Smaller items

- **No `for c in "string"`** — `error: cannot iterate over String`.
  Character iteration requires a manual `while` loop with `char_at`.
- **`Ord` is not derivable**; enum variants with named-field payloads are
  not derivable at all.
- **Enum constructors require qualification** (`Color.Green`); the bare
  form works only in patterns. Documented, but an ergonomic wart that
  shows up constantly.
- **No const generics**, so fixed-size arrays and dimension-checked
  numeric types are out of reach.
- **No `unsafe` block or escape hatch** other than raw `CPtr` FFI —
  which is unmarked and unaudited, so there is no way to grep for the
  dangerous parts of a codebase.

---

## 3. Tooling — the largest gap overall

A language succeeds or fails on tooling. This is where Raven is furthest
from professional standard, and it matters more than any single language
feature above.

### 3.1 No language server (LSP)

**This is the single biggest adoption blocker.**

`raven-vscode` contributes a TextMate grammar, snippets, a
`language-configuration.json`, and a "Run File" command. Its
`package.json` has **no dependencies** — no `vscode-languageclient`.
The hover provider is a hardcoded JavaScript dictionary of ~20 builtin
names (`extension.ts:38+`).

So there is no:

- completion (members, imports, keywords)
- go-to-definition, find-references, call hierarchy
- rename refactoring
- **live diagnostics in the editor** — you only see errors by running
  `raven build` in a terminal
- hover types for user code, signature help, inlay hints
- document/workspace symbols, outline

The compiler already has everything an LSP needs: a resolver with
`DeclId`s, a type checker with resolved types, precise `Span`s carrying
file/line/col, and structured `RavenError`s. The pipeline is the hard
part and it exists. What's missing is (a) a query/incremental layer so
re-analysis on keystroke is cheap, (b) an error-tolerant parse mode that
produces a usable tree from incomplete input, and (c) the LSP server
binary.

Realistically this is worth more to adoption than every item in §2
combined.

### 3.2 No debugger support

No DWARF, no debug info of any kind is emitted — `grep` for
`debug`/`Dwarf` in `src/codegen/` returns nothing. Cranelift can emit
debug info; Raven doesn't ask for it.

Consequence: `gdb`/`lldb` see a stripped binary. No breakpoints on
Raven source lines, no stepping, no variable inspection, no stack
unwinding into named frames. Debugging is `print` statements only.

### 3.3 No panic backtraces

```
raven panic: boom
```

That is the entire output for a panic four frames deep. No stack trace,
no source file, no line number. For a runtime panic in a large program
there is no way to locate the failure short of bisecting with prints.
`grep -rn backtrace raven-runtime/src/` returns nothing.

The GC already maintains **precise shadow stacks** for rooting — the
frame information needed for a symbolized backtrace is largely there
already. This should be comparatively cheap to build.

### 3.4 No warnings, no lints, no error codes

There is no warning infrastructure at all. Verified: unused variables,
unused functions, and unused imports all compile silently with exit 0.
`grep -rn "warning:" src/` returns nothing; `src/error.rs` has no
severity concept, only `RavenError`.

Missing as a result:

- dead code, unused variable/import/parameter detection
- unreachable code, unused `Result` (`must_use`) — the last is serious
  in a language whose error handling is `Result`-based; silently
  discarding an `Err` is the classic bug this catches
- suspicious-construct lints (the §2.7 closure trap would be one)
- `@allow(...)` / `@deny(...)` attributes for suppression
- stable error codes (`E0308`) and a `--explain` command
- `--json` diagnostic output, which is what editors, CI annotations,
  and error-reporting tools consume

### 3.5 No `check` mode, no incremental or separate compilation

`raven` has exactly one subcommand: `build`. There is no `raven check`
for a fast type-check-only pass, no `--emit` to inspect intermediate
artifacts, no `-O` level (opt level 2 is hardcoded at
`linker.rs:195`), no debug/release distinction.

More fundamentally, `driver` produces **one object file for the entire
program** (`raven_program.o`), with the stdlib source-merged in on every
build (`resolve/stdlib.rs` splices modules into the user's AST). There
is no separate compilation of dependencies, no artifact cache keyed by
module fingerprint, no incremental recompilation. Every build recompiles
everything: user code, all dependencies, and all of the stdlib.

This is fine at the current corpus size and will become the dominant
complaint as soon as anyone writes a 50k-line program.

### 3.6 No coverage, no profiling, no runtime benchmarks

- No coverage instrumentation of any kind.
- No profiler integration — without §3.2's debug info, `perf` and
  friends can't symbolize Raven frames either.
- No `rvpm bench` and no benchmarking harness in the stdlib.
- `benchmarks/` contains **only compile-time** benchmarks. There is no
  runtime performance suite, despite "performance and control of C++"
  being the first stated design pillar. There is no data showing
  whether generated code is competitive, and no regression net against
  a codegen change making everything 3x slower.

### 3.7 No cross-compilation, and thin platform support

`Triple::host()` is hardcoded throughout `src/codegen/linker.rs` (7
call sites). There is no `--target` flag. You can only build for the
machine you are on.

Release matrix is `x86_64-unknown-linux-gnu` and
`x86_64-pc-windows-msvc`. That means:

- **No macOS at all** — not Intel, not Apple Silicon. A large share of
  developers cannot run the language.
- **No ARM64 anywhere** — no Apple Silicon, no ARM servers (AWS
  Graviton), no Raspberry Pi, no Android.
- No musl/static-linux target, no WASM, no freestanding/embedded.

For a compiled systems-adjacent language in 2026, x86_64-only is a
serious constraint. Cranelift itself supports aarch64, so the backend is
not the blocker — the linker layer and CI matrix are.

CI (`ci.yml`) builds and tests on `ubuntu-latest` only; Windows is
exercised only in the release workflow's smoke job. `cargo clippy` runs
without `-D warnings`, so lint regressions don't fail the build.

### 3.8 Test framework is minimal

`std/test` is 13 assertion functions that call `__panic`. `rvpm test`
discovers `fun test_*` in `*_test.rv` and runs each in its own process.

Missing: name filtering (`rvpm test foo::bar`), parallel execution,
setup/teardown or fixtures, expected-panic tests, table-driven test
helpers, output capture control, test timing, `--nocapture`, doc-tests,
property-based/fuzz testing, and any coverage integration.

Running one process per test also means the whole program links once and
then forks per test, which will not scale to thousands of tests.

**And the standard library has zero tests written in Raven** —
`find stdlib -name "*_test.rv"` returns nothing, for a stdlib that
includes a 44-function JSON module, a 38-function HTTP module, TLS, and
regex. The 211 golden examples test the *compiler*; nothing tests the
*library*. That is the highest-risk untested surface in the project.

### 3.9 Documentation tooling

`rvpm doc` emits a single Markdown file. There is no HTML output, no
search, no cross-linking between types, no source links, no versioned
doc hosting, and no dedicated doc-comment syntax (`src/doc.rs` takes any
contiguous `//` run above an item). Code examples in documentation are
never compiled or run, so they rot silently.

The hand-written docs at `martian56.github.io/raven` are genuinely good
— thorough, well-organized, with real tutorials. The gap is generated
*API* documentation for user packages.

---

## 4. Package management

`rvpm` covers `init/new/add/install/update/build/dist/run/test/doc/fmt/
fetch/lock/cache/workspace`. Workspaces, a content-hashed lock file, and
distribution packaging are all real and working. The gaps:

### 4.1 No semver resolution

From `src/lock/mod.rs:9`:

> "full semver range resolution is not implemented yet. For now a
> `[dependencies]` constraint string in `rv.toml` is treated as the
> literal git ref (tag or branch) to fetch"

So there are no version *ranges*. Every dependency is pinned to an exact
tag or branch. Consequences: no `^1.2` compatible-update semantics, no
unification when two dependencies need the same package at different
versions (the resolver has no choice to make, so conflicts are either
silently duplicated or hard failures), no minimum-version selection, no
`rvpm update` that means anything beyond re-fetching a moving ref.

Depending on a *branch* is also inherently non-reproducible; the tree
hash catches the drift but only as an error.

### 4.2 No registry, no publishing

Packages are `github.com/<user>/<repo>` only. There is no `rvpm publish`,
no central index, no name ownership or squatting protection, no
immutable version artifacts (a git tag can be moved), no yanking of a
broken or malicious release, no download statistics, no discovery/search,
no mirroring or vendoring for offline and air-gapped builds, and no
security advisory database or `rvpm audit`.

The GitHub-native model is a defensible bootstrap choice, and the tree
hashing in `rv.lock` is a good mitigation for tag mutation. But "install
arbitrary code from a mutable git ref, with no advisory feed and no
revocation path" is not a supply chain a company will accept.

### 4.3 Missing commands

`check`, `bench`, `clean`, `tree` (dependency graph), `audit`, `vendor`,
`publish`, `outdated`.

---

## 5. Standard library

27 modules, well-documented, with a sensible shape. The gaps are
concentrated in security and data formats.

### 5.1 No cryptography — and TLS ships anyway

`std/hash` opens with: *"non-cryptographic hashing building blocks. NOT
for security."* It provides FNV-1a, djb2, splitmix64, CRC-32, and a
checksum. There is no SHA-2, SHA-3, BLAKE3, HMAC, PBKDF2/argon2/bcrypt,
AES, or signature primitive.

Yet the language ships `std/tls` and `std/http`. So you can build a web
service but cannot hash a password, sign a token, verify a webhook
signature, or checksum a download. Every one of those is a routine
requirement, and the absence pushes users toward writing their own —
the worst outcome.

### 5.2 No CSPRNG

`std/random` is seeded splitmix64 with a time/pid entropy source. There
is nothing suitable for session tokens, nonces, salts, or key material.
Same concern as above: TLS and HTTP are present, so the use case is
present.

### 5.3 No binary data type

Covered in §2.5. Everything is `String`. No compression (gzip/zlib/zstd),
no base-N beyond what `std/encoding` offers, no binary protocol support.

### 5.4 Time is impoverished

`std/time` is 9 functions over an integer epoch. No `Duration` type, no
timezone support, no UTC/local distinction, no monotonic clock separate
from wall clock, no calendar arithmetic. Formatting/parsing is
pattern-string based against a bare `Int`.

### 5.5 Missing common modules

No logging framework, CLI argument parsing, UUID, CSV, YAML, TOML, XML,
config loading, structured error chaining/backtraces, database drivers,
templating, or async I/O abstraction. Several of these are reasonable to
leave to the ecosystem — but with §4.2's registry situation, there is no
ecosystem to leave them to. Early languages usually ship more in the
stdlib for exactly this reason.

### 5.6 Manual resource management leaks in a GC'd language

- `std/regex`: *"There is no destructor, so a caller should `free` a
  pattern when done; not freeing leaks the registry entry for the life
  of the process."*
- `std/ffi` `to_cstr`: *"copies into a buffer outside the GC and does
  not free it, so it leaks one buffer per call."*

Both are honestly documented, but both put manual lifetime management
back on the user in a garbage-collected language, with no `defer`-based
RAII idiom or finalizer hook to make it reliable. A `Drop`/finalizer
mechanism, or GC-aware handles, would close this.

---

## 6. Process and governance

Good: `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, `SECURITY.md` with a
private advisory channel, an `edition` field already in the manifest
schema, a real CHANGELOG, and a versioned release pipeline.

Missing:

- **No formal grammar.** The specs in `docs/v2/specs/` are prose. There
  is no EBNF, no reference grammar, and therefore no basis for
  alternative implementations, grammar-based fuzzing, or precise
  answers to "is this valid Raven?".
- **No stability policy.** Nothing states what backward compatibility
  guarantees a 2.x release makes, what "edition" actually gates, or how
  breaking changes are introduced. Given how much of §2 will require
  breaking changes (visibility, mutability defaults), a policy should
  exist *before* those land.
- **No conformance test suite** separable from the implementation.
- No RFC process for language changes.
- **No fuzzing.** For a compiler with a hand-written lexer and parser,
  `cargo-fuzz` on the front end would likely find crashes quickly — and
  §1.1 suggests the type-checker/codegen interface is worth
  differential-testing too.

---

## 7. Recommended priority

Ordered by value delivered per unit of effort.

**Tier 1 — fix now (soundness and honesty)**

1. Fix the `String` index segfault (§1.1). Memory unsafety in safe code.
2. Fix the corrupt binary-operator diagnostic (§1.2).
3. Make the silent-closure-capture case (§2.7) an error or warning.
4. Document the data-race hazard (§1.4) and soften the "safety of Rust"
   claim until a race detector exists.

**Tier 2 — highest adoption leverage**

5. **Language server.** Diagnostics + completion + go-to-definition
   first; the compiler already has the semantic data. Nothing else on
   this list moves adoption as much.
6. **Generic channels** (§2.6). Unblocks the entire concurrency story.
7. **Visibility (`pub`)** (§2.1). Do this before the package ecosystem
   grows — it only gets more expensive.
8. **Panic backtraces** (§3.3). Cheap given existing shadow stacks.
9. **Warning infrastructure** (§3.4), starting with unused-variable,
   unused-import, and unused-`Result`.

**Tier 3 — platform and correctness depth**

10. macOS and ARM64 targets, plus `--target` cross-compilation (§3.7).
11. DWARF debug info (§3.2).
12. Sized and unsigned integer types, plus a `Bytes` type (§2.5).
13. Tuples (§2.2).
14. Cryptographic hashing and a CSPRNG (§5.1, §5.2).
15. Stdlib test suite (§3.8) — the largest untested surface today.

**Tier 4 — maturity**

16. Semver range resolution (§4.1).
17. Associated types, supertraits, `where` clauses (§2.3).
18. Operator overloading (§2.4).
19. Incremental/separate compilation (§3.5).
20. Runtime benchmark suite (§3.6) and front-end fuzzing (§6).
21. Registry or a signed, revocable distribution story (§4.2).

---

## Summary

Raven is a real compiler with a real runtime, and the engineering
around it — golden tests, release automation, documentation, packaging —
is more disciplined than most projects at this stage. The front-end
diagnostics in particular are a genuine strength.

The gaps cluster into three groups, in descending order of importance:

1. **Tooling.** No language server, no debugger, no backtraces, no
   warnings. This is what separates a language people admire from a
   language people use, and it is where the distance is greatest.
2. **A safety claim the implementation doesn't support.** Data races are
   unprevented and undetectable, integer overflow is silent, and
   `String` indexing is memory-unsafe. Either the guarantees need to
   catch up to the marketing or the marketing needs to soften.
3. **Language features that libraries need.** No visibility means no
   stable API surface; `Int`-only channels mean no usable concurrency;
   no associated types means no expressive traits; no tuples and two
   numeric types mean a stdlib that fights its own type system.

None of these are architectural dead ends. The pipeline is well-factored
and the resolver and type checker already carry most of the information
the missing tools would need.
