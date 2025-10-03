# Raven Language Specification (v1.0)

Raven is a modern programming language with advanced features including static typing, modules, arrays, string operations, and more.

## 🔡 Variable Declarations

```raven
let name: String = "Alice";
let age: int = 25;
let pi: float = 3.14159;
let isReady: bool = true;
let numbers: int[] = [1, 2, 3, 4, 5];
```

- Variables declared with `let` are **mutable by default**
- Type annotations are optional but recommended for clarity
- Supported types: `int`, `float`, `bool`, `String`, `int[]`, `float[]`, `bool[]`, `String[]`

---

## 🧮 Expressions

### Arithmetic Operations
```raven
let sum = 5 + 10;           // Addition
let diff = 20 - 8;          // Subtraction
let product = 6 * 7;        // Multiplication
let quotient = 15 / 3;      // Division
let remainder = 17 % 5;     // Modulo
```

### Logical Operations
```raven
let andResult = true && false;   // Logical AND
let orResult = true || false;    // Logical OR
let notResult = !true;           // Logical NOT
```

### Comparison Operations
```raven
let equal = 5 == 5;         // Equal
let notEqual = 5 != 3;      // Not equal
let less = 3 < 5;           // Less than
let greater = 7 > 4;        // Greater than
let lessEqual = 5 <= 5;     // Less than or equal
let greaterEqual = 6 >= 4;  // Greater than or equal
```

### String Operations
```raven
let greeting = "Hello, " + "World!";  // String concatenation
let mixed = "Number: " + 42;          // String + number concatenation
```

---

## 🧠 Functions

```raven
fun add(a: int, b: int) -> int {
    return a + b;
}

fun greet(name: String) -> void {
    print("Hello, {}!", name);
}

fun factorial(n: int) -> int {
    if (n <= 1) {
        return 1;
    }
    return n * factorial(n - 1);  // Recursion supported
}
```

- `fun` declares a function
- Supports multiple parameters with type annotations
- `void` return type means no return value
- Recursion is fully supported

---

## 🔁 Control Flow

### `if`, `elseif`, `else`
```raven
if (age < 18) {
    print("Too young");
} elseif (age < 30) {
    print("Young adult");
} else {
    print("Mature");
}
```

### `while` loop
```raven
let i: int = 0;
while (i < 10) {
    print(i);
    i = i + 1;
}
```

### `for` loop (C-style)
```raven
for (let i = 0; i < 10; i = i + 1) {
    print(i);
}
```

---

## 📦 Arrays

### Array Literals
```raven
let numbers: int[] = [1, 2, 3, 4, 5];
let words: String[] = ["hello", "world", "raven"];
let empty: int[] = [];
```

### Array Indexing
```raven
let first = numbers[0];     // First element
let last = numbers[len(numbers) - 1];  // Last element
numbers[2] = 99;           // Modify element
```

### Array Operations
```raven
numbers.push(6);           // Add element to end
let popped = numbers.pop(); // Remove and return last element
let slice = numbers.slice(1, 3);  // Get subarray
let joined = words.join("-");     // Join with delimiter
```

---

## 🔤 String Operations

### String Methods
```raven
let text: String = "Hello World";
let hello = text.slice(0, 5);           // "Hello"
let words = text.split(" ");            // ["Hello", "World"]
let replaced = text.replace("World", "Raven");  // "Hello Raven"
```

### String Formatting
```raven
let name: String = "Alice";
let age: int = 25;
let message = format("Hello, my name is {} and I am {} years old.", name, age);
```

---

## 🧰 Built-in Functions

### Input/Output
```raven
print("Hello World!");                    // Simple print
print("Hello, {}!", "World");            // Formatted print
let input = input("Enter your name: ");   // Read user input
```

### Type Information
```raven
let value = 42;
let typeName = type(value);  // Returns "int"
```

### Array/String Length
```raven
let arr: int[] = [1, 2, 3];
let length = len(arr);       // Returns 3

let str: String = "hello";
let strLength = len(str);    // Returns 5
```

### File I/O
```raven
write_file("data.txt", "Hello from Raven!");     // Write file
let content = read_file("data.txt");             // Read file
append_file("data.txt", "\nMore content");       // Append to file
let exists = file_exists("data.txt");            // Check if file exists
```

---

## 📁 Modules

### Module Definition
```raven
// math.rv
export fun add(a: int, b: int) -> int {
    return a + b;
}

export fun multiply(a: int, b: int) -> int {
    return a * b;
}

export let PI: float = 3.14159;
```

### Module Import
```raven
// Main program
import math from "math.rv";
import { add, multiply } from "math.rv";

let result1 = math.add(5, 3);        // Method call
let result2 = add(5, 3);              // Direct import
let pi = math.PI;                     // Access exported variable
```

---

## 💬 Comments

```raven
// This is a single-line comment
let x: int = 5; // This is an inline comment

/* This is a multi-line comment
   that can span multiple lines */

let y: int = 10; /* This is an inline multi-line comment */
```

---

## 🧪 Complete Example Program

```raven
// Import modules
import { add, multiply } from "math.rv";

// Function definition
fun fibonacci(n: int) -> int {
    if (n <= 1) {
        return n;
    }
    return fibonacci(n - 1) + fibonacci(n - 2);
}

// Main program
let name: String = input("Enter your name: ");
print("Hello, {}!", name);

// Array operations
let numbers: int[] = [1, 2, 3, 4, 5];
numbers.push(6);
print("Numbers: {}", numbers);

// String operations
let text: String = "Programming is fun";
let words: String[] = text.split(" ");
let joined: String = words.join("-");
print("Joined: {}", joined);

// File operations
write_file("output.txt", format("Hello from {}!", name));
let content: String = read_file("output.txt");
print("File content: {}", content);

// Function calls
let result: int = add(10, 20);
print("10 + 20 = {}", result);

// Control flow
for (let i = 0; i < 5; i = i + 1) {
    let fib = fibonacci(i);
    print("fibonacci({}) = {}", i, fib);
}
```

---

## 🚀 Getting Started

1. **Install Raven**: Download the Windows installer from GitHub releases
2. **Run Programs**: `raven -f program.rv`
3. **Interactive Mode**: `raven --repl`
4. **Examples**: Check the `examples/` directory for sample programs

---

## 📚 Language Features Summary

- ✅ **Static Typing**: Type-safe variable declarations
- ✅ **Functions**: With recursion and multiple parameters
- ✅ **Control Flow**: if/else, while, for loops
- ✅ **Arrays**: Dynamic arrays with methods (push, pop, slice, join)
- ✅ **Strings**: String operations and formatting
- ✅ **Modules**: Import/export system with file-based modules
- ✅ **Built-ins**: print, input, len, type, file I/O functions
- ✅ **Comments**: Single-line and multi-line comments
- ✅ **Error Handling**: Comprehensive error messages
- ✅ **REPL**: Interactive development environment
- ✅ **File I/O**: Read, write, append, and file existence checking

Raven v1.0 is a complete, production-ready programming language!