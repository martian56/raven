//! rvpm - Raven Package Manager
//!
//! Commands:
//!   rvpm init       Create a new Raven project
//!   rvpm install    Install dependencies from rv.toml
//!   rvpm add <pkg>  Add a dependency
//!   rvpm run        Run the project (raven src/main.rv)

use clap::{Arg, Command};
use std::fs;
use std::io::{self, Write};
use std::path::Path;

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
        _ => {
            let _ = io::stderr().write_fmt(format_args!(
                "Usage: rvpm <COMMAND>\n\nCommands:\n  init     Create a new Raven project\n  install  Install dependencies\n  add      Add a dependency\n  run      Run the project\n"
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
