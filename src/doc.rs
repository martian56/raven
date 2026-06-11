//! Generate Markdown API documentation from a package's source.
//!
//! `rvpm doc` parses each `.rv` source file in a package, collects its
//! top-level items (`fun`, `struct`, `enum`, `trait`, `const`) together with
//! the `//` comment block written directly above each, and writes a single
//! Markdown file under `target/doc/`. There is no separate doc-comment syntax
//! in Raven, so any contiguous run of `//` lines immediately above an item is
//! taken as its documentation.

use std::fmt;
use std::path::{Path, PathBuf};

use crate::ast::DeclKind;
use crate::lexer::Lexer;
use crate::parser::parse;

/// The result of a `doc` run.
#[derive(Debug, Clone)]
pub struct DocReport {
    pub output: PathBuf,
    pub item_count: usize,
    pub outcome_lines: Vec<String>,
}

/// An error produced while generating documentation.
#[derive(Debug)]
pub enum DocError {
    Io { path: PathBuf, message: String },
    Parse { file: PathBuf, message: String },
}

impl fmt::Display for DocError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DocError::Io { path, message } => {
                write!(f, "{}: {}", path.display(), message)
            }
            DocError::Parse { file, message } => {
                write!(f, "cannot parse '{}': {}", file.display(), message)
            }
        }
    }
}

impl std::error::Error for DocError {}

/// One documented top-level item. The signature already begins with the item
/// keyword (`fun`, `struct`, ...), so the kind is not stored separately.
struct DocItem {
    name: String,
    signature: String,
    doc: String,
}

/// Generate `target/doc/<package>.md` for the package rooted at `project_dir`.
pub fn generate(project_dir: &Path) -> Result<DocReport, DocError> {
    let name = package_name(project_dir);
    let files = discover_source_files(project_dir)?;

    let mut body = String::new();
    let mut item_count = 0usize;
    let mut documented_files = 0usize;
    for file in &files {
        let source = std::fs::read_to_string(file).map_err(|e| DocError::Io {
            path: file.clone(),
            message: e.to_string(),
        })?;
        let items = documented_items(&source, file)?;
        if items.is_empty() {
            continue;
        }
        documented_files += 1;
        let rel = file.strip_prefix(project_dir).unwrap_or(file);
        body.push_str(&format!("## {}\n\n", rel_display(rel)));
        for it in items {
            item_count += 1;
            body.push_str(&format!("### {}\n\n", it.name));
            body.push_str(&format!("```rust\n{}\n```\n\n", it.signature));
            if !it.doc.is_empty() {
                body.push_str(&it.doc);
                body.push_str("\n\n");
            }
        }
    }

    let mut md = format!("# {} API\n\n", name);
    if item_count == 0 {
        md.push_str("No documented top-level items were found.\n");
    } else {
        md.push_str(&format!(
            "{} item(s) across {} file(s).\n\n",
            item_count, documented_files
        ));
        md.push_str(&body);
    }

    let out_dir = project_dir.join("target").join("doc");
    std::fs::create_dir_all(&out_dir).map_err(|e| DocError::Io {
        path: out_dir.clone(),
        message: e.to_string(),
    })?;
    let output = out_dir.join(format!("{}.md", name));
    std::fs::write(&output, &md).map_err(|e| DocError::Io {
        path: output.clone(),
        message: e.to_string(),
    })?;

    Ok(DocReport {
        outcome_lines: vec![format!(
            "Wrote {} ({} item(s) from {} file(s))",
            output.display(),
            item_count,
            documented_files
        )],
        output,
        item_count,
    })
}

/// The package name from `rv.toml`, falling back to the directory name.
fn package_name(project_dir: &Path) -> String {
    if let Ok(text) = std::fs::read_to_string(project_dir.join("rv.toml")) {
        if let Ok(manifest) = crate::manifest::Manifest::from_toml_str(&text) {
            return manifest.package.name;
        }
    }
    project_dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("package")
        .to_string()
}

/// Collect every `.rv` source file under `project_dir`, skipping `*_test.rv`
/// files, the build output, and hidden or VCS directories.
fn discover_source_files(project_dir: &Path) -> Result<Vec<PathBuf>, DocError> {
    let mut out = Vec::new();
    collect_source_files(project_dir, &mut out)?;
    out.sort();
    Ok(out)
}

fn collect_source_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), DocError> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };
    for entry in entries {
        let entry = entry.map_err(|e| DocError::Io {
            path: dir.to_path_buf(),
            message: e.to_string(),
        })?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if name == "target" || name.starts_with('.') {
                continue;
            }
            collect_source_files(&path, out)?;
        } else if name.ends_with(".rv") && !name.ends_with("_test.rv") {
            out.push(path);
        }
    }
    Ok(())
}

/// Parse `source` and collect its documented top-level items in source order.
/// Items whose name begins with `_` are treated as internal and skipped.
fn documented_items(source: &str, file: &Path) -> Result<Vec<DocItem>, DocError> {
    let tokens = Lexer::new(source.to_string(), file.to_path_buf())
        .tokenize()
        .map_err(|e| DocError::Parse {
            file: file.to_path_buf(),
            message: format!("{}", e),
        })?;
    let parsed = parse(&tokens).map_err(|e| DocError::Parse {
        file: file.to_path_buf(),
        message: format!("{}", e),
    })?;

    let lines: Vec<&str> = source.lines().collect();
    let mut out = Vec::new();
    for item in &parsed.items {
        let (name, has_block) = match &item.kind {
            DeclKind::Function(func) => (func.name.clone(), false),
            DeclKind::Struct(s) => (s.name.clone(), true),
            DeclKind::Enum(e) => (e.name.clone(), true),
            DeclKind::Trait(t) => (t.name.clone(), true),
            DeclKind::Const(c) => (c.name.clone(), false),
            _ => continue,
        };
        if name.starts_with('_') {
            continue;
        }
        let signature = if has_block {
            block_text(source, item.span.start)
        } else {
            header_text(source, item.span.start)
        };
        let doc = doc_comment_above(&lines, item.span.line);
        out.push(DocItem {
            name,
            signature,
            doc,
        });
    }
    Ok(out)
}

/// The single-line signature of a `fun` or `const`: the source from `start` up
/// to the body opener (`{` for a block body, `=` for a single expression),
/// with runs of whitespace collapsed.
fn header_text(source: &str, start: usize) -> String {
    let rest = &source[start..];
    let brace = rest.find('{').unwrap_or(rest.len());
    let eq = rest.find('=').unwrap_or(rest.len());
    let cut = brace.min(eq);
    rest[..cut].split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The full block text of a `struct`/`enum`/`trait`: from `start` through the
/// brace that closes the first `{`. Declaration headers carry only structural
/// braces, so a depth count is enough.
fn block_text(source: &str, start: usize) -> String {
    let bytes = source.as_bytes();
    let mut depth = 0i32;
    let mut seen = false;
    let mut end = source.len();
    let mut i = start;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => {
                depth += 1;
                seen = true;
            }
            b'}' => {
                depth -= 1;
                if seen && depth == 0 {
                    end = i + 1;
                    break;
                }
            }
            _ => {}
        }
        i += 1;
    }
    source[start..end].trim_end().to_string()
}

/// The contiguous `//` comment block immediately above the item starting at
/// `item_line` (1-based), with `//` markers stripped. Attribute lines (`@...`)
/// between the comment and the item are skipped.
fn doc_comment_above(lines: &[&str], item_line: u32) -> String {
    if item_line < 2 {
        return String::new();
    }
    let mut idx = (item_line as usize) - 2; // 0-based index of the line above
    let mut collected: Vec<String> = Vec::new();
    loop {
        if idx >= lines.len() {
            break;
        }
        let line = lines[idx].trim();
        if line.starts_with('@') {
            // An attribute decorates the item; keep scanning above it.
        } else if let Some(rest) = line.strip_prefix("//") {
            collected.push(rest.trim().to_string());
        } else {
            break;
        }
        if idx == 0 {
            break;
        }
        idx -= 1;
    }
    collected.reverse();
    collected.join("\n")
}

/// Render a relative path with forward slashes for stable Markdown output.
fn rel_display(rel: &Path) -> String {
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_items_with_doc_comments() {
        let src = "// Add two integers.\n\
                   // Returns their sum.\n\
                   fun add(a: Int, b: Int) -> Int {\n    return a + b\n}\n\
                   \n\
                   fun _internal() {}\n\
                   \n\
                   // A 2D point.\n\
                   struct Point {\n    x: Float,\n    y: Float,\n}\n";
        let items = documented_items(src, Path::new("a.rv")).unwrap();
        assert_eq!(items.len(), 2, "internal _ item should be skipped");
        assert_eq!(items[0].name, "add");
        assert_eq!(items[0].signature, "fun add(a: Int, b: Int) -> Int");
        assert!(items[0].doc.contains("Add two integers."));
        assert!(items[0].doc.contains("Returns their sum."));
        assert_eq!(items[1].name, "Point");
        assert!(items[1].signature.starts_with("struct Point {"));
        assert!(items[1].signature.ends_with("}"));
        assert!(items[1].doc.contains("A 2D point."));
    }

    #[test]
    fn single_expression_function_signature_stops_at_equals() {
        let src = "fun double(x: Int) -> Int = x * 2\n";
        let items = documented_items(src, Path::new("a.rv")).unwrap();
        assert_eq!(items[0].signature, "fun double(x: Int) -> Int");
        assert!(items[0].doc.is_empty());
    }
}
