# VS Code Extension

The Raven VS Code extension adds language support for Raven v2: syntax
highlighting, snippets, hover docs for builtins, basic completion, and a
one-click build-and-run command. The extension is published as
**Raven Language** (`martian56.raven-language`), version 2.2.0.

## Installation

### From the VS Code Marketplace

1. Open VS Code.
2. Go to Extensions (`Ctrl+Shift+X`).
3. Search for "Raven Language".
4. Click Install.

### From the command line

```bash
code --install-extension martian56.raven-language
```

## Features

### Syntax highlighting

- **Keywords**: `fun`, `let`, `const`, `if`, `else if`, `else`, `while`,
  `for`, `loop`, `match`, `struct`, `enum`, `trait`, `impl`, `import`,
  `as`, `extern`, `defer`, `dyn`, `spawn`, `macro`, `return`, `break`,
  `continue`, `in`, `self`, `Self`. Else-if branches are written as two
  words, `else if`, not a single keyword.
- **Types** (PascalCase): `Int`, `Float`, `Bool`, `String`, `Char`,
  `Unit`, `Any`, `Option`, `Result`, `List`, `Map`, `Set`, `Channel`, and
  the C FFI types `CInt`, `CLong`, `CSize`, `CStr`, `CPtr`, `CFloat`,
  `CDouble`, `CFnPtr`.
- **Attributes**: `@derive(...)`, `@repr(C)`.
- **Macros**: invocations such as `name!(...)`, `name![...]`, `name!{...}`.
- **Strings**: double-quoted strings with `${...}` interpolation, `c"..."`
  C strings, and `'...'` chars, with escape highlighting.
- **Comments**: `//` line comments and `/* */` block comments.
- **Numbers**: decimal, hex, binary, and float literals.

### Hover and completion

Hovering a builtin (for example `println`, `type_name`, `to_any`,
`channel`, or the `std/ffi` helpers) shows a short description. Basic
completion offers builtins, keywords, and the core types.

### Code snippets

Type a prefix and press Tab. A selection:

| Prefix | Result |
|----------|--------|
| `let` | `let name: Int = value` |
| `leti` | `let name = value` (inferred type) |
| `const` | `const NAME: Int = value` |
| `fun` | `fun name(params) -> Unit { }` |
| `fune` | `fun name(params) -> Int = expr` |
| `main` | `fun main() { }` |
| `if` | `if condition { }` |
| `ifelse` | `if condition { } else { }` |
| `elseif` | `if` / `else if` / `else` chain |
| `while` | `while condition { }` |
| `loop` | `loop { }` |
| `for` | `for i in 0..n { }` |
| `foreach` | `for item in items { }` |
| `match` | `match value { Pattern -> result, _ -> fallback, }` |
| `struct` | `struct Name { field: Int, }` |
| `enum` | `enum Name { Variant1, Variant2(Int), }` |
| `trait` | `trait Name { fun method(self) -> Unit }` |
| `impl` | `impl Type { ... }` |
| `implfor` | `impl Trait for Type { ... }` |
| `extern` | `extern "C" { fun name(arg: CInt) -> CInt }` |
| `import` | `import std/io` |
| `imports` | `import std/io { println }` |
| `spawn` | `spawn(fun() -> Unit { })` |
| `derive` | `@derive(Eq, Hash, ToString, Debug)` |
| `macro` | `macro name { (matcher) => { template } }` |

### File association

- `.rv` files are recognized as Raven.
- The file explorer shows the Raven icon, and the status bar shows "Raven".

## Usage

### Creating Raven files

1. Create a new file (`Ctrl+N`).
2. Save it with a `.rv` extension.
3. The language mode is set to Raven automatically.

### Building and running

Raven is compiled, so there is no bare-file run mode. The extension's
**Run Raven File** command (the play button in the editor title bar, or the
`.rv` context menu) compiles the current file with `raven build` and runs
the produced native binary in an integrated terminal. The current editor is
saved first, so the build always sees the latest buffer.

From a terminal you can do the same by hand:

```bash
# Compile a single file and run the binary
raven build hello.rv -o hello
./hello
```

For anything past a single file, use the package manager:

```bash
rvpm new my_app
cd my_app
rvpm run          # builds and runs src/main.rv
```

## Troubleshooting

**Extension not working**

- Reload VS Code: `Ctrl+Shift+P`, then "Developer: Reload Window".
- Check that `.rv` files show "Raven" in the status bar.
- Verify the extension is enabled in the Extensions panel.

**Syntax highlighting not working**

- Make sure the file has a `.rv` extension.
- Check that the language mode is set to "Raven".
- Try reopening the file.

**Build command fails**

- Confirm `raven` is on your `PATH` (`raven --version`).
- Compiling needs a C linker: the MSVC build tools on Windows, or
  `cc`/`clang` on Linux.

## Development

The extension source lives in the `raven-vscode/` directory of the
repository.

```bash
git clone https://github.com/martian56/raven.git
cd raven/raven-vscode

# Install dependencies and compile the TypeScript
npm install
npm run compile

# Package a .vsix
vsce package
```

### Contributing

1. Fork the repository.
2. Make your changes in `raven-vscode/`.
3. Test in an Extension Development Host (`F5` in VS Code).
4. Open a pull request.

---

**Next**: [GitHub Repository](https://github.com/martian56/raven) for source code and issues.
