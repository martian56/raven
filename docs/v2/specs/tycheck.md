# Type Checker Spec

## Goal

Statically validate a resolved Raven file. Given a `ResolvedFile` (the output of `src/resolve`), the type checker assigns a concrete type to every expression, verifies operator and call sites, checks struct, trait, impl, and enum declarations against their uses, and confirms that every `match` is exhaustive. The output is a `TypedFile` that pairs the resolved AST with a `TypeMap` (expression span to inferred type) and a `TypeEnv` (declaration to signature).

The type checker is total: every malformed program produces a `RavenError::Type(TypeError, Span, Option<String>)` anchored at the offending source range. It does not infer general generic parameters in this release. `Option<T>`, `Result<T, E>`, and `List<T>` are recognized as built in generic types; any other generic declaration in source is rejected with `GenericsNotYetSupported`, leaving the full mechanism for a follow up issue.

## Pipeline position

```
Source -> Lexer -> Parser -> Resolver -> TypeChecker -> (HIR -> MIR -> codegen)
```

The type checker consumes nodes from `src/ast` plus the resolver's `ResolutionMap`. It does not load files or perform any I/O. The first error halts checking of the current file; multi error recovery is out of scope for this release.

## The `Ty` representation

The AST carries a textual `Type` (`src/ast/ty.rs`) that mirrors what the user wrote. The type checker uses its own internal representation, `Ty`, for several reasons:

* Decoupling: the textual form may contain unresolved paths or sugar (`T?` for `Option<T>`). `Ty` stores the resolved form.
* Hashing: `Ty` is cheap to compare and copy, while `Type` carries spans and nested paths.
* Future generics: a later PR introduces type variables and substitution; threading those through the AST type would be invasive.

```rust
pub enum Ty {
    Unit,
    Bool,
    Int,
    Float,
    Char,
    Str,
    /// A built in or user declared struct. Generic arguments are reserved
    /// for the built in generic types and produce errors elsewhere.
    Struct { id: DeclId, args: Vec<Ty> },
    Enum   { id: DeclId, args: Vec<Ty> },
    /// `Option<T>` and `Result<T, E>` and `List<T>` use synthetic ids so
    /// they do not collide with user declarations.
    Option(Box<Ty>),
    Result(Box<Ty>, Box<Ty>),
    List(Box<Ty>),
    /// `fun(A, B) -> C` function type. Used for lambdas and function items.
    Function { params: Vec<Ty>, ret: Box<Ty> },
    /// `Self` inside an `impl` block, bound to the implementing type.
    SelfTy,
    /// An unknown placeholder. Produced when an earlier error already
    /// reported the problem, so cascades do not spam the user.
    Error,
}
```

There is no `Unknown` inference variable in this release. Lambdas without parameter annotations require a context type or are rejected with `TypeMismatch`.

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

Generic parameter lists are checked for emptiness. A non empty `GenericParam` list on any user item raises `GenericsNotYetSupported`. Type paths referenced in field types, return types, and so on are resolved using the resolver's binding for the head segment, then mapped to `Ty`.

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

### `impl` on built in types

An `impl` block may target a built in type, not just a user struct or enum:

```
impl Int {
    fun doubled(self) -> Int = self * 2
}
```

The implementing type is resolved the same way any type path is resolved, so `impl Int`, `impl String`, `impl<T> List<T>`, `impl Bool`, `impl Char`, `impl Float`, `impl<T> Set<T>`, and `impl<K, V> Map<K, V>` all collect into the same method table the type checker uses for user structs. Inside the body, `self` is the built in receiver and `Self` is the built in type. Resolution then matches the receiver against the impl's `self_ty` exactly as for a user type, so `21.doubled()` resolves to the `Int` impl method.

`Set` and `Map` are recognized as built in type names for the purpose of accepting an `impl` head, but their value types are not yet wired through lowering; an `impl<T> Set<T>` collects and resolves but is not exercised end to end until those types land.

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

Three generic shapes are hard coded:

* `Option<T>`. Variants: `None`, `Some(T)`. Constructed by writing `None` or `Some(value)` at a use site whose context type fixes `T`. The resolver already accepts `Option` and `Result` as built in type names; the type checker recognizes the variant identifiers when the contextual type is known.
* `Result<T, E>`. Variants: `Ok(T)`, `Err(E)`.
* `List<T>`. The array literal `[a, b, c]` infers `T` as the unified element type. Empty list literals require a context type and are otherwise rejected with `TypeMismatch`.

Member access against these types goes through a small inherent method table. None of these types participate in the general generic mechanism: their `T` and `E` are pattern matched directly by the type checker. When the full generic mechanism lands (issue #59), these special cases collapse into the unified path.

## The `?` operator

`expr?` (`ExprKind::Try`) is parsed but not fully typed in this release. Encountering it produces a `TypeError::Custom("? operator is not yet supported, defer to HIR lowering")` so the user gets a clear message rather than a panic. End to end handling lives with the HIR lowering issue (#60).

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
* `GenericsNotYetSupported`. Removed when issue #59 lands.
* `NotCallable(actual)`.
* `Custom(String)` for one off shapes (missing struct fields, unsupported `?`, etc.).

All variants reach the user via `RavenError::Type(error, span, hint)`. The renderer in `RavenError::display` is extended to handle the new arm.

## Out of scope

* User defined generic functions, structs, and traits land in
  `docs/v2/specs/generics.md`. The same document tracks the
  Hindley-Milner inference flavor and trait-bound verification used by
  the v2 checker once that feature ships.
* `dyn Trait` trait objects. Tracked in issue #66.
* C FFI extern signatures beyond syntactic acceptance. Tracked in issue #70.
* `defer` semantics. Tracked in issue #68.
* End to end `?` propagation. Tracked with HIR lowering in issue #60.
* String interpolation expansion (handled by parser side lowering later).
* Cross module member resolution against imported modules. The type checker recognizes the import target but defers member lookups against std modules to the HIR lowering pass.

## Tests

* Unit tests inline at `src/tycheck/tests.rs`: cover primitives, arithmetic, comparisons, struct literal and field access, method dispatch, enum variant construction, `match` exhaustiveness on `Option` and `Result`, `if` branch unification, redundant patterns, unknown fields, ambiguous methods, undefined methods, and rejection of unsupported generics.
* Golden snapshot tests at `tests/tycheck_golden.rs` over a corpus at `tests/tycheck_corpus/`. Each `.rv` source has a committed `.rv.types` baseline produced by dumping every expression site and its inferred type. Refresh with `RAVEN_UPDATE_TYCHECK_GOLDEN=1`.
