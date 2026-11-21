# Raven

Raven is a statically typed, compiled language. You write high level code with traits, generics, and pattern matching, and the compiler turns it into a native binary through Cranelift. A small garbage collector handles memory, so there are no manual `free` calls and no borrow checker to argue with, and a C FFI is there for the moments you need to drop down to a native library.

The short version: the readability of Python, the type system and sum types of Rust, the simplicity of Go, and real C interop, in one compiled language.

```rust
fun main() {
    let names = ["Ada", "Alan", "Grace"]
    for name in names {
        print("Hello, ${name}")
    }
}
```

```bash
raven build hello.rv -o hello
./hello
```

## What you get

- **Native compilation.** Cranelift backend, single static binary, no VM and no interpreter.
- **A real type system.** Generics with trait bounds, traits for polymorphism, and sum types (`enum` with payloads) checked by an exhaustive `match`.
- **Errors in the open.** `Result[T, E]` and `Option[T]` with the `?` operator instead of exceptions or `null`. There is no `null` in the language.
- **Garbage collected.** A tracing collector manages the heap. You allocate freely and never write a destructor.
- **Concurrency.** Lightweight goroutines with `spawn` and channels for passing values between them.
- **C FFI.** Declare `extern "C"` functions and call into native libraries, with C numeric types, pointers, callbacks, and small structs by value.
- **Metaprogramming.** `@derive` for the common traits and JSON, declarative macros, and compile time plus runtime reflection.
- **Tooling that ships with the language.** `rvpm` scaffolds, resolves,
  builds, runs, tests, documents, formats, and packages applications; a VS
  Code extension provides editor support.

## A fuller taste

```rust
trait Shape {
    fun area(self) -> float
}

class Circle: Shape {
    radius: float

    fun area(self) -> float = 3.14159 * self.radius * self.radius
}

class Rect: Shape {
    width: float
    height: float

    fun area(self) -> float = self.width * self.height
}

enum Expr {
    Lit(int),
    Add(list[Expr]),
}

fun eval(e: Expr) -> int {
    match e {
        case Expr.Lit(n) { return n }
        case Expr.Add(parts) { return sum([eval(p) for p in parts]) }
    }
}

fun main() {
    let shapes: list[Shape] = [Circle(2.0), Rect(width: 3.0, height: 4.0)]
    for s in shapes {
        print("area = ${s.area():.2}")
    }
    let tree = Expr.Add([Expr.Lit(2), Expr.Lit(3), Expr.Lit(5)])
    print("eval =", eval(tree))
}
```

## Get started

- New here? Start with [Getting Started](guide/getting-started.md). It takes you from a single file to a managed project.
- Want the whole surface of the language? See the [Language Reference](guide/language-reference.md).
- Looking for what the standard library gives you? The [Standard Library](guide/standard-library.md) page walks through every module.
- Managing dependencies and builds? Read the [rvpm guide](guide/rvpm.md).
- Shipping an application? See [rvpm dist](specs/rvpm-dist.md) for archives
  and native Linux and Windows packages.

## Install

Download the installer or archive for your platform from the [releases page](https://github.com/martian56/raven/releases): `.deb`/`.rpm`/`.tar.gz` for Linux and `.msi`/`.zip` for Windows. Each one installs the `raven` compiler and the `rvpm` package manager and adds them to your `PATH`. Compiling a program also needs a C linker (the MSVC build tools on Windows, `cc` or `clang` on Linux).

If you would rather build from source, or want to track the latest commit:

```bash
git clone https://github.com/martian56/raven.git
cd raven
cargo build --release
```

The `raven` and `rvpm` binaries land in `target/release/`.

The [VS Code extension](https://marketplace.visualstudio.com/items?itemName=martian56.raven-language) adds syntax highlighting and snippets.

## A note on versions

This site documents **Raven 3**, which reads like Python with braces. Raven 2 used an older syntax that Raven 3 no longer compiles, and Raven 1 was a tree walking interpreter; neither is developed any more.
