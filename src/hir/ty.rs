//! Type representation used inside the HIR.
//!
//! The HIR reuses the type checker's `Ty` directly. We re-export it
//! under the alias `HirTy` to make call sites read clearly and to leave
//! room for a future divergence (for instance, monomorphized type
//! identifiers in MIR) without a churny rename.

pub use crate::tycheck::Ty as HirTy;
