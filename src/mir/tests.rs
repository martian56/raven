//! Inline unit tests for the MIR module.
//!
//! Each test runs the full pipeline (lex -> parse -> resolve ->
//! tycheck -> hir -> mir) on a small Raven snippet and asserts
//! structural properties of the resulting program. Wider coverage of
//! the exact textual shape lives in `tests/mir_golden.rs`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::hir::lower_file;
use crate::lexer::Lexer;
use crate::mir::builder::FunctionBuilder;
use crate::mir::ir::{
    MirBlock, MirConstant, MirFunction, MirOperand, MirRvalue, MirStatement, MirTerminator,
};
use crate::mir::ty::MirType;
use crate::mir::{lower_program, MirProgram};
use crate::parser::parse;
use crate::resolve::{resolve_file, LoadedSource, SourceLoader};
use crate::span::Span;
use crate::tycheck::check_file;

struct NoLoader;
impl SourceLoader for NoLoader {
    fn load(&mut self, _i: &Path, _t: &str) -> Option<LoadedSource> {
        None
    }
}

fn dummy_span() -> Span {
    Span::new(Arc::new(PathBuf::from("t.rv")), 0, 0, 1, 1)
}

/// Run the full v2 pipeline on `src` and return the MIR program.
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

/// Like `compile`, but merges the bundled prelude first so `print` of a
/// scalar resolves through its `ToString` impl. Tests that print need
/// this, since `print` requires the prelude's `impl ToString for Int`.
fn compile_with_prelude(src: &str) -> MirProgram {
    let tokens = Lexer::new(src.to_string(), PathBuf::from("t.rv"))
        .tokenize()
        .expect("lex");
    let file = parse(&tokens).expect("parse");
    let file = crate::resolve::expand_with_stdlib(&file).expect("stdlib expand");
    let mut loader = NoLoader;
    let resolved = resolve_file(&file, &mut loader).expect("resolve");
    let typed = check_file(&resolved).expect("tycheck");
    let hir = lower_file(&typed).expect("hir");
    lower_program(&hir).expect("mir")
}

fn find_fn<'a>(p: &'a MirProgram, name: &str) -> &'a MirFunction {
    p.functions
        .iter()
        .find(|f| f.origin == name)
        .unwrap_or_else(|| panic!("function {} not in MIR", name))
}

// ----- Builder smoke tests -----

#[test]
fn empty_program_pretty_prints() {
    let prog = MirProgram::new();
    let rendered = crate::mir::pretty_program(&prog);
    assert!(rendered.contains("(mir"));
}

#[test]
fn builder_emits_single_block_function() {
    let mut b = FunctionBuilder::new("noop".into(), "noop".into(), MirType::Unit, dummy_span());
    let entry = b.new_block();
    b.close_block(
        entry,
        MirTerminator::Return(MirOperand::Const(MirConstant::Unit)),
    );
    let fun = b.finish(entry);
    assert_eq!(fun.blocks.len(), 1);
    assert_eq!(fun.name, "noop");
}

#[test]
fn builder_allocates_distinct_locals() {
    let mut b = FunctionBuilder::new("two".into(), "two".into(), MirType::Int, dummy_span());
    let p = b.add_param("x".into(), MirType::Int);
    let t = b.fresh_temp("tmp", MirType::Int);
    assert_ne!(p.0, t.0);
    assert_eq!(b.locals().len(), 2);
    let entry = b.new_block();
    b.assign(entry, t, MirRvalue::Use(MirOperand::Copy(p)));
    b.close_block(entry, MirTerminator::Return(MirOperand::Copy(t)));
    let fun = b.finish(entry);
    assert_eq!(fun.blocks[0].statements.len(), 1);
}

#[test]
#[should_panic]
fn builder_double_close_panics() {
    let mut b = FunctionBuilder::new("bad".into(), "bad".into(), MirType::Unit, dummy_span());
    let entry = b.new_block();
    b.close_block(
        entry,
        MirTerminator::Return(MirOperand::Const(MirConstant::Unit)),
    );
    b.close_block(
        entry,
        MirTerminator::Return(MirOperand::Const(MirConstant::Unit)),
    );
}

// ----- End to end lowering tests -----

#[test]
fn arithmetic_lowers_to_binop() {
    let prog = compile("fun add(a: Int, b: Int) -> Int { return a + b }");
    let f = find_fn(&prog, "add");
    assert_eq!(f.params.len(), 2);
    let saw_binop = f.blocks.iter().flat_map(|b| b.statements.iter()).any(|s| {
        matches!(
            s,
            MirStatement::Assign {
                rvalue: MirRvalue::BinaryOp(..),
                ..
            }
        )
    });
    assert!(saw_binop, "expected a binop assignment");
}

#[test]
fn if_branches_become_switch_int() {
    let prog = compile("fun pick(c: Bool) -> Int { return if c { 1 } else { 2 } }");
    let f = find_fn(&prog, "pick");
    let switches = f
        .blocks
        .iter()
        .filter(|b| matches!(b.terminator, MirTerminator::SwitchInt { .. }))
        .count();
    assert!(switches >= 1, "expected at least one switch-int");
}

#[test]
fn while_loop_has_back_edge() {
    let prog = compile("fun spin() -> () { let i = 0; while i < 10 { } }");
    let f = find_fn(&prog, "spin");
    // The while header should be reached by a Goto from the body.
    let goto_count = f
        .blocks
        .iter()
        .filter(|b| matches!(b.terminator, MirTerminator::Goto(_)))
        .count();
    assert!(goto_count >= 2, "expected at least two goto terminators");
}

#[test]
fn return_terminates_block() {
    let prog = compile("fun zero() -> Int { return 0 }");
    let f = find_fn(&prog, "zero");
    let returns = f
        .blocks
        .iter()
        .filter(|b| matches!(b.terminator, MirTerminator::Return(_)))
        .count();
    assert!(returns >= 1, "expected a return terminator");
}

#[test]
fn struct_create_emitted_for_struct_literal() {
    let src = r#"
        struct Point { x: Int, y: Int }
        fun mk() -> Point { return Point { x: 1, y: 2 } }
    "#;
    let prog = compile(src);
    let f = find_fn(&prog, "mk");
    let saw_struct_create = f.blocks.iter().flat_map(|b| b.statements.iter()).any(|s| {
        matches!(
            s,
            MirStatement::Assign {
                rvalue: MirRvalue::StructCreate { .. },
                ..
            }
        )
    });
    assert!(saw_struct_create, "expected a struct-create");
}

#[test]
fn option_some_lowers_to_enum_create() {
    // User-written `Some(x)` is recognized as the built in Option
    // constructor and lowers to an `EnumCreate`, so codegen can build the
    // heap value directly rather than calling an undefined `Some` symbol.
    let src = r#"
        fun maybe() -> Option<Int> { return Some(42) }
    "#;
    let prog = compile(src);
    let f = find_fn(&prog, "maybe");
    let saw_enum = f.blocks.iter().flat_map(|b| b.statements.iter()).any(|s| {
        matches!(
            s,
            MirStatement::Assign {
                rvalue: MirRvalue::EnumCreate { variant: 0, .. },
                ..
            }
        )
    });
    assert!(saw_enum, "expected EnumCreate for Some(42)");
}

#[test]
fn try_operator_emits_enum_create_via_some_ctor() {
    let src = r#"
        fun take(o: Option<Int>) -> Option<Int> {
            let v = o?;
            return Some(v)
        }
    "#;
    let prog = compile(src);
    let f = find_fn(&prog, "take");
    let saw_enum = f.blocks.iter().flat_map(|b| b.statements.iter()).any(|s| {
        matches!(
            s,
            MirStatement::Assign {
                rvalue: MirRvalue::EnumCreate { .. },
                ..
            }
        )
    });
    assert!(saw_enum, "expected EnumCreate from `?` desugaring");
}

#[test]
fn non_generic_function_kept_as_root() {
    let prog = compile("fun a() -> Int { return 1 } fun b() -> Int { return 2 }");
    assert!(prog.functions.iter().any(|f| f.origin == "a"));
    assert!(prog.functions.iter().any(|f| f.origin == "b"));
}

#[test]
fn monomorphize_dedupes_repeated_instantiations() {
    // No HIR-level generic call sites are emitted yet, but the
    // worklist seen-set must still treat a repeated insertion of the
    // same root as a no-op. We exercise that by compiling a file with
    // two identical functions and counting outputs.
    let prog = compile("fun a() -> Int { return 1 } fun a2() -> Int { return 1 }");
    let names: Vec<&str> = prog.functions.iter().map(|f| f.origin.as_str()).collect();
    assert!(names.contains(&"a"));
    assert!(names.contains(&"a2"));
    // No accidental duplicates.
    let mut sorted = names.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(names.len(), sorted.len(), "duplicate functions emitted");
}

#[test]
fn mir_type_mangling_is_stable() {
    use crate::mir::ty::MirType;
    assert_eq!(MirType::Int.mangle(), "Int");
    assert_eq!(
        MirType::Option(Box::new(MirType::Int)).mangle(),
        "Option_Int"
    );
    assert_eq!(
        MirType::Result(Box::new(MirType::Int), Box::new(MirType::Str)).mangle(),
        "Result_Int_Str"
    );
}

// ----- Defer lowering -----

/// Collect the integer constant rendered by each `print(N)` call in the
/// block, in statement order. A `print(N)` lowers to an int-to-string
/// conversion (the constant `N` is its argument) followed by the `print`
/// call, so matching the conversion recovers the printed value. Used to
/// assert deferred call ordering.
fn printed_int_args_in_block(block: &MirBlock) -> Vec<i64> {
    let mut out = Vec::new();
    for s in &block.statements {
        if let MirStatement::Assign {
            rvalue: MirRvalue::Call { callee, args },
            ..
        } = s
        {
            // `print(n)` lowers to `Int$to_string(n)` then a string print,
            // so the deferred int is the constant argument to that
            // to_string call. Reading them in block order recovers the
            // order the deferred prints run.
            if callee.mangled == "Int$to_string" {
                if let Some(MirOperand::Const(MirConstant::Int(n))) = args.first() {
                    out.push(*n);
                }
            }
        }
    }
    out
}

#[test]
fn defer_runs_in_reverse_order_before_return() {
    // Two defers at the function body level: print(1) then
    // print(2). The block that ends in `return` must call them in
    // reverse (LIFO) order: 2 then 1.
    let prog = compile_with_prelude(
        r#"
        fun demo() -> Int {
            defer print(1)
            defer print(2)
            return 0
        }
    "#,
    );
    let demo = find_fn(&prog, "demo");
    let ret_block = demo
        .blocks
        .iter()
        .find(|b| matches!(b.terminator, MirTerminator::Return(_)))
        .expect("demo has a return block");
    assert_eq!(
        printed_int_args_in_block(ret_block),
        vec![2, 1],
        "deferred calls must run LIFO before the return"
    );
}

#[test]
fn only_reached_defers_run_on_each_return_path() {
    // The first defer precedes the early return, so it is scheduled on
    // both paths. The second defer follows the early return, so the early
    // path never schedules it. The early-path return block runs only [9];
    // the fall-through return block runs [8, 9] (LIFO).
    let prog = compile_with_prelude(
        r#"
        fun f(early: Bool) -> Int {
            defer print(9)
            if early {
                return 1
            }
            defer print(8)
            return 2
        }
    "#,
    );
    let f = find_fn(&prog, "f");

    let return_blocks: Vec<Vec<i64>> = f
        .blocks
        .iter()
        .filter(|b| matches!(b.terminator, MirTerminator::Return(_)))
        .map(printed_int_args_in_block)
        .collect();

    assert!(
        return_blocks.contains(&vec![9]),
        "the early-return path runs only the first defer, got {:?}",
        return_blocks
    );
    assert!(
        return_blocks.contains(&vec![8, 9]),
        "the fall-through path runs both defers LIFO, got {:?}",
        return_blocks
    );
}

// ----- closure capture and invocation -----

/// Find the single `ClosureCreate` rvalue in a function's blocks.
fn closure_create_in(f: &MirFunction) -> &MirRvalue {
    f.blocks
        .iter()
        .flat_map(|b| b.statements.iter())
        .find_map(|s| match s {
            MirStatement::Assign {
                rvalue: rv @ MirRvalue::ClosureCreate { .. },
                ..
            } => Some(rv),
            _ => None,
        })
        .expect("expected a ClosureCreate")
}

#[test]
fn capturing_lambda_records_captured_local() {
    // The lambda references the enclosing local `n`, so capture analysis
    // records exactly one capture and emits a ClosureCreate carrying it.
    let prog = compile(
        r#"
        fun make_adder(n: Int) -> fun(Int) -> Int {
            return fun(x: Int) -> Int = x + n
        }
    "#,
    );
    let f = find_fn(&prog, "make_adder");
    match closure_create_in(f) {
        MirRvalue::ClosureCreate {
            captures,
            capture_tys,
            ..
        } => {
            assert_eq!(captures.len(), 1, "exactly the captured `n`");
            assert_eq!(capture_tys, &vec![MirType::Int]);
        }
        _ => unreachable!(),
    }
}

#[test]
fn non_capturing_lambda_has_no_captures() {
    // The lambda references only its own parameter, so it captures
    // nothing and the ClosureCreate carries an empty capture list.
    let prog = compile(
        r#"
        fun make() -> fun(Int) -> Int {
            return fun(x: Int) -> Int = x + 1
        }
    "#,
    );
    let f = find_fn(&prog, "make");
    match closure_create_in(f) {
        MirRvalue::ClosureCreate { captures, .. } => {
            assert!(captures.is_empty(), "no enclosing locals referenced");
        }
        _ => unreachable!(),
    }
}

#[test]
fn lambda_body_is_lifted_to_standalone_function() {
    // The lambda body becomes its own MIR function whose leading
    // parameter is the capture environment.
    let prog = compile(
        r#"
        fun make_adder(n: Int) -> fun(Int) -> Int {
            return fun(x: Int) -> Int = x + n
        }
    "#,
    );
    let lifted = prog
        .functions
        .iter()
        .find(|f| f.name.contains("$closure$"))
        .expect("a lifted closure body function");
    // env pointer + the lambda's own parameter.
    assert_eq!(lifted.params.len(), 2, "env param plus lambda param");
    let env_decl = lifted.local_decl(lifted.params[0]);
    assert_eq!(env_decl.name, "__env");
    // The body reads the capture from the env.
    let saw_env_load = lifted
        .blocks
        .iter()
        .flat_map(|b| b.statements.iter())
        .any(|s| {
            matches!(
                s,
                MirStatement::Assign {
                    rvalue: MirRvalue::EnvLoad { .. },
                    ..
                }
            )
        });
    assert!(saw_env_load, "lifted body reads its captures from the env");
}

#[test]
fn invoking_a_closure_value_emits_closure_call() {
    // Calling a local of function type dispatches indirectly through the
    // Closure object via a ClosureCall rvalue, not a direct Call.
    let prog = compile(
        r#"
        fun apply(f: fun(Int) -> Int, x: Int) -> Int {
            return f(x)
        }
    "#,
    );
    let f = find_fn(&prog, "apply");
    let saw_closure_call = f.blocks.iter().flat_map(|b| b.statements.iter()).any(|s| {
        matches!(
            s,
            MirStatement::Assign {
                rvalue: MirRvalue::ClosureCall { .. },
                ..
            }
        )
    });
    assert!(
        saw_closure_call,
        "calling a closure value lowers to ClosureCall"
    );
}

#[test]
fn generic_call_inside_closure_body_is_monomorphized() {
    // Regression for #135: a generic function reachable only through a
    // closure body must still be instantiated. `identity<T>` is called
    // only from the lifted lambda `f`, so its `identity$Int` instance is
    // queued only if the lifted body's pending generic call sites reach
    // the monomorphization worklist. Before the fix those calls were
    // dropped and no instance was emitted, leaving an unresolved callee.
    let prog = compile_with_prelude(
        r#"
        fun identity<T>(x: T) -> T = x

        fun apply(f: fun(Int) -> Int, x: Int) -> Int {
            return f(x)
        }

        fun main() {
            let f = fun(x: Int) -> Int = identity(x) + 1
            print(apply(f, 41))
        }
    "#,
    );
    // The concrete instantiation appears under its mangled symbol.
    assert!(
        prog.functions.iter().any(|f| f.name == "identity$Int"),
        "expected the identity$Int instantiation in the monomorphized program, got {:?}",
        prog.functions
            .iter()
            .map(|f| f.name.as_str())
            .collect::<Vec<_>>()
    );
    // The lifted closure body that drives the instantiation is present
    // and calls the mangled symbol directly.
    let lifted = prog
        .functions
        .iter()
        .find(|f| f.name.contains("$closure$"))
        .expect("a lifted closure body function");
    let calls_identity_instance = lifted
        .blocks
        .iter()
        .flat_map(|b| b.statements.iter())
        .any(|s| {
            matches!(
                s,
                MirStatement::Assign {
                    rvalue: MirRvalue::Call { callee, .. },
                    ..
                } if callee.mangled == "identity$Int"
            )
        });
    assert!(
        calls_identity_instance,
        "the lifted closure body calls the identity$Int instance"
    );
}

#[test]
fn gc_pointer_captures_are_ordered_first() {
    // A closure capturing both a scalar (`k`) and a GC pointer (`s`)
    // places the GC pointer capture first so the runtime's leading
    // `capture_ptr_count` traced-slot contract holds. The lambda is
    // declared with `k` first in source, yet capture ordering puts the
    // String capture ahead of the Int.
    let prog = compile(
        r#"
        fun build(k: Int, s: String) -> fun() -> String {
            return fun() -> String = "${k}${s}"
        }
    "#,
    );
    let f = find_fn(&prog, "build");
    match closure_create_in(f) {
        MirRvalue::ClosureCreate { capture_tys, .. } => {
            assert_eq!(capture_tys.len(), 2);
            assert_eq!(
                capture_tys[0],
                MirType::Str,
                "the GC pointer capture comes first"
            );
            assert_eq!(capture_tys[1], MirType::Int);
        }
        _ => unreachable!(),
    }
}
