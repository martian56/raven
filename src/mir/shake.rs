//! Drop functions unreachable from the program entry points.

use std::collections::{HashMap, HashSet};

use super::ir::{MirFunction, MirProgram, MirRvalue, MirStatement};

const ROOTS: &[&str] = &["__raven_init_globals"];

pub fn remove_unreachable(program: &mut MirProgram) {
    let index: HashMap<&str, usize> = program
        .functions
        .iter()
        .enumerate()
        .map(|(i, f)| (f.name.as_str(), i))
        .collect();
    let mut keep = vec![false; program.functions.len()];
    let mut work: Vec<usize> = Vec::new();
    for (i, f) in program.functions.iter().enumerate() {
        if f.origin == "main" || ROOTS.contains(&f.name.as_str()) {
            keep[i] = true;
            work.push(i);
        }
    }
    if work.is_empty() {
        return;
    }
    let mut seen_names: HashSet<String> = HashSet::new();
    while let Some(i) = work.pop() {
        let mut found: Vec<String> = Vec::new();
        collect_refs(&program.functions[i], &mut found);
        for name in found {
            if !seen_names.insert(name.clone()) {
                continue;
            }
            if let Some(&j) = index.get(name.as_str()) {
                if !keep[j] {
                    keep[j] = true;
                    work.push(j);
                }
            }
        }
    }
    let mut i = 0;
    program.functions.retain(|_| {
        let k = keep[i];
        i += 1;
        k
    });
}

fn collect_refs(f: &MirFunction, out: &mut Vec<String>) {
    for block in &f.blocks {
        for stmt in &block.statements {
            let MirStatement::Assign { rvalue, .. } = stmt else {
                continue;
            };
            match rvalue {
                MirRvalue::Call { callee, .. } => out.push(callee.mangled.clone()),
                MirRvalue::ClosureCreate { fn_name, .. } => out.push(fn_name.clone()),
                MirRvalue::FnAddr { mangled } => out.push(mangled.clone()),
                MirRvalue::DynCoerce {
                    concrete_ty,
                    methods,
                    ..
                } => {
                    for m in methods {
                        out.push(concrete_ty.method_symbol(m));
                    }
                }
                _ => {}
            }
        }
    }
}
