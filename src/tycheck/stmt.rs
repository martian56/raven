//! Statement checking helpers.
//!
//! Statement checking is performed by the body walker in `expr.rs`,
//! which threads its own scope and type map through. This module
//! holds shared helpers and exists so the file tree mirrors the spec.

#[cfg(test)]
mod tests {
    #[test]
    fn module_present() {
        // Intentionally minimal; the body walker covers statements
        // directly. Tests for statement type rules live in
        // `super::tests`.
    }
}
