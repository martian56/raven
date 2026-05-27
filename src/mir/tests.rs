//! Inline unit tests for the MIR module.
//!
//! These tests will be filled out by a later commit; this file exists
//! so the `#[cfg(test)] mod tests;` declaration in `mod.rs` compiles.

#[test]
fn module_compiles() {
    // Smoke test: the MIR data types and pretty printer should at
    // least construct an empty program.
    let prog = crate::mir::MirProgram::new();
    let rendered = crate::mir::pretty_program(&prog);
    assert!(rendered.contains("(mir"));
}
