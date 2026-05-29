# rvpm: package fetching and the shared cache

This spec covers how rvpm fetches a GitHub dependency and where it stores
the result. It describes the on-disk cache layout, the environment
override, the git clone strategy, cache-hit semantics, `.git` handling,
and the error cases. The library implementation lives in `raven::pkg`
(`src/pkg/mod.rs`); the `rvpm` binary exposes it through `rvpm fetch`.

Version-constraint resolution (semver ranges) is out of scope here. This
layer takes an explicit `version` that is a git tag or branch and fetches
exactly it. The lock file (`rv.lock`) is built on top of this layer and is
documented below.

## Package identity

A dependency is identified by a `github.com/<user>/<repo>` path plus an
explicit `version` (a git tag or branch). The path components match what
the manifest parser (`raven::manifest`) and the import resolver
(`raven::resolve::GithubPath`) accept, so all three agree on what a valid
package identity is.

## Cache layout

The cache is shared across all projects on the machine. Each fetched
package version occupies its own directory:

```
<cache_root>/<host>/<user>/<repo>@<version>/
```

For example:

```
<cache_root>/github.com/acme/json@v1.2.3/
```

The directory holds the package's working-tree content (its `rv.toml`,
`src/`, and so on). It does not hold a git history (see ".git handling").

### Cache root

`raven::pkg::cache_root()` resolves the root as follows:

1. If `RVPM_CACHE_DIR` is set, its value is the cache root. This override
   is the supported way to redirect the cache and is used by tests and to
   isolate environments.
2. Otherwise the root is `${HOME}/.rvpm/cache`, where `HOME` is `$HOME` on
   Unix and `%USERPROFILE%` on Windows.
3. If neither is set, the root falls back to `.rvpm/cache` under the
   current directory.

## Fetch and cache-hit semantics

`raven::pkg::fetch(host, user, repo, version)` returns the cache
directory for that version.

1. It computes `cache_dir(host, user, repo, version)`.
2. If that directory exists and is non-empty, it is a cache HIT: the
   directory is returned immediately and the remote is not contacted.
3. Otherwise it clones `https://<host>/<user>/<repo>` at `version` into
   the directory and returns it.

Because a pinned tag or branch is fetched once and reused, a populated
cache entry is never re-cloned or updated in place.

## Clone strategy

Cloning is performed by `raven::pkg::clone_from(url, reference, dest)`,
which runs:

```
git clone --depth 1 --branch <reference> <url> <dest>
```

The clone is shallow (`--depth 1`): only the tip of the named tag or
branch is fetched. git is invoked through `std::process::Command`; git is
expected to be installed and on `PATH`.

`clone_from` is the network seam. The real fetch path passes
`https://<host>/<user>/<repo>` as `url`. Tests pass a local repository
path instead, so the cache and fetch logic are exercised without touching
the network.

## .git handling

After a successful clone, the `dest/.git` directory is removed. The cache
stores only working-tree content. The history is not needed: a pinned tag
or branch is never updated in place, and dropping `.git` keeps the cache
lean.

## Errors

`raven::pkg::PkgError` distinguishes:

- `Io`: a filesystem operation against the cache failed (for example the
  cache directory could not be created).
- `GitNotFound`: the `git` executable could not be launched (not
  installed or not on `PATH`).
- `MissingRef`: the requested tag or branch does not exist on the remote.
  git names the missing ref on the clone path, and that detail is
  surfaced.
- `CloneFailed`: `git clone` failed for some other reason; git's stderr is
  included.

## rvpm fetch

The binary exposes the library through:

```
rvpm fetch github.com/<user>/<repo>@<version>
```

It parses the path and version, calls `raven::pkg::fetch`, and prints the
resulting cache directory. This is a manual way to exercise the fetch
path.

# The lock file (rv.lock)

`rv.lock` pins the exact resolved git ref and a content (tree) hash for
every transitive dependency of a package. It is checked in next to
`rv.toml` so a build is reproducible: a later install or build fetches the
pinned refs and verifies that each fetched tree still hashes to the
recorded value. The library implementation lives in `raven::lock`
(`src/lock/mod.rs`); the `rvpm` binary exposes it through `rvpm lock`.

## Version-constraint scope

Full semver range resolution is not implemented yet. For now a
`[dependencies]` constraint string in `rv.toml` is treated as the literal
git ref (a tag or branch) to fetch, so the resolved version equals the
constraint as written. Range resolution is future work; when it lands the
lock will record the chosen ref while `rv.toml` keeps the range. The lock
always records the resolved ref, not the constraint expression.

## Lock format

`rv.lock` is a TOML file:

```
version = 1            # lock format version

[[package]]
source = "github.com/acme/bar"
version = "v1.0.0"     # resolved git ref
hash = "sha256:<hex>"  # tree content hash

[[package]]
source = "github.com/acme/foo"
version = "v1.0.0"
hash = "sha256:<hex>"
```

Fields:

- `version`: the lock format version. The current version is `1`. A lock
  whose version is newer than the tool understands is rejected.
- `[[package]]`: one entry per transitive dependency.
  - `source`: the `github.com/<user>/<repo>` package identity.
  - `version`: the resolved git ref (tag or branch) that was fetched.
  - `hash`: the content hash of the fetched tree, `sha256:<hex>`.

Packages are written in a deterministic order, sorted by `source` then
`version`, so the lock is stable across runs and diffs cleanly.

## Tree content hash

The content hash is a deterministic digest of a package's cached file
tree. It is computed so the same tree hashes identically on Windows and
Linux: no file mode or timestamp is included, and path separators are
normalized.

Algorithm (`raven::lock::tree_hash`):

1. Walk the package directory recursively and collect every regular file.
   Any `.git` directory is skipped. (The fetch path already removes `.git`
   from a cache entry; the walk guards against it anyway.)
2. Compute each file's path relative to the package root and join its
   components with forward slashes (`/`), so the relative path is
   identical on every platform.
3. Sort the files by that relative path.
4. Feed a single SHA-256 hasher, in sorted order, for each file:
   - the relative path bytes,
   - a single NUL byte (`0x00`) separator,
   - the file length as 8 little-endian bytes,
   - the file's raw bytes.
5. The digest is formatted `sha256:<lowercase-hex>`.

The path, the length, and the NUL separator are absorbed so that moving
content between files, or splitting one file into two, changes the digest.

## Generate vs validate

`rvpm lock` (and the install/build work that builds on it) chooses between
generating and validating:

- Generate when `rv.lock` is absent, or when `rv.toml` has a dependency
  whose `source` is not present in the lock. Resolution walks the full
  transitive dependency set fresh and the lock is rewritten.
- Validate when `rv.lock` exists and already covers every direct
  dependency in `rv.toml`. Each pinned entry is fetched and its tree hash
  is recomputed and compared to the recorded hash.

A dependency present in `rv.toml` but missing from `rv.lock` triggers a
fresh resolve (and a rewrite of the lock), so adding a dependency to the
manifest and re-running picks it up.

## Hash mismatch aborts

During validation, if a fetched tree hashes to a value different from the
recorded hash, validation fails with an error that names the package, its
version, the locked hash, and the hash that was computed. Validation does
not silently update the lock on a mismatch; the caller must investigate.

## Transitive resolution and dedup

`raven::lock::resolve_and_lock(manifest)` walks the dependency graph:

1. Each direct dependency from `rv.toml` is queued with its resolved ref.
2. Each queued package is fetched into the shared cache (via
   `raven::pkg::fetch`), and its tree hash is computed.
3. The fetched package's own `rv.toml` is read, and its dependencies are
   queued in turn.
4. Packages are deduplicated by `(source, version)`, so a diamond in the
   graph (two dependents requiring the same package version) is fetched
   and hashed once.

The result is a `LockFile` with one entry per distinct transitive
`(source, version)`, sorted deterministically.

`raven::lock::validate_lock(lock)` fetches every pinned entry and verifies
its tree hash, returning the mismatch error described above on the first
discrepancy.

For test isolation, `raven::lock` also exposes
`resolve_and_lock_in(manifest, cache_root)` and
`validate_lock_in(lock, cache_root)`, which take an explicit cache root
rather than consulting `RVPM_CACHE_DIR`. Tests pre-seed a temporary cache
root so resolution is a cache hit and never touches the network.

## rvpm lock

The binary exposes the library through:

```
rvpm lock
```

It loads `rv.toml` from the current directory. If `rv.lock` exists and
covers every dependency, it validates the lock against the cache.
Otherwise it resolves the full transitive set fresh and writes `rv.lock`.
`rvpm lock` is a direct way to exercise generation and validation; the
day to day workflow uses `add`, `install`, and `update` below.

# Dependency commands (add, install, update)

`rvpm add`, `rvpm install`, and `rvpm update` are the day to day
dependency workflow. They wire `raven::manifest`, `raven::pkg`, and
`raven::lock` together. The orchestration lives in `raven::ops`
(`src/ops/mod.rs`); the `rvpm` binary stays thin and calls it. Each
operation has an `_in(..., cache_root)` variant that takes an explicit
cache root so it can be tested against a pre-seeded temporary cache
without the global `RVPM_CACHE_DIR` override.

All three resolve against the literal-ref constraint model: a
`[dependencies]` value in `rv.toml` is the git tag or branch to fetch, so
the resolved version equals the constraint as written (see
"Version-constraint scope" above). Range based selection is future work.

## rvpm add

```
rvpm add github.com/<user>/<repo>[@<version>]
```

`add` records the dependency in `rv.toml` under `[dependencies]`, then
resolves the manifest and writes `rv.lock`, populating the cache.

- The key is the `github.com/<user>/<repo>` path. The optional
  `@<version>` is the git ref recorded as the constraint.
- When `@<version>` is omitted, the placeholder constraint `"latest"` is
  recorded. Real latest-tag resolution is future work, so a placeholder
  constraint will not resolve until a concrete ref is supplied. Prefer
  passing `@<version>`.
- An existing entry for the same package is updated in place, not
  duplicated. When the recorded version changes, the command reports the
  previous and new versions. When it is identical, nothing changes.

The manifest edit preserves the rest of the file, including comments,
because it is applied with `toml_edit`. After the edit the new text is
re-parsed with `Manifest::from_toml_str` as a guard; the written
`rv.toml` is always a valid manifest.

The edit is applied before resolution. If resolution then fails (for
example a placeholder constraint, or a ref absent from the cache and the
remote), the `rv.toml` edit still persists but `rv.lock` is not written.
Re-run `add` or `install` with a resolvable ref to complete the lock.

## rvpm install

```
rvpm install
```

`install` re-resolves `rv.toml` against `rv.lock` and fills the cache.

- When `rv.lock` exists and covers every direct dependency in `rv.toml`,
  the lock is validated: each pinned entry is fetched and its tree hash is
  recomputed and compared. A mismatch aborts with the hash-mismatch error
  and a non-zero exit, naming the package; the lock is not rewritten.
- When `rv.lock` is missing, or does not cover the manifest, the full
  transitive set is resolved fresh and `rv.lock` is written.

`install` prints what happened (validated N packages, or resolved N
packages and wrote `rv.lock`).

## rvpm update

```
rvpm update [github.com/<user>/<repo>]
```

`update` re-resolves `rv.toml` and rewrites `rv.lock`.

- With no package path, every entry is re-resolved from `rv.toml` and the
  whole lock is rewritten.
- With a package path, only that dependency's subgraph is re-resolved; its
  lock entry and its transitive entries are refreshed while the rest of
  the lock is preserved. The path must already be a dependency in
  `rv.toml`; otherwise the command reports an error and exits non-zero.

Under the literal-ref model, "update" picks up a ref that was edited in
`rv.toml`: change the constraint, then run `update` (optionally naming the
package) to bump the pinned version and hash in the lock. When range based
constraints land, `update` will choose a newer ref within the range while
`rv.toml` keeps the range; that selection is future work.

# Build and run (build, run)

`rvpm build` and `rvpm run` compile a package and run it. They drive the
same compile pipeline the `raven` binary uses (lex, parse, expand, resolve,
type check, HIR, MIR, Cranelift, link), differing only in that rvpm
supplies a package context so external (`github.com/...`) imports resolve
through the rvpm cache. The orchestration lives in `raven::ops`
(`src/ops/mod.rs`); the reusable pipeline lives in `raven::driver`
(`src/driver/mod.rs`); the `rvpm` binary stays thin and calls them. Both
have an `_in(..., cache_root)` variant that takes an explicit cache root so
they can be tested against a pre-seeded temporary cache without the global
`RVPM_CACHE_DIR` override.

## Entry file and output path conventions

- The package entry file is `src/main.rv`, relative to the project root
  (the directory holding `rv.toml`). It must define `fun main()`.
- The built binary is written to `target/raven-out/<name>`, relative to the
  project root, where `<name>` is `[package].name`. On Windows the binary
  has a `.exe` extension (`target/raven-out/<name>.exe`); on other hosts it
  has no extension.

## External import resolution through the cache

An `import "github.com/<user>/<repo>[/<sub>...]"` in the entry file (or in a
local or external module reachable from it) resolves to a cached source file
as follows:

1. The project's `rv.lock` maps the `github.com/<user>/<repo>` source to its
   pinned `version`. `rvpm build` ensures the lock exists first (see "Ensure
   installed" below).
2. `raven::pkg::cache_dir_in(cache_root, "github.com", <user>, <repo>,
   <version>)` gives the package's cached directory.
3. The import `subpath` selects the `.rv` file within that directory:
   - A bare `github.com/<user>/<repo>` (no subpath) resolves to `lib.rv` at
     the cached package root. This is the package entry convention for a
     library.
   - A subpath selects a file by joining its components and appending `.rv`.
     So `github.com/acme/greet/lib` resolves to `<cachedir>/lib.rv`, and
     `github.com/acme/greet/util/text` resolves to
     `<cachedir>/util/text.rv`.

The resolved source is fed through the same merge core the bundled stdlib
and local module paths use (see `docs/v2/specs/resolver.md`), with an
external namespace `ext.<host>.<user>.<repo>.<hash>`, where `<hash>` is
derived from the resolved source path. The external package's own
dependencies (from its cached `rv.toml`) are merged transitively and
deduplicated by resolved source path, and an external module's own `import
std/...` lines pull in the bundled modules it needs. A selective import
`import "github.com/.../lib" { name }` binds the bare `name` to the
namespaced symbol, exactly as bundled and local imports do, so the type
checker finds the merged declaration.

## Package-context plumbing

The package context (the cache root plus the loaded `rv.lock` source-to-
version map) is `raven::resolve::PackageContext`. It is threaded explicitly
into the pipeline as an `Option<&PackageContext>`:

- `expand_with_stdlib_ctx(file, ctx)` resolves external imports through the
  cache when `ctx` is `Some`. `expand_with_stdlib(file)` is the no-context
  form and behaves exactly as the bundled-plus-local path.
- `resolve_file_ctx(file, loader, ctx)` binds external selectors to the
  `ext.`-namespaced symbols. `resolve_file(file, loader)` is the no-context
  form.

`rvpm build` passes `Some(ctx)`; a plain `raven build` passes `None`, so an
external import in a single-file `raven build` stays deferred and surfaces
as an unresolved import. This keeps the single-file build and the codegen
smoke harness unchanged for bundled and local programs.

## rvpm build

```
rvpm build
```

`build` loads `rv.toml`, ensures dependencies are installed (the install
path: validate an up-to-date `rv.lock` against the cache, or resolve a fresh
lock and write it, reusing `raven::ops::install`), loads the lock to build
the package context, then compiles `src/main.rv` to
`target/raven-out/<name>` and reports the binary path. A missing
`src/main.rv` is reported and the command exits non-zero.

## rvpm run

```
rvpm run [program arguments]
```

`run` builds the package (the same path as `build`), then runs the produced
binary, forwarding any arguments after `run` to the program and exiting with
the program's exit code.
