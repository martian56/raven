//! rvpm - Raven Package Manager
//!
//! Commands:
//!   rvpm init       Create a new Raven project
//!   rvpm install    Install dependencies from rv.toml
//!   rvpm add <pkg>  Add a dependency
//!   rvpm run        Run the project (raven src/main.rv)
//!   rvpm fmt        Format Raven source (.rv) files

use clap::{Arg, Command};
use raven::format::format_source;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const DEFAULT_MAIN_RV: &str = r#"// Raven program - run with: rvpm run
fun main() -> void {
    print("Hello from Raven!");
}

main();
"#;

const DEFAULT_RV_TOML: &str = r#"[package]
name = "my_project"
version = "0.1.0"
authors = []

[dependencies]
# Add packages here, e.g.:
# math = "1.0"
"#;

fn main() {
    let matches = Command::new("rvpm")
        .version(env!("CARGO_PKG_VERSION"))
        .about("Raven Package Manager")
        .subcommand(
            Command::new("init")
                .about("Create a new Raven project")
                .arg(
                    Arg::new("name")
                        .help("Project name (default: current directory name)")
                        .default_value("."),
                ),
        )
        .subcommand(Command::new("install").about("Install dependencies from rv.toml"))
        .subcommand(
            Command::new("add")
                .about("Add a dependency")
                .arg(Arg::new("package").required(true)),
        )
        .subcommand(Command::new("run").about("Run the project"))
        .subcommand(
            Command::new("fmt")
                .about("Format Raven source files")
                .arg(
                    Arg::new("paths")
                        .help("Files or directories to format (default: project src/)")
                        .num_args(0..),
                )
                .arg(
                    Arg::new("check")
                        .long("check")
                        .help("Fail if any file would change (CI / verify formatted)")
                        .action(clap::ArgAction::SetTrue),
                ),
        )
        .get_matches();

    match matches.subcommand() {
        Some(("init", sub_matches)) => {
            let name = sub_matches.get_one::<String>("name").unwrap();
            if let Err(e) = cmd_init(name) {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
        Some(("install", _)) => {
            eprintln!("rvpm install: not yet implemented");
            std::process::exit(1);
        }
        Some(("add", _)) => {
            eprintln!("rvpm add: not yet implemented");
            std::process::exit(1);
        }
        Some(("run", _)) => {
            if let Err(e) = cmd_run() {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
        Some(("fmt", sub_matches)) => {
            let paths: Vec<String> = sub_matches
                .get_many::<String>("paths")
                .map(|p| p.cloned().collect())
                .unwrap_or_default();
            let check = sub_matches.get_flag("check");
            if let Err(e) = cmd_fmt(&paths, check) {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
        _ => {
            let _ = io::stderr().write_fmt(format_args!(
                "Usage: rvpm <COMMAND>\n\nCommands:\n  init     Create a new Raven project\n  install  Install dependencies\n  add      Add a dependency\n  run      Run the project\n  fmt      Format Raven source files\n"
            ));
            std::process::exit(1);
        }
    }
}

fn cmd_init(name: &str) -> Result<(), String> {
    let project_dir = Path::new(name);
    let project_name = if name == "." {
        std::env::current_dir()
            .ok()
            .and_then(|p| p.file_name().map(|s| s.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "my_project".to_string())
    } else {
        name.to_string()
    };

    if name != "." && project_dir.exists() {
        if let Ok(entries) = project_dir.read_dir() {
            if entries.count() > 0 {
                return Err(format!(
                    "Directory '{}' already exists and is not empty",
                    name
                ));
            }
        }
    } else if name == "." && project_dir.join("rv.toml").exists() {
        return Err(
            "rv.toml already exists here. Remove it or use a different directory.".to_string(),
        );
    }

    let src_dir = project_dir.join("src");
    let rv_env_dir = project_dir.join("rv_env");
    let rv_env_packages = rv_env_dir.join("packages");

    fs::create_dir_all(&src_dir).map_err(|e| format!("Failed to create src/: {}", e))?;
    fs::create_dir_all(&rv_env_packages)
        .map_err(|e| format!("Failed to create rv_env/packages/: {}", e))?;

    let rv_toml = DEFAULT_RV_TOML.replace("my_project", &project_name);
    fs::write(project_dir.join("rv.toml"), rv_toml)
        .map_err(|e| format!("Failed to write rv.toml: {}", e))?;

    // Write src/main.rv
    fs::write(src_dir.join("main.rv"), DEFAULT_MAIN_RV)
        .map_err(|e| format!("Failed to write src/main.rv: {}", e))?;

    let gitignore_path = project_dir.join(".gitignore");
    if !gitignore_path.exists() {
        let _ = fs::write(
            gitignore_path,
            "# rvpm - installed packages\nrv_env/packages/\n",
        );
    }

    println!("Created Raven project '{}'", project_name);
    println!("  rv.toml");
    println!("  src/main.rv");
    println!("  rv_env/");
    println!();
    let next_msg = if name == "." {
        "Next: rvpm run".to_string()
    } else {
        format!("Next: cd {} && rvpm run", name)
    };
    println!("{}", next_msg);

    Ok(())
}

fn cmd_run() -> Result<(), String> {
    let cwd = std::env::current_dir().map_err(|e| format!("{}", e))?;
    let mut dir = cwd.as_path();

    loop {
        let rv_toml = dir.join("rv.toml");
        if rv_toml.exists() {
            let main_rv = dir.join("src").join("main.rv");
            if !main_rv.exists() {
                return Err(
                    "src/main.rv not found. Run 'rvpm init' to create a project.".to_string(),
                );
            }
            let status = std::process::Command::new("raven")
                .arg(main_rv)
                .current_dir(dir)
                .status()
                .map_err(|e| format!("Failed to run raven: {}. Is 'raven' in PATH?", e))?;
            std::process::exit(status.code().unwrap_or(1));
        }
        dir = match dir.parent() {
            Some(p) => p,
            None => {
                return Err(
                    "Not a Raven project (no rv.toml found). Run 'rvpm init' first.".to_string(),
                );
            }
        };
    }
}

fn find_rv_project_root() -> Result<PathBuf, String> {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let mut dir = cwd.as_path();
    loop {
        if dir.join("rv.toml").exists() {
            return Ok(dir.to_path_buf());
        }
        dir = dir.parent().ok_or_else(|| {
            "Not a Raven project (no rv.toml found). Run 'rvpm init' or pass explicit paths."
                .to_string()
        })?;
    }
}

fn collect_rv_files(path: &Path) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    if path.is_dir() {
        for entry in fs::read_dir(path).map_err(|e| format!("{}: {}", path.display(), e))? {
            let entry = entry.map_err(|e| e.to_string())?;
            let p = entry.path();
            if p.is_dir() {
                out.extend(collect_rv_files(&p)?);
            } else if p.extension().is_some_and(|e| e == "rv") {
                out.push(p);
            }
        }
    } else if path.is_file() {
        if path.extension().is_some_and(|e| e == "rv") {
            out.push(path.to_path_buf());
        } else {
            return Err(format!(
                "Not a Raven source file (expected .rv): {}",
                path.display()
            ));
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

fn normalize_nl(s: &str) -> String {
    s.replace("\r\n", "\n")
}

fn cmd_fmt(paths: &[String], check: bool) -> Result<(), String> {
    let files: Vec<PathBuf> = if paths.is_empty() {
        let root = find_rv_project_root()?;
        let src = root.join("src");
        if !src.is_dir() {
            return Err(format!(
                "No src/ directory at {}. Pass explicit paths or create src/.",
                root.display()
            ));
        }
        collect_rv_files(&src)?
    } else {
        let mut all = Vec::new();
        for p in paths {
            let path = Path::new(p);
            if !path.exists() {
                return Err(format!("Path not found: {}", p));
            }
            if path.is_dir() {
                all.extend(collect_rv_files(path)?);
            } else if path.extension().is_some_and(|e| e == "rv") {
                all.push(path.to_path_buf());
            } else {
                return Err(format!(
                    "Not a Raven source file (expected .rv): {}",
                    path.display()
                ));
            }
        }
        all.sort();
        all.dedup();
        all
    };

    if files.is_empty() {
        println!("No .rv files to format.");
        return Ok(());
    }

    let mut changed = Vec::new();
    let mut errors = Vec::new();

    for file in &files {
        let source = fs::read_to_string(file).map_err(|e| format!("{}: {}", file.display(), e))?;
        let normalized = normalize_nl(&source);
        let formatted = match format_source(&normalized, &file.display().to_string()) {
            Ok(s) => s,
            Err(e) => {
                errors.push(format!("{}: {}", file.display(), e.format()));
                continue;
            }
        };

        if normalized == formatted {
            continue;
        }

        if check {
            changed.push(file.display().to_string());
        } else {
            fs::write(file, &formatted).map_err(|e| format!("{}: {}", file.display(), e))?;
            println!("Formatted {}", file.display());
        }
    }

    if !errors.is_empty() {
        for err in errors {
            eprintln!("{}", err);
        }
        return Err("fmt: one or more files failed to parse.".to_string());
    }

    if check && !changed.is_empty() {
        for path in &changed {
            eprintln!("Would reformat: {}", path);
        }
        return Err(format!(
            "fmt --check: {} file(s) need formatting.",
            changed.len()
        ));
    }

    Ok(())
}
