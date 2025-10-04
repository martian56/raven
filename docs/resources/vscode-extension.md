# VS Code Extension

The Raven VS Code extension provides a complete development environment for Raven programming language.

## Installation

### From VS Code Marketplace

1. **Open VS Code**
2. **Go to Extensions** (Ctrl+Shift+X)
3. **Search**: "Raven Language"
4. **Click Install**

### From Command Line

```bash
code --install-extension martian56.raven-language
```

### Manual Installation

1. **Download**: `raven-language-1.1.5.vsix` from [GitHub Releases](https://github.com/martian56/raven/releases)
2. **Install**: `code --install-extension raven-language-1.1.5.vsix`

## Features

### Syntax Highlighting
- **Keywords**: `fun`, `let`, `if`, `while`, `for`, `struct`, `enum`
- **Types**: `int`, `float`, `bool`, `String`, `void`
- **Operators**: `+`, `-`, `*`, `/`, `%`, `==`, `!=`, `&&`, `||`
- **Comments**: `//` and `/* */`
- **Strings**: Proper string highlighting
- **Numbers**: Integer and float highlighting

### Code Snippets

Type these shortcuts and press Tab:

| Shortcut | Result |
|----------|--------|
| `let` | `let name: Type = value;` |
| `fun` | `fun name() -> Type { }` |
| `if` | `if (condition) { }` |
| `ifelse` | `if (condition) { } else { }` |
| `while` | `while (condition) { }` |
| `for` | `for (let i: int = 0; i < len; i = i + 1) { }` |
| `struct` | `struct Name { field: Type }` |
| `enum` | `enum Name { Variant1, Variant2 }` |
| `print` | `print("message");` |
| `printf` | `print(format("{}", value));` |
| `format` | `format("template", args)` |
| `main` | `fun main() -> void { }` |

### Language Configuration

- **Comments**: `//` and `/* */`
- **Brackets**: `()`, `[]`, `{}`
- **Auto-indentation**: Smart indentation for blocks
- **Word wrapping**: Proper line wrapping

### File Association

- **Extension**: `.rv` files are recognized as Raven
- **Icon**: Raven logo in file explorer
- **Language Mode**: Shows "Raven" in status bar

## Usage

### Creating Raven Files

1. **Create new file**: `Ctrl+N`
2. **Save as**: `filename.rv`
3. **Language mode**: Automatically set to Raven

### Running Raven Programs

The extension doesn't include a built-in runner, but you can:

1. **Open terminal**: `Ctrl+`` `
2. **Run program**: `raven filename.rv`
3. **Start REPL**: `raven`

### Debugging

Currently, Raven doesn't have a debugger, but you can:

1. **Add print statements**: `print("Debug: ", variable);`
2. **Use REPL**: Test code interactively
3. **Check syntax**: `raven filename.rv -c`

## Configuration

### Settings

You can customize the extension in VS Code settings:

```json
{
  "raven.language.enabled": true,
  "raven.snippets.enabled": true,
  "raven.highlighting.enabled": true
}
```

### Keybindings

Add custom keybindings in `keybindings.json`:

```json
[
  {
    "key": "ctrl+f5",
    "command": "workbench.action.terminal.sendSequence",
    "args": {
      "text": "raven ${file}\n"
    },
    "when": "resourceExtname == '.rv'"
  }
]
```

## Troubleshooting

### Common Issues

**Extension not working**
- Reload VS Code: `Ctrl+Shift+P` → "Developer: Reload Window"
- Check if `.rv` files show "Raven" in status bar
- Verify extension is enabled in Extensions panel

**Syntax highlighting not working**
- Make sure file has `.rv` extension
- Check if language mode is set to "Raven"
- Try reopening the file

**Snippets not working**
- Type the shortcut and press Tab
- Make sure snippets are enabled in settings
- Check if there are conflicts with other extensions

### Getting Help

- **GitHub Issues**: [Report bugs](https://github.com/martian56/raven/issues)
- **VS Code Marketplace**: [Extension page](https://marketplace.visualstudio.com/items?itemName=martian56.raven-language)
- **Documentation**: This site

## Development

### Building the Extension

If you want to modify the extension:

```bash
# Clone the repository
git clone https://github.com/martian56/raven.git
cd raven/raven-vscode-extension

# Install dependencies
npm install

# Build
npm run build

# Package
vsce package
```

### Contributing

1. **Fork the repository**
2. **Make changes** to the extension
3. **Test thoroughly**
4. **Submit pull request**

## Future Features

Planned improvements:
- **Language Server Protocol (LSP)** - Full IntelliSense
- **Debugger support** - Step-through debugging
- **Error highlighting** - Real-time error detection
- **Code formatting** - Auto-formatting
- **Go to definition** - Jump to function definitions
- **Hover documentation** - Function documentation on hover

---

**Next**: [GitHub Repository](https://github.com/martian56/raven) - Source code and issues

