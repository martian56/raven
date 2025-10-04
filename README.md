# 🦅 Raven Programming Language

**Raven** is a modern programming language and interpreter built with [Rust](https://www.rust-lang.org/), combining the best features of Rust, Python, C++, Java, and Go. It's designed to be fast, safe, expressive, and simple—without compromising power or performance.

> 🚀 **Raven v1.1.0 is now available!** A complete, production-ready programming language with professional CLI interface.

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

**Raven v1.1.0 is complete!** All core features implemented:

- [x] **Tokenizer / Lexer** - Complete with comments, strings, numbers, identifiers
- [x] **Parser** - Full support for all language constructs
- [x] **AST Generation** - Complete abstract syntax tree
- [x] **Type Checking** - Static type validation with error reporting
- [x] **Interpreter** - Tree-walking interpreter with full execution
- [x] **CLI Tool** - Complete command-line interface
- [x] **Variables & Types** - int, float, String, bool, arrays
- [x] **Control Flow** - if/else, while, for loops
- [x] **Functions** - Parameters, return types, recursion
- [x] **String Operations** - Concatenation, methods, formatting
- [x] **Array Operations** - Literals, indexing, methods (push, pop, slice, join)
- [x] **Built-in Functions** - print, input, format, len, type
- [x] **File I/O** - read_file, write_file, append_file, file_exists
- [x] **Comments** - Single-line (//) and multi-line (/* */)
- [x] **Module System** - import/export functionality
- [x] **REPL** - Interactive Read-Eval-Print Loop
- [x] **Error Reporting** - Comprehensive error messages with line/column info
- [x] **Operator Precedence** - Correct expression evaluation
- [x] **Variable Scoping** - Proper scope management
- [x] **Method Chaining** - Object.method1().method2() support
- [x] **Structs** - User-defined data structures with fields
- [x] **Enums** - User-defined types with variants and string conversion
- [x] **Complex Assignments** - object.field[index] = value support
- [x] **Professional CLI** - Python-style interface (raven file.rv, raven)

---

## 🎯 Language Features

### Data Types
- **int** - 64-bit signed integers
- **float** - 64-bit floating-point numbers  
- **String** - UTF-8 strings with rich operations
- **bool** - Boolean values (true/false)
- **Arrays** - Dynamic arrays with type safety
- **Structs** - User-defined data structures with named fields
- **Enums** - User-defined types with named variants

### Control Flow
- **if/else** - Conditional statements
- **while** - Loop with condition
- **for** - C-style for loops
- **Functions** - Parameters, return types, recursion

### String Operations
- **Concatenation** - `+` operator
- **Methods** - `slice()`, `split()`, `replace()`
- **Formatting** - `format()` with placeholders
- **Length** - `len()` function

### Array Operations  
- **Literals** - `[1, 2, 3]` syntax
- **Indexing** - `array[0]` access
- **Methods** - `push()`, `pop()`, `slice()`, `join()`
- **Bounds checking** - Automatic array bounds validation

### Built-in Functions
- **print()** - Output with formatting
- **input()** - User input
- **format()** - String formatting with `{}` placeholders
- **len()** - Length of strings and arrays
- **type()** - Type information

### File I/O
- **read_file()** - Read file contents
- **write_file()** - Write to file
- **append_file()** - Append to file
- **file_exists()** - Check file existence

### Advanced Features
- **Static Typing** - All variables must have explicit types
- **Type Checking** - Compile-time type validation
- **Error Reporting** - Detailed error messages with line/column info
- **Comments** - Single-line (`//`) and multi-line (`/* */`)
- **Module System** - `import`/`export` functionality
- **REPL** - Interactive development environment
- **Method Chaining** - `object.method1().method2()` support

---

## 📦 Installation & Usage

### Build from Source

```bash
# Clone the repository
git clone https://github.com/martian56/raven.git
cd raven

# Build the project
cargo build --release

# The binary will be at target/release/raven (or raven.exe on Windows)
```

### Running Raven Programs

```bash
# Run a Raven program (Python-style interface)
raven program.rv

# Interactive REPL mode
raven

# Show verbose output (tokens, AST, type checking)
raven program.rv -v

# Only check syntax and types (don't execute)
raven program.rv -c

# Show the Abstract Syntax Tree
raven program.rv --show-ast
```

---

## 📚 Examples

### Hello World

```raven
let message: String = "Hello, Raven!";
print(message);
```

### Variables and Types

```raven
let name: String = "Raven";
let age: int = 25;
let height: float = 5.9;
let isActive: bool = true;

print(format("Name: {}, Age: {}, Height: {}", name, age, height));
```

### Arrays and String Operations

```raven
let numbers: int[] = [1, 2, 3, 4, 5];
numbers.push(6);
print(numbers);  // [1, 2, 3, 4, 5, 6]

let text: String = "Hello World";
let words: String[] = text.split(" ");
print(len(words));  // 2

let joined: String = words.join("-");
print(joined);  // "Hello-World"
```

### Conditionals

```raven
let age: int = 25;

if (age < 18) {
    print("Too young");
} else {
    if (age < 30) {
        print("Young adult");
    } else {
        print("Mature");
    }
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

let result: int = add(10, 5);
print(result);  // 15
```

### Structs and Enums

```raven
// Struct definition
struct Person {
    name: String,
    age: int,
    isActive: bool
}

// Enum definition
enum HttpStatus {
    OK,
    NotFound,
    InternalError
}

// Usage
let person: Person = Person { name: "Alice", age: 25, isActive: true };
let status: HttpStatus = HttpStatus::OK;

// String to enum conversion (useful for JSON parsing)
let jsonStatus: String = "NotFound";
let parsedStatus: HttpStatus = enum_from_string("HttpStatus", jsonStatus);

print(format("Person: {}, Status: {}", person.name, status));
```

### File I/O

```raven
let content: String = "Hello from Raven!";
write_file("output.txt", content);

if (file_exists("output.txt")) {
    let data: String = read_file("output.txt");
    print(data);
}
```

### Interactive REPL

```bash
raven
raven> let name: String = "World";
raven> print(format("Hello, {}!", name));
Hello, World!
raven> 
```

### Complete Application Example

Check out `examples/working_calculator.rv` for a full-featured application showcasing:
- Interactive menu system
- Calculator with arithmetic operations  
- Text processor with string operations
- Number analysis with mathematical functions
- User input/output and file operations

More examples available in the `examples/` directory!

---

## 🤝 Contributing

Raven v1.1.0 is complete and ready for use! Contributions are welcome for:

- 🐛 Bug fixes and improvements
- 📚 Documentation enhancements  
- 🧪 Additional test cases
- 🚀 Performance optimizations
- 📦 Standard library modules

Feel free to ⭐ star the project and open issues for suggestions!

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
