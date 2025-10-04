# Raven Language Extension for VS Code

This extension provides syntax highlighting, snippets, and language support for the Raven programming language.

## Features

- **Syntax Highlighting**: Full syntax highlighting for Raven code
- **Code Snippets**: Common Raven patterns and constructs
- **Language Configuration**: Proper commenting, brackets, and indentation
- **File Association**: Automatic recognition of `.rv` files

## Installation

1. Copy this extension folder to your VS Code extensions directory
2. Reload VS Code
3. Open a `.rv` file to see syntax highlighting

## Usage

### Snippets

Type these prefixes and press Tab to expand:

- `let` - Variable declaration
- `fun` - Function declaration
- `if` - If statement
- `ifelse` - If-else statement
- `while` - While loop
- `for` - For loop
- `struct` - Struct definition
- `enum` - Enum definition
- `print` - Print statement
- `printf` - Print with format
- `main` - Main function

### Language Features

- **Comments**: `//` for single-line, `/* */` for multi-line
- **Brackets**: Automatic closing of `{}`, `[]`, `()`
- **Indentation**: Smart indentation based on code structure
- **Folding**: Code folding support

## Development

This extension uses:
- TextMate grammar for syntax highlighting
- VS Code language configuration
- JSON snippets for code completion

## Contributing

Feel free to submit issues and enhancement requests!

## License

MIT License
