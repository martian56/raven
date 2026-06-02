# Raven

Raven is a statically typed, compiled language. You write high level code with traits, generics, and pattern matching, and the compiler turns it into a native binary through Cranelift. A small garbage collector handles memory, so there are no manual `free` calls and no borrow checker to argue with, and a C FFI is there for the moments you need to drop down to a native library.

The short version: the readability of Python, the type system and sum types of Rust, the simplicity of Go, and real C interop, in one compiled language.

```raven
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
- **Errors in the open.** `Result<T, E>` and `Option<T>` with the `?` operator instead of exceptions or `null`. There is no `null` in the language.
- **Garbage collected.** A tracing collector manages the heap. You allocate freely and never write a destructor.
- **Concurrency.** Lightweight goroutines with `spawn` and channels for passing values between them.
- **C FFI.** Declare `extern "C"` functions and call into native libraries, with C numeric types, pointers, callbacks, and small structs by value.
- **Metaprogramming.** `@derive` for the common traits and JSON, declarative macros, and compile time plus runtime reflection.
- **Tooling that ships with the language.** One canonical formatter (`rvpm fmt`), GitHub direct packages (`rvpm`), and a VS Code extension.

## A fuller taste

```raven
import std/io { println }

trait Shape {
    fun area(self) -> Float
}

struct Circle { radius: Float }
struct Rect { width: Float, height: Float }

impl Shape for Circle {
    fun area(self) -> Float = 3.14159 * self.radius * self.radius
}

impl Shape for Rect {
    fun area(self) -> Float = self.width * self.height
}

enum Expr {
    Lit(Int),
    Add(List<Expr>),
}

impl Expr {
    fun eval(self) -> Int {
        return match self {
            Lit(n) -> n,
            Add(parts) -> {
                let total = 0
                for p in parts {
                    total = total + p.eval()
                }
                total
            },
        }
    }
}

fun main() {
    let c = Circle { radius: 2.0 }
    println("area = ${c.area()}")

    let tree = Expr.Add([Expr.Lit(2), Expr.Lit(3), Expr.Lit(5)])
    println("eval = ${tree.eval()}")
}
```

## Get started

- New here? Start with [Getting Started](v2/guide/getting-started.md). It takes you from a single file to a managed project.
- Want the whole surface of the language? See the [Language Reference](v2/guide/language-reference.md).
- Looking for what the standard library gives you? The [Standard Library](v2/guide/standard-library.md) page walks through every module.
- Managing dependencies and builds? Read the [rvpm guide](v2/guide/rvpm.md).
- Coming from Raven v1? The [migration guide](v2/migration.md) maps every breaking change.

## Install

Download the installer or archive for your platform from the [releases page](https://github.com/martian56/raven/releases): `.deb`/`.rpm`/`.tar.gz` for Linux, `.msi`/`.zip` for Windows, and `.pkg`/`.tar.gz` for macOS (Intel and Apple Silicon). Each one installs the `raven` compiler and the `rvpm` package manager and adds them to your `PATH`. Compiling a program also needs a C linker (the MSVC build tools on Windows, `cc` or `clang` on Linux and macOS).

If you would rather build from source, or want to track the latest commit:

```bash
git clone https://github.com/martian56/raven.git
cd raven
cargo build --release
```

The `raven` and `rvpm` binaries land in `target/release/`.

The [VS Code extension](https://marketplace.visualstudio.com/items?itemName=martian56.raven-language) adds syntax highlighting and snippets.

## A note on versions

This site documents **Raven v2**, the compiled language. Raven v1 was a tree walking interpreter with a different syntax; its docs still live under [v1 docs](getting-started/installation.md) for anyone maintaining older code. If you are starting fresh, you want v2.
