# Changelog

## [1.1.0] - 2025-10-04

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
- Removed unprofessional startup messages
- Enhanced error messages and type checking
- Improved module loading and resolution

### Fixed
- Release build compilation issues (STATUS_ACCESS_VIOLATION)
- Complex assignment parsing
- Method calls on struct fields
- Type checking for custom types
- Module path resolution
- All compiler warnings

### Removed
- Unnecessary `-f` and `--repl` flags
- Unprofessional startup messages
- Dead code and unused imports

---

## [1.0.0] - 2025-10-03

### Added
- Complete programming language implementation
- Tokenizer/Lexer with comments, strings, numbers, identifiers
- Parser with full support for all language constructs
- AST generation and type checking
- Tree-walking interpreter with full execution
- CLI tool with command-line interface
- Variables & types (int, float, String, bool, arrays)
- Control flow (if/else, while, for loops)
- Functions with parameters, return types, and recursion
- String operations (concatenation, methods, formatting)
- Array operations (literals, indexing, methods)
- Built-in functions (print, input, format, len, type)
- File I/O (read_file, write_file, append_file, file_exists)
- Comments (single-line and multi-line)
- Module system (import/export functionality)
- REPL (Interactive Read-Eval-Print Loop)
- Error reporting with line/column info
- Operator precedence and variable scoping
- Method chaining support
