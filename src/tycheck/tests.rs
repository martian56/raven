//! Inline unit tests for the type checker.
//!
//! End to end tests are populated as the implementation commits land.

use super::TypeMap;

#[test]
fn type_map_starts_empty() {
    let m = TypeMap::new();
    assert!(m.types.is_empty());
}
