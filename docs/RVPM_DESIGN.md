# rvpm – Raven Package Manager Design

## Overview

**rvpm** is Raven’s package manager, similar to `cargo` for Rust, `npm` for Node, and `pip` for Python.

---

## Module Resolution Order

When `import foo` or `import foo from "foo"` is used, Raven resolves modules in this order:

1. **stdlib** – Bundled standard library (e.g. `std/math`, `std/string`, `std/io`)
2. **rv_env/** – Project-local dependencies (installed by `rvpm install`)
3. **lib/** – Raven install’s shared library directory (e.g. `$RAVEN_HOME/lib/` or next to `raven` binary)
4. **./** – Current directory and `./lib/` for local modules

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
│       └── string.rv
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
| std/string| String: trim, split, replace     |
| std/collections | Arrays, maps (future)       |

Import syntax: `import std.math from "std/math"` or `import math from "math"` (when std resolves first).

---

## rvpm Commands

| Command        | Description                                          |
|----------------|------------------------------------------------------|
| `rvpm init`    | Create new project (rv.toml, rv_env/, src/main.rv)   |
| `rvpm install` | Install dependencies from rv.toml                    |
| `rvpm add <pkg>` | Add and install a dependency                        |
| `rvpm run`     | Run project (calls `raven src/main.rv`)             |
| `rvpm build`   | (Future) Compile to binary or archive               |

---

## Integration with `raven`

When executing `raven main.rv`:

1. If `rv.toml` exists in the current (or parent) directory, Raven treats it as a project root.
2. Raven uses `rv_env/packages/` as an additional module search path.
3. `RAVEN_STDLIB` (or equivalent) points to the bundled stdlib.
4. Resolution order: stdlib → rv_env → lib → cwd.

---

## Implementation Phases

### Phase 1: Project scaffolding
- [ ] `rvpm init` – create rv.toml, rv_env/, src/main.rv
- [ ] Minimal rv.toml parsing

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
