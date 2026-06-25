# rvpm (project tool) and formatting

**rvpm** is the Raven project helper shipped with the toolchain (same repository as `raven`). It works on the package in the current directory, loading `rv.toml` from there.

## Commands

| Command | Description |
|--------|-------------|
| `rvpm init [name]` | Scaffold a new package in the current directory (`--lib` for a library). |
| `rvpm new <name>` | Scaffold a new package in a fresh `<name>/` directory (`--lib`). |
| `rvpm add <pkg>` | Add a dependency to `rv.toml`, then resolve and write `rv.lock`. |
| `rvpm install` | Resolve `rv.toml` against `rv.lock` and fill the cache. |
| `rvpm update [pkg]` | Re-resolve `rv.toml` and rewrite `rv.lock` for one package or all. |
| `rvpm build` | Compile `src/main.rv` to a binary, or type-check a `lib.rv` library. |
| `rvpm run [args]` | Build the application then run it, forwarding `args`. |
| `rvpm test` | Run `fun test_*()` tests in `*_test.rv` files. |
| `rvpm doc` | Generate Markdown API docs into `target/doc`. |
| `rvpm fmt [paths]` | Format `.rv` files in place (`--check` to verify only). |
| `rvpm fetch <pkg>` | Fetch `github.com/<user>/<repo>@<version>` into the shared cache. |
| `rvpm lock` | Generate or validate `rv.lock` for the current package. |
| `rvpm cache <sub>` | Inspect or clear the shared package cache (`dir`/`list`/`clean`). |

`rvpm init` scaffolds `rv.toml`, a `.gitignore`, and `src/main.rv` (an application) or `lib.rv` (with `--lib`).

## `rv.toml`

Minimal manifest (what `rvpm init` creates):

```toml
[package]
name = "my_project"
version = "0.1.0"
authors = []

[dependencies]
# "github.com/user/raven-math" = "1.0"
```

### Optional `[fmt]` section

Configure the pretty-printer used by `rvpm fmt`:

```toml
[fmt]
indent_width = 4   # spaces per indent level (1-16, default 4)
wrap_width = 100   # soft wrap width (40-200, default 100)
```

If `[fmt]` (or one of its fields) is missing, the default applies. A value outside its documented range is an error, not silently clamped. When you run `rvpm fmt` outside a package (no `rv.toml`), formatting uses the defaults.

Formatting **preserves** `//` and `/* */` comments by carrying them in the AST; long lines and signatures are wrapped according to `wrap_width` and `indent_width`.

## Typical workflow

```bash
rvpm init my_app      # scaffold rv.toml, .gitignore, src/main.rv here
rvpm run              # build and run src/main.rv
rvpm fmt              # format sources in place
rvpm fmt --check      # verify formatting in CI
rvpm test             # run *_test.rv tests
```

`rvpm fmt` accepts explicit file or directory paths; with none, it formats the `.rv` files of the current package.

## See also

- [Quick Start](quick-start.md) - first program and `raven` CLI
- [RVPM design notes](../RVPM_DESIGN.md) - broader package-manager plans
