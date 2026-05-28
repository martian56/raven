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
use crate::hir::{HirFn, HirItemKind, HirProgram};
use crate::resolve::DeclId;
use crate::tycheck::ty::ParamId;
use crate::tycheck::Ty;

use super::ir::MirProgram;
use super::lower::{lower_function, mono_symbol, DeclTables, FnEntry, FnIndex, SubstMap};

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

/// Run the full monomorphization pass.
pub fn monomorphize(hir: &HirProgram) -> Result<MirProgram, RavenError> {
    let mut program = MirProgram::new();
    program.externs = collect_externs(hir);
    let (index, roots, symbols, generics, fn_index) = collect_roots(hir);
    let decls = collect_decls(hir, fn_index);

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
        let base = symbols
            .get(&decl)
            .cloned()
            .unwrap_or_else(|| hir_fn.name.clone());
        let mangled = mono_symbol(&base, &generic_params, &subst);
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
                });
            }
        }
    }
    out
}

/// Index every struct and enum declaration by its source name so the
/// expression lowering can resolve field offsets and variant payloads.
/// The free-function index built by [`collect_roots`] is folded in so a
/// generic call can be specialized at its call site.
fn collect_decls(hir: &HirProgram, functions: FnIndex) -> DeclTables<'_> {
    let mut tables = DeclTables {
        functions,
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
fn collect_roots(
    hir: &HirProgram,
) -> (HirIndex<'_>, Vec<Item>, SymbolIndex, GenericIndex, FnIndex) {
    let mut index: HirIndex<'_> = HashMap::new();
    let mut roots: Vec<Item> = Vec::new();
    let mut symbols: SymbolIndex = HashMap::new();
    let mut generics: GenericIndex = HashMap::new();
    let mut fn_index: FnIndex = FnIndex::new();
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
                for m in &imp.methods {
                    let id = DeclId(next_id);
                    next_id += 1;
                    index.insert(id, m);
                    symbols.insert(id, super::ty::method_symbol(&type_mangle, &m.name));
                    let gp = generic_params_of(m);
                    generics.insert(id, gp.clone());
                    // A concrete-receiver method with no generic
                    // parameters of its own is a root: it lowers to its
                    // per-type symbol (`Int$to_string`, and so on), which
                    // a static call site references directly. Generic
                    // methods are specialized through their call sites,
                    // a follow-up beyond the free-function path here.
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
    (index, roots, symbols, generics, fn_index)
}

/// Collect a function's generic parameters in declaration order. The
/// HIR does not carry an explicit parameter list, so the parameters are
/// recovered by scanning the parameter and return types for `Ty::Param`
/// occurrences and ordering them by their declaration index. The index
/// on a `ParamId` is its position in the original `<...>` list, which is
/// the order both the call site and the mangled name rely on.
fn generic_params_of(f: &HirFn) -> Vec<ParamId> {
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
