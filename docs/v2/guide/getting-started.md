# Getting started (v2)

Raven v2 is a compiled, statically typed language. The toolchain lexes,
parses, resolves, type checks, lowers to MIR, and emits a native binary
through Cranelift, linking against the Raven runtime.

This page takes you from a single source file to a compiled binary, then
to a managed project with `rvpm`.

## Install

Download the installer or archive for your platform from the
[releases page](https://github.com/martian56/raven/releases): `.deb`,
`.rpm`, or `.tar.gz` for Linux, and `.msi` or `.zip` for Windows. Each
installs the `raven` compiler and the `rvpm` package manager and adds them
to your `PATH`.

Compiling a program also needs a C linker on your machine: the MSVC build
tools on Windows, or `cc`/`clang` on Linux. The compiler uses it to link
the final binary.

To build from source instead (for contributors, or to track the latest
commit):

```bash
git clone https://github.com/martian56/raven.git
cd raven
cargo build --release
```

The binaries land in `target/release/`. Add that directory to your
`PATH`, or call the binaries by full path.

## Your first program

Every program starts at `fun main()`. Create `hello.rv`:

```rust
fun main() {
    print("Hello, Raven!")
}
```

`print` accepts any value that implements `ToString` (the core traits
are always in scope) and appends a newline.

## Compile and run

`raven build` compiles a source file to a native binary. Pass the output
path with `-o`:

```bash
raven build hello.rv -o hello
./hello
```

On Windows the produced binary has a `.exe` extension:

```powershell
raven build hello.rv -o hello.exe
.\hello.exe
```

The build runs the full pipeline (lex, parse, resolve, type check, HIR,
MIR, Cranelift, link). A type or syntax error is reported with the file,
line, and column, and no binary is produced.

## A managed project with rvpm

For anything past a single file, use `rvpm`, the package manager. It owns
the project layout, dependencies, and the build.

Scaffold a new project:

```bash
rvpm init my_app
cd my_app
```

`rvpm init` writes this layout:

```
my_app/
  rv.toml        # the package manifest
  src/
    main.rv      # the entry point, defining fun main()
```

The generated `rv.toml`:

```toml
[package]
name = "my_app"
version = "0.1.0"
edition = "v2"

[dependencies]
```

Build and run the project:

```bash
rvpm run
```

`rvpm run` compiles `src/main.rv` to `target/raven-out/<name>` (with a
`.exe` extension on Windows), then runs it and forwards any arguments
after `run` to your program. Use `rvpm build` to compile without running.

Continue with the [language reference](language-reference.md) for every
construct, the [standard library](standard-library.md) for the bundled
modules, and the [rvpm guide](rvpm.md) for dependencies and the lock file.
