//! HIR -> MIR lowering.
//!
//! Entry point: [`lower_function`] takes one HIR function plus a
//! concrete substitution table (the monomorphization pass owns the
//! substitution machinery) and produces a [`MirFunction`].
//!
//! Submodules organize the lowering by syntactic category. They share
//! the [`LowerCx`] context defined here, which owns the in-progress
//! [`FunctionBuilder`] and tracks the lexical loop and `Self` stack.

pub mod closure;
pub mod expr;
pub mod pattern;
pub mod stmt;

use std::collections::HashMap;

use crate::hir::{HirEnum, HirFn, HirStruct};
use crate::resolve::DeclId;
use crate::tycheck::ty::ParamId;
use crate::tycheck::Ty;

use super::builder::FunctionBuilder;
use super::ir::{MirBlockId, MirFunction, MirLocal};
use super::ty::MirType;

/// Mapping from a generic parameter to its concrete substitute.
pub type SubstMap = HashMap<ParamId, Ty>;

/// Declaration tables the expression lowering consults to resolve
/// struct field offsets and enum variant payloads. Keyed by the source
/// type name, which is stable through monomorphization for the
/// non-generic types the MVP supports.
#[derive(Clone, Default)]
pub struct DeclTables<'a> {
    pub structs: HashMap<String, &'a HirStruct>,
    pub enums: HashMap<String, &'a HirEnum>,
}

/// Mapping from a source identifier name to its current MIR local.
/// Re-binding shadows the previous entry.
pub type Scope = HashMap<String, MirLocal>;

/// One pending loop the caller is lowering. Used to wire `break` and
/// `continue` to the right blocks and (for `break` with a value) the
/// loop's result local.
pub struct LoopFrame {
    pub header: MirBlockId,
    pub continuation: MirBlockId,
    pub result: Option<MirLocal>,
}

/// Per-function lowering context.
pub struct LowerCx<'a> {
    pub builder: FunctionBuilder,
    pub current: MirBlockId,
    pub subst: &'a SubstMap,
    pub scopes: Vec<Scope>,
    pub loops: Vec<LoopFrame>,
    /// Calls that the monomorphizer should specialize once the function
    /// finishes lowering. Each entry is `(decl_id, concrete_type_args)`.
    pub pending_calls: Vec<(DeclId, Vec<Ty>)>,
    /// Struct and enum declaration tables for field and variant lookup.
    pub decls: &'a DeclTables<'a>,
    /// Set when control flow diverged (a `return`, `break`, or `continue`
    /// closed the current block and rolled a fresh dead block). The
    /// function finalizer uses this to mark a trailing empty block as
    /// `Unreachable` rather than falsely treating a real empty-bodied
    /// function (for example `fun f() -> Int = 1`) as dead.
    pub diverged: bool,
    /// Pending deferred expressions, in declaration order. A `defer e`
    /// statement pushes a clone of `e` here; the deferred work runs in
    /// reverse (LIFO) order at each block exit and at every `return`.
    /// See `src/hir` defer lowering and `docs/v2/specs/defer.md`.
    pub defers: Vec<crate::hir::expr::HirExpr>,
    /// Lifted closure bodies produced while lowering this function. Each
    /// lambda expression lifts its body into a standalone `MirFunction`
    /// whose leading parameter is the capture environment. The functions
    /// are returned alongside the enclosing function so the monomorphizer
    /// can add them to the program and codegen can resolve the closure's
    /// function pointer.
    pub lifted: Vec<MirFunction>,
    /// Monotonic counter used to mint unique lifted closure names within
    /// one enclosing function. Combined with the enclosing function's
    /// mangled name to stay globally unique.
    pub lambda_seq: u32,
    /// Mangled name of the enclosing function, used to derive lifted
    /// closure names so two enclosing functions never collide.
    pub enclosing: String,
}

impl LowerCx<'_> {
    /// Look up an identifier across the scope stack.
    pub fn lookup(&self, name: &str) -> Option<MirLocal> {
        for scope in self.scopes.iter().rev() {
            if let Some(l) = scope.get(name) {
                return Some(*l);
            }
        }
        None
    }

    pub fn bind(&mut self, name: String, local: MirLocal) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name, local);
        }
    }

    pub fn push_scope(&mut self) {
        self.scopes.push(Scope::new());
    }

    pub fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    /// Number of deferred expressions registered so far. A block records
    /// this on entry so it can flush only the defers added inside it.
    pub fn defer_mark(&self) -> usize {
        self.defers.len()
    }

    /// Emit the deferred expressions in `self.defers[from..]` into the
    /// current block in reverse (LIFO) declaration order, lowered for
    /// their side effects only. Does nothing when the current block has
    /// already been closed (for example after a `return`), since a dead
    /// block must stay empty. The defers are cloned, not consumed, so a
    /// `return` that escapes several blocks can re-emit the same defers.
    pub fn emit_defers_from(&mut self, from: usize) {
        if from >= self.defers.len() {
            return;
        }
        if self.builder.is_closed(self.current) {
            return;
        }
        // Clone the slice first: lowering each expression borrows `self`
        // mutably, so the pending list cannot be borrowed across the loop.
        let pending: Vec<crate::hir::expr::HirExpr> = self.defers[from..].to_vec();
        for e in pending.iter().rev() {
            let _ = expr::lower_expr(self, e);
        }
    }

    /// Emit every pending deferred expression in reverse order. Used at a
    /// `return`, which escapes all enclosing blocks at once.
    pub fn emit_all_defers(&mut self) {
        self.emit_defers_from(0);
    }
}

/// Apply the monomorphization substitution to a single type, walking
/// recursively. Inference variables are flattened to `Unit` (the type
/// checker should not leave any vars in HIR types, but we guard).
pub fn substitute(ty: &Ty, subst: &SubstMap) -> Ty {
    match ty {
        Ty::Param(p) => subst.get(p).cloned().unwrap_or(Ty::Error),
        Ty::Option(t) => Ty::Option(Box::new(substitute(t, subst))),
        Ty::List(t) => Ty::List(Box::new(substitute(t, subst))),
        Ty::SelfTy(t) => Ty::SelfTy(Box::new(substitute(t, subst))),
        Ty::Result(a, b) => Ty::Result(
            Box::new(substitute(a, subst)),
            Box::new(substitute(b, subst)),
        ),
        Ty::Struct { id, name, args } => Ty::Struct {
            id: *id,
            name: name.clone(),
            args: args.iter().map(|a| substitute(a, subst)).collect(),
        },
        Ty::Enum { id, name, args } => Ty::Enum {
            id: *id,
            name: name.clone(),
            args: args.iter().map(|a| substitute(a, subst)).collect(),
        },
        Ty::Function { params, ret } => Ty::Function {
            params: params.iter().map(|a| substitute(a, subst)).collect(),
            ret: Box::new(substitute(ret, subst)),
        },
        other => other.clone(),
    }
}

/// Translate an HIR type into a concrete `MirType` under the current
/// substitution.
pub fn mir_ty(ty: &Ty, subst: &SubstMap) -> MirType {
    MirType::from_ty(&substitute(ty, subst))
}

/// The product of lowering one HIR function: the finished `MirFunction`,
/// the nested call sites the monomorphizer should compile, and any lifted
/// closure body functions produced while lowering lambda expressions.
pub struct LoweredFunction {
    pub func: MirFunction,
    pub pending: Vec<(DeclId, Vec<Ty>)>,
    pub lifted: Vec<MirFunction>,
}

/// Lower one HIR function under the given substitution. Returns the
/// finished `MirFunction`, the list of nested call sites the
/// monomorphizer should compile, and any lifted closure bodies.
pub fn lower_function(
    mangled: String,
    hir: &HirFn,
    subst: &SubstMap,
    decls: &DeclTables<'_>,
) -> LoweredFunction {
    let ret_ty = mir_ty(&hir.ret, subst);
    let mut builder =
        FunctionBuilder::new(mangled.clone(), hir.name.clone(), ret_ty, hir.span.clone());

    let mut param_scope = Scope::new();
    for (name, ty, _) in &hir.params {
        let mty = mir_ty(ty, subst);
        let local = builder.add_param(name.clone(), mty);
        param_scope.insert(name.clone(), local);
    }

    let entry = builder.new_block();
    let mut cx = LowerCx {
        builder,
        current: entry,
        subst,
        scopes: vec![param_scope],
        loops: Vec::new(),
        pending_calls: Vec::new(),
        decls,
        diverged: false,
        defers: Vec::new(),
        lifted: Vec::new(),
        lambda_seq: 0,
        enclosing: mangled,
    };

    let body = hir
        .body
        .as_ref()
        .expect("HIR function passed to MIR lowering must have a body");

    // Lower the body. The result of the block (its tail or `()`) is
    // the function's return value.
    let result = stmt::lower_block(&mut cx, body);

    if !cx.builder.is_closed(cx.current) {
        if cx.diverged && cx.builder.is_empty_open(cx.current) {
            // The body's final action was a `return` (or `break` /
            // `continue`) which closed its own block and rolled a fresh
            // dead one. Mark the dead block `Unreachable` to keep the dump
            // tidy. A non-diverged empty block carries a real tail value
            // (for example `fun f() -> Int = 1`) and must return it.
            cx.builder
                .close_block(cx.current, super::ir::MirTerminator::Unreachable);
        } else {
            cx.builder
                .close_block(cx.current, super::ir::MirTerminator::Return(result));
        }
    }

    let pending = std::mem::take(&mut cx.pending_calls);
    let lifted = std::mem::take(&mut cx.lifted);
    LoweredFunction {
        func: cx.builder.finish(entry),
        pending,
        lifted,
    }
}
