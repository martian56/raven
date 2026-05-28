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

/// A compile-time constant.
#[derive(Debug, Clone, PartialEq)]
pub enum MirConstant {
    Unit,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Char(char),
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
    Cast {
        operand: MirOperand,
        target: MirType,
    },
    ClosureCreate {
        fn_name: String,
        captures: Vec<MirOperand>,
    },
}

/// One statement inside a basic block.
#[derive(Debug, Clone)]
pub enum MirStatement {
    Assign { dst: MirLocal, rvalue: MirRvalue },
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

/// One full MIR program: a flat list of monomorphic functions.
#[derive(Debug, Clone)]
pub struct MirProgram {
    pub functions: Vec<MirFunction>,
}

impl MirProgram {
    pub fn new() -> Self {
        Self {
            functions: Vec::new(),
        }
    }
}

impl Default for MirProgram {
    fn default() -> Self {
        Self::new()
    }
}
