# Contributing

Thank you for your interest in contributing to Raven! We welcome contributions from developers of all skill levels.

## How to Contribute

### Reporting Issues

1. **Check existing issues** - Search [GitHub Issues](https://github.com/martian56/raven/issues)
2. **Create new issue** - Use the appropriate template
3. **Provide details** - Include steps to reproduce, expected vs actual behavior
4. **Add labels** - Help categorize the issue

### Suggesting Features

1. **Check roadmap** - See [PRODUCTION_ROADMAP.md](../PRODUCTION_ROADMAP.md)
2. **Create feature request** - Use the feature request template
3. **Describe use case** - Explain why this feature would be valuable
4. **Provide examples** - Show how the feature would work

### Code Contributions

1. **Fork the repository**
2. **Create feature branch** - `git checkout -b feature/amazing-feature`
3. **Make changes** - Follow coding standards
4. **Test thoroughly** - Ensure all tests pass
5. **Submit pull request** - Provide clear description

## Development Setup

### Prerequisites

- **Rust** - Install from [rustup.rs](https://rustup.rs/)
- **Git** - For version control
- **VS Code** - Recommended IDE with Rust extension

### Building from Source

```bash
# Clone repository
git clone https://github.com/martian56/raven.git
cd raven

# Build in debug mode
cargo build

# Build in release mode
cargo build --release

# Run tests
cargo test

# Run examples
cargo run --bin raven examples/hello.rv
```

### Running Tests

```bash
# Run all tests
cargo test

# Run specific test
cargo test test_name

# Run with output
cargo test -- --nocapture
```

## Coding Standards

### Rust Code Style

- **Follow Rust conventions** - Use `rustfmt` and `clippy`
- **Documentation** - Add doc comments for public APIs
- **Error handling** - Use proper `Result` types
- **Naming** - Use snake_case for functions, PascalCase for types

### Raven Code Style

- **Indentation** - Use 4 spaces (not tabs)
- **Line length** - Keep lines under 100 characters
- **Comments** - Use `//` for single-line, `/* */` for multi-line
- **Naming** - Use camelCase for variables, PascalCase for types

### Commit Messages

Use clear, descriptive commit messages:

```
feat: Add support for struct field modification
fix: Resolve array bounds checking issue
docs: Update installation instructions
test: Add unit tests for parser
refactor: Simplify type checking logic
```

## Project Structure

```
raven/
├── src/                 # Source code
│   ├── main.rs         # CLI entry point
│   ├── lib.rs          # Library entry point
│   ├── lexer.rs        # Lexical analysis
│   ├── parser.rs       # Syntax analysis
│   ├── ast.rs          # Abstract syntax tree
│   ├── type_checker.rs # Type checking
│   ├── code_gen.rs     # Code generation
│   └── bin/            # Binary executables
├── examples/           # Example programs
├── lib/               # Standard library
├── docs/              # Documentation
├── tests/             # Test files
├── wix/               # Windows installer
├── web/               # Website files
└── raven-vscode-extension/ # VS Code extension
```

## Areas for Contribution

### High Priority

- **Language Server Protocol (LSP)** - Full IDE support
- **Debugger** - Step-through debugging
- **Package Manager** - Dependency management
- **Concurrency** - Async/await support
- **Generics** - Generic type system

### Medium Priority

- **More built-in functions** - Additional standard library
- **Better error messages** - More helpful error reporting
- **Performance optimizations** - Faster execution
- **Cross-platform support** - Linux/macOS support
- **Documentation** - More examples and tutorials

### Low Priority

- **Code formatting** - Auto-formatter
- **Linting** - Code quality checks
- **Profiling** - Performance analysis tools
- **Benchmarking** - Performance comparisons

## Testing Guidelines

### Unit Tests

- **Test all functions** - Cover edge cases
- **Mock dependencies** - Use test doubles
- **Assert behavior** - Check expected outcomes
- **Test error cases** - Verify error handling

### Integration Tests

- **End-to-end tests** - Test complete workflows
- **Example programs** - Verify examples work
- **Standard library** - Test all modules
- **CLI interface** - Test command-line usage

### Test Structure

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_function_name() {
        // Arrange
        let input = "test input";
        
        // Act
        let result = function_under_test(input);
        
        // Assert
        assert_eq!(result, expected_output);
    }
}
```

## Pull Request Process

### Before Submitting

1. **Run tests** - `cargo test`
2. **Check formatting** - `cargo fmt`
3. **Run clippy** - `cargo clippy`
4. **Update documentation** - Add/update docs as needed
5. **Add tests** - Include tests for new features

### Pull Request Template

```markdown
## Description
Brief description of changes

## Type of Change
- [ ] Bug fix
- [ ] New feature
- [ ] Breaking change
- [ ] Documentation update

## Testing
- [ ] Tests pass
- [ ] Manual testing completed
- [ ] Examples updated

## Checklist
- [ ] Code follows style guidelines
- [ ] Self-review completed
- [ ] Documentation updated
- [ ] Tests added/updated
```

## Community Guidelines

### Code of Conduct

- **Be respectful** - Treat everyone with respect
- **Be constructive** - Provide helpful feedback
- **Be patient** - Remember we're all learning
- **Be inclusive** - Welcome contributors of all backgrounds

### Communication

- **GitHub Issues** - For bugs and feature requests
- **GitHub Discussions** - For questions and ideas
- **Pull Requests** - For code contributions
- **Documentation** - For improving docs

## Recognition

Contributors will be recognized in:
- **README.md** - Contributor list
- **Release notes** - Feature acknowledgments
- **GitHub** - Contributor statistics
- **Documentation** - Credit in relevant sections

## Getting Help

If you need help contributing:

1. **Check documentation** - Read this guide thoroughly
2. **Ask questions** - Use GitHub Discussions
3. **Join community** - Connect with other contributors
4. **Start small** - Begin with documentation or simple fixes

---

**Ready to contribute?** Check out our [GitHub Issues](https://github.com/martian56/raven/issues) for good first issues!

