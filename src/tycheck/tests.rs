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
    check("fun f() -> int = 1 + 2 * 3\n").unwrap();
}

#[test]
fn const_local_is_usable() {
    check("fun f() -> int {\n    let K: int = 5\n    return K + 1\n}\n").unwrap();
}

#[test]
fn unit_function_discards_a_trailing_value() {
    check("fun setup() -> bool = true\nfun main() {\n    setup()\n}\n").unwrap();
}

#[test]
fn if_without_else_discards_its_branch_value() {
    check(
        "fun apply(n: int) -> bool = n > 0\nfun run(ready: bool) {\n    if ready {\n        apply(1)\n    }\n}\n",
    )
    .unwrap();
}

#[test]
fn unit_lambda_discards_a_trailing_value() {
    check(
        "fun setup() -> bool = true\nfun main() {\n    let f = () -> Unit => { setup() }\n    f()\n}\n",
    )
    .unwrap();
}

#[test]
fn non_unit_function_still_requires_its_declared_return_type() {
    assert!(check("fun value() -> int { true }").is_err());
}

#[test]
fn if_with_else_still_requires_matching_branch_types() {
    assert!(check("fun run(ready: bool) { if ready { 1 } else { true } }").is_err());
}

#[test]
fn generic_arg_violating_a_bound_is_rejected() {
    // A type written with a generic argument that does not satisfy the
    // declaration's bound is rejected at type-check, not left to surface as an
    // unresolved callee in the back end (the `Map<Uuid, V>` without `Hash`
    // class of bug).
    let err = check(
        "trait Keyable {}\nclass Holder[T: Keyable] { v: T }\nclass Plain { x: int }\nclass Bad { h: Holder[Plain] }\n",
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
fn unary_address_of_is_rejected() {
    // Unary `&` has no reference or pointer type to produce, so it is rejected
    // rather than silently returning the operand (issue #720).
    let err = check("fun main() {\n    let x = 41\n    let y = &x\n}\n").unwrap_err();
    match err {
        RavenError::Type(b, _, _) => assert!(
            matches!(*b, TypeError::Custom(ref m) if m.contains("address-of")),
            "got: {:?}",
            b
        ),
        other => panic!("expected a type error, got {:?}", other),
    }
}

#[test]
fn indexing_a_string_is_rejected() {
    // `s[i]` used to type check as `Char` while the back end had no `String`
    // case for index access, so the index lowered through the list layout and
    // read the string header as a list header, segfaulting at run time
    // (issue #894). It is a type error now, pointing at the byte accessors.
    let err = check("fun main() {\n    let t = \"abc\"\n    let c = t[0]\n}\n").unwrap_err();
    match err {
        RavenError::Type(b, _, _) => assert!(
            matches!(*b, TypeError::Custom(ref m) if m.contains("cannot index into a `str`")),
            "got: {:?}",
            b
        ),
        other => panic!("expected a type error, got {:?}", other),
    }
}

#[test]
fn assigning_through_a_string_index_is_rejected() {
    // The assignment target goes through the same check, so the write side of
    // the crash in issue #894 is rejected too.
    let err = check("fun main() {\n    let t = \"abc\"\n    t[0] = 'z'\n}\n").unwrap_err();
    match err {
        RavenError::Type(b, _, _) => assert!(
            matches!(*b, TypeError::Custom(ref m) if m.contains("cannot index into a `str`")),
            "got: {:?}",
            b
        ),
        other => panic!("expected a type error, got {:?}", other),
    }
}

#[test]
fn indexing_a_list_is_still_allowed() {
    // The guard above is specific to `String`; list indexing keeps working and
    // still yields the element type.
    check("fun main() {\n    let xs = [1, 2, 3]\n    let n: int = xs[1]\n}\n").unwrap();
}

#[test]
fn binary_op_on_one_user_type_names_it_once() {
    // `V + V` used to render "this should be `V and V`, but it's `V and V`":
    // `check_binary` built the expected half from the left operand twice, so
    // both halves were identical whenever the operands shared a type
    // (issue #895). The message now says what is actually wrong.
    let src = "class V { x: int }\nfun main() {\n    let a = V(x: 1)\n    let b = V(x: 2)\n    let c = a + b\n}\n";
    let err = check(src).unwrap_err();
    let rendered = err.render(src, false);
    assert!(
        rendered.contains("`+` is not defined for `V`"),
        "got: {}",
        rendered
    );
    // The degenerate "X and X" phrasing is gone from both halves.
    assert!(!rendered.contains("V and V"), "got: {}", rendered);
    // And the misleading conversion note does not apply here.
    assert!(
        !rendered.contains("no implicit conversions"),
        "got: {}",
        rendered
    );
    // The help names the method that would overload the operator.
    assert!(
        rendered.contains("fun add(self, other: V) -> V"),
        "got: {}",
        rendered
    );
}

#[test]
fn binary_op_on_mixed_types_names_both_sides() {
    // When the operands differ, both types appear and the help names the
    // conversion that fixes it.
    let src = "fun main() {\n    let c = 1 + 2.0\n}\n";
    let err = check(src).unwrap_err();
    let rendered = err.render(src, false);
    assert!(
        rendered.contains("`+` is not defined for `int` and `float`"),
        "got: {}",
        rendered
    );
    assert!(rendered.contains("to_float()"), "got: {}", rendered);
}

#[test]
fn ordering_a_user_type_points_at_derive_ord() {
    // `Ord` is derivable and supplies `compare`, so the ordering operators
    // point there rather than at a hand-written arithmetic-style method.
    let src = "class V { x: int }\nfun main() {\n    let c = V(x: 1) < V(x: 2)\n}\n";
    let err = check(src).unwrap_err();
    let rendered = err.render(src, false);
    assert!(
        rendered.contains("`<` is not defined for `V`"),
        "got: {}",
        rendered
    );
    assert!(rendered.contains("@derive(Ord)"), "got: {}", rendered);
    assert!(rendered.contains("implement `Ord`"), "got: {}", rendered);
    assert!(rendered.contains("compare"), "got: {}", rendered);
}

#[test]
fn logical_and_bitwise_ops_report_their_own_domain() {
    // The logical and bitwise arms carry the operator's own accepted types
    // rather than a generic mismatch.
    let src = "fun main() {\n    let c = 1 and 2\n}\n";
    let rendered = check(src).unwrap_err().render(src, false);
    assert!(
        rendered.contains("`and` is not defined for `int`"),
        "got: {}",
        rendered
    );
    assert!(
        rendered.contains("`and` works on bool"),
        "got: {}",
        rendered
    );

    let src = "fun main() {\n    let c = 1.5 & 2.5\n}\n";
    let rendered = check(src).unwrap_err().render(src, false);
    assert!(
        rendered.contains("`&` is not defined for `float`"),
        "got: {}",
        rendered
    );
    assert!(rendered.contains("`&` works on int"), "got: {}", rendered);
}

#[test]
fn mixed_numeric_hint_matches_the_operator_domain() {
    // A logical operator wants Bool, so a mixed Int/Float pair must not be
    // told to convert to Float; that expression would still be rejected.
    let src = "fun main() {\n    let c = 1 and 2.0\n}\n";
    let rendered = check(src).unwrap_err().render(src, false);
    assert!(
        rendered.contains("`and` is not defined for `int` and `float`"),
        "got: {}",
        rendered
    );
    assert!(!rendered.contains("to_float()"), "got: {}", rendered);
    assert!(!rendered.contains("to_int()"), "got: {}", rendered);

    // A bitwise operator wants Int on both sides, so the Float is the operand
    // to convert.
    let src = "fun main() {\n    let c = 1 & 2.0\n}\n";
    let rendered = check(src).unwrap_err().render(src, false);
    assert!(
        rendered.contains("`&` is not defined for `int` and `float`"),
        "got: {}",
        rendered
    );
    assert!(rendered.contains("to_int()"), "got: {}", rendered);
    assert!(!rendered.contains("to_float()"), "got: {}", rendered);
}

#[test]
fn supported_operators_still_type_check() {
    // The new error path must not narrow what the operators accept.
    check("fun main() {\n    let a = 1 + 2\n    let b = 1.5 * 2.0\n    let c = 7 % 3\n}\n")
        .unwrap();
    check("fun main() {\n    let a = 1 < 2\n    let b = \"a\" < \"b\"\n    let c = 'a' < 'b'\n}\n")
        .unwrap();
    check("fun main() {\n    let a = true and false\n    let b = 6 & 3\n    let c = 1 << 4\n}\n")
        .unwrap();
}

#[test]
fn inferred_type_violating_a_bound_is_rejected() {
    // A call that infers a type argument violating the bound is rejected the
    // moment the inference variable resolves to a concrete type, not deferred
    // to the back end (issue #375).
    let err = check(
        "trait Keyed {}\nclass Box[T: Keyed] { v: T }\nfun wrap[T: Keyed](x: T) -> Box[T] {\n    return Box(v: x)\n}\nclass Plain { x: int }\nfun main() {\n    let b = wrap(Plain(x: 1))\n}\n",
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
fn nested_match_none_arm_infers() {
    // A match arm that is itself a match returning Option, where both the inner
    // and outer matches have a None arm. The inner scrutinee is an inference
    // variable solved during checking; resolving it up front lets the inner
    // None infer and keeps exhaustiveness from flagging a false catch-all.
    // Regression for #515.
    let src = "enum V { Str(str), Int(int) }\n\
               fun f(o: Option[V]) -> Option[str] {\n\
               return match o {\ncase \
               Some(v) { match v {\ncase \
               Str(s) { Some(s) }\ncase \
               Int(n) { None }\n\
               } }\ncase \
               None { None }\n\
               }\n\
               }\n\
               fun main() {}\n";
    assert!(check(src).is_ok());
}

#[test]
fn derive_ord_struct_and_enum_check() {
    // `@derive(Ord)` synthesizes a `compare` method that type-checks for a
    // struct and a tuple enum. Feature for #499.
    assert!(check("@derive(Ord)\nclass P { x: int, y: int }\nfun main() {}\n").is_ok());
    assert!(check("@derive(Ord)\nenum E { A, B(int), C(int, int) }\nfun main() {}\n").is_ok());
}

#[test]
fn module_level_let_non_literal_needs_annotation() {
    // An unannotated module-level `let` with a non-literal initializer is a
    // clean type error rather than an opaque codegen failure. Regression for
    // #497.
    assert!(check("fun make() -> int { return 5 }\nlet g = make()\nfun main() {}\n").is_err());
}

#[test]
fn module_level_let_literal_is_inferred() {
    // An unannotated literal global is still inferred. Companion to #497.
    assert!(check("let n = 42\nlet s = \"hi\"\nfun main() {}\n").is_ok());
}

#[test]
fn module_level_empty_list_adopts_annotation() {
    // A top-level `let xs: List<Int> = []` must adopt its annotated element
    // type, the same as a local binding does, rather than reporting that the
    // empty array needs a context type. Regression for #498.
    assert!(check("let xs: list[int] = []\nfun main() {}\n").is_ok());
}

#[test]
fn nested_constructor_patterns_check_exhaustiveness() {
    let full = "fun f(x: Option[Result[int, int]]) -> int {\n    return match x {\n        case Some(Ok(n)) { n }\n        case Some(Err(e)) { 0 - e }\n        case None { 0 }\n    }\n}\nfun main() {}\n";
    assert!(check(full).is_ok());
    let partial = "fun f(x: Option[Result[int, int]]) -> int {\n    return match x {\n        case Some(Ok(n)) { n }\n        case None { 0 }\n    }\n}\nfun main() {}\n";
    assert!(check(partial).is_err());
}

#[test]
fn generic_method_return_only_param_is_grounded() {
    // A method whose generic parameter appears only in the return type
    // type-checks both via an annotation (the expected type) and via an
    // explicit type argument on the call. Without grounding, the explicit-arg
    // call would leave `T` unresolved and fail finalization. Regression for
    // #384.
    let src = "trait Mk { fun mk() -> Self }\nclass A { t: int }\nimpl Mk for A {\n    fun mk() -> A { return A(t: 1) }\n}\nclass F {}\nimpl F {\n    fun build[T: Mk](self) -> T { return T.mk() }\n}\nfun main() {\n    let f = F()\n    let a: A = f.build()\n    let b = f.build[A]()\n}\n";
    let r = check(src);
    assert!(r.is_ok(), "expected ok, got {:?}", r.err());
}

#[test]
fn generic_instantiation_missing_bound_impl_is_rejected() {
    // A generic instantiation (`Box<Int>`) used where a bound requires a trait
    // its constructor never implements is a clean type error, not an unresolved
    // callee at codegen. Regression for #411.
    let bad = "trait Greet { fun hi(self) -> int }\nclass Box[T] { value: T }\nfun need[K: Greet](k: K) -> int = 0\nfun main() { let x = need(Box(value: 1)) }\n";
    assert!(check(bad).is_err());
}

#[test]
fn dyn_over_self_returning_trait_is_rejected() {
    // A trait with a `-> Self` method is not object-safe: building a `dyn`
    // value of it must be a type error, not a downstream miscompile. Regression
    // for #412.
    let bad = "trait Cloner { fun dup(self) -> Self }\nclass W { id: int }\nimpl Cloner for W {\n    fun dup(self) -> W { return W(id: self.id) }\n}\nfun main() {\n    let w = W(id: 1)\n    let d: dyn Cloner = w\n}\n";
    assert!(check(bad).is_err());
}

#[test]
fn cross_function_unresolved_var_does_not_panic() {
    // A body that leaves an unresolved inference variable must not crash a
    // later body's finalization. Each body resolves only its own type-map
    // entries, so `later` no longer tries to resolve `consumer`'s variable
    // against its own inference context. This must return a clean type error,
    // not panic.
    let src = "fun ambiguous[T]() -> list[T] {\n    let xs: list[T] = []\n    return xs\n}\nfun consumer() {\n    let a = ambiguous()\n}\nfun later() {\n    let b = 1\n}\n";
    assert!(check(src).is_err());
}

#[test]
fn reassigning_a_const_local_is_rejected() {
    let err = check("fun f() {\n    let K: int = 5\n    K = 7\n}\n").unwrap_err();
    match err {
        RavenError::Type(b, _, _) => match *b {
            TypeError::Custom(msg) => assert!(msg.contains("cannot assign"), "got: {}", msg),
            other => panic!("expected a const-assignment error, got {:?}", other),
        },
        other => panic!("expected a type error, got {:?}", other),
    }
}

#[test]
fn compound_assigning_a_const_local_is_rejected() {
    let err = check("fun f() {\n    let K: int = 5\n    K += 1\n}\n").unwrap_err();
    assert!(matches!(err, RavenError::Type(_, _, _)));
}

#[test]
fn reassigning_a_let_local_is_allowed() {
    check("fun f() -> int {\n    var m = 1\n    m = 2\n    return m\n}\n").unwrap();
}

#[test]
fn mixed_int_float_arithmetic_is_rejected() {
    // Still rejected; the diagnostic moved from `TypeMismatch` to the
    // operator-specific variant in issue #895, since the operator, not just
    // one operand's type, is what the message needs to name.
    let err = check("fun f() -> float = 1 + 2.0\n").unwrap_err();
    match err {
        RavenError::Type(b, _, _) => assert!(
            matches!(*b, TypeError::BinaryOpUnsupported { ref op, .. } if op == "+"),
            "got: {:?}",
            b
        ),
        other => panic!("expected BinaryOpUnsupported, got {:?}", other),
    }
}

#[test]
fn bool_logical_ops_produce_bool() {
    check("fun f(a: bool, b: bool) -> bool = a and b or not a\n").unwrap();
}

#[test]
fn comparison_returns_bool() {
    check("fun f(a: int, b: int) -> bool = a < b\n").unwrap();
}

#[test]
fn comparison_requires_compatible_operands() {
    let err = check("fun f(a: int, b: bool) -> bool = a < b\n").unwrap_err();
    assert!(matches!(err, RavenError::Type(_, _, _)));
}

#[test]
fn bound_trait_method_with_self_arg_keeps_its_parameter() {
    // A trait method taking a `Self` argument, called on a generic value
    // bounded by that trait, must keep the argument: only the leading
    // `self` receiver is dropped, not every `Self`-typed parameter.
    check(
        "trait Combine {\n    fun combine(self, other: Self) -> Self\n}\nfun pair[T: Combine](a: T, b: T) -> T {\n    return a.combine(b)\n}\n",
    )
    .unwrap();
}

#[test]
fn struct_literal_and_field_access() {
    check(
        "class Point { x: int, y: int }\nfun f() -> int {\n    let p = Point(x: 1, y: 2)\n    return p.x\n}\n",
    )
    .unwrap();
}

#[test]
fn struct_literal_missing_field_is_error() {
    let err = check("class P { x: int, y: int }\nfun f() -> P = P(x: 1)\n").unwrap_err();
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
    let err = check("class P { x: int }\nfun f(p: P) -> int = p.z\n").unwrap_err();
    match err {
        RavenError::Type(b, _, _) => assert!(matches!(*b, TypeError::UndefinedField { .. })),
        other => panic!("expected UndefinedField, got {:?}", other),
    }
}

const DYN_SPEAK: &str = concat!(
    "trait Speak { fun sound(self) -> int }\n",
    "class Dog {}\n",
    "class Cat {}\n",
    "impl Speak for Dog { fun sound(self) -> int = 1 }\n",
    "impl Speak for Cat { fun sound(self) -> int = 2 }\n",
);

#[test]
fn dyn_argument_coercion_and_dispatch_check() {
    let src = format!(
        "{DYN_SPEAK}\
         fun describe(s: dyn Speak) -> int = s.sound()\n\
         fun main() {{\n    let d = Dog()\n    print(describe(d))\n}}\n"
    );
    check_with_prelude(&src).unwrap();
}

#[test]
fn dyn_let_coercion_checks() {
    let src = format!(
        "{DYN_SPEAK}\
         fun main() {{\n    let s: dyn Speak = Cat()\n    print(s.sound())\n}}\n"
    );
    check_with_prelude(&src).unwrap();
}

#[test]
fn dyn_coercion_of_non_implementor_is_error() {
    let src = format!(
        "{DYN_SPEAK}\
         class Rock {{}}\n\
         fun describe(s: dyn Speak) -> int = s.sound()\n\
         fun main() {{\n    print(describe(Rock()))\n}}\n"
    );
    let err = check(&src).unwrap_err();
    assert!(matches!(err, RavenError::Type(_, _, _)));
}

#[test]
fn dyn_unknown_trait_method_is_error() {
    let src = format!(
        "{DYN_SPEAK}\
         fun describe(s: dyn Speak) -> int = s.bark()\n"
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
        "trait Maker { fun make[T](self, x: T) -> int }\n",
        "fun describe(m: dyn Maker) -> int = 0\n",
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
    check("fun f() -> int {\n    let xs = [1, 2, 3]\n    return xs.len()\n}\n").unwrap();
}

#[test]
fn array_literal_mixed_types_is_error() {
    let err = check("fun f() -> int {\n    let xs = [1, true]\n    return 0\n}\n").unwrap_err();
    assert!(matches!(err, RavenError::Type(_, _, _)));
}

#[test]
fn if_branches_must_unify() {
    let err = check("fun f(c: bool) -> int = if c { 1 } else { true }\n").unwrap_err();
    assert!(matches!(err, RavenError::Type(_, _, _)));
}

#[test]
fn if_returns_unified_type() {
    check("fun f(c: bool) -> int = if c { 1 } else { 2 }\n").unwrap();
}

#[test]
fn unknown_type_in_signature_is_error() {
    let err = check("fun f(x: Widget) -> int = 0\n").unwrap_err();
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
        "fun f(x: Option[int]) -> int {\n    return match x {\n        case None { 0 }\n        case Some(n) { n }\n    }\n}\n",
    )
    .unwrap();
}

#[test]
fn option_match_non_exhaustive_is_error() {
    let err = check(
        "fun f(x: Option[int]) -> int {\n    return match x {\n        case None { 0 }\n    }\n}\n",
    )
    .unwrap_err();
    match err {
        RavenError::Type(b, _, _) => assert!(matches!(*b, TypeError::NonExhaustiveMatch { .. })),
        other => panic!("expected NonExhaustiveMatch, got {:?}", other),
    }
}

#[test]
fn match_wildcard_makes_it_exhaustive() {
    check("fun f(x: Option[int]) -> int {\n    return match x {\n        case _ { 0 }\n    }\n}\n")
        .unwrap();
}

#[test]
fn redundant_pattern_after_wildcard_is_error() {
    let err = check(
        "fun f(x: Option[int]) -> int {\n    return match x {\n        case _ { 0 }\n        case Some(n) { n }\n    }\n}\n",
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
        "class Point { x: int }\nimpl Point { fun get(self) -> int = self.x }\nfun f(p: Point) -> int = p.get()\n",
    )
    .unwrap();
}

#[test]
fn unknown_method_is_error() {
    let err = check("class P { x: int }\nfun f(p: P) -> int = p.nope()\n").unwrap_err();
    match err {
        RavenError::Type(b, _, _) => assert!(matches!(*b, TypeError::UndefinedMethod { .. })),
        other => panic!("expected UndefinedMethod, got {:?}", other),
    }
}

#[test]
fn impl_method_on_builtin_int_resolves() {
    // `impl Int { ... }` collects against the built in `Int` receiver and
    // a call on an integer expression resolves to the impl method.
    check("impl int { fun doubled(self) -> int = self * 2 }\nfun f() -> int = 21.doubled()\n")
        .expect("impl int method resolves");
}

#[test]
fn impl_method_on_builtin_string_resolves() {
    check(
        "impl str { fun first(self) -> int = __str_byte_at(self, 0) }\n\
         fun f() -> int = \"A\".first()\n",
    )
    .expect("impl str method resolves");
}

#[test]
fn impl_method_on_generic_list_resolves() {
    // A generic impl on the built in `List<T>` introduces the impl's type
    // parameter and resolves the method per element type.
    check(
        "impl[T] list[T] { fun head(self) -> T = self.get(0) }\n\
         fun f() -> int {\n    let xs = [1, 2, 3]\n    return xs.head()\n}\n",
    )
    .expect("impl list[T] method resolves");
}

#[test]
fn impl_method_on_builtin_shadows_hardcoded_fast_path() {
    // A user `impl String { fun len(self) -> Int }` takes precedence over
    // the hard coded built in `String::len` fast path. The signature
    // checked is the user impl's, so a return type mismatch against it is
    // reported (the impl returns Int, used where Bool is expected).
    let err = check("impl str { fun len(self) -> int = 0 }\nfun f() -> bool = \"hi\".len()\n")
        .unwrap_err();
    match err {
        RavenError::Type(b, _, _) => assert!(matches!(*b, TypeError::TypeMismatch { .. })),
        other => panic!("expected TypeMismatch, got {:?}", other),
    }
}

#[test]
fn wrong_arity_is_error() {
    let err =
        check("fun f() -> int = add(1)\nfun add(a: int, b: int) -> int = a + b\n").unwrap_err();
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
    check("fun id[T](x: T) -> T = x\n").unwrap();
}

#[test]
fn generic_function_call_infers_type_argument() {
    // The call `id(1)` instantiates `T` to `Int` through unification.
    check("fun id[T](x: T) -> T = x\nfun main() -> int = id(1)\n").unwrap();
}

#[test]
fn generic_function_explicit_type_argument() {
    // The parser admits `id<Int>(1)` as a call when the lookahead
    // disambiguates from comparison. When the parser supports it, the
    // explicit argument unifies with the inferred one.
    let _ = check("fun id[T](x: T) -> T = x\nfun main() -> int = id[int](1)\n");
}

#[test]
fn generic_struct_field_substitutes_type_arg() {
    check("class Box[T] { value: T }\nfun read(b: Box[int]) -> int = b.value\n").unwrap();
}

#[test]
fn generic_struct_literal_infers_field_type() {
    check(
        "class Box[T] { value: T }\nfun main() -> int {\n    let b = Box(value: 1)\n    return b.value\n}\n",
    )
    .unwrap();
}

#[test]
fn generic_impl_on_generic_struct_returns_field_type() {
    check(
        "class Box[T] { value: T }\n\
         impl[T] Box[T] {\n    fun get(self) -> T = self.value\n}\n\
         fun read(b: Box[int]) -> int = b.get()\n",
    )
    .unwrap();
}

#[test]
fn generic_enum_either_pattern_matches() {
    check(
        "enum Either[L, R] { Left(L), Right(R) }\n\
         fun unwrap_left(e: Either[int, str]) -> int {\n    \
            return match e {\n        case \
                Left(x) { x }\n        case \
                Right(_) { 0 }\n    \
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
    let src = "trait Default { fun build() -> int }\n\
               class A { name: str }\n\
               impl Default for A { fun build() -> int = 7 }\n";
    let _ = check(src);
}

#[test]
fn bounded_generic_collects_bound_name() {
    // A bound is parsed and recorded in the signature. The body
    // itself only references T, so it type checks straight away; the
    // bound is observed by looking at the collected signature.
    let src = "trait Display { fun render() -> str }\n\
               fun show[T: Display](x: T) -> T = x\n";
    let _ = check(src);
}

#[test]
fn multi_bound_collects_each_bound() {
    let src = "trait Display { fun render() -> str }\n\
               trait Clone { fun copy() -> int }\n\
               fun util[T: Display + Clone](x: T) -> T = x\n";
    let _ = check(src);
}

#[test]
fn generic_function_unifies_argument_to_parameter_type() {
    // Calling `pair(1, true)` on a generic `pair<T>(a: T, b: T)`
    // should fail because Int and Bool do not unify.
    let err =
        check("fun pair[T](a: T, b: T) -> T = a\nfun main() -> int = pair(1, true)\n").unwrap_err();
    match err {
        RavenError::Type(b, _, _) => {
            assert!(matches!(*b, TypeError::TypeMismatch { .. }))
        }
        other => panic!("expected TypeMismatch, got {:?}", other),
    }
}

#[test]
fn option_some_constructor_infers_inner_type() {
    check("fun f() -> Option[int] = Some(1)\n").unwrap();
}

#[test]
fn list_len_method_returns_int() {
    check("fun f() -> int {\n    let xs = [1, 2, 3]\n    return xs.len()\n}\n").unwrap();
}

#[test]
fn list_push_returns_unit() {
    check("fun f() {\n    let xs = [1]\n    xs.push(2)\n}\n").unwrap();
}

#[test]
fn empty_array_literal_without_any_use_cannot_infer() {
    let err = check(
        "fun f() {
    let xs = []
}
",
    )
    .unwrap_err();
    assert!(matches!(err, RavenError::Type(..)), "got {:?}", err);
}

#[test]
fn empty_array_literal_infers_from_later_use() {
    check(
        "fun f() -> int {
    let xs = []
    xs.push(1)
    return xs.len()
}
",
    )
    .unwrap();
}

#[test]
fn try_operator_on_result_type_checks() {
    // The `?` operator unwraps a Result<T, E> to T. HIR lowering then
    // desugars it into a match; the type checker only needs to assign
    // a type here so subsequent expressions see the inner T.
    check("fun f(x: Result[int, str]) -> Result[int, str] { let v = x?; return Ok(v + 1) }\n")
        .expect("? on Result type-checks");
}

#[test]
fn try_operator_on_option_type_checks() {
    check("fun f(x: int?) -> int? { let v = x?; return Some(v + 1) }\n")
        .expect("? on Option type-checks");
}

#[test]
fn try_operator_on_int_is_error() {
    let err = check("fun f() -> int { let v = 1?; return v }\n").unwrap_err();
    match err {
        RavenError::Type(b, _, _) => match *b {
            TypeError::Custom(m) => assert!(m.contains("?")),
            other => panic!("expected ? hint, got {:?}", other),
        },
        other => panic!("expected TypeError, got {:?}", other),
    }
}

#[test]
fn try_on_result_in_non_result_fn_is_error() {
    // `x?` propagates `Err(String)` out of a function that returns a plain
    // `Int`, which cannot carry the error. The checker must reject it instead
    // of silently dropping the propagated error (issue #527).
    let err = check("fun f(x: Result[int, str]) -> int { let v = x?; return v }\n").unwrap_err();
    match err {
        RavenError::Type(b, _, _) => match *b {
            TypeError::Custom(m) => assert!(m.contains("not a Result"), "got: {}", m),
            other => panic!("expected Custom not-a-Result error, got {:?}", other),
        },
        other => panic!("expected TypeError, got {:?}", other),
    }
}

#[test]
fn try_on_result_with_incompatible_error_type_is_error() {
    // The propagated error type (`String`) must unify with the enclosing
    // function's error type (`Int`); a mismatch is a type error.
    let err =
        check("fun f(x: Result[int, str]) -> Result[int, int] { let v = x?; return Ok(v) }\n")
            .unwrap_err();
    assert!(
        matches!(err, RavenError::Type(_, _, _)),
        "expected a type error, got {:?}",
        err
    );
}

#[test]
fn try_on_option_in_non_option_fn_is_error() {
    // `?` on an `Option` propagates `None`, so the enclosing function must
    // return an `Option`; returning `Int` is rejected.
    let err = check("fun f(x: int?) -> int { let v = x?; return v }\n").unwrap_err();
    match err {
        RavenError::Type(b, _, _) => match *b {
            TypeError::Custom(m) => assert!(m.contains("not an Option"), "got: {}", m),
            other => panic!("expected Custom not-an-Option error, got {:?}", other),
        },
        other => panic!("expected TypeError, got {:?}", other),
    }
}

#[test]
fn break_and_continue_outside_a_loop_are_rejected() {
    assert!(
        check("fun f() { break }\n").is_err(),
        "break outside a loop"
    );
    assert!(
        check("fun f() { continue }\n").is_err(),
        "continue outside a loop"
    );
    // A loop does not extend into a nested lambda: a break inside the lambda is
    // outside any loop.
    assert!(
        check("fun f() { loop { let g = () -> Unit => { break } } }\n").is_err(),
        "break inside a lambda body is outside the enclosing loop"
    );
}

#[test]
fn break_and_continue_inside_a_loop_are_ok() {
    check("fun f() { loop { break } }\n").expect("break in a loop");
    check("fun f() {\n    var i = 0\n    while i < 3 {\n        i = i + 1\n        continue\n    }\n}\n")
        .expect("continue in a while");
}

#[test]
fn break_with_a_value_outside_a_value_loop_is_rejected() {
    assert!(
        check("fun f() { while true { break 5 } }\n").is_err(),
        "break with a value is only valid in a `loop`"
    );
}

#[test]
fn duplicate_enum_variant_is_rejected() {
    assert!(check("enum E { A, A }\nfun main() {}\n").is_err());
}

#[test]
fn duplicate_method_in_impl_is_rejected() {
    assert!(
        check("class S {}\nimpl S {\n    fun m(self) -> int = 1\n    fun m(self) -> int = 2\n}\nfun main() {}\n")
            .is_err()
    );
}

#[test]
fn duplicate_parameter_name_is_rejected() {
    assert!(check("fun f(x: int, x: int) -> int = x\nfun main() {}\n").is_err());
}

#[test]
fn constructor_pattern_on_a_non_enum_is_rejected() {
    let err = check("fun f(n: int) -> int {\n    return match n {\n        case Nope(x) { x }\n        case _ { 0 }\n    }\n}\nfun main() {}\n")
        .unwrap_err();
    match err {
        RavenError::Type(b, _, _) => match *b {
            TypeError::Custom(m) => assert!(m.contains("not a constructor"), "got: {}", m),
            other => panic!("expected Custom, got {:?}", other),
        },
        other => panic!("expected TypeError, got {:?}", other),
    }
}

#[test]
fn struct_variant_pattern_is_rejected_not_crashed() {
    // A named-field variant matched with `{ ... }` is rejected with a
    // diagnostic instead of reaching codegen and crashing Cranelift (#545).
    let err = check("enum E {\n    P(x: int, y: int),\n    Z,\n}\nfun f(e: E) -> int {\n    return match e {\n        case P { x, y } { x + y }\n        case Z { 0 }\n    }\n}\nfun main() {}\n")
        .unwrap_err();
    assert!(
        matches!(err, RavenError::Type(_, _, _)),
        "expected a type error, got {:?}",
        err
    );
}

#[test]
fn duplicate_variant_arm_is_redundant() {
    let err = check("fun f(o: Option[int]) -> int {\n    return match o {\n        case None { 0 }\n        case None { 1 }\n        case Some(n) { n }\n    }\n}\nfun main() {}\n")
        .unwrap_err();
    match err {
        RavenError::Type(b, _, _) => {
            assert!(matches!(*b, TypeError::RedundantPattern), "got {:?}", b)
        }
        other => panic!("expected TypeError, got {:?}", other),
    }
}

#[test]
fn duplicate_literal_arm_is_redundant() {
    let err = check("fun f(n: int) -> str {\n    return match n {\n        case 0 { \"a\" }\n        case 0 { \"b\" }\n        case _ { \"c\" }\n    }\n}\nfun main() {}\n")
        .unwrap_err();
    match err {
        RavenError::Type(b, _, _) => {
            assert!(matches!(*b, TypeError::RedundantPattern), "got {:?}", b)
        }
        other => panic!("expected TypeError, got {:?}", other),
    }
}

#[test]
fn distinct_literal_arms_are_ok() {
    check("fun f(n: int) -> str {\n    return match n {\n        case 0 { \"a\" }\n        case 1 { \"b\" }\n        case _ { \"c\" }\n    }\n}\nfun main() {}\n")
        .expect("distinct literal arms are fine");
}

#[test]
fn ty_display_does_not_panic() {
    // Sanity check that exotic Ty values render.
    let t = Ty::List(Box::new(Ty::Option(Box::new(Ty::Int))));
    assert_eq!(format!("{}", t), "list[Option[int]]");
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
        "@repr(C)\nclass Point {\n    x: CInt\n    y: CInt\n}\nextern \"C\" {\n    fun f(p: Point) -> Point\n}\nfun main() {\n    let q = f(Point(x: 1, y: 2))\n    print(q.x)\n}\n",
    )
    .unwrap();
}

#[test]
fn repr_c_struct_rejects_non_scalar_field() {
    // A repr(C) struct field must be a C scalar; a `String` field is
    // rejected at the declaration.
    let err = check("@repr(C)\nclass Bad {\n    s: str\n}\nfun main() {}\n").unwrap_err();
    assert!(matches!(err, RavenError::Type(_, _, _)));
}

#[test]
fn repr_c_struct_allows_two_register() {
    // A repr(C) struct up to two registers (here three CInts, 12 bytes, and
    // four, 16 bytes) crosses the FFI by value.
    check("@repr(C)\nclass V3 {\n    a: CInt\n    b: CInt\n    c: CInt\n}\nfun main() {}\n")
        .unwrap();
    check("@repr(C)\nclass R {\n    a: CInt\n    b: CInt\n    c: CInt\n    d: CInt\n}\nfun main() {}\n")
        .unwrap();
}

#[test]
fn repr_c_struct_allows_oversize() {
    // A repr(C) struct larger than two registers (here three CLongs, 24
    // bytes) crosses the FFI by value too: in memory on System V, by
    // reference on Windows x64 and AArch64.
    check("@repr(C)\nclass Big {\n    a: CLong\n    b: CLong\n    c: CLong\n}\nfun main() {}\n")
        .unwrap();
}

#[test]
fn non_repr_c_struct_rejected_at_c_call() {
    // A plain heap struct (no `@repr(C)`) is a GC pointer and must not be
    // handed to a C function as if it had C layout.
    let err = check(
        "class P {\n    x: CInt\n}\nextern \"C\" {\n    fun g(p: P)\n}\nfun main() {\n    g(P(x: 1))\n}\n",
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
        "extern \"C\" {\n    fun takes_cb(cmp: CFnPtr)\n}\nfun compare(a: CPtr[CInt], b: CPtr[CInt]) -> CInt {\n    return __ptr_load[CInt](a) - __ptr_load[CInt](b)\n}\nfun main() {\n    takes_cb(compare)\n}\n",
    )
    .unwrap();
}

#[test]
fn cfnptr_arg_with_non_ffi_signature_is_rejected() {
    // A function with a native `Int` parameter has no defined C ABI, so it
    // cannot be passed as a `CFnPtr`.
    let err = check(
        "extern \"C\" {\n    fun takes_cb(cmp: CFnPtr)\n}\nfun bad(x: int) -> CInt {\n    return 0\n}\nfun main() {\n    takes_cb(bad)\n}\n",
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
        "extern \"C\" {\n    fun takes_cb(cb: CFnPtr, data: CPtr[Unit])\n}\nfun main() {\n    let f = (a: CLong) -> CLong => a\n    takes_cb(f, f)\n}\n",
    )
    .unwrap();
}

#[test]
fn cfnptr_arg_rejects_non_ffi_closure_signature() {
    // A closure callback whose parameter is not a C-FFI type has no defined
    // C ABI, so it is rejected.
    let err = check(
        "extern \"C\" {\n    fun takes_cb(cb: CFnPtr, data: CPtr[Unit])\n}\nfun main() {\n    let f = (a: int) -> int => a\n    takes_cb(f, f)\n}\n",
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
        "import std.collections\nfun main() {\n    let s: set[int] = {1, 2, 2}\n    let _ = s.len()\n}\n",
    )
    .unwrap();
}

#[test]
fn set_literal_infers_element_type() {
    check_with_prelude(
        "import std.collections\nfun main() {\n    let s = {1, 2, 3}\n    let _ = s.contains(2)\n}\n",
    )
    .unwrap();
}

#[test]
fn map_literal_type_checks_and_infers() {
    check_with_prelude(
        "import std.collections\nfun main() {\n    let m = {\"a\": 1, \"b\": 2}\n    let _ = m.get(\"a\")\n}\n",
    )
    .unwrap();
}

#[test]
fn map_literal_infers_bool_values() {
    check_with_prelude(
        "import std.collections\nfun main() {\n    let m = {\"x\": true}\n    let _ = m.has(\"x\")\n}\n",
    )
    .unwrap();
}

#[test]
fn empty_map_literal_type_checks_with_annotation() {
    check_with_prelude(
        "import std.collections\nfun main() {\n    let m: dict[str, int] = {}\n    let _ = m.len()\n}\n",
    )
    .unwrap();
}

#[test]
fn set_literal_needs_no_import() {
    check_with_prelude("fun main() {\n    let s = {1, 2}\n}\n").unwrap();
}

#[test]
fn dict_literal_needs_no_import() {
    check_with_prelude("fun main() {\n    let m = {\"a\": 1}\n}\n").unwrap();
}

#[test]
fn reassigning_a_let_binding_is_rejected() {
    let err = check("fun f() {\n    let x = 1\n    x = 2\n}\n").unwrap_err();
    assert!(matches!(err, RavenError::Type(_, _, _)), "got: {}", err);
}

#[test]
fn private_class_takes_named_arguments() {
    check(
        "class _Pair { a: int, b: int }
fun f() -> int {
    let p = _Pair(a: 1, b: 2)
    return p.b
}
",
    )
    .unwrap();
}

#[test]
fn reassigning_a_module_let_is_rejected() {
    let err = check(
        "let hits = 0
fun f() {
    hits = hits + 1
}
",
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("declared with `let`"),
        "got: {}",
        err
    );
}

#[test]
fn reassigning_a_module_var_is_allowed() {
    check(
        "var hits = 0
fun f() {
    hits = hits + 1
}
",
    )
    .unwrap();
}

#[test]
fn type_name_accepts_scalar_and_struct() {
    check(
        r#"
        class Point { x: int, y: int }
        fun a() -> str = type_name[int]()
        fun b() -> str = type_name[Point]()
    "#,
    )
    .unwrap();
}

#[test]
fn field_names_yields_list_of_string() {
    check(
        r#"
        class Point { x: int, y: int }
        fun a() -> list[str] = field_names[Point]()
    "#,
    )
    .unwrap();
}

#[test]
fn type_name_resolves_a_generic_parameter() {
    check("fun describe[T]() -> str = type_name[T]()\n").unwrap();
}

#[test]
fn field_types_yields_list_of_string() {
    check(
        r#"
        class Point { x: int, y: int }
        fun a() -> list[str] = field_types[Point]()
    "#,
    )
    .unwrap();
}

#[test]
fn field_names_on_scalar_is_rejected() {
    let err = check("fun a() -> list[str] = field_names[int]()\n").unwrap_err();
    match err {
        RavenError::Type(b, _, _) => assert!(matches!(*b, TypeError::Custom(_))),
        other => panic!("expected a type error, got {:?}", other),
    }
}

#[test]
fn field_types_on_scalar_is_rejected() {
    let err = check("fun a() -> list[str] = field_types[int]()\n").unwrap_err();
    match err {
        RavenError::Type(b, _, _) => assert!(matches!(*b, TypeError::Custom(_))),
        other => panic!("expected a type error, got {:?}", other),
    }
}

#[test]
fn variant_names_yields_list_of_string() {
    check(
        "enum Shape {\n    Circle(r: int)\n    Dot\n}\n\
         fun a() -> list[str] = variant_names[Shape]()\n",
    )
    .unwrap();
}

#[test]
fn variant_field_types_yields_nested_list() {
    check(
        "enum Shape {\n    Circle(r: int)\n    Dot\n}\n\
         fun a() -> list[list[str]] = variant_field_types[Shape]()\n",
    )
    .unwrap();
}

#[test]
fn variant_names_on_struct_is_rejected() {
    let err = check(
        r#"
        class P { x: int }
        fun a() -> list[str] = variant_names[P]()
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
            let p = __ptr_alloc[CInt](4)
            __ptr_store[CInt](p, 10)
            let q = __ptr_offset[CInt](p, 1)
            let v = __ptr_load[CInt](q)
            __ptr_free[CInt](p)
            return v
        }
        fun nulls() -> bool {
            let n = __ptr_null[CInt]()
            return __ptr_is_null[CInt](n)
        }
    "#,
    )
    .unwrap();
}

#[test]
fn ptr_load_yields_the_pointee_type() {
    // __ptr_load<CLong> returns a CLong, which unifies with the CLong
    // return; a mismatched return type would be a tycheck error.
    check("fun f(p: CPtr[CLong]) -> CLong { return __ptr_load[CLong](p) }\n").unwrap();
}

#[test]
fn ptr_builtins_resolve_a_generic_parameter() {
    // The pointee may be a generic parameter, so the std/ffi wrappers can
    // forward it; grounded per monomorphization in MIR.
    check("fun deref[T](p: CPtr[T]) -> T { return __ptr_load[T](p) }\n").unwrap();
}

#[test]
fn ptr_builtin_rejects_a_non_pointer_pointee() {
    let err =
        check("fun f() -> bool { return __ptr_is_null[str](__ptr_null[str]()) }\n").unwrap_err();
    match err {
        RavenError::Type(b, _, _) => assert!(matches!(*b, TypeError::Custom(_))),
        other => panic!("expected a type error, got {:?}", other),
    }
}

#[test]
fn reports_an_error_in_each_sibling_function() {
    // Two unrelated functions each have a return type mismatch; both are
    // reported in one compile, not just the first.
    let errs = check_all("fun f() -> int = \"x\"\nfun g() -> bool = 5\n");
    assert_eq!(errs.len(), 2, "got: {:?}", errs);
}

#[test]
fn reports_an_error_for_each_failed_statement() {
    // Two `let` bindings with annotation mismatches both report; the later
    // use does not cascade because each binding adopts its annotated `Int`.
    let errs = check_all(
        "fun h() -> int {\n    let x: int = \"a\"\n    let y: int = true\n    return x + y\n}\n",
    );
    assert_eq!(errs.len(), 2, "got: {:?}", errs);
}

#[test]
fn a_failed_unannotated_let_does_not_cascade() {
    // `let x = <type error>` records one error and binds `x` to `Error`, so
    // the later `x + z` use adds no diagnostic.
    let errs = check_all(
        "fun u() -> int {\n    let x = 1 + \"oops\"\n    let z = 5\n    return x + z\n}\n",
    );
    assert_eq!(errs.len(), 1, "got: {:?}", errs);
}

#[test]
fn a_clean_program_reports_no_errors() {
    assert!(check_all("fun f() -> int = 1 + 2\n").is_empty());
}

#[test]
fn duplicate_diagnostics_are_deduplicated() {
    // The same message at the same span is collapsed to one entry.
    let errs = check_all("fun f() -> int = \"x\"\n");
    assert_eq!(errs.len(), 1, "got: {:?}", errs);
}

#[test]
fn return_inside_a_for_loop_does_not_end_the_function() {
    let err = check("fun f(xs: list[int]) -> int {\n    for x in xs {\n        return x\n    }\n    let z = 1\n}\n")
        .unwrap_err();
    assert!(matches!(err, RavenError::Type(..)), "got: {}", err);
}

#[test]
fn panic_on_the_right_of_or_does_not_end_the_function() {
    let err = check("fun f(c: bool) -> int {\n    let ok = c or panic(\"no\")\n    let z = 2\n}\n")
        .unwrap_err();
    assert!(matches!(err, RavenError::Type(..)), "got: {}", err);
}
