# dyn Trait Spec

## Goal

Specify `dyn Trait` trait objects with vtable-based dynamic dispatch.
A trait object lets a value of any concrete type that implements a trait
be used through a single uniform handle, with the method called resolved
at run time from a per-type method table (the vtable). This is the
dynamic counterpart to the static, monomorphized trait method calls that
the compiler already lowers to direct calls.

The covered surface is: the `dyn Trait` type, unsizing coercions at
assignment, argument passing, and return, vtable emission, virtual
dispatch, an object-safety check, and the garbage collector interaction.
See the out-of-scope section for what is deferred.

## Pipeline position

```
Source -> Lexer -> Parser -> Resolver -> Tycheck -> HIR -> MIR -> Codegen
```

* The parser already accepts `dyn TraitPath` as a type (`TypeKind::Dyn`).
* The type checker resolves it to `Ty::Dyn`, records coercions, and
  validates object safety.
* HIR materializes each recorded coercion as a `DynCoerce` node.
* MIR lowers coercions and virtual calls to dedicated rvalues.
* Codegen emits vtables and the fat pointer construction and dispatch.

## Type representation

`Ty::Dyn { name, methods }` is a first-class type. `name` is the trait's
short name; `methods` is the trait's method names in declaration order.
Carrying the method order in the type fixes the vtable slot order once,
so every later pass picks the same slot for a method without re-reading
the trait declaration. `MirType::Dyn` mirrors the same shape.

A trait declaration is treated as a `Self`-bearing context by the
resolver, so trait methods may take a `self` receiver and reference
`Self` exactly like an `impl` block.

## Fat pointer representation

A `dyn Trait` value is a fat pointer with two pointer-width words: the
data word (the erased concrete value) and the vtable word (a pointer to
the static method table). Rather than pass these as two registers or two
stack slots, the back end boxes them as a small heap object: a two-slot
struct value `{ data, vtable }`.

```
slot 0  data    the erased concrete value (a GC pointer)
slot 1  vtable  address of the (concrete_type, trait) vtable (static)
```

The box reuses the standard struct value layout (a 16-byte object header
followed by 8-byte slots), so the trait object value is a single GC
pointer. This choice keeps every other part of the back end unchanged:
the value flows through one-word locals, one-word function parameters and
returns, and one-word collection elements without any special handling.
The cost is one extra allocation and one extra indirection per coercion,
which is acceptable for the first implementation. A future revision may
pass the two words unboxed once the calling convention and stack slot
machinery grow two-word value support.

## Vtable layout and emission

For each `(concrete_type, trait)` pair that is coerced to `dyn Trait`,
the back end emits one vtable as a read-only data object in the object
file. The vtable is an array of pointer-width slots, one per trait
method, in the trait's declaration order. Slot `i` holds the address of
the concrete type's implementation of the trait's `i`-th method.

```
vtable for (Dog, Speak):
  slot 0  &Dog$sound
  ...
```

Each slot is filled with a function-address relocation against the
method's symbol, so the linker patches the final address. Vtables are
interned by the key `<concrete_type_mangle>$<trait>`, so one pair shares
a single vtable across every coercion site. No size, align, or drop slot
is included: the collector owns memory reclamation, so a destructor slot
is unnecessary for now.

Method symbols are per-type. An impl method `sound` on `Dog` has the
symbol `Dog$sound`. This per-type mangling is what lets several types
implement a method of the same name without colliding at the object
level, and it is the same symbol a static call site computes from its
receiver type.

## Coercion sites and where they are inserted

A `dyn Trait` value is created by an unsizing coercion: a concrete value
whose type implements the trait is used where `dyn Trait` is expected.
The supported sites are:

* assignment with an annotation: `let d: dyn Speak = some_dog`
* argument passing: `describe(some_dog)` where the parameter is
  `dyn Speak`
* return: a function with return type `dyn Speak` returning a concrete
  value

The type checker drives coercion insertion. Every one of these sites
funnels through a single `unify(expected, actual, span)` call. Before
reporting a mismatch, `unify` checks whether `expected` is a `Ty::Dyn`
and `actual` is a concrete struct or enum that implements the trait. When
it is, the checker records a `DynCoercion { trait_name, methods,
concrete_ty }` keyed by the coerced expression's span and reports
success. A concrete type that does not implement the trait still reports
a mismatch.

HIR lowering reads the recorded coercion at each expression's span and
wraps the lowered value in a `DynCoerce` node whose type is the trait
object type. MIR lowers that node to a `DynCoerce` rvalue, and codegen
materializes the box. Keeping the decision in the type checker (which
already has both the expected type and the trait impl environment) and
the materialization in the back end keeps each pass focused.

## Dispatch lowering

A method call on a `dyn Trait` receiver is virtual dispatch. The type
checker resolves the call against the trait's method signatures (the
method must belong to the trait) and yields the method's return type. MIR
lowers it to a `VirtualCall` rvalue carrying the receiver, the method's
vtable slot index (its position in the trait method order), the
non-receiver argument operands, and the call signature shape.

Codegen lowers a `VirtualCall` by:

1. loading the data word (slot 0) and vtable word (slot 1) from the
   receiver box,
2. loading the method pointer from the vtable at the method's slot,
3. building the indirect call signature: the data pointer as the
   receiver, followed by the declared parameters, returning the method's
   return type,
4. emitting an indirect call to the loaded method pointer with the data
   word as the first argument.

A static method call on a concrete receiver stays a direct call, now to
the per-type symbol `<RecvType>$<method>` so multiple impls of a
same-named method resolve to distinct functions.

## Object safety

`dyn Trait` is allowed only for an object-safe trait. A trait is
object-safe in this subset when:

* no method is generic (has its own type parameters), and
* no method takes `Self` by value in a non-receiver parameter position.

The receiver `self` is always allowed. A `dyn` of a non-object-safe trait
is a type error with a hint to use a generic bound (`<T: Trait>`) instead.
This is a good-faith subset: associated types, `Self` return positions
beyond the receiver, and richer rules are not modeled yet.

## GC interaction

The data word is the only GC-managed word in the fat pointer. The box's
descriptor marks slot 0 (data) as a GC pointer and leaves slot 1 (vtable)
unmarked, because the vtable is a static read-only address, not a heap
object. The descriptor is keyed by the trait object's mangled name
(`dyn_<trait>`), so every coercion to the same `dyn Trait` shares it.

Because the trait object value is a single GC pointer to the box, the
existing root frame logic covers it with no special case: a `dyn Trait`
local is classified as a GC pointer, rooted in the shadow stack like any
heap value, and a collection traces the box, which in turn traces the
data word. The data word is therefore reachable as a transitive root for
as long as the trait object local is live. See `docs/v2/specs/gc.md` and
the GC root frame section of `docs/v2/specs/codegen.md`.

## Out of scope

* Trait upcasting (`dyn Sub` to `dyn Super`).
* `dyn` with generic methods: such methods are not object-safe and are
  rejected with a clear error.
* Associated types in trait objects.
* Auto-trait bounds (`dyn Trait + Send`).
* Heterogeneous `List<dyn Trait>` as a runnable showcase. The fat pointer
  is a single GC pointer, so it fits the existing one-word collection
  storage, but two pieces are missing: the type checker does not yet push
  an expected `dyn` element type down to each array element to coerce it,
  and the `List` element access methods (`get`, `push`) are themselves
  deferred to the stdlib collection issues. A trait object stored in a
  list would box correctly today; building and reading such a list end to
  end lands with those issues.
```

