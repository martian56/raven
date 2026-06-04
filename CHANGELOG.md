# Changelog

All notable changes to Raven are documented in this file.

## [Unreleased]

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
