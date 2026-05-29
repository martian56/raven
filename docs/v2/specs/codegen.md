# Codegen Spec

## Goal

Translate a monomorphized `MirProgram` into a native object file, link it
with the `raven-runtime` staticlib through the host toolchain linker, and
produce an executable. The backend reuses Cranelift for instruction
selection and register allocation. The covered surface is primitives,
binary and unary operators, branches, switches, function calls with
static dispatch, returns, the `print` intrinsic, and
heap value construction: structs, enums (including `Option` and
`Result`), field access, enum dispatch, and closures (capturing,
returned, passed, and invoked through their value), all with garbage
collector root frames.

Rich stdlib collection methods (`List` and `Map` operations beyond
construction) are out of scope here and tracked by the follow up issues
listed below.

## Pipeline position

```
Source -> Lexer -> Parser -> Resolver -> Tycheck -> HIR -> MIR -> Codegen -> object file
                                                                              |
                                                                              v
                                                                      link with raven-runtime
                                                                              |
                                                                              v
                                                                          executable
```

The codegen entry point consumes a fully monomorphized `MirProgram`. No
generic functions, no inference variables, no type parameters. Every
`MirType` is ground.

## Cranelift integration

Three Cranelift crates are pulled in at the workspace level:

* `cranelift-codegen`: instruction selection, register allocation, the
  `ir::Function` and `Signature` builders.
* `cranelift-frontend`: the `FunctionBuilder` helper that smooths the
  process of emitting Cranelift IR.
* `cranelift-module`: the `Module` trait that abstracts symbol
  declaration and linking, plus `DataDescription` for static data.
* `cranelift-object`: the `ObjectModule` that writes the final relocatable
  object file via the `object` crate.

The backend produces a single object file per `MirProgram`. Cross
function references resolve through `cranelift-module`'s `FuncId` and
`DataId` indices; symbol mangling is the responsibility of MIR (every
`MirFunction::name` is already the mangled monomorphic name).

## Value representation

Primitive Raven values map to fixed Cranelift types:

| MIR type | Cranelift type | Notes |
|----------|----------------|-------|
| `Unit`   | (none)         | Carried as the zero sized "no value". A `Unit` constant emits no Cranelift value; a function returning `Unit` has an empty `returns` list in its signature. |
| `Bool`   | `types::I8`    | `0` or `1`. Logical operators produce the same width so `if` over a `Bool` compares against zero. |
| `Int`    | `types::I64`   | Signed 64 bit integers. Overflow is wrap on add, sub, mul (matching Cranelift defaults). Division by zero traps via Cranelift's `sdiv` semantics on supported targets; an explicit zero check is inserted on targets where this is not guaranteed. |
| `Float`  | `types::F64`   | IEEE 754 doubles. |
| `Char`   | `types::I32`   | Reserved. The MVP does not exercise `Char` operations; ground work only. |
| `Str`    | pointer        | A heap object pointer. String literals reaching the `print` intrinsic still pull bytes from the static data table directly; a `Str` flowing through a local is a single traced GC pointer. |
| `Struct`, `Enum`, `Option`, `Result`, `List`, `Function` | pointer | A single GC pointer to a heap object. Struct and enum construction, field access, and closure allocation are lowered (see the heap value section below). `List` and `Map` rich methods, and trait object dispatch, remain tracked by issues #66 and the stdlib issues. |

The calling convention is whatever Cranelift picks for the host triple
(the system V AMD64 ABI on x86_64 Linux and Windows, ARM64 ABI on
aarch64). The backend never inspects the chosen convention. Parameters
are passed through the standard Cranelift `Signature`; aggregate
returns are not used in the MVP because every supported return type is
a single scalar or absent.

## Function lowering

Each `MirFunction` becomes one Cranelift `ir::Function`. The lowering
pass walks the MIR locals first to allocate `StackSlot`s for every
non parameter local and a Cranelift `Block` for every `MirBlockId`. The
mapping is direct: `MirLocal(i)` indexes the function's slot vector and
`MirBlockId(i)` indexes the function's block vector.

Parameters live in Cranelift block parameters on the entry block. The
backend reads each parameter once, stores it into the matching stack
slot, and from that point treats parameters and locals uniformly.

Per block: every `MirStatement::Assign` translates into a recipe that
reads its operand locals into Cranelift values, performs the operation,
and writes the result back to the destination slot. `StoreField` and
`StoreIndex` write a value through a heap object pointer rather than into
a local slot; see the assignment section below. `StorageLive` and
`StorageDead` are no ops in the MVP (the future allocator may grow real
lifetimes); `Nop` is also a no op.

## Assignment targets and store statements

An assignment target (an lvalue) is a place rooted in a name. The parser
accepts a plain identifier, `self`, and any chain of field accesses
(`a.b.c`) and indexes (`xs[i]`, `obj.items[k]`) built on top of those.
Genuine value expressions (literals, calls, arithmetic) are rejected with
an "invalid assignment target" parse error. The grammar is checked
recursively: the receiver of a field or index in a target position is
itself a target.

HIR lowering maps each target to one of three shapes. A plain identifier
(or bare `self`, which binds by the fixed name `self`) becomes a local
assignment, lowered to `MirStatement::Assign` writing the local's slot. A
field target lowers to `MirStatement::StoreField { base, index, value }`,
and an index target to `MirStatement::StoreIndex { base, index, value }`.
A `self.field = ...` write is just a field target whose base is the
`self` receiver. Compound assignment (`+=` and friends) desugars to a
read of the same place, a binary op, and a plain store, evaluating a
field or index base into a fresh local so it is not evaluated twice.

A store does not introduce a new GC root. The store's `base` is an
already-rooted GC pointer (a local in the function's root frame, or the
`self` parameter), and the stored value becomes reachable through that
live container the moment it is written. Overwriting a slot simply drops
the container's last reference to the old value; the collector reclaims
it on a later cycle. The new value is held by the live container, so it
survives the next collection. This is the same reasoning the struct
constructor and `list.push` already rely on. See the GC root frame
section below.

## Operand and rvalue lowering

| MIR construct | Cranelift expansion |
|---------------|---------------------|
| `Use(Copy(l))` | `stack_load` from `l`'s slot. |
| `Use(Const(Int n))` | `iconst.i64 n`. |
| `Use(Const(Float f))` | `f64const f`. |
| `Use(Const(Bool b))` | `iconst.i8 (if b { 1 } else { 0 })`. |
| `Use(Const(Unit))` | No value emitted. |
| `Use(Const(Str s))` | Allocates or reuses a static data symbol holding the bytes plus a length companion, emits `global_value` for the address. Currently only consumed by the `print` intrinsic. |
| `BinaryOp(op, lhs, rhs)` on `Int` | `iadd`, `isub`, `imul`, `sdiv`, `srem`, `icmp` with the matching condition code. |
| `BinaryOp(op, lhs, rhs)` on `Float` | `fadd`, `fsub`, `fmul`, `fdiv`, `fcmp`. The modulus operator is not defined on `Float` here. |
| `BinaryOp(And/Or)` on `Bool` | `band` / `bor` over `i8`. The frontend already short circuits to branches; what reaches MIR is value level. |
| `BinaryOp(BitAnd/BitOr/BitXor/Shl/Shr)` on `Int` | `band`, `bor`, `bxor`, `ishl`, `sshr`. |
| `UnaryOp(Neg)` on `Int` | `ineg`. |
| `UnaryOp(Neg)` on `Float` | `fneg`. |
| `UnaryOp(Not)` on `Bool` | `bxor` with the constant `1`. |
| `UnaryOp(Ref)` | Deferred; emits `Unreachable`. |
| `Call { callee, args }` | Loads each argument from its slot, declares the callee through `cranelift-module`, emits `call` with the resulting `FuncRef`, stores the return value. |
| `StructCreate`, `EnumCreate`, `FieldAccess`, `ClosureCreate`, `EnvLoad`, `ClosureCall` | Lowered. See the heap value and closure sections below. |
| `Cast` | Integer and float conversions through `sextend`, `ireduce`, `fcvt_from_sint`, `fcvt_to_sint_sat`. |
| `ArrayLit`, `IndexAccess`, `ListMethod` | Lowered. See the List section below. |

## Terminator lowering

| MIR terminator | Cranelift expansion |
|----------------|---------------------|
| `Goto(b)` | `jump b`. |
| `SwitchInt { discr, targets, otherwise }` | A linear cascade of `icmp` plus `brif` against each `(value, block)` pair; the final fall through is `jump otherwise`. A dense fan out could later switch to `br_table`; the MVP keeps the cascade for simplicity. |
| `SwitchEnum { discr, targets, otherwise }` | Loads the enum value's discriminant slot (slot 0) and emits an `icmp` plus `brif` cascade against each `(variant, block)` pair. With an explicit otherwise it is the default; without one the last target is the default, since a well typed match is exhaustive. |
| `Return(op)` | Reads the operand, then `return value` if the function has a non Unit return type, otherwise a bare `return`. |
| `Unreachable` | `trap unreachable`. |

## Stack slot layout

Every non parameter local with a sized primitive type gets an
`ExplicitSlot` sized to the primitive's byte width and aligned to the
same. `Unit` locals occupy a zero sized slot that is never loaded or
stored. Parameter locals also live in stack slots; the entry block
spills the incoming Cranelift block parameter into the slot before any
other code runs so subsequent reads can use the uniform slot path.

## Print intrinsic

The MIR call site `Call { callee: MirFnRef { mangled: "print", .. } }`
is recognized as an intrinsic. It produces a `(pointer, length)` pair
for its single argument and calls `raven_println_str(ptr, len)`, which
prints the slice and writes a trailing newline. The pointer width is
whatever `module.target_config().pointer_type()` reports (i64 on every
supported host).

The argument can be a string in one of two shapes:

1. A bare string literal (`Const(Str s)`). The bytes are interned into
   the per module string table and the call receives the address of the
   bytes plus the compile-time byte length. This is the allocation-free
   fast path so `print("literal")` never touches the heap.
2. Any other `Str` value (a `let`-bound string, a returned string, an
   interpolation result). It is a heap `String` pointer, so the codegen
   calls `raven_string_bytes` and `raven_string_len` on it to read the
   byte buffer pointer and length (the `u32` length is widened to
   pointer width for the ABI), then calls `raven_println_str`.

The string table is a `BTreeMap<Vec<u8>, DataId>` keyed by the bytes, so
identical literals share a single allocation in the object file. Each
data symbol is named `__raven_str_<n>` for stable diagnostic output.

## String values

A string constant used as a *value* (assigned to a local, passed to a
function, concatenated, interpolated) is promoted to a heap `String`:
the codegen interns the bytes, then calls `raven_string_from_bytes(ptr,
len)` to allocate a GC-managed `String` object. Every `Str`-typed local
therefore holds a real GC pointer, so the GC root frame traces it and
the runtime string functions can consume it directly. Only the bare
`print("literal")` argument keeps the static fast path.

String interpolation lowers (in MIR) to a chain of runtime calls that
the back-end recognizes by mangled name and routes to the runtime:
`__raven_str_concat` to `raven_string_concat`, and the per-type
conversions `__raven_int_to_string`, `__raven_bool_to_string`,
`__raven_float_to_string`, and `__raven_char_to_string` to their
matching `raven_*_to_string` symbols. Each returns a heap `String`
pointer that flows like any other string value.

### String equality

`==` and `!=` on `String` compare contents, not object identity. A plain
`BinaryOp(Eq/Ne)` would emit an `icmp` on the two `String` pointers,
which would report two distinct heap objects as unequal even when their
bytes match. To avoid that, MIR lowering special-cases `==`/`!=` whose
left operand is `String`: it emits a `Call` to the `__raven_str_eq`
intrinsic, which the back-end routes to `raven_string_eq(a, b) -> i8`
(the runtime compares lengths, then bytes). `==` uses the `i8` result
directly; `!=` lowers an extra `UnaryOp(Not)` over it. `Int`, `Float`,
`Bool`, and `Char` operands are unaffected and keep the `icmp`/`fcmp`
value compare above. Comparison of user structs and enums is not
special-cased and remains an identity (pointer) compare.

### Printing C integer FFI values

The integer C FFI types (`CInt`, `CLong`, `CSize`) have no `ToString`
impl of their own; they reach `print` and string interpolation by
widening to `Int`. The type checker accepts them where `ToString` is
required, and MIR lowering rewrites the `to_string` call (inserted for a
`print` argument or an interpolation fragment) on an FFI integer receiver
into a `Cast` to `Int` followed by the `__raven_int_to_string` intrinsic.
The cast sign-extends a narrower value (`CInt` is i32); `CLong` and
`CSize` are already pointer-width and pass through. `CSize` is rendered as
a signed `Int`, correct for realistic sizes (below 2^63). Reusing the
`Int` to-string path means no dedicated integer print symbol or intrinsic
arm.

## Heap value layout and lowering

A struct or enum value is a heap object: the standard 16-byte
`ObjectHeader` followed by a sequence of uniform 8-byte field slots, one
per field, in declaration order. Using a single slot width means the
back end never computes per-field offsets from alignment, every
primitive (`Int`, `Float`) fits a slot exactly, a smaller scalar
(`Bool`, `Char`) is widened into one, and a heap value is a single
pointer. The slots begin at offset 16; field `i` is at offset
`16 + 8 * i`; the total size is `16 + 8 * field_count`, already 8-byte
aligned. See `docs/v2/specs/object-layout.md`.

### Struct construction and field access

`StructCreate { ty, fields, field_tys }`:

1. Compute the struct's GC pointer bitmask from `field_tys`: bit `i` is
   set when field slot `i` holds a GC pointer (any heap value).
2. Intern a descriptor id for the struct type (keyed by the mangled type
   name) and record the bitmask. The id is registered with the collector
   once, from the program entry shim (see below).
3. Evaluate each field operand into a register, widening scalars to the
   8-byte slot width.
4. Call `raven_struct_new(field_count, type_id)` to allocate the body,
   then `raven_struct_fields(obj)` to get the field base pointer, then
   `store` each field at `16 + 8 * i`.

`FieldAccess { base, index }` loads the base pointer, calls
`raven_struct_fields`, and `load`s the pointer-width slot at
`16 + 8 * index`. The destination local's machine type narrows the
loaded value: a `Float` field's bits are reinterpreted with `bitcast`,
a narrow scalar is reduced. The MIR lowering resolves `index` to the
field's position in declaration order, so creation and access agree.

`StoreField { base, index, value }` is the write mirror of
`FieldAccess`. It evaluates `base` to the object pointer, widens `value`
to the 8-byte slot width (a scalar is zero extended, a `Float`'s bits are
reinterpreted), calls `raven_struct_fields` for the field base, and
`store`s the value at `8 * index` from that base (the same slot a
`FieldAccess` reads). The MIR lowering resolves `index` from the
receiver's struct declaration order, so a read and a write of the same
field address the same slot. Because a struct is a heap object reached
through a pointer, a `self.field = ...` write inside a method mutates the
object the caller still holds, so the change is observed after the call
returns.

### Per-instantiation struct descriptors (generic structs)

A generic struct (`struct Box<T> { value: T }`) is monomorphized in MIR
lowering, which grounds each field type per instantiation (see `mir.md`).
The back end needs no separate code path: each instantiation arrives as a
`StructCreate` whose `ty` is a distinct `MirType::Struct` carrying the
concrete type arguments and whose `field_tys` are the ground field types.

Two consequences follow from the uniform 8-byte slot model:

* The slot layout is identical across instantiations. Every field, scalar
  or GC pointer, occupies one 8-byte slot, so `Box<Int>` and
  `Box<String>` have the same field count and the same offsets. No
  per-instantiation offset table is needed.
* The GC pointer descriptor differs per instantiation. The descriptor is
  keyed by the mangled type name (`intern_descriptor(ty.mangle(), mask)`),
  and a generic struct's instantiations mangle distinctly (`Box_Int`,
  `Box_Str`). Each interns its own descriptor id with its own pointer
  mask: `Box_Int` has mask `0` (the `Int` slot is opaque to the
  collector), while `Box_Str` has bit 0 set (the `String` slot is a traced
  pointer). The program entry shim registers every interned descriptor, so
  the collector traces each instantiation's slots correctly.

Because the mangle and mask are derived from the ground field types the
MIR lowering already produced, the existing `lower_struct_create` path
handles generic structs without change; monomorphization is what makes the
field types concrete.

### Enum construction and dispatch

An enum value reuses the struct value layout. Slot 0 holds the variant
discriminant (a pointer-width integer); slots `1..` hold the active
variant's payload. `EnumCreate { ty, variant, payload, payload_tys }`
allocates `1 + payload.len()` slots, stores the discriminant in slot 0,
and the payload in slots `1..`. The GC pointer bitmask shifts each
payload pointer to slot `i + 1`. Because each variant is constructed
independently, the enum type's descriptor is the union of every
variant's payload pointer mask; an inactive variant's slots are zero and
trace harmlessly.

`SwitchEnum` loads slot 0 and branches on it. A `match` arm that binds a
payload reads slot `index + 1`, since slot 0 is the discriminant.

This makes `Option`, `Result`, and user enums runnable. The built in
constructors `Some(x)`, `Ok(x)`, `Err(x)`, and `None` are recognized
during HIR lowering and become `EnumCreate`, rather than calls to
undefined symbols.

### Closures

Closures are fully first class: a lambda is allocated as a heap `Closure`
object, captures the free variables it references, and can be returned,
passed as an argument, stored in a local or struct field, and invoked
through its value.

#### Capture analysis

Capture analysis runs in HIR-to-MIR lowering (`src/mir/lower/closure.rs`).
For a lambda it walks the body and collects every free identifier: a name
that is neither one of the lambda's own parameters nor a binding
introduced inside the body. A free name is a capture exactly when it
resolves to a local or parameter that is in scope at the definition site
(found through the lowering context's scope stack). A free name that does
not resolve to an in-scope local is a reference to a top-level function or
constant, which codegen addresses by symbol rather than capturing.

#### Capture-by-value semantics

Captures are by value: the value at closure-creation time is copied into
the capture environment. For a scalar (`Int`, `Float`, `Bool`, `Char`)
that copy is the bits. For a GC-managed value (`Str`, `Struct`, `Enum`,
`Option`, `Result`, `List`, a closure `Function`, or a `dyn Trait`) the
copied value is the same pointer, so the captured object aliases the
original; mutations made through that heap object after the closure is
built remain visible inside the closure. Capture-by-reference (rebinding a
captured variable so a later assignment to the original is seen) would
need upvalue or cell machinery and is not provided in this release.

#### Environment layout

The capture environment is a positional record of pointer-width (8 byte)
slots stored in the `Closure` object's owned capture buffer. Capture
analysis orders GC-pointer captures first, then scalar captures, so the
leading `capture_ptr_count` slots are exactly the traced GC pointers the
collector follows (see the runtime closure layout in
`docs/v2/specs/object-layout.md`). `ClosureCreate { fn_name, captures,
capture_tys }` allocates the closure through `raven_closure_new(fn_ptr,
size, align, count, ptr_count)` where `size = 8 * count`, `align = 8` (or
zero when there are no captures), and `ptr_count` is the number of
GC-pointer captures. It then loads the capture buffer base through
`raven_closure_captures` and stores each capture value into slot `i` at
byte offset `i * 8`. Scalars narrower than a pointer and floats are
widened to a slot the same way struct fields are.

#### Lifted body and the env parameter

Each lambda body is lifted into a standalone MIR function whose leading
parameter is the capture environment (`__env`, a raw pointer-width value
the GC does not trace), followed by the lambda's own parameters. The
lifted body opens by reading each capture from the env with an `EnvLoad
{ env, slot, ty }` rvalue (a pointer-width load at `slot * 8`, narrowed to
the capture's type) into a local bound under the capture's source name, so
the rest of the body references captures as ordinary locals. A capture
that is a GC pointer, once read into a body local, is rooted by the lifted
body's own shadow-stack frame like any other GC local.

A lifted body is lowered exactly like an ordinary function body, so any
generic function it calls is specialized at the call site and contributes
its `(decl, substitution)` to the monomorphization worklist. Those pending
calls travel up through the enclosing function into the worklist, so a
generic function reachable only through a closure body is still
instantiated for the concrete type arguments seen inside that body.

#### Invoking a closure value

A call whose callee is a value of function type (an in-scope local of
`fun(T) -> U` type, a returned closure, a closure stored in a field, ...)
lowers to `ClosureCall { closure, args, param_tys, ret_ty }`. Codegen
loads the function pointer through `raven_closure_fn_ptr` and the capture
env base through `raven_closure_captures`, builds the indirect signature
`(env_ptr, <user params...>) -> ret`, and emits a `call_indirect` passing
the env base as the leading argument followed by the user arguments. The
signature is uniform regardless of the capture count or types, so the call
site needs only the user parameter and return types; a top-level function
reference is still dispatched directly by symbol.

#### GC

A closure local is a GC pointer rooted in the enclosing function's shadow
stack frame like any other GC local. The captured GC pointers inside the
env are traced by the collector through the `Closure` descriptor: the
runtime's closure-tracing arm reads `capture_ptr_count` and traces the
leading pointer slots, which is why capture analysis places GC-pointer
captures first.

### Lists

A `List<T>` value is a single GC pointer to a heap `List` object (header,
`element_size`, `element_align`, `elements_are_gc_ptrs`, and an owned
element buffer; see `docs/v2/specs/object-layout.md`). Codegen stores
every element in a uniform eight-byte slot, the same slot width struct
and enum fields use, so `element_size == element_align == 8` for all
element types. Scalars narrower than eight bytes are widened on store and
narrowed on load; a GC pointer is already eight bytes. The
`elements_are_gc_ptrs` flag is set from the static element type
(`layout::is_gc_pointer`): nonzero for heap element types (`String`,
`List`, `Map`, `Set`, struct, enum, closure, `Box`, `dyn`) and zero for
the scalars (`Int`, `Float`, `Bool`, `Char`). The flag is independent of
the slot size: an eight-byte `Int` slot is not a pointer, while an
eight-byte `String` slot is.

#### Element representation and the GC-pointer flag

The flag drives the collector's element tracing. The runtime's
`TAG_LIST` arm reads `elements_are_gc_ptrs` and, when nonzero, traces the
first `len` pointer slots of the buffer; when zero, the buffer is opaque
scalar bytes and is not traced. So a `List<String>` keeps its elements
reachable, and a `List<Int>` never misreads an integer as a pointer.

#### `ArrayLit`

`[a, b, c]` (MIR `ArrayLit { ty, elements }`, where `ty` is `List<T>`)
lowers to:

1. Evaluate every element operand into a register and widen each to an
   eight-byte slot value.
2. `raven_list_new(8, 8, len, gc_ptrs)` to allocate the list, where
   `gc_ptrs` is the element type's GC-pointer flag.
3. For each element, write the widened value into a one-slot scratch
   stack slot and call `raven_list_push(list, scratch_addr)`, which
   copies the eight bytes into the list (growing the buffer when needed).

The result is the list pointer.

#### `IndexAccess`

`xs[i]` (MIR `IndexAccess { base, index }`) lowers to:

1. Load the element count with `raven_list_len(list)` and the buffer base
   with `raven_list_elements(list)`.
2. Bounds-check the index: an unsigned `i < len` compare. On the
   out-of-bounds path, call `raven_panic("list index out of bounds")`
   (which writes the message to stderr and exits 101) and `trap`; the
   in-bounds path continues.
3. Load the eight-byte slot at `base + i * 8`. The loaded value is
   pointer-width and is narrowed to the destination local's machine type
   on store (a `Float` or narrow scalar element is reinterpreted there).

#### `StoreIndex`

`xs[i] = v` (MIR `StoreIndex { base, index, value }`) is the write
mirror of `IndexAccess`. It evaluates `base` to the list pointer and
widens `value` to the eight-byte slot width, loads the length and buffer
base, bounds-checks `i` with the same unsigned `i < len` compare (calling
`raven_panic("list index out of bounds")` and trapping on the
out-of-bounds path), then `store`s the value at `base + i * 8`. `List` is
a heap object, so the write mutates the shared list through the pointer
and every alias observes the overwritten element.

#### List methods

The built-in `List<T>` methods are recognized during MIR lowering and
routed to a `ListMethod { op, receiver, arg, elem_ty }` rvalue (rather
than to an unresolved per-type method symbol), so codegen has the element
type at hand. The mapping to runtime calls:

| Method | Lowering |
|--------|----------|
| `len(self) -> Int` | `raven_list_len`, zero-extended to `Int`. |
| `is_empty(self) -> Bool` | `raven_list_len` compared `== 0`. |
| `push(self, x)` | Widen `x` to an eight-byte slot, spill it, call `raven_list_push`. `List` is a heap object, so the push mutates the shared object through the pointer and every alias observes the new element; the list pointer itself is unchanged across a buffer grow. |
| `pop(self) -> T` | `raven_list_pop(list, out)` copies the last element into a scratch slot and shrinks the list, returning a success flag; a zero flag (empty list) calls `raven_panic` and traps, otherwise the scratch slot is loaded as the result. |
| `get(self, i) -> T` | `raven_list_get(list, i, out)` copies the element at `i` into a scratch slot, returning a success flag; a zero flag (out of range) calls `raven_panic` and traps, otherwise the scratch slot is loaded as the result. |

`pop` and `get` return the element type `T` directly (the type checker's
built-in `List` signatures), so an empty `pop` or an out-of-range `get`
panics rather than returning a sentinel, matching the index bounds check.

#### GC

A `List` local is a GC pointer rooted in the function's shadow-stack
frame like any other GC local, so a collection during list building still
sees the list rooted. Because the list object roots its own elements
through `elements_are_gc_ptrs`, a pushed GC-pointer element stays
reachable for as long as it is in the list, and a popped element is no
longer traced once `len` drops below its slot.

## GC root frames

The collector finds its roots through a shadow stack the back end
maintains (see `docs/v2/specs/gc.md`). For every function, the codegen:

1. Identifies the locals whose type is a GC pointer (`Str`, `Struct`,
   `Enum`, `Option`, `Result`, `List`, closure `Function`).
2. Allocates one contiguous root array stack slot holding one
   pointer-sized entry per GC local.
3. At entry, after spilling parameters: zeroes every non-parameter GC
   local (so a collection before the body assigns it never follows a
   stale pointer), writes each GC local's slot address into the root
   array, and calls `raven_gc_enter_frame(array_addr, count)`.
4. Before every `Return`, calls `raven_gc_leave_frame()`. The return
   value is evaluated before leaving the frame so a collection during
   evaluation still sees the locals rooted. Frames nest in strict
   last-in-first-out order.

A function with no GC pointer locals sets up no frame at all.

### Worked example

For `fun add_coords(p: Point) -> Int = p.x + p.y`, the parameter `p` is
a GC pointer and any temporaries for the field reads are scalars. The
entry block emits, in order:

```
;; spill the incoming parameter pointer into p's slot
v0 = stack_addr.i64 p_slot          ; address of p's slot
;; p is a parameter, so it is not re-zeroed
stack_store v0, root_array+0        ; root_array[0] = &p
v1 = stack_addr.i64 root_array
v2 = iconst.i64 1                   ; one GC local
call raven_gc_enter_frame(v1, v2)
;; ... body: load p, read fields at +16 and +24, add ...
call raven_gc_leave_frame()
return vsum
```

The root array holds the *address* of `p`'s slot, not `p` itself, so a
collection reads `*(&p)` and observes whatever pointer the slot currently
holds. This matches the ABI example in `docs/v2/specs/gc.md`.

### Struct descriptor registration

The collector traces a struct or enum by tag plus a per-type GC pointer
bitmask. The back end registers every interned descriptor with
`raven_struct_register(type_id, ptr_mask)` from the `int main(void)`
entry shim, before calling the Raven `main`, so any value the program
builds is traceable from its first allocation. The struct value stores
its `type_id` in `header.cap` and its field count in `header.len`.

## Driver

A new `build` subcommand on the `raven` binary owns the end to end
pipeline:

```
raven build path/to/program.rv -o out
```

The driver runs lexing, parsing, resolution, type checking, HIR
lowering, MIR lowering, and monomorphization in order, then hands the
`MirProgram` to `codegen::compile_to_object`. The returned `Vec<u8>` is
written to a temporary `.o` file. The driver then links the object with
the `raven-runtime` staticlib (built by the same Cargo invocation as
the compiler) into the requested output.

`compile_to_object(program, target_isa)` is also exposed for in process
use by tests, returning the raw object bytes so a test can call the
linker itself or inspect the object with `goblin`.

### Entry point

The exported program entry is an `int main(void)` shim, not the Raven
`main` directly. Cranelift lowers the Raven `main` under an internal
symbol that returns whatever the function returns (unit for a typical
program, which is no machine value at all). A C runtime starting that
symbol as `int main()` would read an uninitialized register as the
process exit code. The shim calls the Raven body and returns a literal
`0`, so a successful program exits with status `0` on every host.

### Linker selection

Cranelift emits an object in the host's native format: an MSVC-flavor
COFF on windows-msvc, ELF on Linux, Mach-O on macOS. The link step is
therefore toolchain aware and keyed on the host target triple
(`target_lexicon::Triple::host()`):

* `*-windows-msvc`: the MSVC `link.exe`, located through the Windows
  registry with `cc::windows_registry::find_tool`. The `cc` crate hands
  back a `Command` preloaded with the SDK and CRT `LIB`, `INCLUDE`, and
  `PATH` environment, which is why this path is preferred. The link line
  is `/NOLOGO /OUT:<output> <object> <runtime.lib> <native system libs>
  /SUBSYSTEM:CONSOLE`. If the registry lookup fails, the driver falls
  back (best effort) to the Rust toolchain's bundled `rust-lld` in
  `lld-link` flavor, located under `rustc --print sysroot`. The fallback
  needs the SDK `LIB` paths already present in the environment because
  `rust-lld` does not supply them.
* `*-windows-gnu`: a `cc`/`gcc` driver, which must be a 64-bit
  MinGW-w64. A 32-bit MinGW.org `gcc` cannot read the 64-bit object, so
  a link failure surfaces a hint about the architecture mismatch.
* Linux, macOS, and other Unix: the system `cc` driver, which brings the
  system linker and its default library search paths, plus the Rust std
  system libraries the runtime depends on (`-lpthread`, `-ldl`, `-lm`,
  and so on).

### Rust std native system libraries on MSVC

The Rust staticlib references system import libraries that must appear
on the MSVC link line, because the staticlib does not carry them. The
exact set is captured with:

```
cargo rustc -p raven-runtime --crate-type staticlib -- --print native-static-libs
```

The `note: native-static-libs: ...` line it prints is hardcoded as a
`const` in `linker.rs` (currently `kernel32.lib advapi32.lib ntdll.lib
userenv.lib ws2_32.lib dbghelp.lib /defaultlib:msvcrt`, captured against
rustc 1.85.0). The driver does not shell out to cargo at link time.
Refresh the list the same way if the runtime crate gains native
dependencies.

If no linker is available for the host at all, the smoke test that
compiles and runs a hello world program prints a diagnostic and short
circuits with a successful exit. On a correctly configured host it links
and runs the program for real and asserts the output. The unit tests
that only consult MIR lowering do not depend on a linker.

## Out of scope (tracked by follow up issues)

* ~~Trait object (`dyn Trait`) dispatch through vtables (issue #66).~~
  Implemented: a trait object is a boxed fat pointer, vtables are emitted
  as read-only data, and a `dyn` method call dispatches through the
  vtable. See `docs/v2/specs/dyn-trait.md`. Static trait method calls are
  monomorphized by MIR into direct calls.
* ~~Calling closure values and capturing closures (lambda body lifting and
  capture analysis).~~ Implemented: capture analysis lifts each lambda
  body into a standalone function, captures free variables by value, and
  invokes closure values through an indirect call. See the Closures
  section above.
* ~~`defer` ordering and runtime hooks.~~ Implemented: function-scoped
  defers register a thunk closure on a per-call runtime defer frame and
  run in LIFO order at every return path before leaving the GC frame. See
  `docs/v2/specs/defer.md`.
* String interpolation (issue #69) and C FFI (issue #70).
* ~~`List<T>` literals, indexing, and the built-in methods (`len`,
  `is_empty`, `push`, `pop`, `get`).~~ Implemented: see the Lists section
  above.
* `Map` and `Set` literals and methods, and richer iterator pipelines
  built on the collections (issues #71 onwards).
* Cross platform installer packaging (issue #92).

## Test coverage

* Unit tests on `compile_to_object` covering: a function returning a
  constant `Int`, an arithmetic `+` lowering, an `if` with a value, a
  call between two functions, and the `print("hello")` lowering.
* Layout unit tests on the field offset, GC pointer classification, and
  struct and enum pointer masks.
* End to end smoke tests that compile, link, run, and check the stdout of
  `examples/v2/hello.rv` (`Hello, Raven!`), `point.rv` (a struct,
  prints `7`), `option_sum.rv` (Option construction and matching, prints
  `104`), `closure_value.rv` (a non-capturing closure allocation, prints
  `42`), `closure_capture.rv` (a returned closure capturing a local by
  value, prints `15` then `42`), `closure_arg.rv` (a capturing closure
  passed as an argument and invoked indirectly, prints `21`),
  `list_ops.rv` (list literals, indexing, `len`, and `push` over `Int`
  elements, prints `3`, `20`, `4`, `40`), and `list_strings.rv` (a list
  of heap `String` elements exercising the GC-pointer element path,
  prints `raven`, `bird`, `3`). Each gates
  on linker and runtime staticlib presence and
  emits an `eprintln!` plus a successful exit only when one is missing;
  on a correctly configured host it links and runs the program for real.

## Crate layout

```
src/codegen/
  mod.rs                Entry point: compile_to_object, compile_program
  context.rs            Per module Cranelift context, function table, string table, struct descriptors
  function.rs           Per function lowering, heap rvalues, GC root frames
  layout.rs             Struct and enum heap layout: field offsets and GC pointer masks
  intrinsics.rs         Recognized intrinsic mangled names and runtime ABI symbols
  linker.rs             per-platform linker selection by target triple
src/main.rs             build subcommand wired through minimal arg parsing
```
