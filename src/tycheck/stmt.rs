//! Statement checking helpers.
//!
//! Statement walking is mostly threading: each statement is reduced to
//! an expression check, with the additional side effect of binding
//! names introduced by `let` and pattern destructuring. The bulk of
//! the work lives in `expr.rs`; this module provides the entry points
//! the expression walker calls on encountering a block.

use crate::error::RavenError;

/// Sentinel used by `expr.rs`. Filled in by the body checking commit.
pub fn check_placeholder() -> Result<(), RavenError> {
    Ok(())
}
