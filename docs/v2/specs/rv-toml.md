# rv.toml manifest

Every Raven package is described by an `rv.toml` manifest at its root.
The manifest declares the package identity, its dependencies on other
Raven packages, optional native linker pass-through, and optional
formatter settings. This document defines the schema parsed by
`raven::manifest`. Dependency version-constraint resolution and the
wiring of `[ffi]` into the link step land in later rvpm work; this
schema defines and parses those fields but does not act on them yet.

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

- Keys must be `github.com/<user>/<repo>` paths, optionally with a
  subpath (`github.com/<user>/<repo>/<sub>...`). These are the same
  strings used by `import "github.com/..."` in source, validated by the
  shared `raven::resolve::GithubPath` parser so the manifest and the
  resolver agree on package identity. A key that is not a recognized
  GitHub path is rejected with a clear message.
- Values are the raw constraint strings, for example `"1.0"` or
  `"v1.2.3"`. They are stored as written. Constraint parsing and version
  resolution are deferred to later rvpm work.

### `[ffi]` (optional)

Native linker pass-through. Parsed and exposed here; wiring into the
actual link step is a later concern.

| Field       | Type            | Required | Default | Notes |
|-------------|-----------------|----------|---------|-------|
| `libs`      | Array of String | no       | `[]`    | Library names to link, for example `["m", "z"]`. |
| `link_args` | Array of String | no       | `[]`    | Extra raw linker arguments. |

### `[fmt]` (optional)

Formatter settings, carried from v1. Absent fields take the documented
defaults.

| Field          | Type | Required | Default | Notes |
|----------------|------|----------|---------|-------|
| `indent_width` | Int  | no       | `4`     | Spaces per indent level. |
| `wrap_width`   | Int  | no       | `100`   | Target column for wrapping. |

## Examples

### Minimal

```toml
[package]
name = "tiny"
version = "0.0.1"
```

This parses with `edition = "v2"`, no authors, no dependencies, default
`[ffi]`, and default `[fmt]`.

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
libs = ["m", "z"]
link_args = ["-L/opt/lib"]

[fmt]
indent_width = 2
wrap_width = 80
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

`rvpm init [name]` scaffolds a new package in the current directory:

- Writes `rv.toml` with `[package]` (`name`, `version = "0.1.0"`,
  `edition = "v2"`) and an empty `[dependencies]` table. When `name` is
  omitted it defaults to the current directory name.
- Writes `src/main.rv` with a hello-world `main` that compiles under the
  `raven` build.
- Refuses to overwrite an existing `rv.toml`.
