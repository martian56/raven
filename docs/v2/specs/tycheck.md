# Type Checker Spec

## Goal

Statically validate a resolved Raven file. Given a `ResolvedFile` (the output of `src/resolve`), the type checker assigns a concrete type to every expression, verifies operator and call sites, checks struct, trait, impl, and enum declarations against their uses, and confirms that every `match` is exhaustive. The output is a `TypedFile` that pairs the resolved AST with a `TypeMap` (expression span to inferred type) and a `TypeEnv` (declaration to signature).

The type checker is total: every malformed program produces a
`RavenError::Type(TypeError, Span, Option<String>)` anchored at the offending
source range. It supports local inference, user-declared generic functions and
types, trait bounds, generic impls, built-in collection types, and `dyn Trait`.
See `generics.md` and `dyn-trait.md` for the extensions to the foundational
rules in this document.

## Pipeline position

```
Source -> Lexer -> Parser -> Resolver -> TypeChecker -> (HIR -> MIR -> codegen)
```

The type checker consumes nodes from `src/ast` plus the resolver's `ResolutionMap`. It does not load files or perform any I/O. The body pass recovers at item and statement boundaries so one compile reports many independent errors (see "Error recovery" below).

## The `Ty` representation

The AST carries a textual `Type` (`src/ast/ty.rs`) that mirrors what the user wrote. The type checker uses its own internal representation, `Ty`, for several reasons:

* Decoupling: the textual form may contain unresolved paths or sugar (`T?` for `Option<T>`). `Ty` stores the resolved form.
* Normalization: `Ty` is cheap to compare and clone, while `Type` carries spans and unresolved nested paths.
* Generics: inference variables and substitutions stay internal to the checker
  instead of being threaded through the source AST.

```rust
pub enum Ty {
    Unit, Bool, Int, Float, Char, Str,
    Struct { id: DeclId, name: String, args: Vec<Ty> },
    Enum   { id: DeclId, name: String, args: Vec<Ty> },
    Option(Box<Ty>),
    Result(Box<Ty>, Box<Ty>),
    List(Box<Ty>),
    Function { params: Vec<Ty>, ret: Box<Ty> },
    Dyn { name: String, methods: Vec<String> },
    SelfTy(Box<Ty>),
    Any,
    Ffi(FfiTy),
    Param(ParamId),
    Var(InferVarId),
    Error,
}
```

Inference variables are created for generic use sites and unannotated values.
Lambda parameters may infer from a contextual function type; otherwise they
need annotations.

## Algorithm

Two passes per file.

### Pass 1: declared type collection

Walk every top level `Decl` once and populate a `TypeEnv`:

* Each `Function` records its parameter and return types resolved into `Ty`.
* Each `Struct` records its field name to `Ty` mapping.
* Each `Enum` records its variants and their payload types.
* Each `Trait` records its member signatures.
* Each `Impl` block is keyed by the implementing type plus the optional trait it implements, and its members are stored as a method table.
* Each `Const` and module level `Let` records its annotated type (the initializer is checked in pass 2).
* Built in generic types (`Option`, `Result`, `List`) are pre populated.

Generic parameter lists are collected with their trait bounds. Type paths
referenced in field types, return types, and so on are resolved using the
resolver's binding for the head segment, then mapped to `Ty`; use sites create
fresh inference variables and verify bounds after unification.

### Pass 2: body checking

Walk every body expression in declaration order. For each expression, synthesize a `Ty` and record it in the `TypeMap`. Statement walking is mostly threading: `let name = e` checks `e`, binds `name -> Ty` in the local typing scope, and stores the type for the introduced span.

Method calls are looked up by the receiver's resolved type:

1. Search user declared inherent impls of the receiver type for a method with that name. The receiver type may be a user struct or enum, or a built in type (`Int`, `Float`, `Bool`, `Char`, `String`, `List<T>`, `Set<T>`, `Map<K, V>`, `Option<T>`, `Result<T, E>`).
2. If none, search trait impls of the receiver type for a method with that name.
3. If still none, fall back to the hard coded built in fast path methods (`Option`/`Result`/`List`/`String`).
4. Multiple impl matches raise `AmbiguousMethod`; zero matches raise `UndefinedMethod`.

For struct literals, every field declared on the struct must be initialized exactly once; unknown fields raise `UndefinedField`; missing fields raise a `TypeError::Custom` describing what is missing.

For `match`, the scrutinee's type drives both pattern checking and exhaustiveness. The arm bodies are unified to a common type using `unify_branches`, which accepts equal types and otherwise reports `TypeMismatch`.

## Type rules

Operators (selection):

| Operator           | Operand types    | Result |
|--------------------|------------------|--------|
| `+ - * / %`        | `Int, Int`       | `Int`  |
| `+ - * /`          | `Float, Float`   | `Float`|
| `==` `!=`          | `T, T` (eqable)  | `Bool` |
| `< <= > >=`        | `T, T` (ordable) | `Bool` |
| `&& \|\|`          | `Bool, Bool`     | `Bool` |
| `& \| ^ << >>`     | `Int, Int`       | `Int`  |
| unary `-`          | `Int` or `Float` | same   |
| unary `!`          | `Bool`           | `Bool` |
| unary `&`          | any `T`          | `T` (reference semantics deferred to lowering) |

Mixed numeric operands are rejected; there is no implicit promotion.

`==` and `!=` accept two operands of the same type and yield `Bool`. The comparison semantics depend on the operand type, and the back-end (not the type checker) selects them:

* `Int`, `Float`, `Bool`, and `Char` compare by value.
* `String` compares by content: two strings are equal when they hold the same bytes, regardless of whether they are the same heap object. `!=` is the negation. See `docs/v2/specs/codegen.md`.
* User structs and enums currently compare by object identity (the two operands are equal only when they are the same heap object). Structural `==` for user types, and routing `==` through an `Eq` trait, are out of scope here; user code that wants value comparison defines and calls a method such as `equals`.

Control flow:

* `if c { a } else { b }` requires `c: Bool` and unifies `a` and `b`. A bare `if` without `else` has type `Unit`; both branches must therefore be `Unit`.
* `match s { arms... }` requires every arm body to unify, and yields the unified type.
* `while`, `for`, and `loop` have type `Unit`. `break expr` inside a `loop` contributes its operand type to the loop's overall value; in this release loops are typed as `Unit` regardless and a `break expr` is permitted but its value is discarded.

Calls:

* `f(args)` requires `f` to be a callable (function item, lambda value, or function typed local). Arity must match exactly; each argument unifies with its declared parameter type. `WrongArity` is raised on mismatch.
* `obj.method(args)` follows the method resolution rules above.
* `obj.field` looks `field` up on `obj`'s struct type; missing raises `UndefinedField`.

## Method resolution

Method dispatch is statically resolved. When the user writes `r.m(...)` and `r: T`:

1. Look up inherent methods of `T` (impl blocks with no `for` clause).
2. Look up methods of any trait `Tr` such that `impl Tr for T` exists.
3. If no user `impl` method matched, fall back to the hard coded built in fast path methods.
4. If exactly one candidate has the name `m`, use it.
5. If more than one impl matches, raise `AmbiguousMethod` listing the candidates. The user disambiguates by calling `Tr::m(r, ...)` (qualified call syntax). Qualified calls are accepted as a regular function call.

A method call on a value of `Self` type inside an `impl` block resolves through the implementing type. Each impl's generic parameters get fresh inference variables and the substituted `self_ty` is unified against the receiver, so a generic impl such as `impl<T> List<T>` matches a concrete `List<Int>` receiver and the method's `T` binds to `Int`.

### Enum variant construction

A user enum variant is constructed with a qualified name: `EnumName.Variant` for a unit variant and `EnumName.Variant(args)` for a payload variant. When the receiver of a field access or call is an `Ident` bound to an enum and the name is one of its variants, the checker treats it as construction rather than a field access or an associated function call. A unit variant types as the enum directly; a payload variant types as a constructor function whose parameters are the payload types and whose result is the enum, so the surrounding call applies and checks the arguments. For a generic enum the declared generic parameters become fresh inference variables, solved from the argument types and the surrounding expected type, the same way generic struct fields are substituted.

Bare-name construction (`Red`, `Circle(2.0)`) is not supported yet and is a follow-up; it needs expected-type disambiguation. Match patterns are unchanged and keep using bare variant names. Struct-shaped variants (`Variant { field: ... }`) are not yet constructible.

### Associated functions

When the receiver of `r.m(...)` is a bare type reference (a struct or enum binding, or a built-in type name) rather than a value, the call is an associated function call `Type.func(args)`. The named function on that type must have no `self`. The implementing type fixes the impl's generic parameters: explicit arguments (`Set<Int>.new()`) bind them directly, otherwise they are inference variables solved from later use. Arguments are checked against the function's parameters (there is no `self` to drop), and the result is its declared return type with the impl's parameters substituted. See `docs/v2/specs/associated-functions.md`.

### `impl` on built in types

An `impl` block may target a built in type, not just a user struct or enum:

```
impl Int {
    fun doubled(self) -> Int = self * 2
}
```

The implementing type is resolved the same way any type path is resolved, so `impl Int`, `impl String`, `impl<T> List<T>`, `impl Bool`, `impl Char`, `impl Float`, `impl<T> Set<T>`, and `impl<K, V> Map<K, V>` all collect into the same method table the type checker uses for user structs. Inside the body, `self` is the built in receiver and `Self` is the built in type. Resolution then matches the receiver against the impl's `self_ty` exactly as for a user type, so `21.doubled()` resolves to the `Int` impl method.

`Set` and `Map` use the same generic impl collection, resolution,
monomorphization, and lowering paths as other generic types.

### Precedence vs hard coded inherent methods

The hard coded built in fast path methods (`Option`/`Result`/`List`/`String`) are only consulted when no user `impl` method matched. A user `impl` method therefore always wins over a hard coded method of the same name. This keeps the checked signature in step with code generation: a method call lowers to the per type symbol `<RecvType>$<method>` (see below), and a user `impl` method defines exactly that symbol, so the type the checker assigns is the type of the method that actually runs.

The hard coded tables remain available for receivers with no user impl: `Option<T>` and `Result<T, E>` expose `is_some`, `is_none`, `unwrap`, `unwrap_or`, `is_ok`, `is_err`; `List<T>` exposes `len`, `push`, `pop`, `is_empty`, `get`; `String` exposes `len`, `is_empty`.

### Symbol naming

A statically dispatched method lowers to the symbol `<TypeMangle>$<method>`, where `<TypeMangle>` is the receiver type's identifier safe mangling (`Int`, `Str`, `Bool`, `Char`, `Float`, `List_Int`, `Option_Int`, a struct or enum name, and so on). So `impl Int { fun doubled }` defines `Int$doubled` and `impl String { fun shout }` defines `Str$shout`. The definition site (HIR lowering of the impl) and the call site (MIR lowering of the method call) both compute this name from the same receiver type, so they always agree, and methods of the same name on different types never collide. Generic built in impls monomorphize per concrete element type through the same path that specializes generic user types: `impl<T> List<T>` produces one method instance per element type the program uses.

### Methods in bundled stdlib modules

A bundled stdlib module (loaded by `expand_with_stdlib` when a program writes `import std/<module>`) may declare `impl` blocks on built in types. Unlike the module's free functions, which are renamed to `std.<module>.<name>` to avoid colliding with user names, an `impl` method keeps its name: it is dispatched by the receiver's type through the `<RecvType>$<method>` symbol, not by a free function name, so it never collides with user code and needs no namespacing. The expander still rewrites sibling free function calls inside an impl method body to their namespaced names (a stdlib `impl String` method calling the module's own `to_upper` reaches `std.string.to_upper`). A user calls the method by writing `value.method()` after importing the module; no name needs to be brought into scope with a selector, because resolution is by receiver type.

## Exhaustiveness check

The exhaustiveness analysis is a simple variant set check:

* If the scrutinee is `Bool`, the arm set must contain both `true` and `false`, or a wildcard.
* If the scrutinee is an `Enum` (including `Option`, `Result`), every variant must be covered, or a wildcard must be present. Variant patterns may bind inner names, but the variant identity is what counts for coverage.
* For all other scrutinee types, a wildcard arm is required (no decision tree analysis on integers in this release).
* A subsequent arm whose pattern is fully shadowed by an earlier arm raises `RedundantPattern`.

The check runs after each arm body has been typed so that pattern binding types are known.

## Built in generic types

Five built-in generic shapes receive dedicated literal or constructor handling:

* `Option<T>`. Variants: `None`, `Some(T)`. Constructed by writing `None` or `Some(value)` at a use site whose context type fixes `T`. The resolver already accepts `Option` and `Result` as built in type names; the type checker recognizes the variant identifiers when the contextual type is known.
* `Result<T, E>`. Variants: `Ok(T)`, `Err(E)`.
* `List<T>`. The array literal `[a, b, c]` infers `T` as the unified element type. Empty list literals require a context type and are otherwise rejected with `TypeMismatch`.
* `Set<T>` and `Map<K, V>` (bundled `std/collections` types). The set literal `{a, b}` types as `Set<T>` with `T` unified across the elements; the map literal `["k": v]` (and the empty `[:]`) types as `Map<K, V>` with `K` and `V` unified across the keys and values. The literal binds to the imported `Set`/`Map` declaration, so it requires `import std/collections` in scope and otherwise reports a `TypeError`. `T` and `K` carry the declarations' `Eq` bound. HIR lowers the literals to the `Set.new()`/`Map.new()` constructors plus an `add`/`set` per element or pair.

These built-in literal forms receive dedicated contextual inference. Their
declared methods, generic parameters, trait bounds, and implementations then
participate in the normal generic method and trait-resolution paths.

## The `?` operator

`expr?` requires an `Option<T>` or `Result<T, E>`. In a function returning the
matching container, it unwraps `Some`/`Ok` and returns `None`/`Err` early. The
type checker verifies the container and residual types; HIR lowers the
operation to the corresponding match and early return.

## Errors

`TypeError` lives next to the other variants in `src/error.rs`:

* `TypeMismatch { expected, actual }`.
* `UndefinedField { struct_name, field }`.
* `UndefinedMethod { receiver_ty, method }`.
* `AmbiguousMethod { receiver_ty, method, candidates }`.
* `WrongArity { func, expected, actual }`.
* `NonExhaustiveMatch { missing }`.
* `RedundantPattern`.
* `UnknownType(name)`.
* `CannotInferType` and `OccursCheck { var, ty }`.
* `BoundNotSatisfied { ty, trait_name }`.
* `GenericArityMismatch { decl, expected, actual }`.
* `OverlappingImpls { ty, trait_name, candidates }`.
* `NotCallable(actual)`.
* `Custom(String)` for one-off shapes such as missing struct fields.

All variants reach the user via `RavenError::Type(error, span, hint)`. The renderer in `RavenError::display` is extended to handle the new arm.

## Error recovery

The body pass collects multiple errors per compile instead of stopping at the first one. A `Checker` holds an `errors: Vec<RavenError>` sink, and checking recovers at two boundaries:

* **Item boundary.** `check_bodies` checks each top-level item (function, impl method, trait default body, `const`, `let`) independently and accumulates their errors, so a mistake in one function does not hide errors in the next.
* **Statement boundary.** `check_block` checks each statement through `check_stmt`, which never bubbles: a failed sub-check records its error and continues. Within a single statement or expression, errors still fail fast up to the enclosing statement, which is the recovery point.

Recovery uses the `Ty::Error` placeholder (which unifies with anything) to suppress cascades. A statement that introduces a binding always introduces it: a `let` with a type annotation binds the annotated type even when its initializer fails; an unannotated `let` whose initializer fails binds `Ty::Error`. Later references therefore type-check against a known (or error) type rather than spraying "undefined" follow-on errors. Identical diagnostics (same span and message) are de-duplicated before rendering.

The collection pass (Pass 1) stays fail-fast: a malformed signature stops it, because later items depend on the collected signatures. `check_file_all` returns `Result<TypedFile, Vec<RavenError>>`; the driver renders each error with the #283 renderer, separated by a blank line. `check_file` remains a thin wrapper returning only the first error, for callers (tests, golden harnesses) that surface one.

## Tests

* Unit tests inline at `src/tycheck/tests.rs`: cover primitives, arithmetic,
  comparisons, struct literal and field access, method dispatch, enum variant
  construction, exhaustive matches, generics and bounds, trait objects,
  propagation, redundant patterns, and error recovery.
* Golden snapshot tests at `tests/tycheck_golden.rs` over a corpus at `tests/tycheck_corpus/`. Each `.rv` source has a committed `.rv.types` baseline produced by dumping every expression site and its inferred type. Refresh with `RAVEN_UPDATE_TYCHECK_GOLDEN=1`.
