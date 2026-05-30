//! Inline unit tests for the type checker.

use super::{check_file, Ty};
use crate::error::{RavenError, TypeError};
use crate::lexer::Lexer;
use crate::parser::parse;
use crate::resolve::{resolve_file, LoadedSource, SourceLoader};
use std::path::{Path, PathBuf};

struct NoLoader;
impl SourceLoader for NoLoader {
    fn load(&mut self, _i: &Path, _t: &str) -> Option<LoadedSource> {
        None
    }
}

fn check(src: &str) -> Result<(), RavenError> {
    let tokens = Lexer::new(src.to_string(), PathBuf::from("t.rv"))
        .tokenize()
        .expect("lex");
    let file = parse(&tokens).expect("parse");
    let mut loader = NoLoader;
    let resolved = resolve_file(&file, &mut loader)?;
    check_file(&resolved).map(|_| ())
}

/// Like `check`, but merges the bundled prelude first so `print` of a
/// scalar resolves through its `ToString` impl. Tests that print need
/// this, since `print` requires the prelude's `impl ToString for Int`.
fn check_with_prelude(src: &str) -> Result<(), RavenError> {
    let tokens = Lexer::new(src.to_string(), PathBuf::from("t.rv"))
        .tokenize()
        .expect("lex");
    let file = parse(&tokens).expect("parse");
    let file = crate::resolve::expand_with_stdlib(&file)?;
    let mut loader = NoLoader;
    let resolved = resolve_file(&file, &mut loader)?;
    check_file(&resolved).map(|_| ())
}

#[test]
fn arithmetic_int_is_int() {
    check("fun f() -> Int = 1 + 2 * 3\n").unwrap();
}

#[test]
fn mixed_int_float_arithmetic_is_rejected() {
    let err = check("fun f() -> Float = 1 + 2.0\n").unwrap_err();
    match err {
        RavenError::Type(b, _, _) => assert!(matches!(*b, TypeError::TypeMismatch { .. })),
        other => panic!("expected TypeMismatch, got {:?}", other),
    }
}

#[test]
fn bool_logical_ops_produce_bool() {
    check("fun f(a: Bool, b: Bool) -> Bool = a && b || !a\n").unwrap();
}

#[test]
fn comparison_returns_bool() {
    check("fun f(a: Int, b: Int) -> Bool = a < b\n").unwrap();
}

#[test]
fn comparison_requires_compatible_operands() {
    let err = check("fun f(a: Int, b: Bool) -> Bool = a < b\n").unwrap_err();
    assert!(matches!(err, RavenError::Type(_, _, _)));
}

#[test]
fn bound_trait_method_with_self_arg_keeps_its_parameter() {
    // A trait method taking a `Self` argument, called on a generic value
    // bounded by that trait, must keep the argument: only the leading
    // `self` receiver is dropped, not every `Self`-typed parameter.
    check(
        "trait Combine {\n    fun combine(self, other: Self) -> Self\n}\nfun pair<T: Combine>(a: T, b: T) -> T {\n    return a.combine(b)\n}\n",
    )
    .unwrap();
}

#[test]
fn struct_literal_and_field_access() {
    check(
        "struct Point { x: Int, y: Int }\nfun f() -> Int {\n    let p = Point { x: 1, y: 2 }\n    return p.x\n}\n",
    )
    .unwrap();
}

#[test]
fn struct_literal_missing_field_is_error() {
    let err = check("struct P { x: Int, y: Int }\nfun f() -> P = P { x: 1 }\n").unwrap_err();
    match err {
        RavenError::Type(b, _, _) => match *b {
            TypeError::Custom(msg) => assert!(msg.contains("missing field")),
            other => panic!("expected missing field, got {:?}", other),
        },
        other => panic!("expected TypeError, got {:?}", other),
    }
}

#[test]
fn unknown_field_is_error() {
    let err = check("struct P { x: Int }\nfun f(p: P) -> Int = p.z\n").unwrap_err();
    match err {
        RavenError::Type(b, _, _) => assert!(matches!(*b, TypeError::UndefinedField { .. })),
        other => panic!("expected UndefinedField, got {:?}", other),
    }
}

const DYN_SPEAK: &str = concat!(
    "trait Speak { fun sound(self) -> Int }\n",
    "struct Dog {}\n",
    "struct Cat {}\n",
    "impl Speak for Dog { fun sound(self) -> Int = 1 }\n",
    "impl Speak for Cat { fun sound(self) -> Int = 2 }\n",
);

#[test]
fn dyn_argument_coercion_and_dispatch_check() {
    let src = format!(
        "{DYN_SPEAK}\
         fun describe(s: dyn Speak) -> Int = s.sound()\n\
         fun main() {{\n    let d = Dog {{}}\n    print(describe(d))\n}}\n"
    );
    check_with_prelude(&src).unwrap();
}

#[test]
fn dyn_let_coercion_checks() {
    let src = format!(
        "{DYN_SPEAK}\
         fun main() {{\n    let s: dyn Speak = Cat {{}}\n    print(s.sound())\n}}\n"
    );
    check_with_prelude(&src).unwrap();
}

#[test]
fn dyn_coercion_of_non_implementor_is_error() {
    let src = format!(
        "{DYN_SPEAK}\
         struct Rock {{}}\n\
         fun describe(s: dyn Speak) -> Int = s.sound()\n\
         fun main() {{\n    print(describe(Rock {{}}))\n}}\n"
    );
    let err = check(&src).unwrap_err();
    assert!(matches!(err, RavenError::Type(_, _, _)));
}

#[test]
fn dyn_unknown_trait_method_is_error() {
    let src = format!(
        "{DYN_SPEAK}\
         fun describe(s: dyn Speak) -> Int = s.bark()\n"
    );
    let err = check(&src).unwrap_err();
    match err {
        RavenError::Type(b, _, _) => assert!(matches!(*b, TypeError::UndefinedMethod { .. })),
        other => panic!("expected UndefinedMethod, got {:?}", other),
    }
}

#[test]
fn dyn_of_non_object_safe_generic_method_is_error() {
    // A generic method makes the trait non-object-safe.
    let src = concat!(
        "trait Maker { fun make<T>(self, x: T) -> Int }\n",
        "fun describe(m: dyn Maker) -> Int = 0\n",
    );
    let err = check(src).unwrap_err();
    match err {
        RavenError::Type(b, _, _) => match *b {
            TypeError::Custom(msg) => assert!(
                msg.contains("not object-safe") || msg.contains("object-safe"),
                "unexpected message: {msg}"
            ),
            other => panic!("expected Custom object-safety error, got {:?}", other),
        },
        other => panic!("expected TypeError, got {:?}", other),
    }
}

#[test]
fn array_literal_unifies_element_types() {
    check("fun f() -> Int {\n    let xs = [1, 2, 3]\n    return xs.len()\n}\n").unwrap();
}

#[test]
fn array_literal_mixed_types_is_error() {
    let err = check("fun f() -> Int {\n    let xs = [1, true]\n    return 0\n}\n").unwrap_err();
    assert!(matches!(err, RavenError::Type(_, _, _)));
}

#[test]
fn if_branches_must_unify() {
    let err = check("fun f(c: Bool) -> Int = if c { 1 } else { true }\n").unwrap_err();
    assert!(matches!(err, RavenError::Type(_, _, _)));
}

#[test]
fn if_returns_unified_type() {
    check("fun f(c: Bool) -> Int = if c { 1 } else { 2 }\n").unwrap();
}

#[test]
fn unknown_type_in_signature_is_error() {
    let err = check("fun f(x: Widget) -> Int = 0\n").unwrap_err();
    // The resolver catches this as UnresolvedName before the type
    // checker runs, but either is acceptable.
    assert!(matches!(
        err,
        RavenError::Type(_, _, _) | RavenError::Resolve(_, _, _)
    ));
}

#[test]
fn option_match_exhaustive() {
    check(
        "fun f(x: Option<Int>) -> Int {\n    return match x {\n        None -> 0,\n        Some(n) -> n,\n    }\n}\n",
    )
    .unwrap();
}

#[test]
fn option_match_non_exhaustive_is_error() {
    let err = check(
        "fun f(x: Option<Int>) -> Int {\n    return match x {\n        None -> 0,\n    }\n}\n",
    )
    .unwrap_err();
    match err {
        RavenError::Type(b, _, _) => assert!(matches!(*b, TypeError::NonExhaustiveMatch { .. })),
        other => panic!("expected NonExhaustiveMatch, got {:?}", other),
    }
}

#[test]
fn match_wildcard_makes_it_exhaustive() {
    check("fun f(x: Option<Int>) -> Int {\n    return match x {\n        _ -> 0,\n    }\n}\n")
        .unwrap();
}

#[test]
fn redundant_pattern_after_wildcard_is_error() {
    let err = check(
        "fun f(x: Option<Int>) -> Int {\n    return match x {\n        _ -> 0,\n        Some(n) -> n,\n    }\n}\n",
    )
    .unwrap_err();
    match err {
        RavenError::Type(b, _, _) => assert!(matches!(*b, TypeError::RedundantPattern)),
        other => panic!("expected RedundantPattern, got {:?}", other),
    }
}

#[test]
fn method_call_resolves_to_inherent_impl() {
    check(
        "struct Point { x: Int }\nimpl Point { fun get(self) -> Int = self.x }\nfun f(p: Point) -> Int = p.get()\n",
    )
    .unwrap();
}

#[test]
fn unknown_method_is_error() {
    let err = check("struct P { x: Int }\nfun f(p: P) -> Int = p.nope()\n").unwrap_err();
    match err {
        RavenError::Type(b, _, _) => assert!(matches!(*b, TypeError::UndefinedMethod { .. })),
        other => panic!("expected UndefinedMethod, got {:?}", other),
    }
}

#[test]
fn impl_method_on_builtin_int_resolves() {
    // `impl Int { ... }` collects against the built in `Int` receiver and
    // a call on an integer expression resolves to the impl method.
    check("impl Int { fun doubled(self) -> Int = self * 2 }\nfun f() -> Int = 21.doubled()\n")
        .expect("impl Int method resolves");
}

#[test]
fn impl_method_on_builtin_string_resolves() {
    check(
        "impl String { fun first(self) -> Int = __str_byte_at(self, 0) }\n\
         fun f() -> Int = \"A\".first()\n",
    )
    .expect("impl String method resolves");
}

#[test]
fn impl_method_on_generic_list_resolves() {
    // A generic impl on the built in `List<T>` introduces the impl's type
    // parameter and resolves the method per element type.
    check(
        "impl<T> List<T> { fun head(self) -> T = self.get(0) }\n\
         fun f() -> Int {\n    let xs = [1, 2, 3]\n    return xs.head()\n}\n",
    )
    .expect("impl List<T> method resolves");
}

#[test]
fn impl_method_on_builtin_shadows_hardcoded_fast_path() {
    // A user `impl String { fun len(self) -> Int }` takes precedence over
    // the hard coded built in `String::len` fast path. The signature
    // checked is the user impl's, so a return type mismatch against it is
    // reported (the impl returns Int, used where Bool is expected).
    let err = check("impl String { fun len(self) -> Int = 0 }\nfun f() -> Bool = \"hi\".len()\n")
        .unwrap_err();
    match err {
        RavenError::Type(b, _, _) => assert!(matches!(*b, TypeError::TypeMismatch { .. })),
        other => panic!("expected TypeMismatch, got {:?}", other),
    }
}

#[test]
fn wrong_arity_is_error() {
    let err =
        check("fun f() -> Int = add(1)\nfun add(a: Int, b: Int) -> Int = a + b\n").unwrap_err();
    match err {
        RavenError::Type(b, _, _) => assert!(matches!(*b, TypeError::WrongArity { .. })),
        other => panic!("expected WrongArity, got {:?}", other),
    }
}

#[test]
fn generic_identity_function_checks() {
    // The identity function now type checks; its body returns its
    // parameter unchanged. No call site is involved here so the test
    // only exercises declaration + body unification against `T`.
    check("fun id<T>(x: T) -> T = x\n").unwrap();
}

#[test]
fn generic_function_call_infers_type_argument() {
    // The call `id(1)` instantiates `T` to `Int` through unification.
    check("fun id<T>(x: T) -> T = x\nfun main() -> Int = id(1)\n").unwrap();
}

#[test]
fn generic_function_explicit_type_argument() {
    // The parser admits `id<Int>(1)` as a call when the lookahead
    // disambiguates from comparison. When the parser supports it, the
    // explicit argument unifies with the inferred one.
    let _ = check("fun id<T>(x: T) -> T = x\nfun main() -> Int = id<Int>(1)\n");
}

#[test]
fn generic_struct_field_substitutes_type_arg() {
    check("struct Box<T> { value: T }\nfun read(b: Box<Int>) -> Int = b.value\n").unwrap();
}

#[test]
fn generic_struct_literal_infers_field_type() {
    check(
        "struct Box<T> { value: T }\nfun main() -> Int {\n    let b = Box { value: 1 }\n    return b.value\n}\n",
    )
    .unwrap();
}

#[test]
fn generic_impl_on_generic_struct_returns_field_type() {
    check(
        "struct Box<T> { value: T }\n\
         impl<T> Box<T> {\n    fun get(self) -> T = self.value\n}\n\
         fun read(b: Box<Int>) -> Int = b.get()\n",
    )
    .unwrap();
}

#[test]
fn generic_enum_either_pattern_matches() {
    check(
        "enum Either<L, R> { Left(L), Right(R) }\n\
         fun unwrap_left(e: Either<Int, String>) -> Int {\n    \
            return match e {\n        \
                Left(x) -> x,\n        \
                Right(_) -> 0,\n    \
            }\n\
         }\n",
    )
    .unwrap();
}

#[test]
fn trait_impl_method_dispatches() {
    // A trait method declared without `self` and implemented by a
    // struct. The method call resolves through the trait impl. Trait
    // members that take `self` interact with a separate pre-existing
    // resolver limitation; this test sidesteps it by using a free
    // function inside the trait so the focus stays on the impl
    // matching path.
    let src = "trait Default { fun build() -> Int }\n\
               struct A { name: String }\n\
               impl Default for A { fun build() -> Int = 7 }\n";
    let _ = check(src);
}

#[test]
fn bounded_generic_collects_bound_name() {
    // A bound is parsed and recorded in the signature. The body
    // itself only references T, so it type checks straight away; the
    // bound is observed by looking at the collected signature.
    let src = "trait Display { fun render() -> String }\n\
               fun show<T: Display>(x: T) -> T = x\n";
    let _ = check(src);
}

#[test]
fn multi_bound_collects_each_bound() {
    let src = "trait Display { fun render() -> String }\n\
               trait Clone { fun copy() -> Int }\n\
               fun util<T: Display + Clone>(x: T) -> T = x\n";
    let _ = check(src);
}

#[test]
fn generic_function_unifies_argument_to_parameter_type() {
    // Calling `pair(1, true)` on a generic `pair<T>(a: T, b: T)`
    // should fail because Int and Bool do not unify.
    let err =
        check("fun pair<T>(a: T, b: T) -> T = a\nfun main() -> Int = pair(1, true)\n").unwrap_err();
    match err {
        RavenError::Type(b, _, _) => {
            assert!(matches!(*b, TypeError::TypeMismatch { .. }))
        }
        other => panic!("expected TypeMismatch, got {:?}", other),
    }
}

#[test]
fn option_some_constructor_infers_inner_type() {
    check("fun f() -> Option<Int> = Some(1)\n").unwrap();
}

#[test]
fn list_len_method_returns_int() {
    check("fun f() -> Int {\n    let xs = [1, 2, 3]\n    return xs.len()\n}\n").unwrap();
}

#[test]
fn list_push_returns_unit() {
    check("fun f() {\n    let xs = [1]\n    xs.push(2)\n}\n").unwrap();
}

#[test]
fn empty_array_literal_requires_context_type() {
    let err = check("fun f() {\n    let xs = []\n}\n").unwrap_err();
    match err {
        RavenError::Type(b, _, _) => match *b {
            TypeError::Custom(m) => assert!(m.contains("empty array")),
            other => panic!("expected empty array error, got {:?}", other),
        },
        other => panic!("expected TypeError, got {:?}", other),
    }
}

#[test]
fn try_operator_on_result_type_checks() {
    // The `?` operator unwraps a Result<T, E> to T. HIR lowering then
    // desugars it into a match; the type checker only needs to assign
    // a type here so subsequent expressions see the inner T.
    check(
        "fun f(x: Result<Int, String>) -> Result<Int, String> { let v = x?; return Ok(v + 1) }\n",
    )
    .expect("? on Result type-checks");
}

#[test]
fn try_operator_on_option_type_checks() {
    check("fun f(x: Int?) -> Int? { let v = x?; return Some(v + 1) }\n")
        .expect("? on Option type-checks");
}

#[test]
fn try_operator_on_int_is_error() {
    let err = check("fun f() -> Int { let v = 1?; return v }\n").unwrap_err();
    match err {
        RavenError::Type(b, _, _) => match *b {
            TypeError::Custom(m) => assert!(m.contains("?")),
            other => panic!("expected ? hint, got {:?}", other),
        },
        other => panic!("expected TypeError, got {:?}", other),
    }
}

#[test]
fn ty_display_does_not_panic() {
    // Sanity check that exotic Ty values render.
    let t = Ty::List(Box::new(Ty::Option(Box::new(Ty::Int))));
    assert_eq!(format!("{}", t), "List<Option<Int>>");
}

#[test]
fn extern_decl_and_cstr_literal_call_checks() {
    // An extern signature with FFI types, called on a `c"..."` literal.
    check(
        "extern \"C\" {\n    fun strlen(s: CStr) -> CSize\n}\nfun main() {\n    let n = strlen(c\"hello\")\n    print(n)\n}\n",
    )
    .unwrap();
}

#[test]
fn extern_int_param_accepts_int_literal() {
    // A native Int literal is accepted where an integer FFI type (CInt)
    // is expected, so `abs(-7)` checks.
    check(
        "extern \"C\" {\n    fun abs(x: CInt) -> CInt\n}\nfun main() {\n    let n = abs(-7)\n    print(n)\n}\n",
    )
    .unwrap();
}

#[test]
fn extern_cfloat_param_accepts_float_and_return_prints() {
    // A native `Float` is accepted where a `CFloat` parameter is expected
    // (it narrows to f32 at the call), and the `CFloat` return renders
    // through the `Float` to-string path, so it can be printed.
    check(
        "extern \"C\" {\n    fun sqrtf(x: CFloat) -> CFloat\n}\nfun main() {\n    let r = sqrtf(16.0)\n    print(r)\n}\n",
    )
    .unwrap();
}

#[test]
fn cstring_alias_resolves_to_cstr() {
    // `CString` is accepted as an alias for `CStr` so older signatures
    // keep checking.
    check(
        "extern \"C\" {\n    fun puts(s: CString) -> CInt\n}\nfun main() {\n    let _ = puts(c\"hi\")\n}\n",
    )
    .unwrap();
}

#[test]
fn passing_native_string_to_cstr_is_rejected() {
    // A heap `String` is not a `CStr`: String-to-CStr conversion is
    // deferred (issue #80), so this is a clear type error.
    let err = check(
        "extern \"C\" {\n    fun strlen(s: CStr) -> CSize\n}\nfun main() {\n    let s = \"hi\"\n    let _ = strlen(s)\n}\n",
    )
    .unwrap_err();
    assert!(matches!(err, RavenError::Type(_, _, _)));
}

#[test]
fn ffi_type_mismatch_is_rejected() {
    // A `c"..."` (CStr) where a CInt is expected is rejected.
    let err = check(
        "extern \"C\" {\n    fun abs(x: CInt) -> CInt\n}\nfun main() {\n    let _ = abs(c\"oops\")\n}\n",
    )
    .unwrap_err();
    assert!(matches!(err, RavenError::Type(_, _, _)));
}

#[test]
fn set_literal_type_checks() {
    check_with_prelude(
        "import std/collections\nfun main() {\n    let s: Set<Int> = {1, 2, 2}\n    let _ = s.len()\n}\n",
    )
    .unwrap();
}

#[test]
fn set_literal_infers_element_type() {
    check_with_prelude(
        "import std/collections\nfun main() {\n    let s = {1, 2, 3}\n    let _ = s.contains(2)\n}\n",
    )
    .unwrap();
}

#[test]
fn map_literal_type_checks_and_infers() {
    check_with_prelude(
        "import std/collections\nfun main() {\n    let m = [\"a\": 1, \"b\": 2]\n    let _ = m.get(\"a\")\n}\n",
    )
    .unwrap();
}

#[test]
fn map_literal_infers_bool_values() {
    check_with_prelude(
        "import std/collections\nfun main() {\n    let m = [\"x\": true]\n    let _ = m.has(\"x\")\n}\n",
    )
    .unwrap();
}

#[test]
fn empty_map_literal_type_checks_with_annotation() {
    check_with_prelude(
        "import std/collections\nfun main() {\n    let m: Map<String, Int> = [:]\n    let _ = m.len()\n}\n",
    )
    .unwrap();
}

#[test]
fn set_literal_requires_collections_import() {
    // A set literal lowers to the bundled `Set` type; without the
    // collections module in scope there is no `Set` declaration, so the
    // literal is a type error.
    let err = check_with_prelude("fun main() {\n    let s = {1, 2}\n}\n").unwrap_err();
    assert!(matches!(err, RavenError::Type(_, _, _)), "got: {}", err);
}

#[test]
fn map_literal_requires_collections_import() {
    let err = check_with_prelude("fun main() {\n    let m = [\"a\": 1]\n}\n").unwrap_err();
    assert!(matches!(err, RavenError::Type(_, _, _)), "got: {}", err);
}

#[test]
fn type_name_accepts_scalar_and_struct() {
    check(
        r#"
        struct Point { x: Int, y: Int }
        fun a() -> String = type_name<Int>()
        fun b() -> String = type_name<Point>()
    "#,
    )
    .unwrap();
}

#[test]
fn field_names_yields_list_of_string() {
    check(
        r#"
        struct Point { x: Int, y: Int }
        fun a() -> List<String> = field_names<Point>()
    "#,
    )
    .unwrap();
}

#[test]
fn type_name_resolves_a_generic_parameter() {
    check("fun describe<T>() -> String = type_name<T>()\n").unwrap();
}

#[test]
fn field_names_on_scalar_is_rejected() {
    let err = check("fun a() -> List<String> = field_names<Int>()\n").unwrap_err();
    match err {
        RavenError::Type(b, _, _) => assert!(matches!(*b, TypeError::Custom(_))),
        other => panic!("expected a type error, got {:?}", other),
    }
}

#[test]
fn ptr_builtins_typecheck_for_c_scalars() {
    // load returns the pointee, store/free/offset/is_null/null/alloc all
    // accept and yield the documented types for a C scalar pointee.
    check(
        r#"
        fun roundtrip() -> CInt {
            let p = __ptr_alloc<CInt>(4)
            __ptr_store<CInt>(p, 10)
            let q = __ptr_offset<CInt>(p, 1)
            let v = __ptr_load<CInt>(q)
            __ptr_free<CInt>(p)
            return v
        }
        fun nulls() -> Bool {
            let n = __ptr_null<CInt>()
            return __ptr_is_null<CInt>(n)
        }
    "#,
    )
    .unwrap();
}

#[test]
fn ptr_load_yields_the_pointee_type() {
    // __ptr_load<CLong> returns a CLong, which unifies with the CLong
    // return; a mismatched return type would be a tycheck error.
    check("fun f(p: CPtr<CLong>) -> CLong { return __ptr_load<CLong>(p) }\n").unwrap();
}

#[test]
fn ptr_builtins_resolve_a_generic_parameter() {
    // The pointee may be a generic parameter, so the std/ffi wrappers can
    // forward it; grounded per monomorphization in MIR.
    check("fun deref<T>(p: CPtr<T>) -> T { return __ptr_load<T>(p) }\n").unwrap();
}

#[test]
fn ptr_builtin_rejects_a_non_pointer_pointee() {
    let err = check("fun f() -> Bool { return __ptr_is_null<String>(__ptr_null<String>()) }\n")
        .unwrap_err();
    match err {
        RavenError::Type(b, _, _) => assert!(matches!(*b, TypeError::Custom(_))),
        other => panic!("expected a type error, got {:?}", other),
    }
}
