//! Raven v2 compiler library.
//!
//! The pipeline (each stage gets its own module as it lands):
//!
//! - `lexer`: tokenization with spans
//! - `parser`: recursive descent over tokens to produce an AST
//! - `ast`: AST node definitions
//! - `resolve`: name resolution and import wiring
//! - `tycheck`: type checking with generics and trait resolution
//! - `hir`: high-level intermediate representation (desugared AST)
//! - `mir`: mid-level intermediate representation (basic blocks)
//! - `codegen`: Cranelift IR generation and linking
//! - `driver`: pipeline orchestrator
//!
//! Lexer, span, and error infrastructure ship today; the rest land in
//! subsequent PRs.

pub mod ast;
pub mod error;
pub mod lexer;
pub mod parser;
pub mod resolve;
pub mod span;
pub mod tycheck;
