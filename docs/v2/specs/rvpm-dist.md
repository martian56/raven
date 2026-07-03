# rvpm dist

`rvpm dist` builds the application in the current directory and packages it
into distributable artifacts. A Raven binary is a single static file, so
packaging is one staged file tree plus each format's own metadata, which
rvpm generates from the manifest.

## Usage

```
rvpm dist [--target <t1,t2>] [--out-dir <dir>]
```

Targets: `tar`, `zip`, `deb`, `rpm`, `msi`, `inno`. `--target` overrides the
manifest's `[dist].targets`; without either, the host default applies
(`zip` on Windows, `tar` elsewhere). Libraries have no binary and are
rejected.

Artifacts land in `[dist].out_dir` (default `target/dist`), with a `work/`
scratch directory beside them holding the staging trees and generated
packaging files, which is useful when debugging a format.

## The [dist] manifest section

Every field is optional; an absent section behaves like an empty one.

```toml
[dist]
targets = ["deb", "zip"]           # what plain `rvpm dist` produces
out_dir = "target/dist"
display_name = "Rook"              # installer titles; default: package name
description = "A coding agent"     # default: "<name> <version>"
license = "MIT"                    # rpm License, installers
homepage = "https://example.com"   # deb Homepage, rpm URL, installers
maintainer = "Ada <ada@x.com>"     # deb Maintainer; default: first author
vendor = "Acme"                    # rpm Vendor, msi Manufacturer; default: maintainer

[[dist.assets]]                    # extra files installed with the binary
source = "README.md"               # read relative to the package root
dest = "share/doc/rook/README.md"  # forward-slash path under the install prefix

[dist.linux]
depends = ["libc6 (>= 2.31)"]      # deb Depends and rpm Requires, verbatim
section = "utils"                  # deb
priority = "optional"              # deb

[dist.windows]
icon = "assets/rook.ico"           # Inno Setup icon
upgrade_code = "9f0c86a1-2b3c-4d5e-8f90-112233445566"  # msi upgrade GUID
```

Asset `source` and `dest` must be relative forward-slash paths with no `..`
components; the same containment rule `[ffi].sources` follows. The install
prefix per format: `/usr/` for deb and rpm (the binary goes to
`/usr/bin/<name>`), the archive root for tar and zip, and the application
folder for msi and inno.

## Formats and their tools

rvpm generates each format's packaging text and shells out to the format's
own tool, the same approach dependency fetching takes with curl and tar. A
missing tool fails with an install hint.

| Target | Tool | Artifact |
|---|---|---|
| `tar` | tar | `<name>-<version>-<arch>.tar.gz`, top-level `<name>-<version>-<arch>/` directory |
| `zip` | bsdtar or zip | `<name>-<version>-<arch>.zip`, flat |
| `deb` | dpkg-deb | `<name>_<version>_<deb-arch>.deb` |
| `rpm` | rpmbuild | rpmbuild's own `<name>-<version>-1.<arch>.rpm` naming |
| `msi` | WiX 3 (candle, light) | `<name>-<version>-<arch>.msi` |
| `inno` | Inno Setup ISCC | `<name>-<version>-setup.exe` |

Version strings are normalized per format: a leading `v` is dropped
everywhere, rpm turns a `-` pre-release separator into `~`, and the msi
ProductVersion keeps only the numeric `X.Y.Z` prefix.

The msi upgrade GUID is what lets a newer installer replace an older
install. When `[dist.windows].upgrade_code` is not set, rvpm derives a
stable GUID from the package name and prints it with a note; pin it in the
manifest so it never changes by accident.

## What is out of scope

Packages are not signed (deb signing, rpm signing, Authenticode are all
post-processing on the produced artifacts). Cross-format scripting hooks
(postinst and friends) and desktop shortcuts are not modeled yet. The
installers do not modify PATH.
