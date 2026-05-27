//! Match arm exhaustiveness analysis.
//!
//! The check runs as a simple variant set walk: for each known closed
//! type (`Bool`, user enums, `Option`, `Result`), the arms must cover
//! every variant or include a wildcard. For other types a wildcard is
//! required.

use crate::error::RavenError;

/// Sentinel used by `expr.rs`. Filled in by the exhaustiveness commit.
pub fn check_placeholder() -> Result<(), RavenError> {
    Ok(())
}
