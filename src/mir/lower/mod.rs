//! HIR -> MIR lowering.
//!
//! Entry point: [`lower_function`] takes one HIR function plus a
//! concrete substitution table (the monomorphization pass owns the
//! substitution machinery) and produces a [`MirFunction`].
//!
//! Submodules organize the lowering by syntactic category. They share
//! the [`LowerCx`] context defined here, which owns the in-progress
//! [`FunctionBuilder`] and tracks the lexical loop and `Self` stack.

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

/// Lower one HIR function under the given substitution. Returns the
/// finished `MirFunction` plus the list of nested call sites the
/// monomorphizer should compile.
pub fn lower_function(
    mangled: String,
    hir: &HirFn,
    subst: &SubstMap,
    decls: &DeclTables<'_>,
) -> (MirFunction, Vec<(DeclId, Vec<Ty>)>) {
    let ret_ty = mir_ty(&hir.ret, subst);
    let mut builder = FunctionBuilder::new(mangled, hir.name.clone(), ret_ty, hir.span.clone());

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
    };

    let body = hir
        .body
        .as_ref()
        .expect("HIR function passed to MIR lowering must have a body");

    // Lower the body. The result of the block (its tail or `()`) is
    // the function's return value.
    let result = stmt::lower_block(&mut cx, body);

    if !cx.builder.is_closed(cx.current) {
        if cx.builder.is_empty_open(cx.current) {
            // The body's final action was a `return` which closed its
            // own block and rolled a fresh dead one. Leave the dead
            // block as `Unreachable` to keep the dump tidy.
            cx.builder
                .close_block(cx.current, super::ir::MirTerminator::Unreachable);
        } else {
            cx.builder
                .close_block(cx.current, super::ir::MirTerminator::Return(result));
        }
    }

    let pending = std::mem::take(&mut cx.pending_calls);
    (cx.builder.finish(entry), pending)
}
