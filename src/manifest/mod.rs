//! The `rv.toml` manifest schema and parser for the rvpm package
//! manager.
//!
//! A manifest describes one Raven package: its identity (`[package]`),
//! its dependencies on other Raven packages (`[dependencies]`), optional
//! native linker pass-through (`[ffi]`), and optional formatter settings
//! (`[fmt]`). This module owns the schema and validation only. Fetching
//! dependencies, resolving version constraints, and wiring `[ffi]` into
//! the link step are handled by later rvpm work.
//!
//! See `docs/v2/specs/rv-toml.md` for the full field reference.

use std::fmt;
use std::path::Path;

use serde::Deserialize;

use crate::resolve::GithubPath;

pub mod init;

/// The default edition stamped into a freshly initialized manifest and
/// assumed when `[package].edition` is absent.
pub const DEFAULT_EDITION: &str = "v2";

/// The accepted `[package].edition` values.
pub const ACCEPTED_EDITIONS: &[&str] = &["v2", "2026"];

/// Whether `name` is a valid package name: a non-empty run of ASCII letters,
/// digits, `-`, and `_` that does not start with `-`. The name is interpolated
/// into scaffolded source and joined onto the output directory as a binary
/// name, so a path separator, `..`, or Windows device filename must be rejected.
pub fn is_valid_package_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('-')
        && !is_windows_reserved_package_name(name)
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn is_windows_reserved_package_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    matches!(lower.as_str(), "con" | "prn" | "aux" | "nul")
        || (lower.len() == 4
            && (lower.starts_with("com") || lower.starts_with("lpt"))
            && lower.as_bytes()[3].is_ascii_digit()
            && lower.as_bytes()[3] != b'0')
}

/// The default `[fmt].indent_width` when the section or field is absent.
pub const DEFAULT_INDENT_WIDTH: u32 = 4;

/// The default `[fmt].wrap_width` when the section or field is absent.
pub const DEFAULT_WRAP_WIDTH: u32 = 100;

/// The inclusive bounds for `[fmt].indent_width`, matching the documented range.
pub const MIN_INDENT_WIDTH: u32 = 1;
pub const MAX_INDENT_WIDTH: u32 = 16;

/// The inclusive bounds for `[fmt].wrap_width`, matching the documented range.
pub const MIN_WRAP_WIDTH: u32 = 40;
pub const MAX_WRAP_WIDTH: u32 = 200;

/// A parsed `rv.toml` manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub package: Package,
    /// Dependency keys are validated `github.com/<user>/<repo>` paths;
    /// values are the raw version-constraint strings (resolution is
    /// deferred to later rvpm work). Insertion order is not preserved.
    pub dependencies: Vec<Dependency>,
    pub ffi: Ffi,
    pub fmt: Fmt,
}

/// The `[package]` section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    pub name: String,
    pub version: String,
    pub authors: Vec<String>,
    pub edition: String,
}

/// One `[dependencies]` entry: a GitHub package path and its raw
/// version-constraint string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dependency {
    /// The original `github.com/<user>/<repo>` key as written.
    pub path: String,
    /// The parsed components of `path`.
    pub github: GithubPath,
    /// The raw constraint string, for example `"1.0"` or `"v1.2.3"`.
    /// Resolution lands in later rvpm work.
    pub constraint: String,
}

/// The optional `[ffi]` section: how a package's native code is linked into a
/// program that uses it. `rvpm build` compiles each `sources` C file and links
/// it in, links each `libs` library, and passes `link_args` to the linker. The
/// `[ffi]` of every dependency is collected, so a package can bundle its own C
/// (for example SQLite) and a consumer needs no system setup.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ffi {
    /// Bundled C source files, relative to the package root, compiled and
    /// linked into the final binary. For example `["c/sqlite3.c"]`.
    pub sources: Vec<String>,
    /// Library names to link, for example `["m", "z"]`.
    pub libs: Vec<String>,
    /// Extra raw linker arguments.
    pub link_args: Vec<String>,
}

/// The optional `[fmt]` section, carried from v1. Absent fields take the
/// documented defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fmt {
    pub indent_width: u32,
    pub wrap_width: u32,
}

impl Default for Fmt {
    fn default() -> Self {
        Fmt {
            indent_width: DEFAULT_INDENT_WIDTH,
            wrap_width: DEFAULT_WRAP_WIDTH,
        }
    }
}

/// An error produced while reading or parsing a manifest.
#[derive(Debug)]
pub enum ManifestError {
    /// The manifest file could not be read from disk.
    Io {
        path: String,
        source: std::io::Error,
    },
    /// The TOML did not parse. The message is the toml crate's
    /// diagnostic, prefixed with manifest context.
    Toml(String),
    /// A required field was absent.
    MissingField { section: String, field: String },
    /// A dependency key was not a recognized `github.com/<user>/<repo>`
    /// path.
    InvalidDependencyKey { key: String },
    /// A field held a value the schema does not accept.
    InvalidValue {
        section: String,
        field: String,
        message: String,
    },
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ManifestError::Io { path, source } => {
                write!(f, "cannot read manifest '{}': {}", path, source)
            }
            ManifestError::Toml(msg) => write!(f, "invalid rv.toml: {}", msg),
            ManifestError::MissingField { section, field } => {
                write!(
                    f,
                    "rv.toml is missing required field [{}].{}",
                    section, field
                )
            }
            ManifestError::InvalidDependencyKey { key } => write!(
                f,
                "invalid dependency key '{}': expected a 'github.com/<user>/<repo>' path",
                key
            ),
            ManifestError::InvalidValue {
                section,
                field,
                message,
            } => write!(f, "invalid value for [{}].{}: {}", section, field, message),
        }
    }
}

impl std::error::Error for ManifestError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ManifestError::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// The serde view of the raw TOML document, before validation. Every
/// field is optional here so we can produce precise MissingField errors
/// rather than serde's generic ones.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    package: Option<RawPackage>,
    #[serde(default)]
    dependencies: std::collections::BTreeMap<String, String>,
    ffi: Option<RawFfi>,
    fmt: Option<RawFmt>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPackage {
    name: Option<String>,
    version: Option<String>,
    #[serde(default)]
    authors: Vec<String>,
    edition: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFfi {
    #[serde(default)]
    sources: Vec<String>,
    #[serde(default)]
    libs: Vec<String>,
    #[serde(default)]
    link_args: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFmt {
    indent_width: Option<u32>,
    wrap_width: Option<u32>,
}

impl Manifest {
    /// Parse and validate a manifest from a TOML string.
    pub fn from_toml_str(s: &str) -> Result<Manifest, ManifestError> {
        let raw: RawManifest =
            toml::from_str(s).map_err(|e| ManifestError::Toml(e.message().to_string()))?;

        let raw_pkg = raw.package.ok_or_else(|| ManifestError::MissingField {
            section: "package".to_string(),
            field: "name".to_string(),
        })?;

        let name = raw_pkg.name.ok_or_else(|| ManifestError::MissingField {
            section: "package".to_string(),
            field: "name".to_string(),
        })?;
        let version = raw_pkg.version.ok_or_else(|| ManifestError::MissingField {
            section: "package".to_string(),
            field: "version".to_string(),
        })?;
        if name.trim().is_empty() {
            return Err(ManifestError::InvalidValue {
                section: "package".to_string(),
                field: "name".to_string(),
                message: "must not be empty".to_string(),
            });
        }
        if !is_valid_package_name(&name) {
            return Err(ManifestError::InvalidValue {
                section: "package".to_string(),
                field: "name".to_string(),
                message: "must contain only ASCII letters, digits, '-', and '_', may not start with '-', and may not be a reserved Windows device name (the name is used as a file and output binary name)".to_string(),
            });
        }
        let edition = match raw_pkg.edition {
            Some(e) => {
                if !ACCEPTED_EDITIONS.contains(&e.as_str()) {
                    return Err(ManifestError::InvalidValue {
                        section: "package".to_string(),
                        field: "edition".to_string(),
                        message: format!(
                            "'{}' is not accepted; use one of {}",
                            e,
                            ACCEPTED_EDITIONS.join(", ")
                        ),
                    });
                }
                e
            }
            None => DEFAULT_EDITION.to_string(),
        };

        let mut dependencies = Vec::with_capacity(raw.dependencies.len());
        for (key, constraint) in raw.dependencies {
            let github = GithubPath::parse(&key)
                .ok_or_else(|| ManifestError::InvalidDependencyKey { key: key.clone() })?;
            // A dependency identifies a whole repository, so its key must be a
            // bare `github.com/<user>/<repo>`. A subpath (`.../repo/lib`) would
            // be recorded in the lock as the source, but import resolution looks
            // up the repository identity `github.com/<user>/<repo>`, so the lock
            // entry could never be matched.
            if !github.subpath.is_empty() {
                return Err(ManifestError::InvalidDependencyKey { key });
            }
            dependencies.push(Dependency {
                path: key,
                github,
                constraint,
            });
        }

        let ffi = raw
            .ffi
            .map(|f| Ffi {
                sources: f.sources,
                libs: f.libs,
                link_args: f.link_args,
            })
            .unwrap_or_default();

        let fmt = match raw.fmt {
            Some(f) => {
                let indent_width = f.indent_width.unwrap_or(DEFAULT_INDENT_WIDTH);
                if !(MIN_INDENT_WIDTH..=MAX_INDENT_WIDTH).contains(&indent_width) {
                    return Err(ManifestError::InvalidValue {
                        section: "fmt".to_string(),
                        field: "indent_width".to_string(),
                        message: format!(
                            "must be between {} and {}",
                            MIN_INDENT_WIDTH, MAX_INDENT_WIDTH
                        ),
                    });
                }
                let wrap_width = f.wrap_width.unwrap_or(DEFAULT_WRAP_WIDTH);
                if !(MIN_WRAP_WIDTH..=MAX_WRAP_WIDTH).contains(&wrap_width) {
                    return Err(ManifestError::InvalidValue {
                        section: "fmt".to_string(),
                        field: "wrap_width".to_string(),
                        message: format!(
                            "must be between {} and {}",
                            MIN_WRAP_WIDTH, MAX_WRAP_WIDTH
                        ),
                    });
                }
                Fmt {
                    indent_width,
                    wrap_width,
                }
            }
            None => Fmt::default(),
        };

        Ok(Manifest {
            package: Package {
                name,
                version,
                authors: raw_pkg.authors,
                edition,
            },
            dependencies,
            ffi,
            fmt,
        })
    }

    /// Read and parse the manifest at `path`.
    pub fn load(path: impl AsRef<Path>) -> Result<Manifest, ManifestError> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|e| ManifestError::Io {
            path: path.display().to_string(),
            source: e,
        })?;
        Manifest::from_toml_str(&text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_name_validation() {
        assert!(is_valid_package_name("raven-http"));
        assert!(is_valid_package_name("my_lib2"));
        // Path-traversal and separator forms are rejected.
        assert!(!is_valid_package_name("../../../escaped"));
        assert!(!is_valid_package_name("a/b"));
        assert!(!is_valid_package_name(".."));
        assert!(!is_valid_package_name("-leading"));
        assert!(!is_valid_package_name(""));
        assert!(!is_valid_package_name("has space"));
        assert!(!is_valid_package_name("con"));
        assert!(!is_valid_package_name("COM1"));
        assert!(!is_valid_package_name("lpt9"));
        assert!(is_valid_package_name("com10"));
        assert!(is_valid_package_name("console"));
    }

    #[test]
    fn manifest_rejects_a_traversal_name() {
        let src = "[package]\nname = \"../../escaped\"\nversion = \"0.1.0\"\n";
        assert!(Manifest::from_toml_str(src).is_err());
    }

    #[test]
    fn full_manifest_parses() {
        let src = r#"
[package]
name = "demo"
version = "0.1.0"
authors = ["Ada", "Grace"]
edition = "v2"

[dependencies]
"github.com/martian56/raven-http" = "1.0"
"github.com/acme/json" = "v2.3.1"

[ffi]
libs = ["m", "z"]
link_args = ["-L/opt/lib"]

[fmt]
indent_width = 2
wrap_width = 80
"#;
        let m = Manifest::from_toml_str(src).expect("parses");
        assert_eq!(m.package.name, "demo");
        assert_eq!(m.package.version, "0.1.0");
        assert_eq!(m.package.authors, vec!["Ada", "Grace"]);
        assert_eq!(m.package.edition, "v2");
        assert_eq!(m.dependencies.len(), 2);
        let http = m
            .dependencies
            .iter()
            .find(|d| d.path == "github.com/martian56/raven-http")
            .expect("http dep present");
        assert_eq!(http.constraint, "1.0");
        assert_eq!(http.github.user, "martian56");
        assert_eq!(http.github.repo, "raven-http");
        assert_eq!(m.ffi.libs, vec!["m", "z"]);
        assert_eq!(m.ffi.link_args, vec!["-L/opt/lib"]);
        assert_eq!(m.fmt.indent_width, 2);
        assert_eq!(m.fmt.wrap_width, 80);
    }

    #[test]
    fn minimal_manifest_uses_defaults() {
        let src = r#"
[package]
name = "tiny"
version = "0.0.1"
"#;
        let m = Manifest::from_toml_str(src).expect("parses");
        assert_eq!(m.package.name, "tiny");
        assert_eq!(m.package.edition, DEFAULT_EDITION);
        assert!(m.package.authors.is_empty());
        assert!(m.dependencies.is_empty());
        assert_eq!(m.ffi, Ffi::default());
        assert_eq!(m.fmt.indent_width, DEFAULT_INDENT_WIDTH);
        assert_eq!(m.fmt.wrap_width, DEFAULT_WRAP_WIDTH);
    }

    #[test]
    fn fmt_widths_outside_bounds_are_rejected() {
        let indent_too_big =
            "[package]\nname = \"x\"\nversion = \"0.0.1\"\n[fmt]\nindent_width = 100\n";
        assert!(Manifest::from_toml_str(indent_too_big).is_err());
        let wrap_too_small =
            "[package]\nname = \"x\"\nversion = \"0.0.1\"\n[fmt]\nwrap_width = 10\n";
        assert!(Manifest::from_toml_str(wrap_too_small).is_err());
        // The documented bounds (indent 1..=16, wrap 40..=200) parse fine.
        let in_bounds =
            "[package]\nname = \"x\"\nversion = \"0.0.1\"\n[fmt]\nindent_width = 16\nwrap_width = 200\n";
        let m = Manifest::from_toml_str(in_bounds).expect("in-bounds widths parse");
        assert_eq!(m.fmt.indent_width, 16);
        assert_eq!(m.fmt.wrap_width, 200);
    }

    #[test]
    fn dependency_subpath_key_is_rejected() {
        // A dependency key with a subpath cannot be matched to a lock entry, so
        // it is rejected (issue #718).
        let with_subpath = "[package]\nname = \"x\"\nversion = \"0.0.1\"\n[dependencies]\n\"github.com/acme/demo/lib\" = \"1.0\"\n";
        assert!(Manifest::from_toml_str(with_subpath).is_err());
        // A bare repository key is accepted.
        let bare = "[package]\nname = \"x\"\nversion = \"0.0.1\"\n[dependencies]\n\"github.com/acme/demo\" = \"1.0\"\n";
        let m = Manifest::from_toml_str(bare).expect("bare dependency key parses");
        assert_eq!(m.dependencies.len(), 1);
    }

    #[test]
    fn missing_name_is_reported() {
        let src = r#"
[package]
version = "0.1.0"
"#;
        let err = Manifest::from_toml_str(src).unwrap_err();
        match err {
            ManifestError::MissingField { section, field } => {
                assert_eq!(section, "package");
                assert_eq!(field, "name");
            }
            other => panic!("expected MissingField, got {:?}", other),
        }
    }

    #[test]
    fn missing_version_is_reported() {
        let src = r#"
[package]
name = "x"
"#;
        let err = Manifest::from_toml_str(src).unwrap_err();
        match err {
            ManifestError::MissingField { section, field } => {
                assert_eq!(section, "package");
                assert_eq!(field, "version");
            }
            other => panic!("expected MissingField, got {:?}", other),
        }
    }

    #[test]
    fn missing_package_section_reports_name() {
        let src = r#"
[dependencies]
"github.com/a/b" = "1.0"
"#;
        let err = Manifest::from_toml_str(src).unwrap_err();
        assert!(matches!(err, ManifestError::MissingField { .. }));
        assert!(err.to_string().contains("[package].name"));
    }

    #[test]
    fn malformed_dependency_key_is_reported() {
        let src = r#"
[package]
name = "x"
version = "0.1.0"

[dependencies]
"gitlab.com/a/b" = "1.0"
"#;
        let err = Manifest::from_toml_str(src).unwrap_err();
        match err {
            ManifestError::InvalidDependencyKey { key } => {
                assert_eq!(key, "gitlab.com/a/b");
            }
            other => panic!("expected InvalidDependencyKey, got {:?}", other),
        }
    }

    #[test]
    fn dependency_key_missing_repo_is_reported() {
        let src = r#"
[package]
name = "x"
version = "0.1.0"

[dependencies]
"github.com/onlyuser" = "1.0"
"#;
        let err = Manifest::from_toml_str(src).unwrap_err();
        assert!(matches!(err, ManifestError::InvalidDependencyKey { .. }));
    }

    #[test]
    fn toml_syntax_error_is_wrapped() {
        let src = "[package\nname = \"x\"\n";
        let err = Manifest::from_toml_str(src).unwrap_err();
        match &err {
            ManifestError::Toml(msg) => assert!(!msg.is_empty()),
            other => panic!("expected Toml, got {:?}", other),
        }
        assert!(err.to_string().starts_with("invalid rv.toml:"));
    }

    #[test]
    fn unknown_edition_is_rejected() {
        let src = r#"
[package]
name = "x"
version = "0.1.0"
edition = "2015"
"#;
        let err = Manifest::from_toml_str(src).unwrap_err();
        match err {
            ManifestError::InvalidValue { field, .. } => assert_eq!(field, "edition"),
            other => panic!("expected InvalidValue, got {:?}", other),
        }
    }

    #[test]
    fn ffi_section_parses_alone() {
        let src = r#"
[package]
name = "x"
version = "0.1.0"

[ffi]
libs = ["ssl", "crypto"]
"#;
        let m = Manifest::from_toml_str(src).expect("parses");
        assert_eq!(m.ffi.libs, vec!["ssl", "crypto"]);
        assert!(m.ffi.link_args.is_empty());
    }

    #[test]
    fn fmt_partial_fills_defaults() {
        let src = r#"
[package]
name = "x"
version = "0.1.0"

[fmt]
indent_width = 8
"#;
        let m = Manifest::from_toml_str(src).expect("parses");
        assert_eq!(m.fmt.indent_width, 8);
        assert_eq!(m.fmt.wrap_width, DEFAULT_WRAP_WIDTH);
    }

    #[test]
    fn unknown_field_is_rejected() {
        let src = r#"
[package]
name = "x"
version = "0.1.0"
license = "MIT"
"#;
        let err = Manifest::from_toml_str(src).unwrap_err();
        assert!(matches!(err, ManifestError::Toml(_)));
    }
}
