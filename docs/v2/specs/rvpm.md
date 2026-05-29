# rvpm: package fetching and the shared cache

This spec covers how rvpm fetches a GitHub dependency and where it stores
the result. It describes the on-disk cache layout, the environment
override, the git clone strategy, cache-hit semantics, `.git` handling,
and the error cases. The library implementation lives in `raven::pkg`
(`src/pkg/mod.rs`); the `rvpm` binary exposes it through `rvpm fetch`.

Version-constraint resolution (semver ranges) and the lock file
(`rv.lock`) are out of scope here and are covered by issue #83. This
layer takes an explicit `version` that is a git tag or branch and fetches
exactly it.

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
path; dependency resolution that reads `[dependencies]` from `rv.toml`
lands with the resolver work in #83.
