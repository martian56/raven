# reflection

Compile-time type reflection exposes a small amount of type information to
user code, resolved while the program is compiled. This is the first slice
of the reflection work tracked under issue #216. It ships these builtins:

```rust
type_name<T>() -> String
field_names<T>() -> List<String>
field_types<T>() -> List<String>
variant_names<T>() -> List<String>
variant_field_types<T>() -> List<List<String>>
```

Both take a single type argument and no value arguments. The type argument
must be statically known: a concrete type (`Int`, `Point`, `Pair<Int,
String>`) or a generic parameter `T` that is bound to a concrete type at
each monomorphization of the enclosing generic function. There is no
runtime reflection over a value of unknown type in this slice; that needs
an `Any` type and is a separate follow-up (see below).

## type_name

`type_name<T>()` evaluates to the rendered name of the concrete type `T`,
as a `String`.

```rust
type_name<Int>()                  // "Int"
type_name<String>()               // "String"
type_name<Point>()                // "Point"
type_name<Pair<Int, String>>()    // "Pair<Int, String>"
type_name<List<Int>>()            // "List<Int>"
```

The rendering is the language's own type spelling, the same form the type
checker prints in diagnostics: a scalar by its keyword name (`Int`,
`Float`, `Bool`, `Char`, `String`, `()`), a struct or enum by its name with
its concrete type arguments in angle brackets (`Pair<Int, String>`), and
the built-in generics by their spelling (`List<Int>`, `Option<Int>`,
`Result<Int, String>`). `String` renders as `String` (not the internal
`Str`).

Inside a generic function, `type_name<T>()` resolves to the concrete type
bound to `T` at each instantiation:

```rust
fun describe<T>() -> String {
    return type_name<T>()
}

describe<Int>()      // "Int"
describe<Point>()    // "Point"
```

This is the load-bearing property: the call is lowered once per
monomorphization with the concrete substitution applied, so two
instantiations of the same generic body produce two different names.

## field_names

`field_names<T>()` evaluates to the field names of the struct type `T`, in
declaration order, as a `List<String>`.

```rust
struct Point { x: Int, y: Int }

field_names<Point>()    // ["x", "y"]
```

`T` must be a struct. Applying `field_names` to a non-struct type (a
scalar, an enum, a built-in generic) is a compile error.

Inside a generic function, `field_names<T>()` resolves to the fields of the
struct bound to `T` at each instantiation, the same per-monomorphization
mechanic as `type_name`.

## field_types

`field_types<T>()` evaluates to the field types of the struct type `T`, in
declaration order, as a `List<String>` of rendered type names. It is the
positional counterpart to `field_names`: index `i` of one lines up with
index `i` of the other, so the two together describe each field.

```rust
struct User { id: Int, name: String, active: Bool }

field_names<User>()    // ["id", "name", "active"]
field_types<User>()    // ["Int", "String", "Bool"]
```

`T` must be a struct, with the same non-struct rejection as `field_names`.
For a generic struct, each field type is rendered at its concrete
instantiation, so a generic field reads as the type it is bound to:

```rust
struct Box<T> { value: T }

field_types<Box<Int>>()       // ["Int"]
field_types<Box<String>>()    // ["String"]
```

A typed descriptor object (a struct the user can read, rather than parallel
string lists) remains a possible future surface; this slice pairs
`field_names` and `field_types` by position.

## variant_names

`variant_names<T>()` evaluates to the variant names of the enum type `T`, in
declaration order, as a `List<String>`. It is the enum counterpart to
`field_names`.

```rust
enum Shape { Circle(radius: Float) Rectangle(width: Float, height: Float) Dot }

variant_names<Shape>()    // ["Circle", "Rectangle", "Dot"]
```

`T` must be an enum. Applying `variant_names` to a non-enum type (a struct, a
scalar, a built-in generic) is a compile error. The built-in `Option` and
`Result` are not user enums and are not accepted in this slice.

## variant_field_types

`variant_field_types<T>()` describes each variant's payload, as a
`List<List<String>>`: one inner list per variant in declaration order,
holding that variant's payload field type names. A unit variant has an empty
inner list, so the inner list's length is the variant's payload arity.

```rust
enum Shape { Circle(radius: Float) Rectangle(width: Float, height: Float) Dot }

variant_names<Shape>()        // ["Circle", "Rectangle", "Dot"]
variant_field_types<Shape>()  // [["Float"], ["Float", "Float"], []]
```

For a generic enum each payload type is rendered at its concrete
instantiation, the same per-monomorphization mechanic as `field_types`:

```rust
enum Tree<T> { Leaf(value: T) Branch(left: T, right: T) Empty }

variant_field_types<Tree<Int>>()    // [["Int"], ["Int", "Int"], []]
```

The unit / tuple / named-field distinction beyond payload types and arity is
not exposed in this slice.

## Model: compile-time, per-monomorphization

Both builtins are resolved entirely at compile time. There is no runtime
type tag lookup, no metadata table, and no value argument. The type must be
statically known.

The mechanic is in MIR lowering. The call carries the resolved type
argument as an HIR type (`Ty::Param(T)` for a generic parameter, or a
concrete `Ty` for an explicit type). Each generic function is lowered once
per concrete instantiation under a substitution map that binds every
generic parameter to a ground type. When the builtin is lowered, the
substitution is applied to its type argument first, so `Ty::Param(T)`
becomes the concrete type for that instantiation:

- `type_name<T>()` lowers to a `String` constant of the grounded type's
  rendered name.
- `field_names<T>()` lowers to a `List<String>` literal built from the
  grounded struct's field names, looked up from the struct declaration the
  lowering pass already holds.
- `field_types<T>()` lowers to a `List<String>` literal of the grounded
  struct's field type names. A per-instantiation substitution built from the
  concrete type arguments grounds each declared field type first, so a
  generic field renders its concrete type.
- `variant_names<T>()` lowers to a `List<String>` literal of the grounded
  enum's variant names.
- `variant_field_types<T>()` lowers to a `List<List<String>>` literal, one
  inner list per variant, built from the grounded enum's variant payload
  types under the same per-instantiation substitution as `field_types`.

Because the lowering runs per instantiation with the concrete substitution,
a generic `type_name<T>()` produces the right name at each call, not one
generic placeholder. A program that does not call either builtin is
unaffected: nothing is emitted and no runtime symbol is referenced.

## Recognition

`type_name`, `field_names`, `field_types`, `variant_names`, and
`variant_field_types` are builtin names, recognized the same way the `print`
builtin and the internal stdlib intrinsics are: the resolver allows the bare
name without a binding, the type checker assigns the result type at the call
site (and validates that `field_names`/`field_types` target a struct and
`variant_names`/`variant_field_types` target an enum), and HIR lowering
rewrites the call into a dedicated reflection node carrying the resolved type
argument. A user can shadow any of these names by binding it in scope (an
import or a local), in which case the binding wins.

## Deferred

This slice is deliberately bounded. The following are follow-ups, each
filed against #216:

- A typed field descriptor object (a readable struct, not parallel string
  lists); `field_types` and `variant_field_types` cover the type information
  for now.
- The unit / tuple / named-field distinction for enum variants, beyond
  payload arity and types.
- Variant introspection over the built-in `Option` and `Result` enums.
