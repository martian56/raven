# Generics and Trait Bounds Spec

## Goal

Lift the monomorphic-core type checker to a Hindley-Milner style checker
with let-polymorphism. Users may declare type parameters on functions,
structs, enums, traits, and `impl` blocks, optionally constrained by
trait bounds. The checker infers types for local bindings and at generic
use sites, then verifies bounds and arity once unification settles.

This spec extends `docs/v2/specs/tycheck.md`. Read that document first.

## Pipeline position

The pipeline is unchanged. Generics live entirely inside the type
checker. The lexer, parser, and resolver already accept the syntax: the
parser produces `Vec<GenericParam>` on every declaration that can carry
one (see `src/ast/decl.rs`), and the resolver records uses of generic
parameter names with the `Binding::GenericParam { owner, name }` variant
(see `src/resolve/bindings.rs`).

## What is supported

* Generic function declarations:
  `fun map<T, U>(xs: List<T>, f: fun(T) -> U) -> List<U> { ... }`.
* Generic struct and enum declarations:
  `struct Box<T> { value: T }`, `enum Either<L, R> { Left(L), Right(R) }`.
* Generic trait declarations:
  `trait Container<T> { fun get(self, i: Int) -> T }`.
* Generic `impl` blocks, both inherent and trait:
  `impl<T> Container<T> for List<T> { ... }`.
* Inline trait bounds, single and multi:
  `<T: Display>`, `<T: Display + Clone>`.
* Bounds on `impl` blocks. The bounds participate in method resolution.
* Inference of local binding types. A `let` without a type annotation
  takes the type of its initializer, including instantiated type
  parameters.
* Let-polymorphism. A generic function instantiates fresh inference
  variables at each call site, so callers see fully concrete signatures.
* Method resolution with bounds. Method lookup considers generic impls
  and substitutes the receiver's instantiation; bounds on the impl's
  parameters become constraints on the receiver's type arguments.

## What is deferred

* Trait inheritance (`trait Bar: Foo`). The parser does not accept it
  today.
* `where` clauses. The parser does not accept them today; they remain
  future work and the spec is open to either inline or `where` form
  whenever the parser learns the syntax.
* Higher-kinded types.
* Default type parameters.
* Const generics.
* Associated types and projections (`<T as Iterator>::Item`).
* Full trait coherence enforcement (orphan rules) beyond the existing
  duplicate-impl check.
* `dyn Trait`. Tracked separately in issue #66.
* Variance.

## The `Ty` representation

`Ty` (in `src/tycheck/ty.rs`) is extended with two new variants:

```rust
pub enum Ty {
    // existing variants ...
    Struct { id: DeclId, name: String, args: Vec<Ty> },
    Enum   { id: DeclId, name: String, args: Vec<Ty> },
    // ...
    /// A declared generic parameter, identified by the declaration that
    /// introduces it (its owner span) plus an ordinal index.
    Param(ParamId),
    /// An inference variable solved by the inference table.
    Var(InferVarId),
}
```

`ParamId` carries the owner span (the declaration that introduced the
parameter list) and the parameter's index inside that list, so the same
name on two different declarations does not collide.

`Struct` and `Enum` now carry a `Vec<Ty>` of type arguments. The
existing monomorphic uses pass an empty vector, which prints the same
as before. The built-in synthetic types `Option`, `Result`, and `List`
keep their dedicated variants because their special-case methods are
implemented directly against those shapes; absorbing them into the
general path is left as a follow-up cleanup.

The `Display` implementation prints `Var` as `?N` (where `N` is the
variable id) before resolution and as the resolved type after. It prints
`Param` as the original name from the declaration. Struct and enum
types with non-empty `args` print `Name<A, B>`.

## Inference

A new module `src/tycheck/infer.rs` implements a small union-find
inference table:

```rust
pub struct InferCtx {
    parents: Vec<InferVarId>,
    rank:    Vec<u32>,
    solved:  Vec<Option<Ty>>,
    bounds:  Vec<Vec<String>>,
    spans:   Vec<Span>,
}

impl InferCtx {
    pub fn fresh(&mut self, span: Span) -> InferVarId { ... }
    pub fn add_bound(&mut self, v: InferVarId, trait_name: String) { ... }
    pub fn unify(&mut self, a: &Ty, b: &Ty, span: &Span) -> Result<(), RavenError>;
    pub fn resolve(&self, ty: &Ty) -> Ty;
    pub fn finalize(&self, ty: &Ty) -> Result<Ty, RavenError>;
}
```

`unify` walks both types structurally. When either side is a `Var`, it
points the variable at the other side after applying the occurs check.
`Param(p)` unifies only with itself. The `Error` placeholder unifies
with anything (preserving the existing cascade-suppression behavior).
`resolve` returns the current best approximation of `ty` (variables
substituted, recursive types walked). `finalize` is `resolve` plus a
post-check that no variable remains; if one does, the span of its
introduction is reported as `CannotInferType`.

Bounds are stored per inference variable. When an inference variable
unifies with a concrete type, the pending bounds are immediately
verified against the concrete type; if a bound is not satisfied,
`BoundNotSatisfied` is raised. Bounds on `Param` are checked at the
call site by walking the substitution map; if a substituted type does
not satisfy the parameter's bounds, the same error is raised.

## Generalization

Generalization happens at function declaration boundaries. The
collection pass reads each `GenericParam` list and records each
parameter as a `Param(ParamId)` in the signature. Inside the body, the
checker treats `Param` as an opaque type that unifies only with itself.

At every call to a generic function, the call site creates a fresh
inference variable for each declared parameter and substitutes through
the signature. The variables collect the bound constraints from the
declaration. If the call site is fully concrete, the variables resolve
right away; otherwise they remain pending and are resolved by later
unifications.

A free inference variable that escapes into a top-level value (e.g. a
`const` with `let xs = []`) is an error: the user must annotate. The
checker reports `CannotInferType` at the span of the originating
expression.

## Collection pass changes

The collection pass (`collect.rs`) adds a generic-parameter scope around
each declaration's signature. The scope is a small map from parameter
name to `ParamId`. When `resolve_ty` encounters a `TypePath` whose head
is bound to `Binding::GenericParam { owner, name }` in the resolver, it
returns `Ty::Param(ParamId::new(owner, index))`.

Signatures record their parameter list and trait bounds:

```rust
pub struct GenericParamSig {
    pub name: String,
    pub bounds: Vec<String>,
    pub span: Span,
}

pub struct FnSig {
    pub name: String,
    pub generics: Vec<GenericParamSig>,
    pub params: Vec<Ty>,
    pub ret: Ty,
    pub span: Span,
    pub has_self: bool,
}
// StructSig, EnumSig, TraitSig, ImplSig grow the same field.
```

Bound names are resolved against the resolver's binding for the bound
path's head: the binding must be `Binding::Trait(_)`. Anything else is
reported as `UnknownType` at the bound span. Bound storage is by name
because the runtime does not yet need decl-id stability for trait
bounds; the body pass uses the trait name for lookup against the impl
table.

## Body pass changes

Each `Checker` carries an `InferCtx`. The flow is unchanged from the
monomorphic core, with three additions:

1. **Identifier with generic args.** `name<T1, T2>` reads the
   declaration's signature, allocates fresh inference variables for
   each generic parameter, attaches their bounds, unifies the named
   variables against the explicit type arguments (if any were
   supplied), and substitutes through the signature.
2. **Calls of generic functions.** Like (1) but at call sites without
   explicit type arguments; the variables are unified with the
   argument types.
3. **Method calls on generic types.** Method lookup walks every impl
   that could match: each impl's `self_ty` is substituted with fresh
   inference variables for its own generic parameters, unified against
   the receiver type, and its method signatures used after the same
   substitution. Bounds on the impl's parameters are added as
   constraints on the substituted variables.

`expect_assignable` is replaced by a call to `InferCtx::unify`, which
treats assignability as ordered unification with sub-type relations
limited to the `Error` placeholder (preserving the existing cascade
behavior).

After body checking, the type map is walked and every recorded `Ty` is
finalized through `InferCtx`. The finalization step is what surfaces
`CannotInferType` errors.

## Method resolution with bounds

Method dispatch becomes a candidate gathering plus filtering step:

1. Walk every impl in `env.impls`.
2. For each impl, allocate fresh inference variables for its
   generic parameters.
3. Substitute through the impl's `self_ty` and method signatures.
4. Try to unify the substituted `self_ty` against the receiver. If
   unification fails, the impl is not a candidate.
5. For each successful match, look up the method by name. The matched
   method's parameter and return types are returned to the caller.
6. Bounds on the impl's parameters are added as constraints; they are
   checked once unification finishes for the receiver and arguments.

Inherent impls are preferred over trait impls when both match. When
two or more candidates match at the same priority, the method is
`AmbiguousMethod`.

`Option`, `Result`, `List`, and `String` retain their existing
`lookup_method` fast path. The general path applies when the receiver
is a `Struct` or `Enum` (possibly with arguments) or a `SelfTy`.

## Errors

`TypeError` (in `src/error.rs`) grows:

* `CannotInferType` (alias `UnsolvedInferenceVariable`). Raised when an
  inference variable could not be solved by the end of body checking.
* `OccursCheck { ty, var }`. Raised when unifying a variable with a
  type that references the variable.
* `BoundNotSatisfied { ty, trait_name }`. Raised when a type does not
  satisfy a required trait bound.
* `GenericArityMismatch { decl, expected, actual }`. Raised when an
  identifier supplies the wrong number of explicit generic arguments.
* `OverlappingImpls { ty, trait_name, candidates }`. Raised when two or
  more impls match the same `(type, trait)` pair.

`GenericsNotYetSupported` is removed. The handful of tests that asserted
it are updated to assert success or to use a different error.

## Tests

Inline tests in `src/tycheck/tests.rs`:

* `generic_function_identity_inferred`: `fun id<T>(x: T) -> T = x` plus
  `id(1)` infers `Int`.
* `generic_function_explicit_type_arg`: `id<Bool>(true)` accepts the
  explicit argument.
* `generic_function_arity_mismatch`: `id<Int, Int>(1)` is rejected.
* `generic_struct_field_types_substitute`: `struct Box<T> { value: T }`
  plus `let b = Box { value: 1 }; b.value` infers `Int`.
* `generic_impl_method_resolves`: `impl<T> Box<T> { fun get(self) -> T = self.value }`
  plus call `b.get()` infers `Int`.
* `bound_not_satisfied_reports_trait`: a bound `T: Display` violated by
  a call site with a non-`Display` type.
* `cannot_infer_type_when_ambiguous`: an unannotated empty container in
  a position with no context still surfaces `CannotInferType`.
* `occurs_check_rejects_self_reference`: synthetic test poking
  `InferCtx::unify` directly.

Golden snapshot corpus in `tests/tycheck_corpus/`:

* `generic_function.rv` and `.rv.types`. Simple identity plus use.
* `generic_struct.rv` and `.rv.types`. `Box<T>` with a field read.
* `generic_trait.rv` and `.rv.types`. A small `Display`-like trait with
  two impls.
* `bounded_generic.rv` and `.rv.types`. A function constrained on a
  trait, called against a type that implements the trait.
* `multi_bound.rv` and `.rv.types`. `<T: Display + Clone>`.
* `inference.rv` and `.rv.types`. Local bindings whose types are fully
  inferred.

The golden harness understands `.rv.expect_err` companion files so a
single source can assert a specific error variant by name on the
`expect-err` line. For this initial drop only `occurs_check.rv` uses
that mechanism if it lands; otherwise the test is inline.
