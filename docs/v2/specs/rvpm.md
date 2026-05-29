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
The full add/install/update UX lands in later releases; `rvpm lock` is a
direct way to exercise generation and validation.
