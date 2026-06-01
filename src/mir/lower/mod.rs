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

/// One free function's shape, used at a call site to specialize a
/// generic call. The declared parameter and return types may contain
/// `Ty::Param`; unifying them against the concrete argument types at a
/// call site yields the substitution for that instantiation.
#[derive(Clone)]
pub struct FnEntry {
    /// The function's monomorphization decl id, matching `collect_roots`.
    pub decl: DeclId,
    /// Declared parameter types, in order (may contain `Ty::Param`).
    pub params: Vec<Ty>,
    /// Declared return type (may contain `Ty::Param`).
    pub ret: Ty,
    /// The function's generic parameters in declaration order. Empty for
    /// a non-generic function. The order fixes the mangled-name suffix so
    /// the call site and the worklist agree on each instantiation's name.
    pub generic_params: Vec<ParamId>,
}

/// Index of free functions by their source name, consulted when lowering
/// a call so a generic callee is specialized to a per-instantiation
/// symbol. Built once by the monomorphization driver.
pub type FnIndex = HashMap<String, FnEntry>;

/// One impl method's shape, consulted at a method call site so a generic
/// method (a method whose declared types carry `Ty::Param`, for example
/// `impl<T> Box<T> { fun unwrap(self) -> T }`) is specialized for the
/// concrete receiver type. A concrete-receiver method (no `Ty::Param`)
/// is already a monomorphization root, so the call site only queues an
/// instantiation when the method is generic.
#[derive(Clone)]
pub struct MethodEntry {
    /// The method's monomorphization decl id, matching `collect_roots`.
    pub decl: DeclId,
    /// The implementing type as written on the `impl` block. May carry
    /// the impl's `Ty::Param`s (`Box<T>`); matching it against the
    /// concrete receiver type binds them.
    pub self_ty: Ty,
    /// The method's user parameter types, in order, excluding the leading
    /// `self`. May carry `Ty::Param`. Matched against the concrete
    /// argument types to bind any method-level parameters.
    pub params: Vec<Ty>,
    /// The method's own generic parameters, in declaration order: those
    /// the method introduces (`fun mapped<U>`) that do not appear in the
    /// implementing type. They are encoded into the mangled symbol so two
    /// instantiations of the method at different method-level type
    /// arguments do not collide. Empty for a method whose only generic
    /// parameters come from the implementing type.
    pub method_params: Vec<ParamId>,
    /// True when the method's declared types carry any `Ty::Param`, so it
    /// must be specialized at each call site. A concrete-receiver method
    /// is `false` and is reached through its own root instead.
    pub generic: bool,
}

/// Index of impl methods by method name, consulted at a method call site
/// to specialize a generic method to its per-receiver symbol. Several
/// impls may define a method of the same name on different types, so the
/// call site picks the entry whose `self_ty` matches the concrete
/// receiver. Built once by the monomorphization driver.
pub type MethodIndex = HashMap<String, Vec<MethodEntry>>;

/// Declaration tables the expression lowering consults to resolve
/// struct field offsets and enum variant payloads. Keyed by the source
/// type name, which is stable through monomorphization for the
/// non-generic types the MVP supports.
#[derive(Clone, Default)]
pub struct DeclTables<'a> {
    pub structs: HashMap<String, &'a HirStruct>,
    pub enums: HashMap<String, &'a HirEnum>,
    /// Free-function index used to specialize generic calls.
    pub functions: FnIndex,
    /// Impl-method index used to specialize generic methods at their call
    /// sites, the same way `functions` specializes generic free calls.
    pub methods: MethodIndex,
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
    /// finishes lowering. Each entry is `(decl_id, substitution)`, where
    /// the substitution maps the callee's generic parameters to the
    /// concrete types derived at this call site.
    pub pending_calls: Vec<(DeclId, SubstMap)>,
    /// Struct and enum declaration tables for field and variant lookup.
    pub decls: &'a DeclTables<'a>,
    /// Set when control flow diverged (a `return`, `break`, or `continue`
    /// closed the current block and rolled a fresh dead block). The
    /// function finalizer uses this to mark a trailing empty block as
    /// `Unreachable` rather than falsely treating a real empty-bodied
    /// function (for example `fun f() -> Int = 1`) as dead.
    pub diverged: bool,
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
    /// Runtime reflection metadata collected while lowering this function,
    /// keyed by mangled type name. Each type boxed into an `Any` (and
    /// transitively its field types) gets one entry; the monomorphizer
    /// drains these into the program for the back end to register.
    pub reflect_types: HashMap<String, super::ir::ReflectType>,
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

/// Compute the monomorphized symbol for an instantiation: the base
/// symbol followed by `$<MangledType>` for each generic parameter, in
/// declaration order. A non-generic function (empty `generic_params`)
/// keeps its base symbol. The call site and the worklist both call this
/// so they agree on every instantiation's name.
pub fn mono_symbol(base: &str, generic_params: &[ParamId], subst: &SubstMap) -> String {
    if generic_params.is_empty() {
        return base.to_string();
    }
    let mut s = base.to_string();
    for p in generic_params {
        let concrete = subst.get(p).cloned().unwrap_or(Ty::Error);
        s.push('$');
        s.push_str(&MirType::from_ty(&concrete).mangle());
    }
    s
}

/// Compute the monomorphized symbol for a method instantiation. The base
/// is the concrete implementing type's mangle followed by `$<method>`
/// (`Box_Int$mapped`). When the method introduces its own generic
/// parameters that do not appear in the implementing type (a method-level
/// `<U>`, distinct from the impl's `<T>`), each is appended as
/// `$<MangledType>` in declaration order, so two instantiations of the
/// same method at different method-level type arguments get distinct
/// symbols (`Box_Int$mapped$Int` and `Box_Int$mapped$Bool`). The call
/// site and the worklist both call this so they agree on every
/// instantiation's name. The implementing type's own arguments are
/// already encoded by `concrete_self_mangle`, so only the method-level
/// parameters contribute a suffix here.
pub fn method_mono_symbol(
    concrete_self_mangle: &str,
    method: &str,
    method_params: &[ParamId],
    subst: &SubstMap,
) -> String {
    let mut s = super::ty::method_symbol(concrete_self_mangle, method);
    for p in method_params {
        let concrete = subst.get(p).cloned().unwrap_or(Ty::Error);
        s.push('$');
        s.push_str(&MirType::from_ty(&concrete).mangle());
    }
    s
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
        Ty::Ffi(crate::tycheck::FfiTy::CPtr(inner)) => Ty::Ffi(crate::tycheck::FfiTy::CPtr(
            Box::new(substitute(inner, subst)),
        )),
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
    pub pending: Vec<(DeclId, SubstMap)>,
    pub lifted: Vec<MirFunction>,
    pub reflect_types: HashMap<String, super::ir::ReflectType>,
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
        lifted: Vec::new(),
        lambda_seq: 0,
        enclosing: mangled,
        reflect_types: HashMap::new(),
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
    let reflect_types = std::mem::take(&mut cx.reflect_types);
    LoweredFunction {
        func: cx.builder.finish(entry),
        pending,
        lifted,
        reflect_types,
    }
}
