# Installation

## Windows Installer (Recommended)

The easiest way to install Raven on Windows is using our professional MSI installer.

### Download and Install

1. **Download**: Go to [GitHub Releases](https://github.com/martian56/raven/releases/tag/v1.1.0)
2. **Download**: `raven-windows-x64-v1.1.0.msi` (401KB)
3. **Install**: Double-click the MSI file and follow the installer
4. **Verify**: Open Command Prompt and run `raven --version`

### What's Included

The installer includes:
- ✅ Raven executable (`raven.exe`)
- ✅ Complete standard library
- ✅ Example programs
- ✅ Documentation
- ✅ Automatic PATH integration

## Build from Source

If you prefer to build from source or you're on a different platform:

### Prerequisites

- **Rust**: Install from [rustup.rs](https://rustup.rs/)
- **Git**: For cloning the repository

### Build Steps

```bash
# Clone the repository
git clone https://github.com/martian56/raven.git
cd raven

# Build in release mode
cargo build --release

# The binary will be at target/release/raven.exe (Windows)
# or target/release/raven (Linux/macOS)
```

### Add to PATH (Manual)

If you built from source, add Raven to your PATH:

**Windows:**
```bash
# Add to PATH environment variable
setx PATH "%PATH%;C:\path\to\raven\target\release"
```

**Linux/macOS:**
```bash
# Add to ~/.bashrc or ~/.zshrc
export PATH="$PATH:/path/to/raven/target/release"
```

## VS Code Extension

For the best development experience, install the Raven VS Code extension:

### Installation

1. **Open VS Code**
2. **Go to Extensions** (Ctrl+Shift+X)
3. **Search**: "Raven Language"
4. **Install**: Click Install

### Alternative Installation

```bash
# Using VS Code command line
code --install-extension martian56.raven-language
```

### Features

The extension provides:
- ✅ Syntax highlighting for `.rv` files
- ✅ Code snippets and IntelliSense
- ✅ Hover documentation
- ✅ Auto-completion
- ✅ Run commands

## Verification

After installation, verify everything works:

```bash
# Check version
raven --version

# Run a simple program
echo 'print("Hello, Raven!");' > test.rv
raven test.rv

# Start REPL
raven
```

## Troubleshooting

### Common Issues

**"raven is not recognized"**
- Make sure Raven is in your PATH
- Restart your terminal/command prompt
- On Windows, reinstall the MSI

**"Permission denied"**
- On Linux/macOS, you may need `chmod +x raven`
- Make sure the file is executable

**VS Code Extension not working**
- Reload VS Code after installation
- Check that `.rv` files show "Raven" in the bottom-right corner
- Try opening a `.rv` file to activate the extension

### Getting Help

If you encounter issues:
- Check the [GitHub Issues](https://github.com/martian56/raven/issues)
- Create a new issue with details
- Join our discussions for community help

---

**Next**: [Quick Start Guide](quick-start.md) - Your first Raven program
