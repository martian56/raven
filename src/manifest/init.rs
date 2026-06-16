//! Project scaffolding for `rvpm init`.
//!
//! Writes a starter manifest, a `.gitignore`, and an entry file into a target
//! directory: `src/main.rv` for an application, or `lib.rv` at the root for a
//! library (the entry point other projects import). The generated manifest
//! round-trips through [`super::Manifest`] and the generated source compiles
//! under the `raven` build.

use std::io;
use std::path::Path;

/// The starter version stamped into a new manifest.
pub const STARTER_VERSION: &str = "0.1.0";

/// What `init` scaffolds: an application (an executable entered through
/// `src/main.rv`) or a library (a package exposing `lib.rv` at its root, the
/// file other projects load via `import "github.com/<user>/<repo>"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectKind {
    App,
    Lib,
}

/// Reasons `init` could not scaffold a project.
#[derive(Debug)]
pub enum InitError {
    /// An `rv.toml` already exists in the target directory.
    ManifestExists,
    /// The package name is not a valid identifier.
    InvalidName(String),
    /// A filesystem operation failed.
    Io(io::Error),
}

impl std::fmt::Display for InitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InitError::ManifestExists => {
                write!(f, "rv.toml already exists; refusing to overwrite")
            }
            InitError::InvalidName(name) => write!(
                f,
                "'{}' is not a valid package name; use only ASCII letters, digits, '-', and '_'",
                name
            ),
            InitError::Io(e) => write!(f, "{}", e),
        }
    }
}

impl std::error::Error for InitError {}

impl From<io::Error> for InitError {
    fn from(e: io::Error) -> Self {
        InitError::Io(e)
    }
}

/// Render the starter manifest text for `name`.
pub fn manifest_template(name: &str) -> String {
    format!(
        "[package]\n\
         name = \"{name}\"\n\
         version = \"{STARTER_VERSION}\"\n\
         edition = \"v2\"\n\
         \n\
         [dependencies]\n"
    )
}

/// Render the starter `src/main.rv` for `name`.
pub fn main_rv_template(name: &str) -> String {
    format!("fun main() {{\n    print(\"hello from {name}\")\n}}\n")
}

/// Render the starter `lib.rv` for a library named `name`. The declarations
/// here are what other projects import via `import "github.com/<user>/<repo>"`.
pub fn lib_rv_template(name: &str) -> String {
    format!(
        "// lib.rv is the entry point of the `{name}` package: the items declared\n\
         // here are what other projects import, for example:\n\
         //   import \"github.com/<user>/{name}\" {{ greet, Greeting }}\n\
         //\n\
         // A sub-path import like `github.com/<user>/{name}/util` loads `util.rv`\n\
         // next to this file.\n\
         \n\
         import std/string\n\
         \n\
         struct Greeting {{\n    who: String,\n}}\n\
         \n\
         fun greet(who: String) -> String {{\n    return \"hello, \".concat(who)\n}}\n"
    )
}

/// The starter `.gitignore`: ignore the build output. `rvpm build` writes the
/// binary under `target/raven-out/`, and the runtime staticlib is staged in
/// `target/` too, so the whole directory is generated.
pub fn gitignore_template() -> &'static str {
    "# Raven build output\n/target/\n"
}

/// Scaffold a new project named `name` rooted at `dir`. Writes `<dir>/rv.toml`,
/// a `<dir>/.gitignore`, and the entry file: `<dir>/src/main.rv` for
/// [`ProjectKind::App`] or `<dir>/lib.rv` for [`ProjectKind::Lib`]. Fails
/// without writing anything if `<dir>/rv.toml` already exists; an existing
/// `.gitignore` or entry file is left untouched.
pub fn init_project(dir: &Path, name: &str, kind: ProjectKind) -> Result<(), InitError> {
    if !super::is_valid_package_name(name) {
        return Err(InitError::InvalidName(name.to_string()));
    }
    let manifest_path = dir.join("rv.toml");
    if manifest_path.exists() {
        return Err(InitError::ManifestExists);
    }
    std::fs::write(&manifest_path, manifest_template(name))?;

    // A `.gitignore` for the generated build output. Do not clobber one the
    // user already wrote.
    let gitignore_path = dir.join(".gitignore");
    if !gitignore_path.exists() {
        std::fs::write(gitignore_path, gitignore_template())?;
    }

    // The entry file. Do not clobber an existing one: a directory can already
    // hold source (a project that lost its manifest, or one laid out by hand),
    // and `init` should add the manifest without throwing that source away.
    match kind {
        ProjectKind::App => {
            let src_dir = dir.join("src");
            std::fs::create_dir_all(&src_dir)?;
            let main_path = src_dir.join("main.rv");
            if !main_path.exists() {
                std::fs::write(main_path, main_rv_template(name))?;
            }
        }
        ProjectKind::Lib => {
            let lib_path = dir.join("lib.rv");
            if !lib_path.exists() {
                std::fs::write(lib_path, lib_rv_template(name))?;
            }
        }
    }
    Ok(())
}

/// Derive a package name from a directory path, falling back to
/// `"app"` when no usable file name is present.
pub fn name_from_dir(dir: &Path) -> String {
    dir.file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("app")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Manifest;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let mut d = std::env::temp_dir();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        d.push(format!("rvpm_init_{}_{}", tag, nanos));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn init_writes_files_that_parse() {
        let dir = temp_dir("ok");
        init_project(&dir, "demo_pkg", ProjectKind::App).expect("init");

        let manifest_text = std::fs::read_to_string(dir.join("rv.toml")).unwrap();
        let m = Manifest::from_toml_str(&manifest_text).expect("generated manifest parses");
        assert_eq!(m.package.name, "demo_pkg");
        assert_eq!(m.package.version, STARTER_VERSION);
        assert_eq!(m.package.edition, "v2");
        assert!(m.dependencies.is_empty());

        let main_rv = std::fs::read_to_string(dir.join("src").join("main.rv")).unwrap();
        assert!(main_rv.contains("fun main()"));
        assert!(main_rv.contains("hello from demo_pkg"));

        // A .gitignore is written that ignores the build output.
        let gitignore = std::fs::read_to_string(dir.join(".gitignore")).unwrap();
        assert!(gitignore.contains("/target/"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn init_lib_writes_lib_rv_at_root() {
        let dir = temp_dir("lib");
        init_project(&dir, "mylib", ProjectKind::Lib).expect("init lib");

        // The entry point is `lib.rv` at the root, not `src/main.rv`.
        let lib_rv = std::fs::read_to_string(dir.join("lib.rv")).unwrap();
        assert!(lib_rv.contains("fun greet"));
        assert!(!dir.join("src").join("main.rv").exists());
        assert!(dir.join(".gitignore").exists());
        assert!(dir.join("rv.toml").exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn init_keeps_an_existing_gitignore() {
        let dir = temp_dir("keepignore");
        std::fs::write(dir.join(".gitignore"), "custom\n").unwrap();
        init_project(&dir, "x", ProjectKind::App).unwrap();
        let gi = std::fs::read_to_string(dir.join(".gitignore")).unwrap();
        assert_eq!(gi, "custom\n", "init clobbered an existing .gitignore");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn init_refuses_to_overwrite() {
        let dir = temp_dir("exists");
        std::fs::write(
            dir.join("rv.toml"),
            "[package]\nname=\"a\"\nversion=\"0.1.0\"\n",
        )
        .unwrap();
        let err = init_project(&dir, "x", ProjectKind::App).unwrap_err();
        assert!(matches!(err, InitError::ManifestExists));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn init_keeps_an_existing_main_rv() {
        let dir = temp_dir("keepmain");
        let src = dir.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("main.rv"), "fun main() { print(\"mine\") }\n").unwrap();
        init_project(&dir, "x", ProjectKind::App).unwrap();
        let main_rv = std::fs::read_to_string(src.join("main.rv")).unwrap();
        assert!(
            main_rv.contains("mine"),
            "init clobbered the existing source"
        );
        assert!(dir.join("rv.toml").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn name_from_dir_falls_back() {
        assert_eq!(name_from_dir(Path::new("/tmp/myproj")), "myproj");
    }
}
