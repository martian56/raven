//! Golden-output test suite for examples/*.rv.
//!
//! For each examples/*.rv (skipping files whose first 5 lines contain
//! `// golden:skip`), run the `raven` binary and compare stdout to the
//! committed examples/<name>.out file.
//!
//! Set RAVEN_UPDATE_GOLDEN=1 to overwrite .out baselines.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

const EXAMPLES_DIR: &str = "examples";

fn raven_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_raven"))
}

fn first_5_lines(path: &std::path::Path) -> String {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .take(5)
        .collect::<Vec<_>>()
        .join("\n")
}

fn normalize(s: &str) -> String {
    s.replace("\r\n", "\n")
}

#[test]
fn golden_examples() {
    let update = std::env::var("RAVEN_UPDATE_GOLDEN").ok().as_deref() == Some("1");

    let entries: Vec<_> = fs::read_dir(EXAMPLES_DIR)
        .expect("examples/ dir")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("rv"))
        .map(|e| e.path())
        .collect();

    assert!(!entries.is_empty(), "no examples found");

    let mut failures = Vec::new();
    let mut skipped = Vec::new();
    let mut ran = 0;

    for src in entries {
        let name = src.file_stem().unwrap().to_string_lossy().into_owned();
        let header = first_5_lines(&src);

        if header.contains("golden:skip") {
            let reason = header
                .lines()
                .find(|l| l.contains("golden:skip"))
                .unwrap_or("")
                .trim_start_matches("//")
                .trim()
                .to_string();
            skipped.push(format!("{} ({})", name, reason));
            continue;
        }

        let output = Command::new(raven_binary())
            .arg(&src)
            .output()
            .expect("invoke raven");

        let stdout = normalize(&String::from_utf8_lossy(&output.stdout));
        let baseline_path = src.with_extension("rv.out");

        if update {
            fs::write(&baseline_path, &stdout).expect("write baseline");
            continue;
        }

        let expected = match fs::read_to_string(&baseline_path) {
            Ok(s) => normalize(&s),
            Err(_) => {
                failures.push(format!(
                    "{}: no baseline at {} — run with RAVEN_UPDATE_GOLDEN=1 to capture",
                    name,
                    baseline_path.display()
                ));
                continue;
            }
        };

        if stdout != expected {
            failures.push(format!(
                "{}: stdout mismatch\n--- expected ---\n{}\n--- actual ---\n{}",
                name, expected, stdout
            ));
        }
        ran += 1;
    }

    eprintln!("golden: ran {} examples, skipped {}", ran, skipped.len());
    for s in &skipped {
        eprintln!("  skipped: {}", s);
    }

    if !failures.is_empty() {
        panic!(
            "{} golden failure(s):\n\n{}",
            failures.len(),
            failures.join("\n\n")
        );
    }
}
