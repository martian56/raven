# rvpm: the package manager (v2)

`rvpm` owns a Raven project: its manifest, dependencies, lock file, and
build. It drives the same compile pipeline as `raven build`, adding a
package context so `github.com/...` imports resolve through a shared
cache.

## Project layout

A package has an `rv.toml` manifest at its root. Its entry file decides
its kind: an application has `src/main.rv` (which defines `fun main()`)
and a library has `lib.rv` at the root (the file other projects import).

```
my_app/                  my_lib/
  rv.toml                  rv.toml
  rv.lock                  rv.lock
  .gitignore               .gitignore
  src/                     lib.rv         # the package API others import
    main.rv
```

`rvpm init` scaffolds an application; `rvpm init --lib` scaffolds a
library. Both also write a `.gitignore` that ignores the generated
`target/` directory. An application's binary is written to
`target/raven-out/<name>` (with a `.exe` extension on Windows); a library
has no `main`, so it is type-checked rather than compiled to a binary.

## Workspaces

A workspace groups several ordinary packages under one repository. The root
`rv.toml` may be virtual:

```toml
[workspace]
members = ["apps/api", "apps/worker", "tools/schema"]
default-member = "api"

[commands]
schema = { package = "schema", args = ["migrate"] }
```

Each member directory has its own `rv.toml`, `rv.lock`, dependencies, and
`target/` output. A root manifest may also contain `[package]`; that root
package is included automatically and must not appear in `members`. A virtual
root contains only `[workspace]` and optional `[commands]` sections.

Member paths are relative to the root and must remain inside it. rvpm rejects
missing manifests, duplicate paths, duplicate package names, symlink escapes,
unknown defaults, and commands that name unknown packages. When invoked from a
member or one of its nested directories, rvpm discovers the workspace by
walking upward and selects that member. At the root it selects
`default-member`, or the sole member when only one exists. Use `-p <name>` to
select explicitly.

`rvpm build --workspace` and `rvpm test --workspace` process every member.
`rvpm workspace list` prints the discovered packages and commands.

### Registered commands

A `[commands]` entry is structured data, not a shell command. Its `package`
must name an application member. rvpm builds that package through the normal
compiler and dependency pipeline, then executes it. `args` provides arguments
that are prepended to those supplied at invocation:

```bash
rvpm run schema up
# executes the schema package with: migrate up
```

The executable is stored in the member's normal `target/raven-out` directory.
rvpm records a fingerprint of local package files, dependency hashes, compiler,
and runtime. A later
command reuses the executable when those inputs are unchanged. Arguments and
the executable's exit code are forwarded unchanged. Use
`rvpm run -p api -- schema` when `schema` should be an ordinary argument to
`api` instead of a registered command.

## The rv.toml manifest

```toml
[package]
name = "demo"
version = "0.1.0"
authors = ["Ada", "Grace"]
edition = "v2"

[dependencies]
"github.com/martian56/raven-http" = "1.0"

[ffi]
sources = ["c/sqlite3.c"]
libs = ["m", "z"]
link_args = ["-L/opt/lib"]

[fmt]
indent_width = 4
wrap_width = 100

[dist]
targets = ["zip", "deb"]
description = "Demo application"

[[dist.assets]]
source = "assets"
dest = "assets"

[workspace]
members = ["tools/schema"]

[commands]
schema = { package = "schema", args = [] }
```

Sections:

- `[package]` (required): `name` and `version` are required. `authors`
  defaults to empty. `edition` defaults to `v2` (accepted: `v2`, `2026`).
- `[dependencies]` (optional): keys identify a whole repository and must be
  bare `github.com/<user>/<repo>` paths. Values are git refs (a tag or branch).
  Source imports may add a subpath, but manifest keys may not.
- `[ffi]` (optional): native code linked into a program that uses the
  package. `sources` are bundled C files (relative to the package root)
  that `rvpm build` compiles and links in, `libs` are libraries to link
  (`-l<name>`), and `link_args` are raw linker arguments. The `[ffi]` of
  every dependency is collected, so a package can ship its own C (for
  example a bundled SQLite) and a consumer needs no system library
  preinstalled. Building bundled `sources` does need a **C compiler** on
  the machine doing the build (see below); pure-Raven packages with no
  `[ffi]` sources do not.
- `[fmt]` (optional): `indent_width` (default 4) and `wrap_width`
  (default 100) for the formatter.
- `[dist]` (optional): package formats, output directory, application metadata,
  extra assets, Linux package dependencies, and Windows installer settings.
  See [Distributing applications](../specs/rvpm-dist.md) for the complete
  schema and required platform tools.
- `[workspace]` (optional): relative package member directories and an optional
  `default-member` package name. A manifest with this section is a workspace
  root.
- `[commands]` (optional): structured executable commands. Each entry selects a
  workspace application by package name and may provide default `args`.

Unknown fields are rejected so typos surface early.

### A C compiler for `[ffi]` sources

Compiling a package's bundled C `sources` (for example raven-sqlite's
`sqlite3.c`) needs a C compiler that matches the Raven build's ABI. On
**Windows** that is the MSVC toolchain: install the Visual Studio C++ Build
Tools, which rvpm then finds automatically, with no Developer Command Prompt
needed:

```text
winget install Microsoft.VisualStudio.2022.BuildTools --override "--quiet --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
```

A MinGW `gcc` will not work for the prebuilt Windows release: it emits GNU-ABI
objects that do not link into the MSVC-targeted build. On **Linux and macOS**,
any `cc`, `gcc`, or `clang` on `PATH` is used. Packages with no `[ffi]` sources
need only a linker, which the release ships with, so they build with no extra
toolchain.

## Dependencies

A dependency is a `github.com/<user>/<repo>` path pinned to an explicit
git ref. In source, import it by its path:

```rust
import "github.com/martian56/raven-http" as http
import "github.com/martian56/raven-http" { get }
```

A bare `github.com/<user>/<repo>` resolves to `lib.rv` at the package
root. A subpath such as `github.com/acme/greet/util/text` resolves to
`util/text.rv` within the package.

Version constraint ranges (semver) are future work. For now the
constraint string is treated as the literal git ref to fetch, so the
resolved version equals what you write.

## Commands

### rvpm init

```bash
rvpm init [name] [--lib]
```

Scaffolds a package in the current directory: `rv.toml`, a `.gitignore`,
and an entry file. By default that is an application (`src/main.rv`); with
`--lib` it is a library (`lib.rv` at the root). The name defaults to the
directory name. It will not overwrite an existing `rv.toml`, and it leaves
an existing `.gitignore` or entry file untouched.

### rvpm new

```bash
rvpm new <name> [--lib]
```

Like `init`, but scaffolds into a fresh `<name>/` directory instead of the
current one. The package name is the final path component. Use `--lib` for
a library.

### rvpm add

```bash
rvpm add github.com/<user>/<repo>[@<version>]
```

Records the dependency in `rv.toml`, then resolves and writes `rv.lock`,
populating the cache. An existing entry is updated in place. Prefer
passing `@<version>`; omitting it records the placeholder `latest`, which
does not resolve until a concrete ref is supplied. The manifest edit
preserves comments and formatting.

### rvpm install

```bash
rvpm install
```

Resolves `rv.toml` against `rv.lock` and fills the cache. When the lock
covers every direct dependency, it validates each pinned entry by
re-hashing its fetched tree; a hash mismatch aborts. When the lock is
missing or incomplete, it resolves the full transitive set fresh and
writes `rv.lock`.

### rvpm update

```bash
rvpm update [github.com/<user>/<repo>]
```

Re-resolves and rewrites `rv.lock`. With no path, every entry is
refreshed. With a path, only that dependency's subgraph is refreshed. To
bump a dependency, edit its ref in `rv.toml`, then run `update`.

### rvpm fetch

```bash
rvpm fetch github.com/<user>/<repo>@<version>
```

Fetches one exact GitHub tag or branch into the shared cache and prints its
directory. This is a low-level cache operation; normal projects should use
`add` or `install`, which also maintain `rv.lock` and resolve transitive
dependencies.

### rvpm lock

```bash
rvpm lock
```

Generates `rv.lock` when it is missing or incomplete. If the lock already
covers the manifest, the command validates every cached package against its
recorded tree hash. `install` and `build` perform the same lock maintenance
automatically; `lock` is useful for CI and explicit verification.

### rvpm build

```bash
rvpm build [-p <package> | --workspace]
```

Ensures dependencies are installed and builds the package context from the
lock. For an application it compiles `src/main.rv` to
`target/raven-out/<name>` and reports the binary path. For a library it
type-checks `lib.rv` (and its modules) without producing a binary, so a
package author can verify the library compiles before publishing.

Inside a workspace, `-p` builds one named member and `--workspace` builds every
member. With neither option, the current, default, or sole member is selected.

On Windows, an `.ico` configured as `[dist.windows].icon` is also embedded
in the executable. MSVC builds use the Windows SDK resource compiler;
GNU builds use MinGW-w64 `windres`.

### rvpm dist

```bash
rvpm dist [--target <t1,t2>] [--out-dir <dir>]
```

Builds an application and packages it as one or more of `tar`, `zip`, `deb`,
`rpm`, `msi`, or `inno`. `--target` overrides `[dist].targets`, and
`--out-dir` overrides `[dist].out_dir`. Without configured targets, the host
default is `zip` on Windows and `tar` elsewhere. Libraries are rejected
because they do not produce an application binary.

Assets may be files or directories. Directories are copied recursively while
preserving nested files and empty directories. See
[Distributing applications](../specs/rvpm-dist.md) for manifest fields,
artifact names, and the external packaging tool required by each format.

### rvpm run

```bash
rvpm run [-p <package>] [command | program arguments]
```

Builds the application (the same path as `build`), then runs the produced
binary, forwarding any arguments after `run` and exiting with the
program's exit code. A library has no executable, so `run` reports that
and exits non-zero; use `build` to type-check a library.

When the first argument matches a workspace `[commands]` entry, rvpm runs that
command's package instead. `-p` always selects a package directly and disables
registered-command matching.

### rvpm test

```bash
rvpm test [-p <package> | --workspace]
```

Discovers and runs the package's tests. A test is a zero-argument function
named `test_*` in a `*_test.rv` file anywhere in the package (commonly
under `src/` or a `tests/` directory). It asserts with `std/test`; a failed
assertion panics, which the runner reports as a failure.

Inside a workspace, `-p` tests one named member and `--workspace` tests all
members, returning failure if any member fails.

```rust
// src/math_test.rv
import std/test { assert_eq_int }
import "./main" { add }

fun test_add() {
    assert_eq_int(add(2, 3), 5)
}
```

Each test runs in its own process, so a panic from one failed assertion
fails only that test. The command prints a per-test `ok`/`FAIL` line and a
summary, and exits non-zero if any test fails:

```
running 2 tests
  ok   test_add
  FAIL test_add_wrong (raven panic: assertion failed: 4 != 5)
test result: FAILED. 1 passed; 1 failed
```

Test function names must be unique within a file. Libraries are supported:
a `*_test.rv` at the package root that imports `./lib` works without a
`src/main.rv`.

### rvpm workspace

```bash
rvpm workspace [list]
```

Discovers the workspace root from the current directory and lists its package
names, paths, and registered commands.

### rvpm fmt

```bash
rvpm fmt [--check] [paths...]
```

Formats Raven sources using the `[fmt]` settings from `rv.toml`. With no
paths it formats every `.rv` file in the package (the build output and
hidden directories are skipped), so it works for both an application
(`src/main.rv`) and a library (`lib.rv` at the root). Pass explicit files
or directories to format only those, and `--check` to verify formatting
without writing (it lists unformatted files and exits non-zero).

### rvpm doc

```bash
rvpm doc
```

Generates Markdown API documentation from the package sources into
`target/doc/<name>.md`. For each `.rv` file (excluding `*_test.rv`) it lists
the top-level `fun`, `struct`, `enum`, `trait`, and `const` items with their
signatures and the `//` comment block written directly above each. Raven has
no separate doc-comment syntax, so any contiguous run of `//` lines above an
item is its documentation; an attribute line such as `@derive(...)` between
the comment and the item is skipped. Items whose name begins with `_` are
treated as internal and omitted.

```rust
// A semantic version, ordered by major then minor.
@derive(Ord)
struct Version {
    major: Int,
    minor: Int,
}
```

documents `Version` with that comment as its description.

### rvpm cache

```bash
rvpm cache dir                              # print the cache root
rvpm cache list                             # list cached packages
rvpm cache clean                            # remove the whole cache
rvpm cache clean github.com/<user>/<repo>   # remove one package's versions
```

Inspects or clears the shared package cache. `clean` with no argument
removes the entire cache; with a `github.com/<user>/<repo>` argument it
removes every cached version of that one package. The cache is repopulated
on the next `install` or `build`.

### rvpm version

```bash
rvpm version          # also: rvpm --version, rvpm -V
```

Prints the rvpm version.

## The cache and rv.lock

Fetched dependencies live in a shared cache, one directory per resolved
version:

```
<cache_root>/github.com/<user>/<repo>@<version>/
```

The cache root is `$RVPM_CACHE_DIR` if set, otherwise
`${HOME}/.rvpm/cache` (`%USERPROFILE%` on Windows). A populated entry is
reused and never re-fetched. A new version is downloaded as a gzip tarball
(a single HTTP GET, no history), falling back to a shallow `git clone` when
that is unavailable; either way the cache stores only working-tree content.
The dependencies of one install are fetched concurrently, and each version's
tree hash is recorded in a sidecar so a warm install does not re-hash an
unchanged tree. Commands that fetch print a line per package (downloaded or
cached) and a short summary. Use `rvpm cache list` to see what is cached and
`rvpm cache clean` to clear it.

`rv.lock` pins every transitive dependency by `(source, version)` plus a
SHA-256 tree content hash, sorted deterministically so the file is stable
across runs. The hash normalizes path separators and ignores file modes
and timestamps, so the same tree hashes identically on Windows and Linux.
Check `rv.lock` in next to `rv.toml` for reproducible builds: a later
install or build fetches the pinned refs and verifies each tree still
hashes to the recorded value.
