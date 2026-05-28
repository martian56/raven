//! MIR data types: functions, basic blocks, statements, terminators,
//! rvalues, and operands.
//!
//! The shape mirrors a scoped-down version of Rust's MIR. Each function
//! is a control flow graph: a flat vector of basic blocks, each ending
//! in exactly one terminator. Local variables are dense indices into
//! the function's locals table.

use crate::resolve::DeclId;
use crate::span::Span;

use super::ty::MirType;

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
}

/// One full MIR program: a flat list of monomorphic functions plus the
/// foreign functions declared in `extern` blocks.
#[derive(Debug, Clone)]
pub struct MirProgram {
    pub functions: Vec<MirFunction>,
    /// Foreign functions declared in `extern "C"` blocks. Declared as
    /// imported symbols by the back end and resolved at link time.
    pub externs: Vec<MirExternFn>,
}

impl MirProgram {
    pub fn new() -> Self {
        Self {
            functions: Vec::new(),
            externs: Vec::new(),
        }
    }
}

impl Default for MirProgram {
    fn default() -> Self {
        Self::new()
    }
}
