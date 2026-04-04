# Welcome to Raven

**Raven** is a modern programming language built with Rust, combining the best features of Rust, Python, C++, Java, and Go. It's designed to be fast, safe, expressive, and simple—without compromising power or performance.

## 🚀 Why Raven?

- **🔥 Fast** like C++ - Built with Rust for maximum performance
- **🛡️ Memory-safe** like Rust - No garbage collection overhead
- **🧠 Readable** like Python - Clean, intuitive syntax
- **🧱 Scalable** like Java - Robust type system
- **🎯 Simple** like Go - Easy to learn and use

## ✨ Key Features

### **Professional CLI Interface**
```bash
# Execute files (Python-style)
raven hello.rv

# Interactive REPL
raven

# Type-check only (parse + types; no run)
raven hello.rv -c

# Optional: project helper (init, run, fmt)
rvpm init my_app && cd my_app && rvpm run
rvpm fmt
```

### **Modern Language Features**
- **Static Typing** - Type safety without complexity
- **Structs & Enums** - User-defined data structures
- **Struct Methods** - OOP-style methods via `impl` blocks
- **Modules** - Import/export system
- **Standard Library** - Comprehensive built-in functions
- **VS Code Support** - Full syntax highlighting and IntelliSense

### **Rich Type System**
```raven
// Basic types
let name: string = "Raven";
let version: int = 1;
let isActive: bool = true;

// Arrays
let numbers: int[] = [1, 2, 3, 4, 5];

// Structs
struct Person {
    name: string,
    age: int,
    isActive: bool
}

// Enums
enum HttpStatus {
    OK,
    NotFound,
    InternalError
}
```

## 📦 Installation

### **Windows Installer**
Download the latest MSI from [GitHub Releases](https://github.com/martian56/raven/releases).

### **VS Code Extension**
Install the Raven language extension from the [VS Code Marketplace](https://marketplace.visualstudio.com/items?itemName=martian56.raven-language).

### **Build from Source**
```bash
git clone https://github.com/martian56/raven.git
cd raven
cargo build --release
```

## 🎯 Quick Start

1. **Install Raven** using the Windows installer
2. **Create a file** `hello.rv`:
   ```raven
   fun main() -> void {
       print("Hello, Raven!");
   }
   
   main();
   ```
3. **Run it**: `raven hello.rv`
4. **Try the REPL**: `raven`

## 📚 Documentation

- **[Language Reference](syntax.md)** - Complete syntax guide
- **[Standard Library](standard-library/overview.md)** - Built-in functions and modules
- **[rvpm and formatting](getting-started/rvpm-and-format.md)** - `rv.toml`, `rvpm fmt`, `[fmt]`
- **[Examples](examples/basic.md)** - Sample programs and tutorials
- **[VS Code Extension](resources/vscode-extension.md)** - Development environment setup

## 🤝 Community

- **GitHub**: [martian56/raven](https://github.com/martian56/raven)
- **Issues**: Report bugs and request features
- **Discussions**: Ask questions and share ideas

## 🚀 What's Next?

The current toolchain includes the interpreter, type checker, **rvpm** scaffolding (`init`, `run`, `fmt`), and optional **`[fmt]`** settings in `rv.toml`. Planned work includes:
- Bytecode VM or compiled backend
- Advanced type system (generics, traits)
- Concurrency (async/await)
- Full **rvpm** package install / registry
- Language server protocol (LSP)

---

**Ready to start coding with Raven?** Check out our [Getting Started Guide](getting-started/installation.md)!
