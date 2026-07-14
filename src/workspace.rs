//! rvpm workspace discovery, validation, and package selection.
//!
//! A workspace root is an `rv.toml` containing `[workspace]`. It may also be
//! a package, or it may be virtual and contain only workspace configuration
//! and registered commands. Each member remains an ordinary package with its
//! own manifest, lock, dependencies, and build output.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Component, Path, PathBuf};

use crate::manifest::{Manifest, ManifestError, RegisteredCommand, WorkspaceManifest};
use crate::ops::MANIFEST_FILE_NAME;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceMember {
    pub name: String,
    pub root: PathBuf,
}

#[derive(Debug, Clone)]
pub struct Workspace {
    root: PathBuf,
    members: Vec<WorkspaceMember>,
    default_member: Option<String>,
    commands: BTreeMap<String, RegisteredCommand>,
}

#[derive(Debug)]
pub enum WorkspaceError {
    Io {
        action: String,
        path: PathBuf,
        source: std::io::Error,
    },
    Manifest(ManifestError),
    InvalidMember {
        member: String,
        message: String,
    },
    DuplicatePackage(String),
    UnknownPackage(String),
    UnknownCommandPackage {
        command: String,
        package: String,
    },
    NoWorkspace(PathBuf),
    NoPackageSelected(PathBuf),
}

impl fmt::Display for WorkspaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WorkspaceError::Io {
                action,
                path,
                source,
            } => write!(f, "cannot {} '{}': {}", action, path.display(), source),
            WorkspaceError::Manifest(error) => write!(f, "{}", error),
            WorkspaceError::InvalidMember { member, message } => {
                write!(f, "invalid workspace member '{}': {}", member, message)
            }
            WorkspaceError::DuplicatePackage(name) => write!(
                f,
                "workspace package name '{}' is used by more than one member",
                name
            ),
            WorkspaceError::UnknownPackage(name) => {
                write!(f, "workspace has no package named '{}'", name)
            }
            WorkspaceError::UnknownCommandPackage { command, package } => write!(
                f,
                "workspace command '{}' names unknown package '{}'",
                command, package
            ),
            WorkspaceError::NoWorkspace(path) => write!(
                f,
                "no workspace root found from '{}'; expected an rv.toml with [workspace]",
                path.display()
            ),
            WorkspaceError::NoPackageSelected(root) => write!(
                f,
                "workspace '{}' has multiple packages and no default-member; select one with -p <name>",
                root.display()
            ),
        }
    }
}

impl std::error::Error for WorkspaceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            WorkspaceError::Io { source, .. } => Some(source),
            WorkspaceError::Manifest(error) => Some(error),
            _ => None,
        }
    }
}

impl From<ManifestError> for WorkspaceError {
    fn from(value: ManifestError) -> Self {
        WorkspaceError::Manifest(value)
    }
}

impl Workspace {
    /// Load and fully validate the workspace rooted at `root`.
    pub fn load(root: &Path) -> Result<Workspace, WorkspaceError> {
        let root = canonicalize(root, "canonicalize workspace root")?;
        let root_manifest_path = root.join(MANIFEST_FILE_NAME);
        let root_manifest = WorkspaceManifest::load(&root_manifest_path)?;
        let mut members = Vec::new();
        let mut names = BTreeSet::new();
        let mut paths = BTreeSet::new();

        if let Some(package) = root_manifest.package {
            names.insert(package.name.clone());
            paths.insert(root.clone());
            members.push(WorkspaceMember {
                name: package.name,
                root: root.clone(),
            });
        }

        for configured in &root_manifest.workspace.members {
            validate_member_path(configured)?;
            let joined = root.join(configured);
            if !joined.is_dir() {
                return Err(WorkspaceError::InvalidMember {
                    member: configured.clone(),
                    message: format!("directory '{}' does not exist", joined.display()),
                });
            }
            let member_root = canonicalize(&joined, "canonicalize workspace member")?;
            if member_root == root {
                return Err(WorkspaceError::InvalidMember {
                    member: configured.clone(),
                    message: "the root package is included automatically".to_string(),
                });
            }
            if !member_root.starts_with(&root) {
                return Err(WorkspaceError::InvalidMember {
                    member: configured.clone(),
                    message: "resolved path escapes the workspace root".to_string(),
                });
            }
            if !paths.insert(member_root.clone()) {
                return Err(WorkspaceError::InvalidMember {
                    member: configured.clone(),
                    message: "another member resolves to the same directory".to_string(),
                });
            }
            let manifest_path = member_root.join(MANIFEST_FILE_NAME);
            if !manifest_path.is_file() {
                return Err(WorkspaceError::InvalidMember {
                    member: configured.clone(),
                    message: format!("'{}' is missing", manifest_path.display()),
                });
            }
            let manifest = Manifest::load(&manifest_path)?;
            if !names.insert(manifest.package.name.clone()) {
                return Err(WorkspaceError::DuplicatePackage(manifest.package.name));
            }
            members.push(WorkspaceMember {
                name: manifest.package.name,
                root: member_root,
            });
        }

        if let Some(default) = root_manifest.workspace.default_member.as_deref() {
            if !names.contains(default) {
                return Err(WorkspaceError::UnknownPackage(default.to_string()));
            }
        }
        for (name, command) in &root_manifest.commands {
            if !names.contains(&command.package) {
                return Err(WorkspaceError::UnknownCommandPackage {
                    command: name.clone(),
                    package: command.package.clone(),
                });
            }
        }

        Ok(Workspace {
            root,
            members,
            default_member: root_manifest.workspace.default_member,
            commands: root_manifest.commands,
        })
    }

    /// Search `start` and its ancestors for the nearest workspace manifest.
    pub fn discover(start: &Path) -> Result<Option<Workspace>, WorkspaceError> {
        let start = canonicalize(start, "canonicalize current directory")?;
        for dir in start.ancestors() {
            let manifest = dir.join(MANIFEST_FILE_NAME);
            if manifest.is_file() && manifest_has_workspace(&manifest)? {
                return Workspace::load(dir).map(Some);
            }
        }
        Ok(None)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn members(&self) -> &[WorkspaceMember] {
        &self.members
    }

    pub fn command(&self, name: &str) -> Option<&RegisteredCommand> {
        self.commands.get(name)
    }

    pub fn commands(&self) -> &BTreeMap<String, RegisteredCommand> {
        &self.commands
    }

    pub fn member(&self, name: &str) -> Result<&WorkspaceMember, WorkspaceError> {
        self.members
            .iter()
            .find(|member| member.name == name)
            .ok_or_else(|| WorkspaceError::UnknownPackage(name.to_string()))
    }

    /// Select an explicit package, the package containing `current`, the
    /// configured default, or the sole workspace member in that order.
    pub fn select<'a>(
        &'a self,
        current: &Path,
        requested: Option<&str>,
    ) -> Result<&'a WorkspaceMember, WorkspaceError> {
        if let Some(name) = requested {
            return self.member(name);
        }
        let current = canonicalize(current, "canonicalize current directory")?;
        if let Some(member) = self
            .members
            .iter()
            .filter(|member| current.starts_with(&member.root))
            .max_by_key(|member| member.root.components().count())
        {
            return Ok(member);
        }
        if let Some(default) = self.default_member.as_deref() {
            return self.member(default);
        }
        if self.members.len() == 1 {
            return Ok(&self.members[0]);
        }
        Err(WorkspaceError::NoPackageSelected(self.root.clone()))
    }
}

/// Resolve a package from a workspace or fall back to the nearest standalone
/// package manifest for backwards-compatible single-package behavior.
pub fn resolve_package(start: &Path, requested: Option<&str>) -> Result<PathBuf, WorkspaceError> {
    if let Some(workspace) = Workspace::discover(start)? {
        return Ok(workspace.select(start, requested)?.root.clone());
    }
    if requested.is_some() {
        return Err(WorkspaceError::NoWorkspace(start.to_path_buf()));
    }
    let start = canonicalize(start, "canonicalize current directory")?;
    for dir in start.ancestors() {
        if dir.join(MANIFEST_FILE_NAME).is_file() {
            return Ok(dir.to_path_buf());
        }
    }
    Err(WorkspaceError::Io {
        action: "find package manifest".to_string(),
        path: start.join(MANIFEST_FILE_NAME),
        source: std::io::Error::new(std::io::ErrorKind::NotFound, "rv.toml was not found"),
    })
}

fn validate_member_path(member: &str) -> Result<(), WorkspaceError> {
    let path = Path::new(member);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(WorkspaceError::InvalidMember {
            member: member.to_string(),
            message: "paths must be relative and stay inside the workspace root".to_string(),
        });
    }
    Ok(())
}

fn manifest_has_workspace(path: &Path) -> Result<bool, WorkspaceError> {
    let text = std::fs::read_to_string(path).map_err(|source| WorkspaceError::Io {
        action: "read".to_string(),
        path: path.to_path_buf(),
        source,
    })?;
    let value: toml::Value = toml::from_str(&text).map_err(|error| {
        WorkspaceError::Manifest(ManifestError::Toml(error.message().to_string()))
    })?;
    Ok(value
        .as_table()
        .map(|table| table.contains_key("workspace"))
        .unwrap_or(false))
}

fn canonicalize(path: &Path, action: &str) -> Result<PathBuf, WorkspaceError> {
    let canonical = path.canonicalize().map_err(|source| WorkspaceError::Io {
        action: action.to_string(),
        path: path.to_path_buf(),
        source,
    })?;
    Ok(normalize_windows_verbatim(canonical))
}

#[cfg(not(windows))]
fn normalize_windows_verbatim(path: PathBuf) -> PathBuf {
    path
}

/// `std::fs::canonicalize` returns `\\?\` paths on Windows. Rust accepts
/// those paths, but tools in the MSVC build chain do not consistently accept
/// them on command lines. Rebuild ordinary drive and UNC paths component by
/// component without converting non-Unicode names through a string.
#[cfg(windows)]
fn normalize_windows_verbatim(path: PathBuf) -> PathBuf {
    use std::path::Prefix;

    let mut components = path.components();
    let Some(Component::Prefix(prefix)) = components.next() else {
        return path;
    };
    let mut normalized = match prefix.kind() {
        Prefix::VerbatimDisk(drive) => PathBuf::from(format!("{}:\\", drive as char)),
        Prefix::VerbatimUNC(server, share) => {
            let mut root = PathBuf::from(r"\\");
            root.push(server);
            root.push(share);
            root
        }
        _ => return path,
    };
    for component in components {
        if !matches!(component, Component::RootDir) {
            normalized.push(component.as_os_str());
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtual_workspace_loads_and_selects_members() {
        let root = tempdir("select");
        write(
            &root.join("rv.toml"),
            "[workspace]\nmembers = [\"apps/api\", \"tools/task\"]\ndefault-member = \"api\"\n\n[commands]\ncheck = { package = \"task\", args = [\"all\"] }\n",
        );
        package(&root.join("apps/api"), "api");
        package(&root.join("tools/task"), "task");

        let workspace = Workspace::load(&root).expect("workspace loads");
        assert_eq!(workspace.members().len(), 2);
        assert_eq!(workspace.select(&root, None).unwrap().name, "api");
        assert_eq!(
            workspace.member("task").unwrap().root,
            normalize_windows_verbatim(root.join("tools/task").canonicalize().unwrap())
        );
        assert_eq!(workspace.command("check").unwrap().args, ["all"]);
        cleanup(&root);
    }

    #[test]
    fn discovery_from_a_nested_member_prefers_that_member() {
        let root = tempdir("discover");
        write(
            &root.join("rv.toml"),
            "[workspace]\nmembers = [\"one\", \"two\"]\n",
        );
        package(&root.join("one"), "one");
        package(&root.join("two"), "two");
        let nested = root.join("two/src/deep");
        std::fs::create_dir_all(&nested).unwrap();

        let workspace = Workspace::discover(&nested).unwrap().unwrap();
        assert_eq!(workspace.select(&nested, None).unwrap().name, "two");
        cleanup(&root);
    }

    #[test]
    fn member_escape_and_duplicate_names_are_rejected() {
        let base = tempdir("invalid");
        let root = base.join("root");
        std::fs::create_dir_all(&root).unwrap();
        package(&base.join("outside"), "outside");
        write(
            &root.join("rv.toml"),
            "[workspace]\nmembers = [\"../outside\"]\n",
        );
        assert!(matches!(
            Workspace::load(&root),
            Err(WorkspaceError::InvalidMember { .. })
        ));

        write(
            &root.join("rv.toml"),
            "[workspace]\nmembers = [\"a\", \"b\"]\n",
        );
        package(&root.join("a"), "same");
        package(&root.join("b"), "same");
        assert!(matches!(
            Workspace::load(&root),
            Err(WorkspaceError::DuplicatePackage(name)) if name == "same"
        ));
        cleanup(&base);
    }

    fn package(root: &Path, name: &str) {
        std::fs::create_dir_all(root).unwrap();
        write(
            &root.join("rv.toml"),
            &format!("[package]\nname = \"{}\"\nversion = \"0.1.0\"\n", name),
        );
    }

    fn write(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, contents).unwrap();
    }

    fn tempdir(label: &str) -> PathBuf {
        let mut root = std::env::temp_dir();
        root.push(format!(
            "raven-workspace-{}-{}-{}",
            label,
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
}
