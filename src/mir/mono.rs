//! Monomorphization driver.
//!
//! Starts from the program's "roots" (every non-generic top-level
//! function), lowers each root to MIR, and walks the resulting MIR
//! for any call site that targets a still-generic function. Each
//! `(decl, concrete_args)` pair is queued onto a worklist and lowered
//! exactly once. The result is a flat list of monomorphic MIR
//! functions ready for codegen.
//!
//! The current implementation focuses on the structural pieces. The
//! analysis pass that walks an HIR body and collects call sites with
//! their concrete type arguments is intentionally simple: with no
//! call-site type annotations in HIR today, the algorithm conservatively
//! treats every non-generic top-level function as already monomorphic.
//! Once HIR begins to record callee type arguments (issue #62 follow-up),
//! the worklist below specializes them through the same code path.

use std::collections::{HashMap, HashSet};

use crate::error::RavenError;
use crate::hir::{HirFn, HirItemKind, HirProgram};
use crate::resolve::DeclId;
use crate::tycheck::Ty;

use super::ir::MirProgram;
use super::lower::{lower_function, DeclTables, SubstMap};

/// One entry in the monomorphization worklist.
type Item = (DeclId, Vec<Ty>);

/// Map from a function declaration to its HIR shape, used by the
/// worklist to retrieve bodies.
type HirIndex<'a> = HashMap<DeclId, &'a HirFn>;

/// Map from a function declaration to the base symbol the back end uses.
/// Free functions keep their source name; impl methods get a per-type
/// symbol (`<TypeMangle>$<method>`) so several types implementing a
/// method of the same name do not collide.
type SymbolIndex = HashMap<DeclId, String>;

/// Run the full monomorphization pass.
pub fn monomorphize(hir: &HirProgram) -> Result<MirProgram, RavenError> {
    let mut program = MirProgram::new();
    program.externs = collect_externs(hir);
    let (index, roots, symbols) = collect_roots(hir);
    let decls = collect_decls(hir);

    let mut seen: HashSet<(DeclId, Vec<MangleKey>)> = HashSet::new();
    let mut worklist: Vec<Item> = roots;

    while let Some((decl, args)) = worklist.pop() {
        let key = (decl, args.iter().map(MangleKey::from).collect());
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
        let subst = build_subst(hir_fn, &args);
        let mangled = mangle_name(&base, &args);
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
fn collect_decls(hir: &HirProgram) -> DeclTables<'_> {
    let mut tables = DeclTables::default();
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

/// Collect every top-level function plus impl methods. Returns the
/// HIR lookup table, the initial worklist of non-generic roots, and the
/// per-declaration base symbol used by the back end.
fn collect_roots<'a>(hir: &'a HirProgram) -> (HirIndex<'a>, Vec<Item>, SymbolIndex) {
    let mut index: HirIndex<'a> = HashMap::new();
    let mut roots: Vec<Item> = Vec::new();
    let mut symbols: SymbolIndex = HashMap::new();
    let mut next_id: usize = 0;

    for item in &hir.items {
        match &item.kind {
            HirItemKind::Function(f) => {
                let id = DeclId(next_id);
                next_id += 1;
                index.insert(id, f);
                if !is_generic(f) {
                    roots.push((id, Vec::new()));
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
                    if !is_generic(m) && m.body.is_some() {
                        roots.push((id, Vec::new()));
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
    (index, roots, symbols)
}

/// Best-effort generic test. A function counts as generic if any of
/// its parameter or return types is a `Ty::Param`. The HIR walker
/// already strips other indirections.
fn is_generic(f: &HirFn) -> bool {
    fn walk(t: &Ty) -> bool {
        match t {
            Ty::Param(_) => true,
            Ty::Option(t) | Ty::List(t) | Ty::SelfTy(t) => walk(t),
            Ty::Result(a, b) => walk(a) || walk(b),
            Ty::Struct { args, .. } | Ty::Enum { args, .. } => args.iter().any(walk),
            Ty::Function { params, ret } => params.iter().any(walk) || walk(ret),
            _ => false,
        }
    }
    for (_, ty, _) in &f.params {
        if walk(ty) {
            return true;
        }
    }
    walk(&f.ret)
}

/// Build a substitution table from the function's generic parameters
/// to the concrete arguments at this call site.
fn build_subst(_hir: &HirFn, _args: &[Ty]) -> SubstMap {
    // The HIR currently does not surface the function's generic
    // parameter list explicitly; non-generic roots have empty args, so
    // the substitution is empty. Once HIR carries the parameter list,
    // pair them up here.
    SubstMap::new()
}

/// Compute the mangled name for an instantiation.
fn mangle_name(source: &str, args: &[Ty]) -> String {
    if args.is_empty() {
        source.to_string()
    } else {
        let mut s = source.to_string();
        for a in args {
            s.push('$');
            s.push_str(&mangle_ty(a));
        }
        s
    }
}

fn mangle_ty(t: &Ty) -> String {
    super::ty::MirType::from_ty(t).mangle()
}

/// Hashable companion of `Ty` used as the worklist seen-set key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct MangleKey(String);

impl MangleKey {
    fn from(t: &Ty) -> Self {
        MangleKey(mangle_ty(t))
    }
}
