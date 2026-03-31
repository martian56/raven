# Contributing to Raven

Thanks for your interest in contributing to Raven.

## Ways to contribute

- Report bugs and regressions
- Propose language or tooling improvements
- Improve documentation and examples
- Submit code fixes and new features

## Before you start

- Search existing issues and pull requests to avoid duplicates.
- For larger changes, open an issue first so we can align on scope and design.

## Development setup

1. Fork and clone the repository.
2. Install Rust (stable toolchain).
3. Build:
   - `cargo build`
4. Run tests/checks:
   - `cargo test`
   - `cargo check`

## Coding guidelines

- Keep changes focused and minimal.
- Follow existing style and naming conventions.
- Update docs/examples when language behavior changes.
- Add tests for bug fixes and new behavior when possible.

## Commit and pull request guidelines

- Use clear commit messages that explain intent.
- Reference related issues (for example: `Fixes #123`).
- Ensure CI passes before requesting review.
- Include a short summary and test plan in the PR description.

## Standard library changes

If you add or change files under `lib/`:

- Ensure all release packaging includes the new/changed stdlib file(s)
  (Linux package metadata, Windows installer, release artifacts).
- Add or update docs and examples that reference the module.

## Security issues

Please do not open public issues for security vulnerabilities.
See `SECURITY.md` for private reporting instructions.
