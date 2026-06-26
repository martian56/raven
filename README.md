<p align="center">
  <img src="./images/raven.png" alt="Raven Logo" width="260" />
</p>

<p align="center">
  A modern programming language built with Rust.<br/>
  Fast, safe, expressive, and easy to read.
</p>

<p align="center">
  <a href="https://github.com/martian56/raven/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/martian56/raven/ci.yml?branch=main&label=CI&style=for-the-badge" alt="CI Status"></a>
  <a href="https://github.com/martian56/raven/releases"><img src="https://img.shields.io/github/v/release/martian56/raven?style=for-the-badge" alt="Latest Release"></a>
  <a href="https://github.com/martian56/raven/blob/main/LICENSE"><img src="https://img.shields.io/github/license/martian56/raven?style=for-the-badge" alt="License"></a>
  <a href="https://marketplace.visualstudio.com/items?itemName=martian56.raven-language"><img src="https://img.shields.io/visual-studio-marketplace/v/martian56.raven-language?style=for-the-badge&label=VS%20Code" alt="VS Code Extension"></a>
</p>

<p align="center">
  <a href="https://martian56.github.io/raven/">Documentation</a>
  ·
  <a href="https://raven.ufazien.com/">Website</a>
  ·
  <a href="https://github.com/martian56/raven/releases">Releases</a>
  ·
  <a href="https://github.com/martian56/raven/issues">Issues</a>
</p>

## Why Raven

- Compiled to native machine code through Cranelift, into a single static binary.
- Static typing with generics, traits, and sum types checked by an exhaustive `match`.
- A tracing garbage collector and `Result`/`Option` instead of `null`.
- Goroutines and channels that run in parallel across CPU cores (an M:N scheduler over a multi-threaded collector), with mutexes, wait groups, and `select`, and a C FFI for native libraries.
- A package manager (`rvpm`), one canonical formatter, and a VS Code extension.

## Quick Example

```rust
struct User {
    name: String,
    age: Int,
}

fun greet(user: User) -> String {
    return "Hello ${user.name}, you are ${user.age}"
}

fun main() {
    let u = User { name: "Raven", age: 2 }
    print(greet(u))
}
```

## Install

Download the installer or archive for your platform from the [releases page](https://github.com/martian56/raven/releases):

- Linux: `.deb`, `.rpm`, or `.tar.gz`
- Windows: `.msi` or `.zip`

This installs the `raven` compiler and the `rvpm` package manager and adds them to your `PATH`. Compiling a program also needs a C linker on your machine (the MSVC build tools on Windows, `cc`/`clang` on Linux).

## Quick Start

```bash
# Compile a source file to a native binary
raven build hello.rv -o hello
./hello
```

Project workflow with `rvpm`:

```bash
rvpm new my_app
cd my_app
rvpm run          # builds and runs src/main.rv
rvpm fmt          # format the .rv sources
```

### Build from source

For contributors, or to track the latest commit:

```bash
git clone https://github.com/martian56/raven.git
cd raven
cargo build --release
```

The `raven` and `rvpm` binaries land in `target/release/`.

## Learn More

- Full docs: [https://martian56.github.io/raven/](https://martian56.github.io/raven/)
- Project website: [https://raven.ufazien.com/](https://raven.ufazien.com/)
- Getting started: [https://martian56.github.io/raven/v2/guide/getting-started/](https://martian56.github.io/raven/v2/guide/getting-started/)
- Language reference: [https://martian56.github.io/raven/v2/guide/language-reference/](https://martian56.github.io/raven/v2/guide/language-reference/)
- Standard library: [https://martian56.github.io/raven/v2/guide/standard-library/](https://martian56.github.io/raven/v2/guide/standard-library/)

## Technologies Used

<p>
  <a href="https://www.rust-lang.org/"><img src="https://cdn.simpleicons.org/rust" alt="Rust" width="34" height="34" /></a>
  <a href="https://www.typescriptlang.org/"><img src="https://cdn.simpleicons.org/typescript" alt="TypeScript" width="34" height="34" /></a>
  <a href="https://github.com/features/actions"><img src="https://cdn.simpleicons.org/githubactions" alt="GitHub Actions" width="34" height="34" /></a>
  <a href="https://www.docker.com/"><img src="https://cdn.simpleicons.org/docker" alt="Docker" width="34" height="34" /></a>
</p>

## Star History

<a href="https://www.star-history.com/?repos=martian56%2Fraven&type=date&legend=top-left">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/image?repos=martian56/raven&type=date&theme=dark&legend=top-left" />
   <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/image?repos=martian56/raven&type=date&legend=top-left" />
   <img alt="Star History Chart" src="https://api.star-history.com/image?repos=martian56/raven&type=date&legend=top-left" />
 </picture>
</a>

## Repo Activity
![Alt](https://repobeats.axiom.co/api/embed/e187d96a01084aee8baae40ab0638927b2032dec.svg "Repobeats analytics image")

## Contributors

<a href="https://github.com/martian56/raven/graphs/contributors">
  <img src="https://contrib.rocks/image?repo=martian56/raven" alt="Contributors" />
</a>

## Community

- Contributing guide: [CONTRIBUTING.md](./CONTRIBUTING.md)
- Code of conduct: [CODE_OF_CONDUCT.md](./CODE_OF_CONDUCT.md)
- Security policy: [SECURITY.md](./SECURITY.md)

## License

MIT License. See [LICENSE](./LICENSE).
<!-- GitAds-Verify: Q5CON76MHBAPN78Y87F3Q18WQ1W4L13Z -->
## GitAds Sponsored
[![Sponsored by GitAds](https://gitads.dev/v1/ad-serve?source=martian56/raven@github)](https://gitads.dev/v1/ad-track?source=martian56/raven@github)
