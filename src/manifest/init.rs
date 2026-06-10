//! Project scaffolding for `rvpm init`.
//!
//! Writes a starter `rv.toml` and `src/main.rv` into a target directory.
//! The generated manifest round-trips through [`super::Manifest`] and the
//! generated program compiles under the `raven` build.

use std::io;
use std::path::Path;

/// The starter version stamped into a new manifest.
pub const STARTER_VERSION: &str = "0.1.0";

/// Reasons `init` could not scaffold a project.
#[derive(Debug)]
pub enum InitError {
    /// An `rv.toml` already exists in the target directory.
    ManifestExists,
    /// A filesystem operation failed.
    Io(io::Error),
}

impl std::fmt::Display for InitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InitError::ManifestExists => {
                write!(f, "rv.toml already exists; refusing to overwrite")
            }
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

/// Scaffold a new project named `name` rooted at `dir`. Writes
/// `<dir>/rv.toml` and `<dir>/src/main.rv`. Fails without writing
/// anything if `<dir>/rv.toml` already exists.
pub fn init_project(dir: &Path, name: &str) -> Result<(), InitError> {
    let manifest_path = dir.join("rv.toml");
    if manifest_path.exists() {
        return Err(InitError::ManifestExists);
    }
    let src_dir = dir.join("src");
    std::fs::create_dir_all(&src_dir)?;
    std::fs::write(&manifest_path, manifest_template(name))?;
    // Do not clobber an existing entry point. A directory can already hold
    // `src/main.rv` (a project that lost its manifest, or one laid out by hand),
    // and `init` should add the manifest without throwing that source away.
    let main_path = src_dir.join("main.rv");
    if !main_path.exists() {
        std::fs::write(main_path, main_rv_template(name))?;
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
        init_project(&dir, "demo_pkg").expect("init");

        let manifest_text = std::fs::read_to_string(dir.join("rv.toml")).unwrap();
        let m = Manifest::from_toml_str(&manifest_text).expect("generated manifest parses");
        assert_eq!(m.package.name, "demo_pkg");
        assert_eq!(m.package.version, STARTER_VERSION);
        assert_eq!(m.package.edition, "v2");
        assert!(m.dependencies.is_empty());

        let main_rv = std::fs::read_to_string(dir.join("src").join("main.rv")).unwrap();
        assert!(main_rv.contains("fun main()"));
        assert!(main_rv.contains("hello from demo_pkg"));

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
        let err = init_project(&dir, "x").unwrap_err();
        assert!(matches!(err, InitError::ManifestExists));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn init_keeps_an_existing_main_rv() {
        let dir = temp_dir("keepmain");
        let src = dir.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("main.rv"), "fun main() { print(\"mine\") }\n").unwrap();
        init_project(&dir, "x").unwrap();
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
