//! Raven v2 compiler library.
//!
//! Module placeholders for the v2 pipeline. Each will be populated in
//! its own phase per `docs/v2/2026-05-22-v2-roadmap.md`:
//!
//! - `lexer`: Phase 1
//! - `parser`: Phase 2
//! - `ast`: Phase 2
//! - `resolve`: Phase 3
//! - `tycheck`: Phases 4 and 5
//! - `hir`: Phase 6
//! - `mir`: Phase 6
//! - `codegen`: Phase 7
//! - `driver`: orchestrator, grown alongside the above
//!
//! Phase 0 shipped a doc-shaped skeleton; Phase 1 adds the lexer, span, and error modules.

pub mod error;
pub mod lexer;
pub mod span;
