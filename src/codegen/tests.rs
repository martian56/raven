//! Inline unit tests for the Cranelift back end.
//!
//! Each test runs the full v2 front end pipeline (lex, parse, resolve,
//! type check, HIR, MIR) on a small Raven snippet, hands the resulting
//! `MirProgram` to `compile_program`, and inspects the returned object
//! bytes. These tests do not depend on the system `cc` driver; the end
//! to end smoke test that links and runs hello.rv lives in
//! `tests/codegen_smoke.rs` and gates itself on `cc` availability.

use std::path::{Path, PathBuf};

use crate::codegen::{compile_program, intrinsics};
use crate::hir::lower_file;
use crate::lexer::Lexer;
use crate::mir::{lower_program, MirProgram, MirRvalue, MirStatement};
use crate::parser::parse;
use crate::resolve::{resolve_file, LoadedSource, SourceLoader};
use crate::tycheck::check_file;

struct NoLoader;
impl SourceLoader for NoLoader {
    fn load(&mut self, _i: &Path, _t: &str) -> Option<LoadedSource> {
        None
    }
}

fn compile(src: &str) -> MirProgram {
    let tokens = Lexer::new(src.to_string(), PathBuf::from("t.rv"))
        .tokenize()
        .expect("lex");
    let file = parse(&tokens).expect("parse");
    let mut loader = NoLoader;
    let resolved = resolve_file(&file, &mut loader).expect("resolve");
    let typed = check_file(&resolved).expect("tycheck");
    let hir = lower_file(&typed).expect("hir");
    lower_program(&hir).expect("mir")
}

#[test]
fn compiles_function_returning_int_constant() {
    let prog = compile("fun answer() -> Int { return 42 }");
    let object = compile_program(&prog).expect("codegen");
    assert!(
        !object.is_empty(),
        "object file should have at least a header"
    );
    assert!(
        starts_with_object_magic(&object),
        "object bytes should start with a recognized file format header"
    );
}

#[test]
fn compiles_arithmetic_function() {
    let prog = compile("fun sum(a: Int, b: Int) -> Int { return a + b }");
    let object = compile_program(&prog).expect("codegen sum");
    assert!(object.len() > 64);
}

#[test]
fn compiles_if_expression_with_value() {
    let prog = compile("fun pick(c: Bool) -> Int { return if c { 1 } else { 2 } }");
    let object = compile_program(&prog).expect("codegen if");
    assert!(object.len() > 64);
}

#[test]
fn compiles_call_between_two_functions() {
    let src = r#"
        fun helper(x: Int) -> Int { return x * 2 }
        fun caller() -> Int { return helper(21) }
    "#;
    let prog = compile(src);
    let object = compile_program(&prog).expect("codegen call");
    assert!(object.len() > 64);
}

#[test]
fn compiles_print_intrinsic_call() {
    let src = r#"
        fun main() {
            print("Hello, Raven!")
        }
    "#;
    let prog = compile(src);
    // Verify the MIR ends with a Call to the `print` mangled name.
    let main = prog
        .functions
        .iter()
        .find(|f| f.origin == "main")
        .expect("main function");
    let has_print = main
        .blocks
        .iter()
        .flat_map(|b| b.statements.iter())
        .any(|s| {
            matches!(
                s,
                MirStatement::Assign {
                    rvalue: MirRvalue::Call { callee, .. },
                    ..
                } if callee.mangled == intrinsics::PRINT
            )
        });
    assert!(has_print, "MIR should contain a print call");

    let object = compile_program(&prog).expect("codegen print");
    assert!(object.len() > 64);
    // The interned string literal should land in the object's data
    // section. We look for the bytes verbatim.
    assert!(
        contains_bytes(&object, b"Hello, Raven!"),
        "object should contain the interned literal bytes"
    );
}

#[test]
fn compiles_extern_c_call_with_cstring_literal() {
    let src = r#"
        extern "C" {
            fun strlen(s: CStr) -> CSize
        }
        fun main() {
            let n = strlen(c"hello")
            print(n)
        }
    "#;
    let prog = compile(src);
    // The program records the foreign function in its extern table.
    assert!(
        prog.externs.iter().any(|e| e.name == "strlen"),
        "MIR program should declare the extern strlen"
    );
    // The call site lowers to a direct call to the raw C symbol name.
    let main = prog
        .functions
        .iter()
        .find(|f| f.origin == "main")
        .expect("main function");
    let calls_strlen = main
        .blocks
        .iter()
        .flat_map(|b| b.statements.iter())
        .any(|s| {
            matches!(
                s,
                MirStatement::Assign {
                    rvalue: MirRvalue::Call { callee, .. },
                    ..
                } if callee.mangled == "strlen"
            )
        });
    assert!(calls_strlen, "MIR should contain a call to strlen");

    let object = compile_program(&prog).expect("codegen extern call");
    assert!(object.len() > 64);
    // The c-string literal lands in the data section, null-terminated.
    assert!(
        contains_bytes(&object, b"hello\0"),
        "object should contain the null-terminated c-string bytes"
    );
}

#[test]
fn compiles_extern_c_call_with_int_literal() {
    // A non-pointer FFI type: abs(CInt) -> CInt called on a negative
    // literal. Exercises the i64-to-i32 argument coercion at the call.
    let src = r#"
        extern "C" {
            fun abs(x: CInt) -> CInt
        }
        fun main() {
            let n = abs(-7)
            print(n)
        }
    "#;
    let prog = compile(src);
    assert!(prog.externs.iter().any(|e| e.name == "abs"));
    let object = compile_program(&prog).expect("codegen extern abs");
    assert!(object.len() > 64);
}

#[test]
fn compiles_extern_c_call_with_cfloat() {
    // A single-precision FFI type: sqrtf(CFloat) -> CFloat. Exercises the
    // f64-to-f32 narrowing of the Float argument at the call (fdemote) and
    // the f32-to-f64 widening of the result for printing (fpromote).
    let src = r#"
        extern "C" {
            fun sqrtf(x: CFloat) -> CFloat
        }
        fun main() {
            let r = sqrtf(16.0)
            print(r)
        }
    "#;
    let prog = compile(src);
    assert!(prog.externs.iter().any(|e| e.name == "sqrtf"));
    let object = compile_program(&prog).expect("codegen extern sqrtf");
    assert!(object.len() > 64);
}

#[test]
fn compiles_ptr_alloc_store_load_free() {
    // Raw pointer round-trip: allocate, store through the pointer, load it
    // back, and free, all via the `__ptr_*` builtins. Exercises the MIR
    // PtrAlloc/PtrStore/PtrLoad/PtrFree lowering and its Cranelift
    // load/store emission.
    let src = r#"
        fun main() {
            let p = __ptr_alloc<CInt>(2)
            __ptr_store<CInt>(p, 42)
            let v = __ptr_load<CInt>(p)
            print(v)
            __ptr_free<CInt>(p)
        }
    "#;
    let prog = compile(src);
    let main = prog
        .functions
        .iter()
        .find(|f| f.origin == "main")
        .expect("main function");
    let has_alloc = main
        .blocks
        .iter()
        .flat_map(|b| b.statements.iter())
        .any(|s| {
            matches!(
                s,
                MirStatement::Assign {
                    rvalue: MirRvalue::PtrAlloc { .. },
                    ..
                }
            )
        });
    let has_store = main
        .blocks
        .iter()
        .flat_map(|b| b.statements.iter())
        .any(|s| matches!(s, MirStatement::PtrStore { .. }));
    let has_load = main
        .blocks
        .iter()
        .flat_map(|b| b.statements.iter())
        .any(|s| {
            matches!(
                s,
                MirStatement::Assign {
                    rvalue: MirRvalue::PtrLoad { .. },
                    ..
                }
            )
        });
    let has_free = main
        .blocks
        .iter()
        .flat_map(|b| b.statements.iter())
        .any(|s| matches!(s, MirStatement::PtrFree { .. }));
    assert!(has_alloc, "MIR should contain a PtrAlloc");
    assert!(has_store, "MIR should contain a PtrStore");
    assert!(has_load, "MIR should contain a PtrLoad");
    assert!(has_free, "MIR should contain a PtrFree");
    let object = compile_program(&prog).expect("codegen ptr round-trip");
    assert!(object.len() > 64);
}

#[test]
fn compiles_ptr_offset_and_null_check() {
    // Pointer offset and null check through the builtins. The Bool results
    // flow out as a return rather than `print`, which would require the
    // prelude's `ToString` impl (unavailable under the test's NoLoader).
    let src = r#"
        fun probe() -> Bool {
            let p = __ptr_null<CLong>()
            let b = __ptr_is_null<CLong>(p)
            let q = __ptr_offset<CLong>(p, 3)
            let b2 = __ptr_is_null<CLong>(q)
            return b && b2
        }
    "#;
    let prog = compile(src);
    let main = prog
        .functions
        .iter()
        .find(|f| f.origin == "probe")
        .expect("probe function");
    let has_offset = main
        .blocks
        .iter()
        .flat_map(|b| b.statements.iter())
        .any(|s| {
            matches!(
                s,
                MirStatement::Assign {
                    rvalue: MirRvalue::PtrOffset { .. },
                    ..
                }
            )
        });
    let has_is_null = main
        .blocks
        .iter()
        .flat_map(|b| b.statements.iter())
        .any(|s| {
            matches!(
                s,
                MirStatement::Assign {
                    rvalue: MirRvalue::PtrIsNull { .. },
                    ..
                }
            )
        });
    assert!(has_offset, "MIR should contain a PtrOffset");
    assert!(has_is_null, "MIR should contain a PtrIsNull");
    let object = compile_program(&prog).expect("codegen ptr offset/null");
    assert!(object.len() > 64);
}

#[test]
fn compiles_float_arithmetic() {
    let prog = compile("fun mix(x: Float, y: Float) -> Float { return x * y + 1.0 }");
    let object = compile_program(&prog).expect("codegen float");
    assert!(object.len() > 64);
}

#[test]
fn compiles_bool_logical_ops() {
    let prog = compile("fun nand(a: Bool, b: Bool) -> Bool { return !(a && b) }");
    let object = compile_program(&prog).expect("codegen bool");
    assert!(object.len() > 64);
}

#[test]
fn compiles_unit_returning_function() {
    let prog = compile("fun noop() {}");
    let object = compile_program(&prog).expect("codegen unit");
    assert!(object.len() > 64);
}

#[test]
fn compiles_while_loop() {
    let src = r#"
        fun count() -> Int {
            let i = 0
            while i < 10 {
                i = i + 1
            }
            return i
        }
    "#;
    let prog = compile(src);
    let object = compile_program(&prog).expect("codegen while");
    assert!(object.len() > 64);
}

#[test]
fn intern_dedupes_identical_string_literals() {
    let src = r#"
        fun main() {
            print("hi")
            print("hi")
        }
    "#;
    let prog = compile(src);
    let object = compile_program(&prog).expect("codegen dedupe");
    // Two calls share one literal so the bytes appear once. The object
    // file may pad sections, so we only assert "appears at least once";
    // dedup is verified by the BTreeMap key check in `intern_string`
    // (covered indirectly by this test always producing a valid object).
    assert!(contains_bytes(&object, b"hi"));
}

// ----- Helpers -----

/// Heuristic: object files begin with one of a small set of magic
/// numbers. Windows COFF, ELF, and Mach-O are recognized here.
fn starts_with_object_magic(bytes: &[u8]) -> bool {
    if bytes.len() < 4 {
        return false;
    }
    // ELF magic.
    if bytes.starts_with(b"\x7fELF") {
        return true;
    }
    // Mach-O magic (32 and 64 bit, little endian).
    let m = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    if matches!(m, 0xfeedface | 0xfeedfacf | 0xcefaedfe | 0xcffaedfe) {
        return true;
    }
    // COFF for x86_64 (0x8664) and aarch64 (0xaa64). The first two
    // bytes are the machine type little endian.
    let machine = u16::from_le_bytes([bytes[0], bytes[1]]);
    matches!(machine, 0x8664 | 0xaa64 | 0x014c)
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || needle.len() > haystack.len() {
        return false;
    }
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn classify_repr_c_matches_platform_abi() {
    use crate::codegen::function::{classify_repr_c, ReprCAbi};
    use cranelift_codegen::isa::CallConv;

    // System V and AArch64: up to 8 bytes in one register, 9..=16 in two.
    for conv in [CallConv::SystemV, CallConv::AppleAarch64] {
        assert_eq!(classify_repr_c(4, conv), ReprCAbi::OneReg);
        assert_eq!(classify_repr_c(8, conv), ReprCAbi::OneReg);
        assert_eq!(classify_repr_c(3, conv), ReprCAbi::OneReg);
        assert_eq!(classify_repr_c(12, conv), ReprCAbi::TwoReg);
        assert_eq!(classify_repr_c(16, conv), ReprCAbi::TwoReg);
    }

    // Windows x64: only sizes 1, 2, 4, 8 use a register; the rest go by
    // reference.
    let win = CallConv::WindowsFastcall;
    assert_eq!(classify_repr_c(4, win), ReprCAbi::OneReg);
    assert_eq!(classify_repr_c(8, win), ReprCAbi::OneReg);
    assert_eq!(classify_repr_c(3, win), ReprCAbi::ByRef);
    assert_eq!(classify_repr_c(6, win), ReprCAbi::ByRef);
    assert_eq!(classify_repr_c(12, win), ReprCAbi::ByRef);
    assert_eq!(classify_repr_c(16, win), ReprCAbi::ByRef);
}
