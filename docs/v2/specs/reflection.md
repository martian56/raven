# reflection

Compile-time type reflection exposes a small amount of type information to
user code, resolved while the program is compiled. This is the first slice
of the reflection work tracked under issue #216. It ships two builtins:

```raven
type_name<T>() -> String
field_names<T>() -> List<String>
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

```raven
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

```raven
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

```raven
struct Point { x: Int, y: Int }

field_names<Point>()    // ["x", "y"]
```

`T` must be a struct. Applying `field_names` to a non-struct type (a
scalar, an enum, a built-in generic) is a compile error. Field types,
not just names, are a follow-up (a typed field descriptor); this slice
returns names only.

Inside a generic function, `field_names<T>()` resolves to the fields of the
struct bound to `T` at each instantiation, the same per-monomorphization
mechanic as `type_name`.

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

Because the lowering runs per instantiation with the concrete substitution,
a generic `type_name<T>()` produces the right name at each call, not one
generic placeholder. A program that does not call either builtin is
unaffected: nothing is emitted and no runtime symbol is referenced.

## Recognition

`type_name` and `field_names` are builtin names, recognized the same way
the `print` builtin and the internal stdlib intrinsics are: the resolver
allows the bare name without a binding, the type checker assigns the result
type at the call site (and validates that `field_names` targets a struct),
and HIR lowering rewrites the call into a dedicated reflection node carrying
the resolved type argument. A user can shadow either name by binding it in
scope (an import or a local), in which case the binding wins.

## Deferred

This slice is deliberately bounded. The following are follow-ups, each
filed against #216:

- Field types and a typed field descriptor (not just names).
- Enum and variant introspection.
- Runtime reflection over a value of unknown type, via an `Any` type.
- Dynamic field get and set.
