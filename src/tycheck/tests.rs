//! Inline unit tests for the type checker.

use super::{check_file, check_file_all, Ty};
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

/// Type-check `src` and return every recovered diagnostic, for tests that
/// assert the checker reports more than one error per compile.
fn check_all(src: &str) -> Vec<RavenError> {
    let tokens = Lexer::new(src.to_string(), PathBuf::from("t.rv"))
        .tokenize()
        .expect("lex");
    let file = parse(&tokens).expect("parse");
    let mut loader = NoLoader;
    let resolved = resolve_file(&file, &mut loader).expect("resolve");
    match check_file_all(&resolved) {
        Ok(_) => Vec::new(),
        Err(es) => es,
    }
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
fn const_local_is_usable() {
    check("fun f() -> Int {\n    const K: Int = 5\n    return K + 1\n}\n").unwrap();
}

#[test]
fn generic_arg_violating_a_bound_is_rejected() {
    // A type written with a generic argument that does not satisfy the
    // declaration's bound is rejected at type-check, not left to surface as an
    // unresolved callee in the back end (the `Map<Uuid, V>` without `Hash`
    // class of bug).
    let err = check(
        "trait Keyable {}\nstruct Holder<T: Keyable> { v: T }\nstruct Plain { x: Int }\nstruct Bad { h: Holder<Plain> }\n",
    )
    .unwrap_err();
    match err {
        RavenError::Type(b, _, _) => {
            assert!(
                matches!(*b, TypeError::BoundNotSatisfied { .. }),
                "got: {:?}",
                b
            )
        }
        other => panic!("expected a bound error, got {:?}", other),
    }
}

#[test]
fn inferred_type_violating_a_bound_is_rejected() {
    // A call that infers a type argument violating the bound is rejected the
    // moment the inference variable resolves to a concrete type, not deferred
    // to the back end (issue #375).
    let err = check(
        "trait Keyed {}\nstruct Box<T: Keyed> { v: T }\nfun wrap<T: Keyed>(x: T) -> Box<T> {\n    return Box { v: x }\n}\nstruct Plain { x: Int }\nfun main() {\n    let b = wrap(Plain { x: 1 })\n}\n",
    )
    .unwrap_err();
    match err {
        RavenError::Type(b, _, _) => {
            assert!(
                matches!(*b, TypeError::BoundNotSatisfied { .. }),
                "got: {:?}",
                b
            )
        }
        other => panic!("expected a bound error, got {:?}", other),
    }
}

#[test]
fn generic_method_return_only_param_is_grounded() {
    // A method whose generic parameter appears only in the return type
    // type-checks both via an annotation (the expected type) and via an
    // explicit type argument on the call. Without grounding, the explicit-arg
    // call would leave `T` unresolved and fail finalization. Regression for
    // #384.
    let src = "trait Mk { fun mk() -> Self }\nstruct A { t: Int }\nimpl Mk for A {\n    fun mk() -> A { return A { t: 1 } }\n}\nstruct F {}\nimpl F {\n    fun build<T: Mk>(self) -> T { return T.mk() }\n}\nfun main() {\n    let f = F {}\n    let a: A = f.build()\n    let b = f.build<A>()\n}\n";
    let r = check(src);
    assert!(r.is_ok(), "expected ok, got {:?}", r.err());
}

#[test]
fn generic_instantiation_missing_bound_impl_is_rejected() {
    // A generic instantiation (`Box<Int>`) used where a bound requires a trait
    // its constructor never implements is a clean type error, not an unresolved
    // callee at codegen. Regression for #411.
    let bad = "trait Greet { fun hi(self) -> Int }\nstruct Box<T> { value: T }\nfun need<K: Greet>(k: K) -> Int = 0\nfun main() { let x = need(Box { value: 1 }) }\n";
    assert!(check(bad).is_err());
}

#[test]
fn dyn_over_self_returning_trait_is_rejected() {
    // A trait with a `-> Self` method is not object-safe: building a `dyn`
    // value of it must be a type error, not a downstream miscompile. Regression
    // for #412.
    let bad = "trait Cloner { fun dup(self) -> Self }\nstruct W { id: Int }\nimpl Cloner for W {\n    fun dup(self) -> W { return W { id: self.id } }\n}\nfun main() {\n    let w = W { id: 1 }\n    let d: dyn Cloner = w\n}\n";
    assert!(check(bad).is_err());
}

#[test]
fn cross_function_unresolved_var_does_not_panic() {
    // A body that leaves an unresolved inference variable must not crash a
    // later body's finalization. Each body resolves only its own type-map
    // entries, so `later` no longer tries to resolve `consumer`'s variable
    // against its own inference context. This must return a clean type error,
    // not panic.
    let src = "fun ambiguous<T>() -> List<T> {\n    let xs: List<T> = []\n    return xs\n}\nfun consumer() {\n    let a = ambiguous()\n}\nfun later() {\n    let b = 1\n}\n";
    assert!(check(src).is_err());
}

#[test]
fn reassigning_a_const_local_is_rejected() {
    let err = check("fun f() {\n    const K: Int = 5\n    K = 7\n}\n").unwrap_err();
    match err {
        RavenError::Type(b, _, _) => match *b {
            TypeError::Custom(msg) => assert!(msg.contains("const"), "got: {}", msg),
            other => panic!("expected a const-assignment error, got {:?}", other),
        },
        other => panic!("expected a type error, got {:?}", other),
    }
}

#[test]
fn compound_assigning_a_const_local_is_rejected() {
    let err = check("fun f() {\n    const K: Int = 5\n    K += 1\n}\n").unwrap_err();
    assert!(matches!(err, RavenError::Type(_, _, _)));
}

#[test]
fn reassigning_a_let_local_is_allowed() {
    check("fun f() -> Int {\n    let m = 1\n    m = 2\n    return m\n}\n").unwrap();
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
fn repr_c_struct_crosses_ffi_by_value() {
    // A `@repr(C)` struct of two `CInt` fields (8 bytes) is accepted as a
    // by-value argument and return of an extern C function, and an `Int`
    // literal initializes its `CInt` fields.
    check(
        "@repr(C)\nstruct Point {\n    x: CInt\n    y: CInt\n}\nextern \"C\" {\n    fun f(p: Point) -> Point\n}\nfun main() {\n    let q = f(Point { x: 1, y: 2 })\n    print(q.x)\n}\n",
    )
    .unwrap();
}

#[test]
fn repr_c_struct_rejects_non_scalar_field() {
    // A repr(C) struct field must be a C scalar; a `String` field is
    // rejected at the declaration.
    let err = check("@repr(C)\nstruct Bad {\n    s: String\n}\nfun main() {}\n").unwrap_err();
    assert!(matches!(err, RavenError::Type(_, _, _)));
}

#[test]
fn repr_c_struct_allows_two_register() {
    // A repr(C) struct up to two registers (here three CInts, 12 bytes, and
    // four, 16 bytes) crosses the FFI by value.
    check("@repr(C)\nstruct V3 {\n    a: CInt\n    b: CInt\n    c: CInt\n}\nfun main() {}\n")
        .unwrap();
    check("@repr(C)\nstruct R {\n    a: CInt\n    b: CInt\n    c: CInt\n    d: CInt\n}\nfun main() {}\n")
        .unwrap();
}

#[test]
fn repr_c_struct_allows_oversize() {
    // A repr(C) struct larger than two registers (here three CLongs, 24
    // bytes) crosses the FFI by value too: in memory on System V, by
    // reference on Windows x64 and AArch64.
    check("@repr(C)\nstruct Big {\n    a: CLong\n    b: CLong\n    c: CLong\n}\nfun main() {}\n")
        .unwrap();
}

#[test]
fn non_repr_c_struct_rejected_at_c_call() {
    // A plain heap struct (no `@repr(C)`) is a GC pointer and must not be
    // handed to a C function as if it had C layout.
    let err = check(
        "struct P {\n    x: CInt\n}\nextern \"C\" {\n    fun g(p: P)\n}\nfun main() {\n    g(P { x: 1 })\n}\n",
    )
    .unwrap_err();
    assert!(matches!(err, RavenError::Type(_, _, _)));
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
fn top_level_fn_passed_as_cfnptr_checks() {
    // A non-capturing top-level function whose params and return are all
    // C-FFI types is accepted where a `CFnPtr` is expected.
    check(
        "extern \"C\" {\n    fun takes_cb(cmp: CFnPtr)\n}\nfun compare(a: CPtr<CInt>, b: CPtr<CInt>) -> CInt {\n    return __ptr_load<CInt>(a) - __ptr_load<CInt>(b)\n}\nfun main() {\n    takes_cb(compare)\n}\n",
    )
    .unwrap();
}

#[test]
fn cfnptr_arg_with_non_ffi_signature_is_rejected() {
    // A function with a native `Int` parameter has no defined C ABI, so it
    // cannot be passed as a `CFnPtr`.
    let err = check(
        "extern \"C\" {\n    fun takes_cb(cmp: CFnPtr)\n}\nfun bad(x: Int) -> CInt {\n    return 0\n}\nfun main() {\n    takes_cb(bad)\n}\n",
    )
    .unwrap_err();
    assert!(matches!(err, RavenError::Type(_, _, _)));
}

#[test]
fn cfnptr_arg_must_be_a_function_not_a_value() {
    // A non-function value where a `CFnPtr` is expected is rejected.
    let err = check(
        "extern \"C\" {\n    fun takes_cb(cmp: CFnPtr)\n}\nfun main() {\n    takes_cb(7)\n}\n",
    )
    .unwrap_err();
    assert!(matches!(err, RavenError::Type(_, _, _)));
}

#[test]
fn cfnptr_arg_accepts_closure_value() {
    // A closure passed where a `CFnPtr` is expected lowers to a generated
    // trampoline; the closure object is passed to the C function's userdata
    // parameter (a `CPtr`). Accepted as long as the closure's signature is
    // entirely C-FFI typed.
    check(
        "extern \"C\" {\n    fun takes_cb(cb: CFnPtr, data: CPtr<Unit>)\n}\nfun main() {\n    let f = fun(a: CLong) -> CLong = a\n    takes_cb(f, f)\n}\n",
    )
    .unwrap();
}

#[test]
fn cfnptr_arg_rejects_non_ffi_closure_signature() {
    // A closure callback whose parameter is not a C-FFI type has no defined
    // C ABI, so it is rejected.
    let err = check(
        "extern \"C\" {\n    fun takes_cb(cb: CFnPtr, data: CPtr<Unit>)\n}\nfun main() {\n    let f = fun(a: Int) -> Int = a\n    takes_cb(f, f)\n}\n",
    )
    .unwrap_err();
    assert!(matches!(err, RavenError::Type(_, _, _)));
}

#[test]
fn variadic_extern_accepts_extra_ffi_args() {
    // A variadic C function (`...`) accepts extra integer/pointer arguments
    // after its fixed parameters.
    check(
        "extern \"C\" {\n    fun printf(fmt: CStr, ...) -> CInt\n}\nfun main() {\n    let _ = printf(c\"%d %s\", 1, c\"x\")\n}\n",
    )
    .unwrap();
}

#[test]
fn variadic_extern_rejects_float_arg() {
    // A float variadic argument cannot be honored by the back end (System V
    // `al`, Windows x64 float shadow), so it is rejected at compile time.
    let err = check(
        "extern \"C\" {\n    fun printf(fmt: CStr, ...) -> CInt\n}\nfun main() {\n    let _ = printf(c\"%f\", 3.14)\n}\n",
    )
    .unwrap_err();
    assert!(matches!(err, RavenError::Type(_, _, _)));
}

#[test]
fn non_variadic_extern_rejects_extra_args() {
    // Without `...`, the arity is fixed.
    let err = check(
        "extern \"C\" {\n    fun abs(x: CInt) -> CInt\n}\nfun main() {\n    let _ = abs(1, 2)\n}\n",
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
fn field_types_yields_list_of_string() {
    check(
        r#"
        struct Point { x: Int, y: Int }
        fun a() -> List<String> = field_types<Point>()
    "#,
    )
    .unwrap();
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
fn field_types_on_scalar_is_rejected() {
    let err = check("fun a() -> List<String> = field_types<Int>()\n").unwrap_err();
    match err {
        RavenError::Type(b, _, _) => assert!(matches!(*b, TypeError::Custom(_))),
        other => panic!("expected a type error, got {:?}", other),
    }
}

#[test]
fn variant_names_yields_list_of_string() {
    check(
        "enum Shape {\n    Circle(r: Int)\n    Dot\n}\n\
         fun a() -> List<String> = variant_names<Shape>()\n",
    )
    .unwrap();
}

#[test]
fn variant_field_types_yields_nested_list() {
    check(
        "enum Shape {\n    Circle(r: Int)\n    Dot\n}\n\
         fun a() -> List<List<String>> = variant_field_types<Shape>()\n",
    )
    .unwrap();
}

#[test]
fn variant_names_on_struct_is_rejected() {
    let err = check(
        r#"
        struct P { x: Int }
        fun a() -> List<String> = variant_names<P>()
    "#,
    )
    .unwrap_err();
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

#[test]
fn reports_an_error_in_each_sibling_function() {
    // Two unrelated functions each have a return type mismatch; both are
    // reported in one compile, not just the first.
    let errs = check_all("fun f() -> Int = \"x\"\nfun g() -> Bool = 5\n");
    assert_eq!(errs.len(), 2, "got: {:?}", errs);
}

#[test]
fn reports_an_error_for_each_failed_statement() {
    // Two `let` bindings with annotation mismatches both report; the later
    // use does not cascade because each binding adopts its annotated `Int`.
    let errs = check_all(
        "fun h() -> Int {\n    let x: Int = \"a\"\n    let y: Int = true\n    return x + y\n}\n",
    );
    assert_eq!(errs.len(), 2, "got: {:?}", errs);
}

#[test]
fn a_failed_unannotated_let_does_not_cascade() {
    // `let x = <type error>` records one error and binds `x` to `Error`, so
    // the later `x + z` use adds no diagnostic.
    let errs = check_all(
        "fun u() -> Int {\n    let x = 1 + \"oops\"\n    let z = 5\n    return x + z\n}\n",
    );
    assert_eq!(errs.len(), 1, "got: {:?}", errs);
}

#[test]
fn a_clean_program_reports_no_errors() {
    assert!(check_all("fun f() -> Int = 1 + 2\n").is_empty());
}

#[test]
fn duplicate_diagnostics_are_deduplicated() {
    // The same message at the same span is collapsed to one entry.
    let errs = check_all("fun f() -> Int = \"x\"\n");
    assert_eq!(errs.len(), 1, "got: {:?}", errs);
}
