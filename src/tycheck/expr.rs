//! Body check pass: synthesize a `Ty` for every expression.

use crate::error::RavenError;
use crate::resolve::ResolvedFile;

use super::env::TypeEnv;
use super::TypeMap;

/// Walk every function body and module level expression in `resolved`,
/// recording each expression's inferred type in `types`.
pub fn check_bodies(
    _resolved: &ResolvedFile<'_>,
    _env: &TypeEnv,
    _types: &mut TypeMap,
) -> Result<(), RavenError> {
    // Filled in by the next commit.
    Ok(())
}
