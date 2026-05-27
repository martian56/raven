//! Declaration type collection pass.
//!
//! Walks every top level `Decl` and inserts its signature into the
//! [`TypeEnv`]. The pass does not look at function bodies.

use crate::error::RavenError;
use crate::resolve::ResolvedFile;

use super::env::TypeEnv;

/// Walk `resolved` and populate `env` with every top level signature.
pub fn collect_declarations(
    _resolved: &ResolvedFile<'_>,
    _env: &mut TypeEnv,
) -> Result<(), RavenError> {
    // Filled in by the next commit.
    Ok(())
}
