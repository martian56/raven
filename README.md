# 🦅 Raven Programming Language

**Raven** is a new programming language and compiler built with [Rust](https://www.rust-lang.org/), combining the best features of Rust, Python, C++, Java, and Go. It’s designed to be fast, safe, expressive, and simple—without compromising power or performance.

> ⚙️ Currently in active development.

---

## ✨ Why Raven?

Raven aims to be:

- 🔥 **Fast** like C++
- 🛡️ **Memory-safe** like Rust
- 🧠 **Readable** like Python
- 🧱 **Scalable** like Java
- 🎯 **Simple** like Go

Whether you're writing system-level code or high-level applications, Raven is built to be your go-to tool—modern, efficient, and elegant.

---

## 🎯 Project Goals

- ✅ Memory safety without garbage collection  
- ✅ Clean, beginner-friendly syntax  
- ✅ First-class support for concurrency and async  
- ✅ Built-in package manager and formatter  
- ✅ Cross-platform compiler written in Rust  
- ✅ Helpful, beginner-friendly compiler errors  

---

## 🛠️ Current Status

Raven has reached its initial MVP milestone! Core features implemented:

- [x] **Tokenizer / Lexer** - Complete with full token support
- [x] **Parser** - Full support for all language features
- [x] **AST Generation** - Complete abstract syntax tree
- [x] **Type Checking** - Static type validation
- [x] **Code Generation** - Interpreter-based execution
- [x] **CLI Tool** - Full command-line interface
- [ ] Standard library (planned)
- [ ] REPL (planned)
- [ ] Advanced optimizations (planned)

---

## 📦 Installation & Usage

### Build from Source

```bash
# Clone the repository
git clone https://github.com/martian58/raven.git
cd raven

# Build the project
cargo build --release

# The binary will be at target/release/raven (or raven.exe on Windows)
```

### Running Raven Programs

```bash
# Run a Raven program
raven -f program.rv

# Show verbose output (tokens, AST, type checking)
raven -f program.rv -v

# Only check syntax and types (don't execute)
raven -f program.rv -c

# Show the Abstract Syntax Tree
raven -f program.rv --show-ast
```

---

## 📚 Examples

### Hello World

```raven
let message: String = "Hello, Raven!";
print(message);
```

### Variables and Arithmetic

```raven
let x: int = 10;
let y: int = 5;

let sum: int = x + y;
print(sum);  // Output: 15
```

### Conditionals

```raven
let age: int = 25;

if (age < 18) {
    print("Too young");
} elseif (age < 30) {
    print("Young adult");
} else {
    print("Mature");
}
```

### Loops

```raven
// While loop
let i: int = 0;
while (i < 5) {
    print(i);
    i = i + 1;
}

// For loop
for (let j: int = 0; j < 5; j = j + 1) {
    print(j);
}
```

### Functions

```raven
fun add(a: int, b: int) -> int {
    return a + b;
}

let result: int = 15;
print(result);
```

More examples available in the `examples/` directory!

---

## 🤝 Contributing

Interested in compilers, languages, or systems programming? Contributions are welcome once the core components are stable!

For now, feel free to ⭐ star the project and follow progress.

---

## 📬 Contact

- GitHub: [martian56](https://github.com/martian56)
- LinkedIn [martian56](www.linkedin.com/in/martian56)
- Issues or suggestions? Feel free to open one!

---

## 🧠 License

MIT License. See [LICENSE](./LICENSE) for details.

---

Made with ❤️ and `rustc` by [@martian56](https://github.com/martian56)
