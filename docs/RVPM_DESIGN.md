# rvpm – Raven Package Manager Design

## Overview

**rvpm** is Raven’s package manager, similar to `cargo` for Rust, `npm` for Node, and `pip` for Python.

---

## Module Resolution Order

When resolving `import` paths, the **reference implementation** searches (see `src/paths.rs`): `lib/` under the current working directory, `RAVEN_LIB_PATH`, directories next to the `raven` executable, OS install locations (`/usr/share/raven/lib` on Linux, `Program Files\raven\lib` on Windows), then the current directory. A fuller **stdlib → rv_env → install lib** story is the target once `rvpm install` lands.

---

## Project Structure (`rvpm init`)

```
my_project/
├── rv.toml           # Package manifest (name, version, dependencies)
├── rv_env/          # Installed packages (like node_modules)
│   └── packages/    # Each package in its own folder
│       └── lodash@1.0.0/
│           └── ...
├── src/
│   └── main.rv      # Entry point (configurable)
└── lib/             # Optional: local modules (not published)
    └── my_util.rv
```

---

## `rv.toml` Manifest

```toml
[package]
name = "my-app"
version = "0.1.0"
authors = ["You <you@example.com>"]

[dependencies]
math = "1.0"           # From registry
json = "github:user/json"  # Git source (future)

# Optional: formatter defaults for `rvpm fmt`
[fmt]
indent_width = 4
wrap_width = 88
```

---

## `rv_env` Layout

```
rv_env/
├── packages/
│   ├── math@1.0.0/
│   │   ├── rv.toml      # Package metadata
│   │   └── math.rv      # Main module file
│   └── string@0.2.0/
│       ├── rv.toml
│       └── str.rv
└── lock.rv             # Lockfile (exact versions)
```

---

## Standard Library (Bundled)

These live inside the Raven installation and do not require `rvpm install`:

| Module   | Purpose                          |
|----------|-----------------------------------|
| std/core | Basics: print, len, type, format  |
| std/math | Math: abs, min, max, sqrt, etc.   |
| std/io   | File I/O: read_file, write_file   |
| std/string| string: trim, split, replace     |
| std/collections | Arrays, maps (future)       |

Import syntax: `import std.math from "std/math"` or `import math from "math"` (when std resolves first).

---

## rvpm Commands

| Command        | Description                                          |
|----------------|------------------------------------------------------|
| `rvpm init`    | Create new project (`rv.toml`, `rv_env/`, `src/main.rv`) — **implemented** |
| `rvpm run`     | Run project (`raven src/main.rv` from project root) — **implemented** |
| `rvpm fmt`     | Format `.rv` files; reads optional `[fmt]` in `rv.toml` — **implemented** |
| `rvpm fmt --check` | Exit with error if any file would be reformatted (CI) — **implemented** |
| `rvpm install` | Install dependencies from `rv.toml` — **not yet implemented** |
| `rvpm add <pkg>` | Add and install a dependency — **not yet implemented** |
| `rvpm build`   | (Future) Compile to binary or archive               |

---

## Integration with `raven`

- **`rvpm run`** changes to the project directory (where `rv.toml` was found) and runs `raven src/main.rv`; it does not pass extra flags to the interpreter today.
- **`rvpm fmt`** loads optional `[fmt]` from `rv.toml` and calls the formatter in the `raven` library.
- The **`raven` executable** resolves modules via `src/paths.rs` (`lib/`, `RAVEN_LIB_PATH`, install `lib/`, cwd). **It does not yet read `rv.toml` or `rv_env/` for imports**—that remains planned work aligned with Phase 2 above.

---

## Implementation Phases

### Phase 1: Project scaffolding
- [x] `rvpm init` – create `rv.toml`, `rv_env/`, `src/main.rv`
- [x] Minimal `rv.toml` presence for project discovery; `[fmt]` parsed for `rvpm fmt`

### Phase 2: Module resolution
- [ ] Add rv_env to Raven’s module search paths
- [ ] Add stdlib search path (bundled with raven)

### Phase 3: Package install
- [ ] Registry (or local) package fetch
- [ ] `rvpm install` and `rvpm add`
- [ ] Lockfile generation

### Phase 4: Advanced
- [ ] Git dependencies
- [ ] `rvpm publish`
- [ ] Version compatibility (semver)
