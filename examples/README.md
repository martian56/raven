# Raven Examples

This directory contains essential examples demonstrating Raven language features.

## Core Language Examples

### `hello.rv`
Basic "Hello World" program demonstrating:
- Variable declarations with types
- string literals
- Print function

### `comprehensive.rv`
Comprehensive example showcasing:
- All data types (int, float, string, bool)
- Control structures (if/else, while, for loops)
- Functions with parameters and return types
- Arithmetic operations
- Boolean logic

### `functions.rv`
Function demonstration showing:
- Function declarations
- Parameters and return types
- Function calls

## Feature-Specific Examples

### `arithmetic.rv`
Mathematical operations:
- Basic arithmetic (+, -, *, /)
- Variable assignments
- Type conversions

### `conditionals.rv`
Conditional logic:
- if/else statements
- Comparison operators
- Boolean expressions

### `loops.rv`
Loop structures:
- while loops
- for loops
- Loop control

### `boolean_logic.rv`
Boolean operations:
- Logical operators (&&, ||, !)
- Boolean variables
- Truth tables

## Applications

### `working_calculator.rv`
Complete working application featuring:
- Interactive menu system
- Calculator with arithmetic operations
- Text processor with string operations
- Number analysis with mathematical functions
- User input/output
- File I/O operations

## File I/O Testing

### `test_write_file.rv`
Basic file writing test:
- write_file() function
- string content writing

### `test_format_write.rv`
Advanced file operations:
- format() function with placeholders
- Complex string formatting
- File creation with formatted content

## Usage

From the repository root (after `cargo build --release`):

```bash
# Windows PowerShell
.\target\release\raven.exe examples\hello.rv

# Unix
./target/release/raven examples/hello.rv
```

Use `raven path\to\file.rv -c` to type-check without running.

## Key Features Demonstrated

- **Static Typing**: All variables must have explicit types
- **Control Flow**: if/else, while, for loops
- **Functions**: Parameters, return types, function calls
- **string Operations**: len(), replace(), split(), format()
- **File I/O**: read_file(), write_file(), append_file(), file_exists()
- **User Interaction**: input(), print()
- **Arrays**: Array literals, indexing, methods
- **Built-in Functions**: len(), type(), format()
