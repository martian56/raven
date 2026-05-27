//! Pattern checking and binding extraction.
//!
//! Patterns appear in `match` arms, `for` heads, and `let` bindings.
//! This module validates that a pattern is compatible with the
//! scrutinee's type and produces the list of names the pattern binds
//! along with their types.

use crate::error::RavenError;

/// Sentinel used by `expr.rs`. Filled in by the pattern checking commit.
pub fn check_placeholder() -> Result<(), RavenError> {
    Ok(())
}
