# Release process

Tagged releases are produced by `.github/workflows/release.yml`. Pushing an
annotated tag of the form `vX.Y.Z` (optionally `vX.Y.Z-suffix`) triggers the
workflow. The workflow can also be started manually with `workflow_dispatch`,
which builds and uploads workflow artifacts but does not publish a GitHub
Release.

## Artifacts per platform

The build matrix covers four targets. Each target produces a portable archive
and the platform native installer.

| Target | Archive | Installer |
|--------|---------|-----------|
| Linux x86_64 (`x86_64-unknown-linux-gnu`) | `.tar.gz` | `.deb`, `.rpm` |
| Windows x86_64 (`x86_64-pc-windows-msvc`) | `.zip` | `.msi` |
| macOS x86_64 (`x86_64-apple-darwin`) | `.tar.gz` | `.pkg` |
| macOS arm64 (`aarch64-apple-darwin`) | `.tar.gz` | `.pkg` |

Every artifact bundles two binaries, `raven` (the compiler and build driver)
and `rvpm` (the package manager), plus the `raven_runtime` static library.

## Embedded standard library

The v2 standard library is compiled into the binaries with `include_str!`
(registered in `src/resolve/stdlib.rs`). It is not shipped as separate `.rv`
files, so installing the binaries is sufficient to get the full stdlib.

## Runtime library packaging and location

The compiler links every user program at build time against the
`raven_runtime` static library (`raven_runtime.lib` on Windows,
`libraven_runtime.a` on Unix). A packaged compiler is therefore unusable
unless it can find that library.

`raven` locates the library (see `locate_runtime_staticlib` and
`candidate_runtime_paths` in `src/driver/mod.rs`) by checking, in order:

1. the `RAVEN_RUNTIME_LIB` environment variable, if it points at a file;
2. the directory containing the `raven` binary, its `deps` subdirectory, and
   its parent directory;
3. `target/debug` and `target/release` under the current working directory.

All packaging puts the runtime library next to the `raven` binary so case (2)
finds it with no configuration:

- tarball and zip: `raven`, `rvpm`, and the runtime library sit together in
  one directory.
- `.deb` and `.rpm`: binaries and the runtime library install to `/usr/bin`.
- `.msi`: all three files install to the application `bin` directory, which is
  added to the system PATH.
- `.pkg`: all three files install to `/usr/local/bin`.

The smoke jobs additionally set `RAVEN_RUNTIME_LIB` to the packaged path as a
belt-and-suspenders check.

## C toolchain requirement

Linking a compiled program needs a C linker on the machine that runs `raven`:
MSVC `link.exe` on Windows, `cc` on Unix. The GitHub runners provide one. The
`.deb` and `.rpm` packages declare a dependency on `gcc`. End users on other
systems must have a C toolchain installed.

## Smoke tests

After the build jobs, the `smoke` jobs run on a clean runner per platform.
Each downloads the produced artifact, extracts or relies on the staged
directory, compiles `fun main() { print("hello") }` with the packaged `raven`,
runs the result, and asserts the output is `hello`. This proves the artifact
ships a compiler that can find its runtime library and link a program end to
end.

## Signing

The issue calls for signed installers. Real signing requires certificates and
secrets that the maintainer controls, so the signing steps are optional and run
only when the relevant secrets are present. Unsigned artifacts still build, so
the workflow is usable before signing is configured.

Windows Authenticode (MSI):

- `WINDOWS_CERT_BASE64`: base64 encoded PFX certificate.
- `WINDOWS_CERT_PASSWORD`: PFX password.

macOS package signing:

- `APPLE_INSTALLER_IDENTITY`: a Developer ID Installer identity available in the
  runner keychain, used by `productsign`.

Add these as repository or organization secrets to enable signing. Notarization
and stapling for macOS can be layered on later with `notarytool` and `stapler`
once an Apple account is wired up.

## Verification status

The full multi-platform installer build runs only on a tag (or
`workflow_dispatch`) using the Linux, Windows, and macOS runners, so it is
confirmed by an actual tagged release run. The packaging configuration and the
runtime-library location approach were validated locally: a staged Windows
layout of `raven.exe`, `rvpm.exe`, and `raven_runtime.lib` in one directory
compiles, links, and runs a hello-world program with no environment
configuration.
