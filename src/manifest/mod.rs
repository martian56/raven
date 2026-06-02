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

/// The default `[fmt].indent_width` when the section or field is absent.
pub const DEFAULT_INDENT_WIDTH: u32 = 4;

/// The default `[fmt].wrap_width` when the section or field is absent.
pub const DEFAULT_WRAP_WIDTH: u32 = 100;

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

/// The optional `[ffi]` section. Linker pass-through; wiring into the
/// actual link step is deferred.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ffi {
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
            dependencies.push(Dependency {
                path: key,
                github,
                constraint,
            });
        }

        let ffi = raw
            .ffi
            .map(|f| Ffi {
                libs: f.libs,
                link_args: f.link_args,
            })
            .unwrap_or_default();

        let fmt = raw
            .fmt
            .map(|f| Fmt {
                indent_width: f.indent_width.unwrap_or(DEFAULT_INDENT_WIDTH),
                wrap_width: f.wrap_width.unwrap_or(DEFAULT_WRAP_WIDTH),
            })
            .unwrap_or_default();

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
