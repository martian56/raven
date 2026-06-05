# Associated Functions Spec

An associated function is a function declared in an `impl` block that does
not take a `self` receiver. It is called on the type, not on a value:
`Type.func(args)`. This is the idiomatic constructor mechanism, so a type
exposes `Type.new()` rather than a free `empty_type()` function.

## Declaration

An associated function is a `fun` inside an `impl` block whose first
parameter is not `self`. The same `impl` block may mix associated
functions and instance methods:

```rust
struct Counter { n: Int }

impl Counter {
    fun new() -> Counter { return Counter { n: 0 } }
    fun with(start: Int) -> Counter { return Counter { n: start } }
    fun bump(self) -> Int { return self.n + 1 }
}
```

`new` and `with` are associated functions; `bump` is an instance method.

A generic implementing type's associated function may return the generic
type:

```rust
impl<T: Eq + Hash> Set<T> {
    fun new() -> Set<T> {
        let buckets: List<List<T>> = []
        return Set { buckets: buckets, count: 0 }
    }
}
```

## Call syntax

```
Type.func(args)
Type<Args>.func(args)
```

The leading `Type` is a type name: a user struct or enum, or a built-in
type (`Int`, `String`, `Array`, ...). Type arguments may be written on the
type for a generic associated function (`Set<Int>.new()`).

```rust
let a = Counter.new()
let b = Counter.with(41)
let s = Set<Int>.new()
```

## Disambiguation

`Type.func(...)` is parsed as a method call whose receiver expression is
the leading name. The resolver and type checker classify it by what the
receiver names:

* The receiver is a bare reference to a type (a struct or enum binding, or
  a built-in type identifier): the call is an associated function call. The
  named function on that type must have no `self`.
* The receiver is a value (a local, parameter, field, or any other
  expression): the call is an ordinary instance method call.

`Color.Red` (an enum variant) is field syntax (`receiver.name` with no
call parens), not a call, so it is unaffected. An imported free function
called as `f(...)` is a plain call, also unaffected.

## Generic instantiation

The implementing type's generic parameters are instantiated for the call.
With explicit type arguments (`Set<Int>.new()`) the parameters are fixed
directly. Without them (`Set.new()`), the element type is left as an
inference variable and solved from later use within the same body:

```rust
let s = Set.new()   // T is an inference variable
s.add(7)            // unifies T = Int
```

When no later use pins the parameters, the type arguments must be written
on the call (`Set<Int>.new()`); an unresolved parameter is a
`cannot infer type` error.

## Codegen

An associated function lowers to a direct static call to the per-type
symbol `<TypeMangle>$<func>`, with no receiver argument (the only
difference from an instance method call, which passes the receiver as the
leading argument). A non-generic associated function is a monomorphization
root, like any concrete-receiver method. A generic one is specialized at
the call site the same way a generic method is: the named implementing type
fixes the impl's type arguments, and the instantiation is queued for the
monomorphizer, which emits the body under the matching per-instantiation
symbol.

## Deferrals

* An associated function that introduces its own generic parameters beyond
  the implementing type's (`fun parse<U>(...)` on `impl<T> Wrap<T>`) is out
  of scope. The supported cases are `impl Counter { fun new() }` and
  `impl<T> Set<T> { fun new() -> Set<T> }`.
* Element inference for `Set.new()` works when a later use pins the type
  parameters. With no such use, write the type arguments on the call.
