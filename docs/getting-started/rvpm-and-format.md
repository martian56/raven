# rvpm (project tool) and formatting

**rvpm** is the Raven project helper shipped with the toolchain (same repository as `raven`). It discovers a project by walking upward from the current directory until it finds `rv.toml`.

## Commands

| Command | Description |
|--------|-------------|
| `rvpm init [name]` | Create a new project: `rv.toml`, `src/main.rv`, `rv_env/`, optional `.gitignore`. |
| `rvpm run` | Run `raven` on `src/main.rv` from the project root (requires `raven` on `PATH`). |
| `rvpm fmt [paths...]` | Format `.rv` files (defaults to `src/` when run inside a project). |
| `rvpm fmt --check` | Exit with failure if any file would change (CI / pre-commit). |

`rvpm install` and `rvpm add` are reserved for future package management; they are not implemented yet.

## `rv.toml`

Minimal manifest (what `rvpm init` creates):

```toml
[package]
name = "my_project"
version = "0.1.0"
authors = []

[dependencies]
# math = "1.0"
```

### Optional `[fmt]` section

Configure the pretty-printer used by `rvpm fmt`:

```toml
[fmt]
indent_width = 4   # spaces per indent level (1–16, default 4)
wrap_width = 88    # soft wrap width (40–200, default 88)
```

If `[fmt]` is missing or a value is out of range, defaults apply. When you run `rvpm fmt` outside a project (or with no discoverable `rv.toml`), formatting uses defaults only.

Formatting **preserves** `//` and `/* */` comments by carrying them in the AST; long lines and signatures are wrapped according to `wrap_width` and `indent_width`.

## Typical workflow

```bash
rvpm init my_app
cd my_app
rvpm run              # execute src/main.rv
rvpm fmt              # format sources under src/
rvpm fmt --check      # verify formatting in CI
```

For single files without a project, you can still run the formatter explicitly from code or tooling that calls `raven`’s format API; `rvpm fmt` accepts explicit file or directory paths.

## See also

- [Quick Start](quick-start.md) — first program and `raven` CLI
- [RVPM design notes](../RVPM_DESIGN.md) — broader package-manager plans
