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
    /// `Some` when the manifest has a `[dist]` section, with absent fields
    /// already filled from `[package]`. `None` means the section is absent;
    /// `rvpm dist` then acts as if it were `Dist::with_defaults`.
    pub dist: Option<Dist>,
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

/// The artifact formats `rvpm dist` can produce.
pub const DIST_TARGETS: &[&str] = &["tar", "zip", "deb", "rpm", "msi", "inno"];

/// The default `[dist].out_dir`, relative to the package root.
pub const DEFAULT_DIST_OUT_DIR: &str = "target/dist";

/// The optional `[dist]` section: how `rvpm dist` packages the built
/// application. Every field has a default derived from `[package]`, so the
/// section can be omitted entirely and `rvpm dist` still produces the host's
/// native archive. See `docs/v2/specs/rvpm-dist.md`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dist {
    /// Artifact formats to produce when `--target` is not given. Empty means
    /// the host default: `tar` on Unix, `zip` on Windows.
    pub targets: Vec<String>,
    /// Where artifacts land, relative to the package root.
    pub out_dir: String,
    /// Human-facing application name, used in installer titles and shortcuts.
    pub display_name: String,
    /// One-line description, used by deb, rpm, and the installers.
    pub description: String,
    /// SPDX-style license name, used by rpm and the installers.
    pub license: String,
    /// Project URL, used by deb, rpm, and the installers.
    pub homepage: String,
    /// `Name <email>` contact, required by deb; defaults to the first
    /// `[package].authors` entry.
    pub maintainer: String,
    /// Organization name for rpm and msi; defaults to the maintainer.
    pub vendor: String,
    /// Extra files installed alongside the binary.
    pub assets: Vec<DistAsset>,
    pub linux: DistLinux,
    pub windows: DistWindows,
}

/// One `[[dist.assets]]` entry. `source` is a file or directory read relative
/// to the package root. Directories are copied recursively beneath `dest`.
/// `dest` is a forward-slash install path relative to the install prefix:
/// `/usr/` for deb and rpm, the archive root for tar and zip, and the
/// application folder for msi and inno.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DistAsset {
    pub source: String,
    pub dest: String,
}

/// The `[dist.linux]` subsection, shared by the deb and rpm backends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DistLinux {
    /// Package dependencies, in each format's own syntax (they are passed
    /// through verbatim to `Depends:` and `Requires:`).
    pub depends: Vec<String>,
    /// The deb archive section.
    pub section: String,
    /// The deb priority.
    pub priority: String,
}

impl Default for DistLinux {
    fn default() -> Self {
        DistLinux {
            depends: Vec::new(),
            section: "utils".to_string(),
            priority: "optional".to_string(),
        }
    }
}

/// The `[dist.windows]` subsection, shared by the msi and inno backends.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DistWindows {
    /// An .ico file relative to the package root, used by the installers.
    pub icon: String,
    /// The stable GUID that lets an msi upgrade an installed older version.
    /// Required by the msi backend; generate one once and keep it.
    pub upgrade_code: String,
    /// When set, the msi appends the install directory to the system PATH, so
    /// a command-line tool is callable from a terminal after installing.
    pub add_to_path: bool,
}

impl Dist {
    /// The `[dist]` configuration an absent section stands for: host-default
    /// target, everything else derived from `[package]`.
    pub fn with_defaults(package: &Package) -> Dist {
        Dist {
            targets: Vec::new(),
            out_dir: DEFAULT_DIST_OUT_DIR.to_string(),
            display_name: package.name.clone(),
            description: format!("{} {}", package.name, package.version),
            license: String::new(),
            homepage: String::new(),
            maintainer: package
                .authors
                .first()
                .cloned()
                .unwrap_or_else(|| format!("{} maintainers", package.name)),
            vendor: String::new(),
            assets: Vec::new(),
            linux: DistLinux::default(),
            windows: DistWindows::default(),
        }
    }
}

/// Whether `p` is safe to join under a staging or install root: relative,
/// forward slashes only, and free of `.` and `..` components. The same
/// containment idea as `checked_ffi_source`, applied at parse time.
pub fn is_safe_dist_path(p: &str) -> bool {
    !p.is_empty()
        && !p.contains('\\')
        && !p.starts_with('/')
        && !p.contains(':')
        && p.split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

fn is_guid(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() != 36 {
        return false;
    }
    for (i, b) in bytes.iter().enumerate() {
        let is_sep = matches!(i, 8 | 13 | 18 | 23);
        if is_sep != (*b == b'-') {
            return false;
        }
        if !is_sep && !b.is_ascii_hexdigit() {
            return false;
        }
    }
    true
}

/// Reject a control character in a `[dist]` text field. These fields are
/// written into line-oriented and scripted packaging files, where a newline
/// or other control byte could inject an extra field or section. A real
/// value (a name, a description, a dependency) never contains one.
fn sanitize_dist_text(field: &str, value: String) -> Result<String, ManifestError> {
    if let Some(bad) = value.chars().find(|c| c.is_control()) {
        return Err(ManifestError::InvalidValue {
            section: "dist".to_string(),
            field: field.to_string(),
            message: format!(
                "must not contain control characters (found U+{:04X}); dist metadata is written into generated packaging files",
                bad as u32
            ),
        });
    }
    Ok(value)
}

/// Apply [`sanitize_dist_text`] to each entry of a `[dist]` string list.
fn sanitize_dist_list(field: &str, values: Vec<String>) -> Result<Vec<String>, ManifestError> {
    values
        .into_iter()
        .map(|v| sanitize_dist_text(field, v))
        .collect()
}

/// Validate a raw `[dist]` section against the schema, filling absent
/// fields from `package`.
fn validate_dist(raw: RawDist, package: &Package) -> Result<Dist, ManifestError> {
    let invalid = |field: &str, message: String| ManifestError::InvalidValue {
        section: "dist".to_string(),
        field: field.to_string(),
        message,
    };

    for t in &raw.targets {
        if !DIST_TARGETS.contains(&t.as_str()) {
            return Err(invalid(
                "targets",
                format!(
                    "'{}' is not a known target; use any of {}",
                    t,
                    DIST_TARGETS.join(", ")
                ),
            ));
        }
    }

    let out_dir = raw
        .out_dir
        .unwrap_or_else(|| DEFAULT_DIST_OUT_DIR.to_string());
    if !is_safe_dist_path(&out_dir) {
        return Err(invalid(
            "out_dir",
            "must be a relative forward-slash path inside the package".to_string(),
        ));
    }

    let mut assets = Vec::with_capacity(raw.assets.len());
    for a in raw.assets {
        if !is_safe_dist_path(&a.source) {
            return Err(invalid(
                "assets.source",
                format!(
                    "'{}' must be a relative forward-slash path inside the package",
                    a.source
                ),
            ));
        }
        if !is_safe_dist_path(&a.dest) {
            return Err(invalid(
                "assets.dest",
                format!(
                    "'{}' must be a relative forward-slash path under the install prefix",
                    a.dest
                ),
            ));
        }
        assets.push(DistAsset {
            source: a.source,
            dest: a.dest,
        });
    }

    let linux = raw.linux.unwrap_or_default();
    let windows = raw.windows.unwrap_or_default();
    if let Some(icon) = &windows.icon {
        if !is_safe_dist_path(icon) {
            return Err(invalid(
                "windows.icon",
                format!(
                    "'{}' must be a relative forward-slash path inside the package",
                    icon
                ),
            ));
        }
    }
    if let Some(code) = &windows.upgrade_code {
        if !is_guid(code) {
            return Err(invalid(
                "windows.upgrade_code",
                format!(
                    "'{}' is not a GUID (expected xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx)",
                    code
                ),
            ));
        }
    }

    // Every text field below is written verbatim into a line-oriented or
    // scripted packaging file (the deb control file, the rpm spec, the Inno
    // Setup script). A control character, above all a newline, could start a
    // new field or section there and, for rpm and inno, run commands during
    // packaging. Reject them at the boundary so no backend has to trust the
    // metadata. Path and GUID fields are already constrained above.
    let defaults = Dist::with_defaults(package);
    let maintainer =
        sanitize_dist_text("maintainer", raw.maintainer.unwrap_or(defaults.maintainer))?;
    Ok(Dist {
        targets: raw.targets,
        out_dir,
        display_name: sanitize_dist_text(
            "display_name",
            raw.display_name.unwrap_or(defaults.display_name),
        )?,
        description: sanitize_dist_text(
            "description",
            raw.description.unwrap_or(defaults.description),
        )?,
        license: sanitize_dist_text("license", raw.license.unwrap_or_default())?,
        homepage: sanitize_dist_text("homepage", raw.homepage.unwrap_or_default())?,
        vendor: sanitize_dist_text("vendor", raw.vendor.unwrap_or_else(|| maintainer.clone()))?,
        maintainer,
        assets,
        linux: DistLinux {
            depends: sanitize_dist_list("linux.depends", linux.depends)?,
            section: sanitize_dist_text(
                "linux.section",
                linux.section.unwrap_or_else(|| "utils".to_string()),
            )?,
            priority: sanitize_dist_text(
                "linux.priority",
                linux.priority.unwrap_or_else(|| "optional".to_string()),
            )?,
        },
        windows: DistWindows {
            icon: windows.icon.unwrap_or_default(),
            upgrade_code: windows.upgrade_code.unwrap_or_default(),
            add_to_path: windows.add_to_path.unwrap_or(false),
        },
    })
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
    dist: Option<RawDist>,
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

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDist {
    #[serde(default)]
    targets: Vec<String>,
    out_dir: Option<String>,
    display_name: Option<String>,
    description: Option<String>,
    license: Option<String>,
    homepage: Option<String>,
    maintainer: Option<String>,
    vendor: Option<String>,
    #[serde(default)]
    assets: Vec<RawDistAsset>,
    linux: Option<RawDistLinux>,
    windows: Option<RawDistWindows>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDistAsset {
    source: String,
    dest: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDistLinux {
    #[serde(default)]
    depends: Vec<String>,
    section: Option<String>,
    priority: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDistWindows {
    icon: Option<String>,
    upgrade_code: Option<String>,
    add_to_path: Option<bool>,
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
        // The version is copied verbatim into generated packaging text (the rpm
        // spec `Version:`, the deb control `Version:`, the Inno Setup
        // `AppVersion=`), where a control character such as a newline could
        // inject an additional directive. A real version never contains one.
        if let Some(bad) = version.chars().find(|c| c.is_control()) {
            return Err(ManifestError::InvalidValue {
                section: "package".to_string(),
                field: "version".to_string(),
                message: format!(
                    "must not contain control characters (found U+{:04X})",
                    bad as u32
                ),
            });
        }
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

        let package = Package {
            name,
            version,
            authors: raw_pkg.authors,
            edition,
        };

        let dist = match raw.dist {
            Some(d) => Some(validate_dist(d, &package)?),
            None => None,
        };

        Ok(Manifest {
            package,
            dependencies,
            ffi,
            fmt,
            dist,
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

    #[test]
    fn manifest_without_dist_has_none() {
        let src = "[package]\nname = \"x\"\nversion = \"0.1.0\"\n";
        let m = Manifest::from_toml_str(src).expect("parses");
        assert!(m.dist.is_none());
    }

    #[test]
    fn empty_dist_section_fills_defaults_from_package() {
        let src = r#"
[package]
name = "rook"
version = "0.2.0"
authors = ["Ada <ada@example.com>"]

[dist]
"#;
        let m = Manifest::from_toml_str(src).expect("parses");
        let d = m.dist.expect("dist present");
        assert!(d.targets.is_empty());
        assert_eq!(d.out_dir, DEFAULT_DIST_OUT_DIR);
        assert_eq!(d.display_name, "rook");
        assert_eq!(d.description, "rook 0.2.0");
        assert_eq!(d.maintainer, "Ada <ada@example.com>");
        assert_eq!(d.vendor, "Ada <ada@example.com>");
        assert_eq!(d.linux.section, "utils");
        assert_eq!(d.linux.priority, "optional");
    }

    #[test]
    fn full_dist_section_parses() {
        let src = r#"
[package]
name = "rook"
version = "0.2.0"

[dist]
targets = ["deb", "zip"]
out_dir = "artifacts"
display_name = "Rook"
description = "A coding agent for Raven"
license = "MIT"
homepage = "https://example.com/rook"
maintainer = "Ada <ada@example.com>"
vendor = "Acme"

[[dist.assets]]
source = "README.md"
dest = "share/doc/rook/README.md"

[dist.linux]
depends = ["libc6 (>= 2.31)"]
section = "devel"

[dist.windows]
icon = "assets/rook.ico"
upgrade_code = "9f0c86a1-2b3c-4d5e-8f90-112233445566"
"#;
        let m = Manifest::from_toml_str(src).expect("parses");
        let d = m.dist.expect("dist present");
        assert_eq!(d.targets, vec!["deb", "zip"]);
        assert_eq!(d.out_dir, "artifacts");
        assert_eq!(d.display_name, "Rook");
        assert_eq!(d.vendor, "Acme");
        assert_eq!(d.assets.len(), 1);
        assert_eq!(d.assets[0].dest, "share/doc/rook/README.md");
        assert_eq!(d.linux.depends, vec!["libc6 (>= 2.31)"]);
        assert_eq!(d.linux.section, "devel");
        assert_eq!(d.linux.priority, "optional");
        assert_eq!(d.windows.icon, "assets/rook.ico");
        assert_eq!(
            d.windows.upgrade_code,
            "9f0c86a1-2b3c-4d5e-8f90-112233445566"
        );
    }

    #[test]
    fn unknown_dist_target_is_rejected() {
        let src = "[package]\nname = \"x\"\nversion = \"0.1.0\"\n[dist]\ntargets = [\"pkg\"]\n";
        let err = Manifest::from_toml_str(src).unwrap_err();
        match err {
            ManifestError::InvalidValue { section, field, .. } => {
                assert_eq!(section, "dist");
                assert_eq!(field, "targets");
            }
            other => panic!("expected InvalidValue, got {:?}", other),
        }
    }

    #[test]
    fn dist_traversal_paths_are_rejected() {
        let bad_dest = "[package]\nname = \"x\"\nversion = \"0.1.0\"\n[[dist.assets]]\nsource = \"a\"\ndest = \"../../etc/passwd\"\n";
        assert!(Manifest::from_toml_str(bad_dest).is_err());
        let abs_source = "[package]\nname = \"x\"\nversion = \"0.1.0\"\n[[dist.assets]]\nsource = \"/etc/passwd\"\ndest = \"a\"\n";
        assert!(Manifest::from_toml_str(abs_source).is_err());
        let bad_out =
            "[package]\nname = \"x\"\nversion = \"0.1.0\"\n[dist]\nout_dir = \"C:/tmp\"\n";
        assert!(Manifest::from_toml_str(bad_out).is_err());
    }

    #[test]
    fn dist_bad_upgrade_code_is_rejected() {
        let src = "[package]\nname = \"x\"\nversion = \"0.1.0\"\n[dist.windows]\nupgrade_code = \"not-a-guid\"\n";
        let err = Manifest::from_toml_str(src).unwrap_err();
        match err {
            ManifestError::InvalidValue { field, .. } => {
                assert_eq!(field, "windows.upgrade_code");
            }
            other => panic!("expected InvalidValue, got {:?}", other),
        }
    }

    #[test]
    fn dist_metadata_with_a_newline_is_rejected() {
        // A newline in a text field could inject an rpm %prep section or an
        // Inno [Run] section into the generated packaging file. Reject it.
        let injected = "[package]\nname = \"x\"\nversion = \"0.1.0\"\n[dist]\ndescription = \"ok\\n%prep\\necho pwned\"\n";
        let err = Manifest::from_toml_str(injected).unwrap_err();
        match err {
            ManifestError::InvalidValue { section, field, .. } => {
                assert_eq!(section, "dist");
                assert_eq!(field, "description");
            }
            other => panic!("expected InvalidValue, got {:?}", other),
        }
    }

    #[test]
    fn dist_dependency_with_a_newline_is_rejected() {
        let injected = "[package]\nname = \"x\"\nversion = \"0.1.0\"\n[dist.linux]\ndepends = [\"libc6\", \"z\\nRequires: evil\"]\n";
        let err = Manifest::from_toml_str(injected).unwrap_err();
        match err {
            ManifestError::InvalidValue { section, field, .. } => {
                assert_eq!(section, "dist");
                assert_eq!(field, "linux.depends");
            }
            other => panic!("expected InvalidValue, got {:?}", other),
        }
    }

    #[test]
    fn package_version_with_a_control_char_is_rejected() {
        let injected = "[package]\nname = \"x\"\nversion = \"1.0\\n%prep\"\n";
        let err = Manifest::from_toml_str(injected).unwrap_err();
        match err {
            ManifestError::InvalidValue { section, field, .. } => {
                assert_eq!(section, "package");
                assert_eq!(field, "version");
            }
            other => panic!("expected InvalidValue, got {:?}", other),
        }
    }

    #[test]
    fn clean_dist_metadata_still_parses() {
        // Ordinary punctuation and spaces are fine; only control characters
        // are rejected.
        let src = "[package]\nname = \"x\"\nversion = \"v1.2.3\"\n[dist]\ndescription = \"A tool: fast, small (and tidy)\"\nmaintainer = \"Ada <ada@example.com>\"\n";
        let m = Manifest::from_toml_str(src).expect("clean metadata parses");
        let d = m.dist.expect("dist present");
        assert_eq!(d.description, "A tool: fast, small (and tidy)");
    }

    #[test]
    fn safe_dist_path_rules() {
        assert!(is_safe_dist_path("share/doc/x/README.md"));
        assert!(is_safe_dist_path("README.md"));
        assert!(!is_safe_dist_path(""));
        assert!(!is_safe_dist_path("/abs"));
        assert!(!is_safe_dist_path("a/../b"));
        assert!(!is_safe_dist_path("a/./b"));
        assert!(!is_safe_dist_path("a//b"));
        assert!(!is_safe_dist_path("a\\b"));
        assert!(!is_safe_dist_path("C:/x"));
    }
}
