//! Monomorphization driver.
//!
//! Starts from the program's "roots" (every non-generic top-level
//! function), lowers each root to MIR, and walks the resulting MIR
//! for any call site that targets a still-generic function. Each
//! `(decl, concrete_args)` pair is queued onto a worklist and lowered
//! exactly once. The result is a flat list of monomorphic MIR
//! functions ready for codegen.
//!
//! Each call site that targets a generic free function is specialized
//! during HIR-to-MIR lowering: the callee's declared parameter types
//! (carrying `Ty::Param`) are matched against the concrete argument
//! types to build a substitution, and the instantiation is queued here.
//! The worklist lowers each `(decl, substitution)` pair exactly once,
//! mangling the function name with the concrete type arguments so two
//! instantiations of the same generic function get distinct symbols. A
//! generic call inside a generic body is specialized transitively: the
//! enclosing substitution is applied to the argument types before they
//! are matched, so a chain of generic calls resolves to ground types.

use std::collections::{HashMap, HashSet};

use crate::error::RavenError;
use crate::hir::{HirFn, HirItemKind, HirProgram, HirStruct};
use crate::resolve::DeclId;
use crate::tycheck::ty::ParamId;
use crate::tycheck::Ty;

use super::ir::MirProgram;
use super::lower::{
    lower_function, mono_symbol, DeclTables, FnEntry, FnIndex, MethodEntry, MethodIndex, SubstMap,
};

/// One entry in the monomorphization worklist: a declaration plus the
/// substitution that specializes its generic parameters.
type Item = (DeclId, SubstMap);

/// Map from a function declaration to its HIR shape, used by the
/// worklist to retrieve bodies.
type HirIndex<'a> = HashMap<DeclId, &'a HirFn>;

/// Map from a function declaration to the base symbol the back end uses.
/// Free functions keep their source name; impl methods get a per-type
/// symbol (`<TypeMangle>$<method>`) so several types implementing a
/// method of the same name do not collide.
type SymbolIndex = HashMap<DeclId, String>;

/// Map from a function declaration to its generic parameters in
/// declaration order. The order fixes the mangled-name suffix.
type GenericIndex = HashMap<DeclId, Vec<ParamId>>;

/// Map from an impl-method declaration to its implementing type, method
/// name, and method-level generic parameters. The worklist computes a
/// generic method's per-instantiation symbol by substituting the concrete
/// type arguments into the implementing type, mangling the result, and
/// appending the concrete method-level type arguments, so the symbol
/// matches what the method call site recomputes from the concrete receiver
/// type and the inferred method-level arguments (`Box_Int$unwrap`, or
/// `Box_Int$mapped$Bool` for a method-level `<U>` bound to `Bool`).
type MethodSymbolIndex = HashMap<DeclId, MethodSymbolEntry>;

/// The implementing type, method name, and method-level generic
/// parameters the worklist needs to recompute a generic method's
/// per-instantiation symbol.
struct MethodSymbolEntry {
    /// The implementing type as written on the `impl` block; may carry the
    /// impl's `Ty::Param`s, grounded by the instantiation's substitution.
    self_ty: Ty,
    /// The method's source name.
    method: String,
    /// The method's own generic parameters (those absent from the
    /// implementing type), in declaration order. Encoded into the symbol
    /// suffix so distinct method-level type arguments do not collide.
    method_params: Vec<ParamId>,
}

/// Run the full monomorphization pass.
pub fn monomorphize(hir: &HirProgram) -> Result<MirProgram, RavenError> {
    let mut program = MirProgram::new();
    program.externs = collect_externs(hir);
    program.repr_c_structs = collect_repr_c_structs(hir);
    program.globals = collect_globals(hir);
    let (index, roots, symbols, generics, fn_index, method_index, method_symbols) =
        collect_roots(hir);
    let decls = collect_decls(hir, fn_index, method_index);

    let mut seen: HashSet<(DeclId, Vec<MangleKey>)> = HashSet::new();
    let mut worklist: Vec<Item> = roots;

    while let Some((decl, subst)) = worklist.pop() {
        let generic_params = generics.get(&decl).cloned().unwrap_or_default();
        let key = (
            decl,
            generic_params
                .iter()
                .map(|p| MangleKey::from(&subst.get(p).cloned().unwrap_or(Ty::Error)))
                .collect(),
        );
        if !seen.insert(key) {
            continue;
        }
        let hir_fn = match index.get(&decl) {
            Some(f) => *f,
            None => continue,
        };
        // A method's symbol is the concrete implementing type followed by
        // `$<method>`, computed by substituting the instantiation's type
        // arguments into the declared implementing type. This matches what
        // the call site recomputes from the concrete receiver type, so a
        // generic method specialized at `Box<Int>` and the call to it both
        // name `Box_Int$unwrap`. A free function uses its base symbol with
        // the generic-parameter suffix. The implementing type already
        // carries its own concrete arguments, but a method-level generic
        // parameter (a `<U>` the method introduces that is absent from the
        // implementing type) contributes an extra suffix, so two
        // instantiations of the same method at different method-level type
        // arguments get distinct symbols (`Box_Int$mapped$Int` and
        // `Box_Int$mapped$Bool`).
        let mangled = match method_symbols.get(&decl) {
            Some(entry) => {
                let concrete_self = super::lower::substitute(&entry.self_ty, &subst);
                let concrete_self_mangle = super::ty::MirType::from_ty(&concrete_self).mangle();
                super::lower::method_mono_symbol(
                    &concrete_self_mangle,
                    &entry.method,
                    &entry.method_params,
                    &subst,
                )
            }
            None => {
                let base = symbols
                    .get(&decl)
                    .cloned()
                    .unwrap_or_else(|| hir_fn.name.clone());
                mono_symbol(&base, &generic_params, &subst)
            }
        };
        let lowered = lower_function(mangled, hir_fn, &subst, &decls);
        program.functions.push(lowered.func);
        // Lifted closure bodies are already monomorphic standalone
        // functions; add them directly so codegen can resolve each
        // closure's function pointer.
        for lifted in lowered.lifted {
            program.functions.push(lifted);
        }
        for next in lowered.pending {
            worklist.push(next);
        }
        // Reflection metadata for any type boxed in this function. The
        // back end registers each entry with the runtime at startup.
        for (mangle, info) in lowered.reflect_types {
            program.reflect_types.entry(mangle).or_insert(info);
        }
    }

    Ok(program)
}

/// Collect every foreign function from the HIR's extern blocks, lowering
/// each resolved signature to ground MIR types. The back end declares
/// these as imported C-ABI symbols.
fn collect_externs(hir: &HirProgram) -> Vec<super::ir::MirExternFn> {
    use super::ty::MirType;
    let mut out = Vec::new();
    for item in &hir.items {
        if let HirItemKind::Extern(ext) = &item.kind {
            for f in &ext.items {
                out.push(super::ir::MirExternFn {
                    name: f.name.clone(),
                    params: f.params.iter().map(MirType::from_ty).collect(),
                    ret: MirType::from_ty(&f.ret),
                    variadic: f.variadic,
                });
            }
        }
    }
    out
}

/// Collect a [`MirGlobal`] for every module-level `let` (a mutable global
/// with runtime storage). Declaration order is preserved, matching the order
/// the synthesized `__raven_init_globals` function assigns them.
fn collect_globals(hir: &HirProgram) -> Vec<super::ir::MirGlobal> {
    use super::ty::MirType;
    use crate::hir::lower::expr::global_symbol;
    let mut out = Vec::new();
    for item in &hir.items {
        if let HirItemKind::Let { name, ty, .. } = &item.kind {
            out.push(super::ir::MirGlobal {
                name: global_symbol(name),
                ty: MirType::from_ty(ty),
            });
        }
    }
    out
}

/// Build the C layout of every `@repr(C)` struct, keyed by the mangled
/// struct name the back end uses for its descriptor. Each field's C offset
/// is computed in declaration order with natural alignment, and the total
/// size is rounded to the struct's alignment. The tycheck pass has already
/// proved every field is an integer-class C scalar and the total is at
/// most one register, so the conversions here cannot fail; a non-scalar
/// field is skipped defensively rather than panicking.
fn collect_repr_c_structs(hir: &HirProgram) -> HashMap<String, super::ir::ReprCLayout> {
    // Index every struct declaration by name so a nested field can recurse.
    let mut structs: HashMap<&str, &HirStruct> = HashMap::new();
    for item in &hir.items {
        if let HirItemKind::Struct(s) = &item.kind {
            structs.insert(s.name.as_str(), s);
        }
    }

    let mut out = HashMap::new();
    for item in &hir.items {
        let HirItemKind::Struct(s) = &item.kind else {
            continue;
        };
        if !s.repr_c {
            continue;
        }
        if let Some((layout, _, _)) = build_repr_c_layout(s, &structs) {
            out.insert(s.name.clone(), layout);
        }
    }
    out
}

/// Build a `@repr(C)` struct's recursive C layout, returning the layout plus
/// its total size and alignment. Fields are laid out in declaration order,
/// each at its naturally aligned offset; a nested `@repr(C)` struct field
/// embeds the nested layout at its offset. Returns `None` on a field the type
/// checker would have rejected (a non-scalar, non-repr(C)-struct field).
fn build_repr_c_layout(
    s: &HirStruct,
    structs: &HashMap<&str, &HirStruct>,
) -> Option<(super::ir::ReprCLayout, u32, u32)> {
    use super::ir::{ReprCField, ReprCFieldKind, ReprCLayout};
    use super::ty::MirFfiTy;

    let mut offset: u32 = 0;
    let mut max_align: u32 = 1;
    let mut fields = Vec::with_capacity(s.fields.len());
    for (_, ty, _) in &s.fields {
        let (fsize, falign, kind) = match ty {
            Ty::Ffi(ffi) => {
                let (size, align) = ffi.c_scalar_layout()?;
                (size, align, ReprCFieldKind::Scalar(MirFfiTy::from_ffi(ffi)))
            }
            Ty::Struct { name, .. } => {
                let nested = structs.get(name.as_str())?;
                if !nested.repr_c {
                    return None;
                }
                let (nested_layout, nsize, nalign) = build_repr_c_layout(nested, structs)?;
                (
                    nsize,
                    nalign,
                    ReprCFieldKind::Nested {
                        mangle: nested.name.clone(),
                        layout: nested_layout,
                    },
                )
            }
            _ => return None,
        };
        offset = (offset + falign - 1) & !(falign - 1);
        fields.push(ReprCField { offset, kind });
        offset += fsize;
        max_align = max_align.max(falign);
    }
    let size = (offset + max_align - 1) & !(max_align - 1);
    Some((ReprCLayout { size, fields }, size, max_align))
}

/// Index every struct and enum declaration by its source name so the
/// expression lowering can resolve field offsets and variant payloads.
/// The free-function index built by [`collect_roots`] is folded in so a
/// generic call can be specialized at its call site.
fn collect_decls(hir: &HirProgram, functions: FnIndex, methods: MethodIndex) -> DeclTables<'_> {
    let mut tables = DeclTables {
        functions,
        methods,
        ..DeclTables::default()
    };
    for item in &hir.items {
        match &item.kind {
            HirItemKind::Struct(s) => {
                tables.structs.insert(s.name.clone(), s);
            }
            HirItemKind::Enum(e) => {
                tables.enums.insert(e.name.clone(), e);
            }
            _ => {}
        }
    }
    tables
}

/// Collect every top-level function plus impl methods. Returns the HIR
/// lookup table, the initial worklist of non-generic roots (each with an
/// empty substitution), the per-declaration base symbol used by the back
/// end, the per-declaration generic-parameter order, and the by-name
/// free-function index a call site consults to specialize a generic
/// call.
#[allow(clippy::type_complexity)]
fn collect_roots(
    hir: &HirProgram,
) -> (
    HirIndex<'_>,
    Vec<Item>,
    SymbolIndex,
    GenericIndex,
    FnIndex,
    MethodIndex,
    MethodSymbolIndex,
) {
    let mut index: HirIndex<'_> = HashMap::new();
    let mut roots: Vec<Item> = Vec::new();
    let mut symbols: SymbolIndex = HashMap::new();
    let mut generics: GenericIndex = HashMap::new();
    let mut fn_index: FnIndex = FnIndex::new();
    let mut method_index: MethodIndex = MethodIndex::new();
    let mut method_symbols: MethodSymbolIndex = MethodSymbolIndex::new();
    let mut next_id: usize = 0;

    for item in &hir.items {
        match &item.kind {
            HirItemKind::Function(f) => {
                let id = DeclId(next_id);
                next_id += 1;
                index.insert(id, f);
                let gp = generic_params_of(f);
                generics.insert(id, gp.clone());
                fn_index.insert(
                    f.name.clone(),
                    FnEntry {
                        decl: id,
                        params: f.params.iter().map(|(_, t, _)| t.clone()).collect(),
                        ret: f.ret.clone(),
                        generic_params: gp.clone(),
                    },
                );
                // A non-generic free function is a root. A generic one is
                // reached (and specialized) only through its call sites.
                if gp.is_empty() {
                    roots.push((id, SubstMap::new()));
                }
            }
            HirItemKind::Impl(imp) => {
                // Each method gets a per-type symbol so several types
                // implementing a method of the same name do not collide
                // at the object level. The symbol matches what a call
                // site recomputes from the receiver type.
                let type_mangle = super::ty::MirType::from_ty(&imp.self_ty).mangle();
                // The impl's own generic parameters are exactly those that
                // appear in the implementing type (`T` in `impl<T>
                // Box<T>`). A method's own parameters are the remainder of
                // its generic parameters, those it introduces (`U` in `fun
                // mapped<U>`).
                let mut impl_params: Vec<ParamId> = Vec::new();
                collect_params(&imp.self_ty, &mut impl_params);
                for m in &imp.methods {
                    let id = DeclId(next_id);
                    next_id += 1;
                    index.insert(id, m);
                    symbols.insert(id, super::ty::method_symbol(&type_mangle, &m.name));
                    let gp = generic_params_of(m);
                    generics.insert(id, gp.clone());
                    // The method's own generic parameters: those it declares
                    // that the implementing type does not. These contribute
                    // the method-level suffix on the mangled symbol so two
                    // instantiations of the method at different method-level
                    // type arguments do not collide.
                    let method_params: Vec<ParamId> = gp
                        .iter()
                        .filter(|p| !impl_params.contains(p))
                        .cloned()
                        .collect();
                    // Record the implementing type and the method-level
                    // parameters so the worklist can recompute a generic
                    // method's per-instantiation symbol from the concrete
                    // type arguments, matching the call site.
                    method_symbols.insert(
                        id,
                        MethodSymbolEntry {
                            self_ty: imp.self_ty.clone(),
                            method: m.name.clone(),
                            method_params: method_params.clone(),
                        },
                    );
                    // Index the method by name so a call site can find and
                    // specialize it. The user parameter types exclude the
                    // leading `self`, which the call site supplies from the
                    // receiver type.
                    if m.body.is_some() {
                        let user_params: Vec<Ty> = m
                            .params
                            .iter()
                            .filter(|(n, _, _)| n != "self")
                            .map(|(_, t, _)| t.clone())
                            .collect();
                        method_index
                            .entry(m.name.clone())
                            .or_default()
                            .push(MethodEntry {
                                decl: id,
                                self_ty: imp.self_ty.clone(),
                                params: user_params,
                                ret: m.ret.clone(),
                                method_params: method_params.clone(),
                                generic: !gp.is_empty(),
                            });
                    }
                    // A concrete-receiver method with no generic parameters
                    // of its own is a root: it lowers to its per-type symbol
                    // (`Int$to_string`, and so on), which a static call site
                    // references directly. A generic method (one whose
                    // declared types carry `Ty::Param`, for example a method
                    // on `impl<T> Box<T>`) is specialized through its call
                    // sites, like a generic free function, so it is not
                    // rooted here.
                    if gp.is_empty() && m.body.is_some() {
                        roots.push((id, SubstMap::new()));
                    }
                }
            }
            HirItemKind::Trait(t) => {
                for m in &t.methods {
                    let id = DeclId(next_id);
                    next_id += 1;
                    index.insert(id, m);
                    // Trait method default bodies are reachable only
                    // via impl resolution; do not root them directly.
                    let _ = m;
                }
            }
            _ => {}
        }
    }
    (
        index,
        roots,
        symbols,
        generics,
        fn_index,
        method_index,
        method_symbols,
    )
}

/// Collect a function's generic parameters in declaration order. The HIR
/// carries the declared parameter list (`HirFn::generics`); it is the
/// authoritative source because a parameter may appear only in the body
/// (for example the `T` of `type_name<T>()`) and so be invisible to a scan
/// of the signature types. The list is already in declaration order, the
/// order both the call site and the mangled name rely on. The signature
/// scan is kept as a fallback for any HIR built without the field set.
fn generic_params_of(f: &HirFn) -> Vec<ParamId> {
    if !f.generics.is_empty() {
        return f.generics.clone();
    }
    let mut found: Vec<ParamId> = Vec::new();
    let mut collect = |t: &Ty| collect_params(t, &mut found);
    for (_, ty, _) in &f.params {
        collect(ty);
    }
    collect(&f.ret);
    found.sort_by_key(|p| p.index);
    found.dedup();
    found
}

/// Walk a type and push every distinct `Ty::Param` into `out`.
fn collect_params(t: &Ty, out: &mut Vec<ParamId>) {
    match t {
        Ty::Param(p) => {
            if !out.contains(p) {
                out.push(p.clone());
            }
        }
        Ty::Option(t) | Ty::List(t) | Ty::SelfTy(t) => collect_params(t, out),
        Ty::Result(a, b) => {
            collect_params(a, out);
            collect_params(b, out);
        }
        Ty::Struct { args, .. } | Ty::Enum { args, .. } => {
            for a in args {
                collect_params(a, out);
            }
        }
        Ty::Function { params, ret } => {
            for p in params {
                collect_params(p, out);
            }
            collect_params(ret, out);
        }
        _ => {}
    }
}

/// Hashable companion of `Ty` used as the worklist seen-set key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct MangleKey(String);

impl MangleKey {
    fn from(t: &Ty) -> Self {
        MangleKey(super::ty::MirType::from_ty(t).mangle())
    }
}
