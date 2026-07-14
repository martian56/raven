//! End-to-end coverage for rvpm workspaces and registered commands.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use raven::codegen::linker;

#[test]
fn workspace_list_works_from_a_nested_member_directory() {
    let _guard = test_lock();
    let root = fixture();
    let nested = root.join("apps/api/src/nested");
    std::fs::create_dir_all(&nested).unwrap();

    let output = rvpm(&nested, &["workspace", "list"]);
    cleanup(&root);

    assert!(
        output.status.success(),
        "workspace list failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("api"), "stdout: {}", stdout);
    assert!(stdout.contains("tool"), "stdout: {}", stdout);
    assert!(stdout.contains("show"), "stdout: {}", stdout);
}

#[test]
fn package_selection_and_registered_commands_build_and_run() {
    let _guard = test_lock();
    if !supported_runtime() {
        return;
    }
    let root = fixture();

    let selected = rvpm(&root, &["run", "-p", "api"]);
    assert!(
        selected.status.success(),
        "selected run failed: {}",
        String::from_utf8_lossy(&selected.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&selected.stdout), "api\n");
    let api_binary = binary(&root.join("apps/api"), "api");
    let first_modified = api_binary.metadata().unwrap().modified().unwrap();
    let selected_again = rvpm(&root, &["run", "-p", "api"]);
    assert!(selected_again.status.success());
    assert_eq!(
        api_binary.metadata().unwrap().modified().unwrap(),
        first_modified,
        "an unchanged command target should reuse its compiled binary"
    );
    write(
        &root.join("apps/api/src/main.rv"),
        "fun main() { print(\"api changed\") }\n",
    );
    let selected_after_change = rvpm(&root, &["run", "-p", "api"]);
    assert!(selected_after_change.status.success());
    assert_eq!(
        String::from_utf8_lossy(&selected_after_change.stdout),
        "api changed\n"
    );

    let command = rvpm(&root, &["run", "show", "extra"]);
    assert!(
        command.status.success(),
        "registered command failed: {}",
        String::from_utf8_lossy(&command.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&command.stdout), "base\nextra\n");

    let separated = rvpm(&root, &["run", "show", "--", "tail"]);
    assert!(
        separated.status.success(),
        "registered command with separator failed: {}",
        String::from_utf8_lossy(&separated.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&separated.stdout), "base\ntail\n");

    let failed = rvpm(&root, &["run", "fail"]);
    cleanup(&root);
    assert_eq!(failed.status.code(), Some(7));
}

#[test]
fn workspace_build_compiles_every_member() {
    let _guard = test_lock();
    if !supported_runtime() {
        return;
    }
    let root = fixture();
    let output = rvpm(&root, &["build", "--workspace"]);
    let api = binary(&root.join("apps/api"), "api").is_file();
    let tool = binary(&root.join("tools/tool"), "tool").is_file();
    cleanup(&root);

    assert!(
        output.status.success(),
        "workspace build failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(api, "api binary was not built");
    assert!(tool, "tool binary was not built");
}

fn fixture() -> PathBuf {
    let root = workdir();
    write(
        &root.join("rv.toml"),
        r#"[workspace]
members = ["apps/api", "tools/tool"]
default-member = "api"

[commands]
show = { package = "tool", args = ["base"] }
fail = { package = "tool", args = ["fail"] }
"#,
    );
    package(
        &root.join("apps/api"),
        "api",
        "fun main() { print(\"api\") }\n",
    );
    package(
        &root.join("tools/tool"),
        "tool",
        r#"import std/env { args, exit }

fun main() {
    let all = args()
    if all.len() > 1 && all[1] == "fail" {
        exit(7)
    }
    let i = 1
    while i < all.len() {
        print(all[i])
        i = i + 1
    }
}
"#,
    );
    root
}

fn package(root: &Path, name: &str, source: &str) {
    write(
        &root.join("rv.toml"),
        &format!("[package]\nname = \"{}\"\nversion = \"0.1.0\"\n", name),
    );
    write(&root.join("src/main.rv"), source);
}

fn write(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, contents).unwrap();
}

fn rvpm(current_dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rvpm"))
        .args(args)
        .current_dir(current_dir)
        .env("RVPM_CACHE_DIR", current_dir.join(".cache"))
        .output()
        .expect("run rvpm")
}

fn supported_runtime() -> bool {
    if !linker::linker_available() {
        eprintln!("rvpm_workspace: skipping, no linker available for the host");
        return false;
    }
    if locate_runtime().is_none() {
        eprintln!("rvpm_workspace: skipping, raven_runtime staticlib is not built");
        return false;
    }
    true
}

fn locate_runtime() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("RAVEN_RUNTIME_LIB") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }
    let name = if cfg!(windows) {
        "raven_runtime.lib"
    } else {
        "libraven_runtime.a"
    };
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    ["target/debug", "target/release"]
        .iter()
        .map(|directory| root.join(directory).join(name))
        .find(|path| path.is_file())
}

fn binary(root: &Path, name: &str) -> PathBuf {
    root.join("target/raven-out").join(if cfg!(windows) {
        format!("{}.exe", name)
    } else {
        name.to_string()
    })
}

fn workdir() -> PathBuf {
    let mut root = std::env::temp_dir();
    root.push(format!(
        "rvpm-workspace-cli-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn cleanup(root: &Path) {
    let _ = std::fs::remove_dir_all(root);
}

fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
