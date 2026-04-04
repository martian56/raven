# Raven Language

[Raven](https://github.com/martian56/raven) is a statically typed language with a small runtime. This extension adds VS Code support for `.rv` files: highlighting, snippets, and a few editor conveniences.

**Docs:** [raven documentation](https://martian56.github.io/raven/) · **Language & compiler:** [GitHub](https://github.com/martian56/raven)

---

## Features

- **Syntax highlighting** for Raven source
- **Snippets** for common patterns (variables, functions, control flow, structs, enums, `main`, printing)
- **Comments & brackets** — line comments (`//`), block comments, sensible bracket/indent behavior for `.rv` files
- **Run Raven File** — opens a terminal and runs the current file with the `raven` CLI (see below)
- **Hovers & completions** for built-in functions and common keywords

This extension does not embed the compiler. For full type-checking and navigation, use the `raven` CLI or your usual workflow outside the editor.

---

## Requirements

The **Run Raven File** command expects a `raven` executable on your system `PATH`.

- Install a [release build](https://github.com/martian56/raven/releases), or build from source using the instructions in the main repository.

If `raven` is not installed, you still get highlighting and snippets; only run-in-terminal will fail until the CLI is available.

---

## Using Run Raven File

1. Open a `.rv` file (or pick one in the Explorer).
2. Either:
   - **Command Palette** (`Ctrl+Shift+P` / `Cmd+Shift+P`) → **Raven: Run Raven File**, or  
   - **Right-click** the file in the editor or Explorer → **Run Raven File**.

The extension runs: `raven "<path-to-your-file>"` in a new terminal.

---

## Snippets

Type the prefix, then **Tab** to expand:

| Prefix | Use for |
|--------|---------|
| `let`, `fun` | Variables and functions |
| `if`, `ifelse`, `elseif`, `ifelseif` | Branches (Raven uses **`elseif`**, not `else if`) |
| `while`, `for` | Loops |
| `struct`, `impl`, `enum` | Types and methods |
| `print`, `printf`, `main` | Output and program entry |

---

## Feedback & contributing

- **Bugs & ideas:** [Issues](https://github.com/martian56/raven/issues)
- **How to contribute:** [Contributing](https://github.com/martian56/raven/blob/main/CONTRIBUTING.md)
- **Extension source** (grammar, snippets, commands): [`raven-vscode/`](https://github.com/martian56/raven/tree/main/raven-vscode) in the same repo

---

## License

[MIT](https://github.com/martian56/raven/blob/main/LICENSE)
