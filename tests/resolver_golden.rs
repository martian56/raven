//! Golden snapshot tests for the resolver.
//!
//! Walks every `tests/resolver_corpus/*.rv`, lexes and parses the
//! source, runs the resolver against a no op `SourceLoader`, and
//! diffs a textual dump of the resolution map against a committed
//! `.rv.resolved` baseline.
//!
//! Refresh baselines with `RAVEN_UPDATE_RESOLVER_GOLDEN=1 cargo test
//! --test resolver_golden`.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use raven::lexer::Lexer;
use raven::parser::parse;
use raven::resolve::{
    resolve_file, Binding, ImportTarget, LoadedSource, ResolutionMap, ResolvedImport, SourceLoader,
};

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("resolver_corpus")
}

/// Loader that never finds anything; corpus tests never use local
/// imports.
struct NoLoader;
impl SourceLoader for NoLoader {
    fn load(&mut self, _i: &Path, _t: &str) -> Option<LoadedSource> {
        None
    }
}

#[test]
fn resolver_golden() {
    let dir = corpus_dir();
    let update = std::env::var("RAVEN_UPDATE_RESOLVER_GOLDEN").is_ok();

    let mut entries: Vec<PathBuf> = fs::read_dir(&dir)
        .expect("corpus directory must exist")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("rv"))
        .collect();
    entries.sort();

    assert!(!entries.is_empty(), "resolver corpus is empty");

    let mut failures: Vec<String> = Vec::new();

    for src_path in entries {
        let src = fs::read_to_string(&src_path).expect("read source");
        let tokens = match Lexer::new(src.clone(), src_path.clone()).tokenize() {
            Ok(t) => t,
            Err(e) => {
                failures.push(format!(
                    "{}: lex error: {}",
                    src_path.display(),
                    e.display(&src)
                ));
                continue;
            }
        };
        let file = match parse(&tokens) {
            Ok(f) => f,
            Err(e) => {
                failures.push(format!(
                    "{}: parse error: {}",
                    src_path.display(),
                    e.display(&src)
                ));
                continue;
            }
        };
        let mut loader = NoLoader;
        let resolved = match resolve_file(&file, &mut loader) {
            Ok(r) => r,
            Err(e) => {
                failures.push(format!(
                    "{}: resolve error: {}",
                    src_path.display(),
                    e.display(&src)
                ));
                continue;
            }
        };
        let rendered = render_resolution(&resolved.map, &src);

        let baseline = src_path.with_extension("rv.resolved");
        if update {
            fs::write(&baseline, &rendered).expect("write baseline");
            continue;
        }

        match fs::read_to_string(&baseline) {
            Ok(expected) => {
                if normalize(&expected) != normalize(&rendered) {
                    failures.push(format!(
                        "snapshot mismatch for {}\n--- expected ---\n{}\n--- actual ---\n{}",
                        src_path.display(),
                        expected,
                        rendered
                    ));
                }
            }
            Err(_) => failures.push(format!(
                "missing baseline for {} (run with RAVEN_UPDATE_RESOLVER_GOLDEN=1)",
                src_path.display()
            )),
        }
    }

    if !failures.is_empty() {
        panic!(
            "{} golden resolver snapshot failure(s):\n{}",
            failures.len(),
            failures.join("\n\n")
        );
    }
}

/// Render the resolution map as a stable, human readable dump.
///
/// Format:
///
/// ```text
/// (resolved
///   imports
///     [0] std/io { println } -> StdlibModule(io)
///   uses
///     1:5..1:8 ident "add" -> Function
///     ...
/// )
/// ```
fn render_resolution(map: &ResolutionMap, src: &str) -> String {
    let mut out = String::new();
    writeln!(&mut out, "(resolved").unwrap();
    writeln!(&mut out, "  imports").unwrap();
    for imp in &map.imports {
        writeln!(&mut out, "    {}", render_import(imp)).unwrap();
    }
    writeln!(&mut out, "  uses").unwrap();
    for (key, binding) in map.sorted_iter() {
        // Recover the source spelling for the use site by slicing the
        // raw source bytes. This is robust against AST churn and gives
        // the baseline a clear anchor.
        let text = src
            .get(key.start..key.end)
            .map(|s| s.to_string())
            .unwrap_or_else(|| String::from("<oob>"));
        writeln!(
            &mut out,
            "    {}..{} \"{}\" -> {}",
            key.start,
            key.end,
            escape(&text),
            render_binding(binding)
        )
        .unwrap();
    }
    writeln!(&mut out, ")").unwrap();
    out
}

fn render_import(imp: &ResolvedImport) -> String {
    let target = match &imp.target {
        ImportTarget::StdlibModule { segments } => {
            format!("StdlibModule(std/{})", segments.join("/"))
        }
        ImportTarget::ExternalPackage {
            host, user, repo, ..
        } => format!("ExternalPackage({}/{}/{})", host, user, repo),
        ImportTarget::LocalModule { canonical_path, .. } => {
            format!("LocalModule({})", canonical_path.display())
        }
    };
    format!("[{}] {} -> {}", imp.id.0, imp.path, target)
}

fn render_binding(b: &Binding) -> String {
    match b {
        Binding::Function(id) => format!("Function(#{})", id.0),
        Binding::Struct(id) => format!("Struct(#{})", id.0),
        Binding::Trait(id) => format!("Trait(#{})", id.0),
        Binding::Enum(id) => format!("Enum(#{})", id.0),
        Binding::Variant {
            enum_id,
            variant_index,
        } => format!("Variant(#{}, {})", enum_id.0, variant_index),
        Binding::Extern {
            decl_id,
            item_index,
        } => format!("Extern(#{}, {})", decl_id.0, item_index),
        Binding::Const(id) => format!("Const(#{})", id.0),
        Binding::Static(id) => format!("Static(#{})", id.0),
        Binding::Param(sp) => format!("Param@{}..{}", sp.start, sp.end),
        Binding::Local(sp) => format!("Local@{}..{}", sp.start, sp.end),
        Binding::PatternBinding(sp) => format!("PatternBinding@{}..{}", sp.start, sp.end),
        Binding::GenericParam { name, .. } => format!("GenericParam({})", name),
        Binding::SelfType => "SelfType".to_string(),
        Binding::SelfValue => "SelfValue".to_string(),
        Binding::ImportAlias(id) => format!("ImportAlias(#{})", id.0),
        Binding::ImportedItem { import_id, name } => {
            format!("ImportedItem(#{}, {})", import_id.0, name)
        }
    }
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

fn normalize(s: &str) -> String {
    s.replace("\r\n", "\n")
        .lines()
        .map(|l| l.trim_end().to_string())
        .collect::<Vec<_>>()
        .join("\n")
}
