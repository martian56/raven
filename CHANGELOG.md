# Changelog

All notable changes to Raven are documented in this file.

## [Unreleased]

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
