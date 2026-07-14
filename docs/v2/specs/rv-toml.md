# rv.toml manifest

Every Raven package is described by an `rv.toml` manifest at its root.
The manifest declares the package identity, its dependencies on other
Raven packages, optional native linker pass-through, and optional
formatter and distribution settings. This document defines the schema parsed
by `raven::manifest`. `rvpm` resolves dependencies, compiles and links `[ffi]`
sources, and packages applications according to `[dist]`.

The manifest is parsed by `Manifest::from_toml_str` (string) and
`Manifest::load` (path). Unknown fields in any section are rejected so
typos surface early.

## Sections

### `[package]` (required)

| Field     | Type            | Required | Default | Notes |
|-----------|-----------------|----------|---------|-------|
| `name`    | String          | yes      | none    | The package name. Must not be empty. |
| `version` | String          | yes      | none    | Semver-style string, for example `0.1.0`. Stored verbatim; full semver validation is not performed here. |
| `authors` | Array of String | no       | `[]`    | Free-form author entries. |
| `edition` | String          | no       | `v2`    | Accepted values: `v2`, `2026`. Any other value is rejected. |

### `[dependencies]` (optional)

A table whose keys are GitHub import paths and whose values are raw
version-constraint strings. An absent or empty table means no
dependencies.

- Keys must be bare `github.com/<user>/<repo>` paths because one dependency
  identifies one whole repository. An import in Raven source may append a
  path within that package, but a manifest key with a subpath is rejected.
- Values are the raw constraint strings, for example `"1.0"` or
  `"v1.2.3"`. They are currently treated as literal Git refs. Semver-range
  selection is future work.

### `[ffi]` (optional)

Native sources and linker configuration. `rvpm build` collects this section
from the root package and all transitive dependencies.

| Field       | Type            | Required | Default | Notes |
|-------------|-----------------|----------|---------|-------|
| `sources`   | Array of String | no       | `[]`    | C source files relative to the package root. Each path must stay inside that root after canonicalization. |
| `libs`      | Array of String | no       | `[]`    | Library names to link, for example `["m", "z"]`. |
| `link_args` | Array of String | no       | `[]`    | Extra raw linker arguments. |

### `[fmt]` (optional)

Formatter settings, carried from v1. Absent fields take the documented
defaults.

| Field          | Type | Required | Default | Notes |
|----------------|------|----------|---------|-------|
| `indent_width` | Int  | no       | `4`     | Spaces per indent level. |
| `wrap_width`   | Int  | no       | `100`   | Target column for wrapping. |

`indent_width` must be in `1..=16`; `wrap_width` must be in `40..=200`.

### `[dist]` (optional)

Controls `rvpm dist`. Every field is optional; without the section, `dist`
uses `target/dist` and produces `zip` on Windows or `tar` elsewhere.

| Field | Type | Default | Notes |
|-------|------|---------|-------|
| `targets` | Array of String | host archive | Any of `tar`, `zip`, `deb`, `rpm`, `msi`, `inno`. |
| `out_dir` | String | `target/dist` | Output directory relative to the package root. |
| `display_name` | String | package name | Human-facing installer name. |
| `description` | String | `<name> <version>` | One-line package description. |
| `license` | String | empty | SPDX-style license label. |
| `homepage` | String | empty | Project URL. |
| `maintainer` | String | first package author | Debian maintainer. |
| `vendor` | String | maintainer | RPM vendor and MSI manufacturer. |

Each `[[dist.assets]]` entry has a required `source` and `dest`. A source may
be a file or directory; directories are copied recursively, including empty
directories. Both paths must be relative forward-slash paths without `..`.

`[dist.linux]` accepts `depends`, `section`, and `priority`.
`[dist.windows]` accepts an executable/installer `icon`, a stable MSI
`upgrade_code`, and `add_to_path` for MSI PATH integration. See
[rvpm dist](rvpm-dist.md) for format behavior and examples.

## Examples

### Minimal

```toml
[package]
name = "tiny"
version = "0.0.1"
```

This parses with `edition = "v2"`, no authors, no dependencies, default
`[ffi]`, default `[fmt]`, and no explicit `[dist]` section. Running `rvpm dist`
still applies the host archive and `target/dist` defaults.

### Full

```toml
[package]
name = "demo"
version = "0.1.0"
authors = ["Ada", "Grace"]
edition = "v2"

[dependencies]
"github.com/martian56/raven-http" = "1.0"
"github.com/acme/json" = "v2.3.1"

[ffi]
sources = ["c/sqlite3.c"]
libs = ["m", "z"]
link_args = ["-L/opt/lib"]

[fmt]
indent_width = 2
wrap_width = 80

[dist]
targets = ["zip", "deb"]
license = "MIT"

[[dist.assets]]
source = "README.md"
dest = "share/doc/demo/README.md"
```

## Diagnostics

The parser produces targeted errors rather than bubbling raw TOML
output:

- A missing `name` or `version` names the section and field.
- A dependency key that is not a `github.com/<user>/<repo>` path names
  the offending key.
- An unaccepted `edition` lists the accepted values.
- A TOML syntax error is reported with the `invalid rv.toml:` prefix and
  the underlying message.

## `rvpm init`

`rvpm init [name] [--lib]` scaffolds a new package in the current directory:

- Writes `rv.toml` with `[package]` (`name`, `version = "0.1.0"`,
  `edition = "v2"`) and an empty `[dependencies]` table. When `name` is
  omitted it defaults to the current directory name.
- Writes `src/main.rv` with a hello-world `main`, or `lib.rv` when `--lib`
  is passed.
- Refuses to overwrite an existing `rv.toml`.

`rvpm new <name> [--lib]` writes the same scaffold in a fresh `<name>/`
directory.
