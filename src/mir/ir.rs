//! MIR data types: functions, basic blocks, statements, terminators,
//! rvalues, and operands.
//!
//! The shape mirrors a scoped-down version of Rust's MIR. Each function
//! is a control flow graph: a flat vector of basic blocks, each ending
//! in exactly one terminator. Local variables are dense indices into
//! the function's locals table.

use std::collections::HashMap;

use crate::resolve::DeclId;
use crate::span::Span;

use super::ty::{MirFfiTy, MirType};

/// Dense index into [`MirFunction::locals`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MirLocal(pub u32);

/// Dense index into [`MirFunction::blocks`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MirBlockId(pub u32);

/// A binary operator preserved through MIR.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MirBinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
}

/// A unary operator preserved through MIR.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MirUnOp {
    Neg,
    Not,
    Ref,
}

/// A built-in method on `List<T>`. The front end resolves these against
/// the receiver type rather than a user `impl`, so the lowering routes
/// them to a [`MirRvalue::ListMethod`] carrying the element type the
/// back end needs to size slots and pick the GC-pointer flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ListMethodOp {
    /// `len(self) -> Int`: the element count.
    Len,
    /// `is_empty(self) -> Bool`: `len == 0`.
    IsEmpty,
    /// `push(self, x)`: append `x`, mutating the shared heap object.
    Push,
    /// `pop(self) -> T`: remove and return the last element. Panics when
    /// the list is empty.
    Pop,
    /// `get(self, i) -> T`: read the element at `i`. Panics when `i` is
    /// out of range.
    Get,
}

/// A compile-time constant.
#[derive(Debug, Clone, PartialEq)]
pub enum MirConstant {
    Unit,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Char(char),
    /// A C string literal `c"..."`. Lowered by the back end to a pointer
    /// to a static, read-only, null-terminated byte buffer rather than a
    /// heap `String`. The stored text excludes the trailing `\0`, which
    /// codegen appends.
    CStr(String),
}

/// One operand: either a copy of a local or a constant.
#[derive(Debug, Clone, PartialEq)]
pub enum MirOperand {
    Copy(MirLocal),
    Const(MirConstant),
}

/// Reference to a callee after monomorphization. The mangled name is
/// the lookup key the back-end uses; the declaration id is preserved
/// for diagnostics and future passes.
#[derive(Debug, Clone, PartialEq)]
pub struct MirFnRef {
    pub mangled: String,
    pub origin: Option<DeclId>,
}

/// Right hand sides of MIR assignments.
#[derive(Debug, Clone)]
pub enum MirRvalue {
    Use(MirOperand),
    BinaryOp(MirBinOp, MirOperand, MirOperand),
    UnaryOp(MirUnOp, MirOperand),
    /// Read a mutable module-level global's data slot. `name` is the global's
    /// mangled symbol; the back end loads a value of `ty` from the slot.
    GlobalLoad {
        name: String,
        ty: MirType,
    },
    Call {
        callee: MirFnRef,
        args: Vec<MirOperand>,
    },
    StructCreate {
        ty: MirType,
        /// Field operands in declaration order.
        fields: Vec<MirOperand>,
        /// Field types in declaration order, parallel to `fields`. The
        /// back-end uses these to decide which field slots hold GC
        /// pointers when it builds the struct's GC descriptor.
        field_tys: Vec<MirType>,
    },
    EnumCreate {
        ty: MirType,
        variant: usize,
        payload: Vec<MirOperand>,
        /// Payload types, parallel to `payload`. The back-end uses these
        /// to decide which payload slots hold GC pointers.
        payload_tys: Vec<MirType>,
    },
    FieldAccess {
        base: MirOperand,
        /// Field slot index in declaration order.
        index: usize,
    },
    IndexAccess {
        base: MirOperand,
        index: MirOperand,
    },
    ArrayLit {
        ty: MirType,
        elements: Vec<MirOperand>,
    },
    /// A built-in `List<T>` method call. The receiver is the list value;
    /// `arg` carries the single extra operand for `push` (the pushed
    /// value) and `get` (the index), and is `None` for `len`, `is_empty`,
    /// and `pop`. `elem_ty` is the element type `T`, which the back end
    /// uses to size element slots uniformly and to decide whether the
    /// elements are GC pointers.
    ListMethod {
        op: ListMethodOp,
        receiver: MirOperand,
        arg: Option<MirOperand>,
        elem_ty: MirType,
    },
    Cast {
        operand: MirOperand,
        target: MirType,
    },
    ClosureCreate {
        fn_name: String,
        captures: Vec<MirOperand>,
        /// Capture types, parallel to `captures`. The back end uses these
        /// to size the capture record, decide which capture slots hold GC
        /// pointers, and copy each value into its slot. Capture analysis
        /// orders GC-pointer captures first so the runtime's leading
        /// `capture_ptr_count` pointer-slot contract holds.
        capture_tys: Vec<MirType>,
    },
    /// Read a captured value from the env record of a lifted closure body.
    /// The env is the lifted body's leading parameter (a raw pointer-width
    /// value). Slot `slot` lives at byte offset `slot * 8`; the back end
    /// loads a pointer-width word and narrows it to `ty`.
    EnvLoad {
        env: MirOperand,
        slot: usize,
        ty: MirType,
    },
    /// Invoke a closure value through its function pointer. The back end
    /// loads the function pointer and the capture env from the `Closure`
    /// object, then emits an indirect call passing the env as the leading
    /// argument followed by `args`. The lifted body's signature is
    /// uniformly `(env_ptr, <user params...>) -> ret`, independent of the
    /// capture count or types, so the call site needs only the user
    /// parameter and return types.
    ClosureCall {
        closure: MirOperand,
        args: Vec<MirOperand>,
        /// The user (non-env) parameter types, used to build the indirect
        /// call signature.
        param_tys: Vec<MirType>,
        ret_ty: MirType,
    },
    /// Unsize a concrete value to a `dyn Trait` value. The back end
    /// allocates a two-slot fat pointer `{ data, vtable }`: slot 0 holds
    /// the concrete value (`value`), slot 1 holds the address of the
    /// `(concrete_type, trait)` vtable. The result is a single GC pointer.
    DynCoerce {
        value: MirOperand,
        /// The concrete source type, used to pick and emit the vtable.
        concrete_ty: MirType,
        /// The target trait's short name.
        trait_name: String,
        /// The trait's method names in declaration order (vtable slots).
        methods: Vec<String>,
    },
    /// Raw FFI pointer load: read the pointee of type `pointee` from the
    /// address `addr` (a pointer-width value). The back end emits a single
    /// Cranelift `load` of the pointee's machine type. See
    /// `docs/v2/specs/std-ffi.md`.
    PtrLoad {
        addr: MirOperand,
        pointee: MirType,
    },
    /// Raw FFI pointer offset: `addr + count * sizeof(pointee)`, yielding a
    /// new pointer-width value. `count` is a native `Int`.
    PtrOffset {
        addr: MirOperand,
        count: MirOperand,
        pointee: MirType,
    },
    /// Raw FFI null check: true when `addr` is the null pointer (0).
    PtrIsNull {
        addr: MirOperand,
    },
    /// The null `CPtr<T>` constant (pointer-width 0).
    PtrNull,
    /// The address of a top-level function as a C function pointer. The
    /// back end emits `func_addr` for the named symbol. Used when a
    /// non-capturing top-level function is passed where a `CFnPtr` is
    /// expected. The function is compiled under the platform C ABI, so the
    /// resulting pointer is callable directly by C. See
    /// `docs/v2/specs/std-ffi.md`.
    FnAddr {
        mangled: String,
    },
    /// Raw FFI allocation: malloc `count * sizeof(pointee)` bytes through
    /// the runtime `raven_ffi_alloc`, returning a pointer-width value. The
    /// memory is outside the GC heap; the caller must `PtrFree` it.
    PtrAlloc {
        count: MirOperand,
        pointee: MirType,
    },
    /// Dynamic dispatch through a `dyn Trait` value. The back end loads
    /// the data and vtable words from the receiver's fat pointer box,
    /// loads the method pointer at `slot` from the vtable, and indirect
    /// calls it with the data word as the receiver plus `args`.
    VirtualCall {
        receiver: MirOperand,
        /// The dispatched method's vtable slot index (trait method order).
        slot: usize,
        /// The remaining (non-receiver) arguments.
        args: Vec<MirOperand>,
        /// Cranelift signature shape: the non-receiver parameter types
        /// and the return type, so the back end can build the indirect
        /// call signature.
        param_tys: Vec<MirType>,
        ret_ty: MirType,
    },
    /// `to_any<T>(v)`: box `value` (of `value_ty`) into a fresh `Any`,
    /// tagging it with `value_ty`'s runtime type id. The back end interns
    /// the id, widens a scalar value into the eight-byte payload, and sets
    /// the GC-pointer flag from `value_ty`. See
    /// `docs/v2/specs/runtime-reflection.md`.
    AnyBox {
        value: MirOperand,
        value_ty: MirType,
    },
    /// `cast<T>(a)`: checked downcast of the `Any` `any` to `Option<T>`.
    /// The back end compares the box's runtime type id against `target_ty`'s
    /// id; on a match it builds `Some(payload-as-T)`, otherwise `None`.
    /// `option_ty` is the `Option<T>` result type used to lay out the enum.
    AnyCast {
        any: MirOperand,
        target_ty: MirType,
        option_ty: MirType,
    },
    /// `type_name_of(a)`: the runtime type name of the value in `any`, as a
    /// fresh `String`.
    AnyTypeName {
        any: MirOperand,
    },
    /// `field_names_of(a)`: the struct field names of the value in `any`, as
    /// a fresh `List<String>` (empty for a non-struct).
    AnyFieldNames {
        any: MirOperand,
    },
    /// `get_field(a, name)`: the field named `name` of the struct in `any`,
    /// boxed as `Option<Any>` (`None` when absent or not a struct). The
    /// back end calls the runtime, then wraps the returned pointer (or
    /// null) into the option.
    AnyGetField {
        any: MirOperand,
        name: MirOperand,
        option_ty: MirType,
    },
    /// `set_field(a, name, value)`: write `value` into the field named `name`
    /// of the struct in `any`. Evaluates to `Unit`; the back end calls the
    /// runtime writer.
    AnySetField {
        any: MirOperand,
        name: MirOperand,
        value: MirOperand,
    },
}

/// One statement inside a basic block.
#[derive(Debug, Clone)]
pub enum MirStatement {
    Assign {
        dst: MirLocal,
        rvalue: MirRvalue,
    },
    /// Store `value` into field slot `index` of the struct or enum object
    /// `base` points to. The back end loads the object's field base
    /// pointer (the same base `FieldAccess` reads from) and writes the
    /// value at the slot's byte offset. `base` is an already-rooted GC
    /// pointer, so the written value becomes reachable through it; no new
    /// root is needed.
    StoreField {
        base: MirOperand,
        /// Field slot index in declaration order.
        index: usize,
        value: MirOperand,
    },
    /// Store `value` into element slot `index` of the `List` object
    /// `base` points to. The back end bounds-checks `index` against the
    /// list length (panicking on an out-of-range index, matching the read
    /// path) and writes the value at `base + index * element_size`.
    StoreIndex {
        base: MirOperand,
        index: MirOperand,
        value: MirOperand,
    },
    /// Raw FFI pointer store: write `value` (of type `pointee`) to the
    /// address `addr`. The back end emits a single Cranelift `store` of the
    /// pointee's machine type. See `docs/v2/specs/std-ffi.md`.
    PtrStore {
        addr: MirOperand,
        value: MirOperand,
        pointee: MirType,
    },
    /// Raw FFI free: hand `addr` to the runtime `raven_ffi_free`.
    PtrFree {
        addr: MirOperand,
    },
    /// Store `value` into a mutable module-level global's data slot. `name` is
    /// the global's mangled symbol. The slot is registered as a permanent GC
    /// root at startup, so storing a heap value into it keeps that value
    /// reachable; no new root is needed.
    StoreGlobal {
        name: String,
        value: MirOperand,
    },
    StorageLive(MirLocal),
    StorageDead(MirLocal),
    Nop,
}

/// The terminator that closes a basic block.
#[derive(Debug, Clone)]
pub enum MirTerminator {
    Goto(MirBlockId),
    SwitchInt {
        discriminant: MirOperand,
        targets: Vec<(i64, MirBlockId)>,
        otherwise: MirBlockId,
    },
    SwitchEnum {
        discriminant: MirOperand,
        targets: Vec<(usize, MirBlockId)>,
        otherwise: Option<MirBlockId>,
    },
    Return(MirOperand),
    Unreachable,
}

/// Declaration of one local variable.
#[derive(Debug, Clone)]
pub struct MirLocalDecl {
    pub name: String,
    pub ty: MirType,
    /// Whether this local participates in the function's parameter list.
    pub is_param: bool,
}

/// One basic block.
#[derive(Debug, Clone)]
pub struct MirBlock {
    pub id: MirBlockId,
    pub statements: Vec<MirStatement>,
    pub terminator: MirTerminator,
}

/// A function in MIR.
#[derive(Debug, Clone)]
pub struct MirFunction {
    /// Mangled, monomorphized name.
    pub name: String,
    /// Source name from HIR (no mangling).
    pub origin: String,
    /// Parameter locals, in declaration order.
    pub params: Vec<MirLocal>,
    pub ret_ty: MirType,
    pub locals: Vec<MirLocalDecl>,
    pub blocks: Vec<MirBlock>,
    pub entry: MirBlockId,
    pub span: Span,
    /// True when the function body registers at least one `defer`. Codegen
    /// opens a runtime defer frame on entry and runs it at every return
    /// path only for such functions. See `docs/v2/specs/defer.md`.
    pub has_defer: bool,
}

impl MirFunction {
    /// Look up a local's declaration.
    pub fn local_decl(&self, l: MirLocal) -> &MirLocalDecl {
        &self.locals[l.0 as usize]
    }

    /// Iterate the parameter declarations (in order).
    pub fn param_decls(&self) -> impl Iterator<Item = &MirLocalDecl> {
        self.params.iter().map(|p| self.local_decl(*p))
    }
}

/// A foreign function declared in an `extern "C"` block. The back end
/// declares each as an imported C-ABI symbol; a call site referencing
/// `name` resolves to it. Foreign functions have no Raven body.
#[derive(Debug, Clone)]
pub struct MirExternFn {
    /// The raw C symbol name, used verbatim as the link-time symbol.
    pub name: String,
    /// Parameter types in declaration order.
    pub params: Vec<MirType>,
    /// Return type, or `MirType::Unit` for a `void` return.
    pub ret: MirType,
    /// True for a variadic C function (`fun printf(fmt: CStr, ...)`): a call
    /// may pass extra C-FFI integer/pointer arguments after the fixed ones.
    pub variadic: bool,
}

/// One C-layout field of a `@repr(C)` struct that crosses the FFI by
/// value: its C byte offset within the parent struct, and either a scalar
/// FFI type or a nested `@repr(C)` struct.
#[derive(Debug, Clone)]
pub struct ReprCField {
    pub offset: u32,
    pub kind: ReprCFieldKind,
}

/// A `@repr(C)` field is either a C scalar or a nested `@repr(C)` struct.
/// The parent heap slot at a nested field's index holds a GC pointer to the
/// nested struct object; the C image inlines the nested struct's bytes.
#[derive(Debug, Clone)]
pub enum ReprCFieldKind {
    Scalar(MirFfiTy),
    Nested {
        /// Mangled name of the nested struct type, to reconstruct its object
        /// on a return.
        mangle: String,
        layout: ReprCLayout,
    },
}

/// The C memory layout of a `@repr(C)` struct: its total byte size and each
/// field's C offset and kind, in declaration order. The back end uses this
/// to move the struct's heap fields through the registers the platform C ABI
/// passes it in, and to rebuild a returned one.
#[derive(Debug, Clone)]
pub struct ReprCLayout {
    pub size: u32,
    pub fields: Vec<ReprCField>,
}

impl ReprCLayout {
    /// Every leaf scalar field as `(absolute C offset, ffi type)`, flattening
    /// nested struct fields. Used by the ABI register classifier, for which a
    /// nested struct behaves exactly like its fields inlined at its offset.
    pub fn leaves(&self) -> Vec<(u32, MirFfiTy)> {
        let mut out = Vec::new();
        self.collect_leaves(0, &mut out);
        out
    }

    fn collect_leaves(&self, base: u32, out: &mut Vec<(u32, MirFfiTy)>) {
        for f in &self.fields {
            match &f.kind {
                ReprCFieldKind::Scalar(ffi) => out.push((base + f.offset, ffi.clone())),
                ReprCFieldKind::Nested { layout, .. } => {
                    layout.collect_leaves(base + f.offset, out)
                }
            }
        }
    }
}

/// One field of a reflectable type: its declared name, the mangled name
/// of its type (so the back end can resolve the field's runtime type id),
/// and whether the field slot holds a GC pointer.
#[derive(Debug, Clone)]
pub struct ReflectField {
    pub name: String,
    pub type_mangle: String,
    pub is_gc_ptr: bool,
}

/// Runtime reflection metadata for one monomorphic type. The back end
/// emits one `raven_type_register` call per entry at program startup so a
/// boxed value of the type can report its name and (for a struct) walk its
/// fields. See `docs/v2/specs/runtime-reflection.md`.
#[derive(Debug, Clone)]
pub struct ReflectType {
    /// The rendered type name (`Point`, `Int`, `Pair<Int, String>`).
    pub name: String,
    /// True when the type is a struct with reflectable fields.
    pub is_struct: bool,
    /// Fields in declaration order. Empty for a non-struct.
    pub fields: Vec<ReflectField>,
}

/// A mutable module-level global. The back end allocates a data slot for it,
/// roots heap-valued slots for the whole program, and runs the synthesized
/// `__raven_init_globals` function to set its initial value before `main`.
#[derive(Debug, Clone)]
pub struct MirGlobal {
    /// The global's mangled symbol name (matches `GlobalLoad`/`StoreGlobal`).
    pub name: String,
    /// The global's type, used to size the slot's load/store and to decide
    /// (via the back end's `is_gc_pointer`) whether the slot must be rooted.
    pub ty: MirType,
}

/// One full MIR program: a flat list of monomorphic functions plus the
/// foreign functions declared in `extern` blocks.
#[derive(Debug, Clone)]
pub struct MirProgram {
    pub functions: Vec<MirFunction>,
    /// Mutable module-level globals (a `let` at file scope). The back end
    /// emits a data slot per global and roots the heap-valued ones.
    pub globals: Vec<MirGlobal>,
    /// Foreign functions declared in `extern "C"` blocks. Declared as
    /// imported symbols by the back end and resolved at link time.
    pub externs: Vec<MirExternFn>,
    /// C layouts of `@repr(C)` structs, keyed by mangled struct name (the
    /// key `MirType::mangle` produces). Populated only for structs that
    /// can cross the FFI by value; the back end consults it at an extern
    /// call boundary to marshal a struct argument or return.
    pub repr_c_structs: HashMap<String, ReprCLayout>,
    /// Runtime reflection metadata, keyed by mangled type name. Populated
    /// for every type boxed into an `Any` (and transitively their field
    /// types). The back end registers each with the runtime at startup.
    pub reflect_types: HashMap<String, ReflectType>,
}

impl MirProgram {
    pub fn new() -> Self {
        Self {
            functions: Vec::new(),
            globals: Vec::new(),
            externs: Vec::new(),
            repr_c_structs: HashMap::new(),
            reflect_types: HashMap::new(),
        }
    }
}

impl Default for MirProgram {
    fn default() -> Self {
        Self::new()
    }
}
