# Raven v2 docs

Specs and design notes for Raven v2, the compiled, GC'd,
OOP-via-traits language shipped from `main`. User-facing documentation starts
with [Getting started](./guide/getting-started.md); files under `specs/` include
current implementation references and clearly labeled historical design
records.

| Document | Purpose |
|---|---|
| [Getting started](./guide/getting-started.md) | The entry point to the language guide, tutorials, standard library, and rvpm documentation. |
| `specs/` | Compiler, runtime, standard-library, and tooling implementation specifications. |
| [`2026-05-22-v2-roadmap.md`](./2026-05-22-v2-roadmap.md) | Historical roadmap for the v2 rewrite. Its phase and branch notes describe the development plan, not current status. |

New implementation specifications live in `specs/`; completed user-facing
features must also be reflected in `guide/`.

## v1 documentation

The v1 (interpreted) language docs have been removed. v1 is no longer developed; its source lives only on the `v1.x-maintenance` branch.
