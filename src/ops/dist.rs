//! `rvpm dist`: package a built application into distributable artifacts.
//!
//! The command builds the package, stages its file tree once per target,
//! and produces each requested artifact by generating the format's own
//! packaging text (a deb control file, an rpm spec, a WiX source, an Inno
//! Setup script) and shelling out to the format's tool, following the same
//! external-tool approach `pkg` takes with curl and tar. Archive targets
//! (`tar`, `zip`) need nothing beyond tar or zip. A missing tool fails with
//! an install hint rather than a raw spawn error.
//!
//! See `docs/v2/specs/rvpm-dist.md` for the manifest surface.

use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

use crate::manifest::{Dist, Manifest};
use crate::pkg;

use super::{build_in, OpError, MANIFEST_FILE_NAME};

/// What one `rvpm dist` invocation produced.
pub struct DistReport {
    pub outcome_lines: Vec<String>,
    pub artifacts: Vec<PathBuf>,
}

/// Command-line overrides for a dist run.
#[derive(Default)]
pub struct DistOptions {
    /// Targets to produce instead of the manifest's `[dist].targets`.
    pub targets: Vec<String>,
    /// Output directory instead of the manifest's `[dist].out_dir`.
    pub out_dir: Option<String>,
}

/// Everything a backend needs to package one application.
struct DistContext {
    name: String,
    version: String,
    dist: Dist,
    /// The built binary on disk.
    binary: PathBuf,
    /// The package root, for resolving asset sources.
    project_dir: PathBuf,
    /// Where artifacts land (absolute).
    out_dir: PathBuf,
    /// Scratch space under `out_dir` for staging trees and generated files.
    work_dir: PathBuf,
}

/// Package the application under `project_dir` using the default cache
/// root. See [`dist_in`].
pub fn dist(project_dir: &Path, opts: DistOptions) -> Result<DistReport, OpError> {
    dist_in(project_dir, &pkg::cache_root(), opts)
}

/// Build the package under `project_dir` against `cache_root`, then produce
/// every requested artifact. Targets come from `opts.targets`, else the
/// manifest's `[dist].targets`, else the host default (`tar` on Unix, `zip`
/// on Windows).
pub fn dist_in(
    project_dir: &Path,
    cache_root: &Path,
    opts: DistOptions,
) -> Result<DistReport, OpError> {
    let manifest = Manifest::load(project_dir.join(MANIFEST_FILE_NAME))?;
    let dist_cfg = manifest
        .dist
        .clone()
        .unwrap_or_else(|| Dist::with_defaults(&manifest.package));

    let build = build_in(project_dir, cache_root)?;
    let binary = build.binary.ok_or(OpError::NotPackageable)?;

    let targets = resolve_targets(&opts.targets, &dist_cfg.targets);
    let out_rel = opts.out_dir.as_deref().unwrap_or(&dist_cfg.out_dir);
    let out_dir = project_dir.join(out_rel);
    let work_dir = out_dir.join("work");

    let ctx = DistContext {
        name: manifest.package.name.clone(),
        version: manifest.package.version.clone(),
        dist: dist_cfg,
        binary,
        project_dir: project_dir.to_path_buf(),
        out_dir,
        work_dir,
    };
    package_targets(&ctx, &targets)
}

/// Produce each target from an already-built binary. Split from [`dist_in`]
/// so tests can package a plain file without compiling a program.
fn package_targets(ctx: &DistContext, targets: &[String]) -> Result<DistReport, OpError> {
    create_dir_all(&ctx.out_dir)?;
    // A fresh work tree per run keeps stale staging out of the artifacts.
    if ctx.work_dir.exists() {
        std::fs::remove_dir_all(&ctx.work_dir).map_err(|e| io_err("clear", &ctx.work_dir, e))?;
    }
    create_dir_all(&ctx.work_dir)?;

    let mut report = DistReport {
        outcome_lines: Vec::new(),
        artifacts: Vec::new(),
    };
    for target in targets {
        let (artifact, note) = match target.as_str() {
            "tar" => (dist_tar(ctx)?, None),
            "zip" => (dist_zip(ctx)?, None),
            "deb" => (dist_deb(ctx)?, None),
            "rpm" => (dist_rpm(ctx)?, None),
            "msi" => dist_msi(ctx)?,
            "inno" => (dist_inno(ctx)?, None),
            other => {
                return Err(OpError::DistFailed {
                    tool: "rvpm".to_string(),
                    detail: format!("unknown dist target '{}'", other),
                })
            }
        };
        report
            .outcome_lines
            .push(format!("Packaged {} ({})", artifact.display(), target));
        if let Some(note) = note {
            report.outcome_lines.push(note);
        }
        report.artifacts.push(artifact);
    }
    Ok(report)
}

/// The targets to produce: explicit overrides win, then the manifest list,
/// then the host's native archive.
fn resolve_targets(overrides: &[String], manifest_targets: &[String]) -> Vec<String> {
    if !overrides.is_empty() {
        return overrides.to_vec();
    }
    if !manifest_targets.is_empty() {
        return manifest_targets.to_vec();
    }
    if cfg!(windows) {
        vec!["zip".to_string()]
    } else {
        vec!["tar".to_string()]
    }
}

/// The architecture label most formats use.
fn host_arch() -> &'static str {
    std::env::consts::ARCH
}

/// Debian's name for the host architecture.
fn deb_arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        other => other,
    }
}

/// A version acceptable to deb and rpm: the common `v` tag prefix is
/// dropped, and for rpm a `-` pre-release separator becomes `~`.
fn plain_version(version: &str) -> String {
    version.strip_prefix('v').unwrap_or(version).to_string()
}

fn rpm_version(version: &str) -> String {
    plain_version(version).replace('-', "~")
}

/// An MSI ProductVersion must be dotted numerics; take the leading
/// `X.Y.Z` of the version and fall back to 0.0.0.
fn msi_version(version: &str) -> String {
    let plain = plain_version(version);
    let numeric = plain.split('-').next().unwrap_or("");
    let ok = !numeric.is_empty()
        && numeric
            .split('.')
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()));
    if ok {
        numeric.to_string()
    } else {
        "0.0.0".to_string()
    }
}

/// The binary's installed file name.
fn binary_file_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{}.exe", name)
    } else {
        name.to_string()
    }
}

// ---------------------------------------------------------------------------
// Staging
// ---------------------------------------------------------------------------

/// Copy the binary and every asset under `root`, with the binary at
/// `bin_prefix` (for example `usr/bin` or the tree root) and assets at
/// `asset_prefix`/dest.
fn stage_tree(
    ctx: &DistContext,
    root: &Path,
    bin_prefix: &str,
    asset_prefix: &str,
) -> Result<(), OpError> {
    let bin_dir = if bin_prefix.is_empty() {
        root.to_path_buf()
    } else {
        root.join(bin_prefix)
    };
    create_dir_all(&bin_dir)?;
    let staged_bin = bin_dir.join(binary_file_name(&ctx.name));
    copy_file(&ctx.binary, &staged_bin)?;
    mark_executable(&staged_bin)?;

    for asset in &ctx.dist.assets {
        let source = ctx.project_dir.join(&asset.source);
        let dest = if asset_prefix.is_empty() {
            root.join(&asset.dest)
        } else {
            root.join(asset_prefix).join(&asset.dest)
        };
        if let Some(parent) = dest.parent() {
            create_dir_all(parent)?;
        }
        copy_file(&source, &dest)?;
    }
    Ok(())
}

#[cfg(unix)]
fn mark_executable(path: &Path) -> Result<(), OpError> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .map_err(|e| io_err("set permissions on", path, e))
}

#[cfg(not(unix))]
fn mark_executable(_path: &Path) -> Result<(), OpError> {
    Ok(())
}

// ---------------------------------------------------------------------------
// Archive backends
// ---------------------------------------------------------------------------

/// `tar`: a `.tar.gz` holding a `<name>-<version>-<arch>/` top directory,
/// the same layout the Raven toolchain archives use.
fn dist_tar(ctx: &DistContext) -> Result<PathBuf, OpError> {
    let top = format!(
        "{}-{}-{}",
        ctx.name,
        plain_version(&ctx.version),
        host_arch()
    );
    let stage = ctx.work_dir.join("tar");
    stage_tree(ctx, &stage.join(&top), "", "")?;
    let artifact = ctx.out_dir.join(format!("{}.tar.gz", top));
    remove_if_present(&artifact)?;
    let mut cmd = Command::new("tar");
    cmd.arg("-czf")
        .arg(&artifact)
        .arg("-C")
        .arg(&stage)
        .arg(&top);
    run_tool(cmd, "tar", TAR_HINT)?;
    Ok(artifact)
}

/// `zip`: a flat archive, binary and assets at the root.
fn dist_zip(ctx: &DistContext) -> Result<PathBuf, OpError> {
    let stage = ctx.work_dir.join("zip");
    stage_tree(ctx, &stage, "", "")?;
    let artifact = ctx.out_dir.join(format!(
        "{}-{}-{}.zip",
        ctx.name,
        plain_version(&ctx.version),
        host_arch()
    ));
    remove_if_present(&artifact)?;
    let entries = top_level_entries(&stage)?;
    if tar_is_bsdtar() {
        let mut cmd = Command::new("tar");
        cmd.arg("-a")
            .arg("-cf")
            .arg(&artifact)
            .arg("-C")
            .arg(&stage);
        cmd.args(&entries);
        run_tool(cmd, "tar", ZIP_HINT)?;
        return Ok(artifact);
    }
    let mut cmd = Command::new("zip");
    cmd.arg("-qr").arg(&artifact);
    cmd.args(&entries);
    cmd.current_dir(&stage);
    run_tool(cmd, "zip", ZIP_HINT)?;
    Ok(artifact)
}

/// Whether the `tar` on PATH is bsdtar, which can write zip archives.
fn tar_is_bsdtar() -> bool {
    Command::new("tar")
        .arg("--version")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("bsdtar"))
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// deb
// ---------------------------------------------------------------------------

/// `deb`: stage a Debian file tree, generate `DEBIAN/control`, and run
/// `dpkg-deb --build`.
fn dist_deb(ctx: &DistContext) -> Result<PathBuf, OpError> {
    let stage = ctx.work_dir.join("deb");
    stage_tree(ctx, &stage, "usr/bin", "usr")?;
    let control_dir = stage.join("DEBIAN");
    create_dir_all(&control_dir)?;
    write_file(&control_dir.join("control"), &deb_control(ctx))?;
    let artifact = ctx.out_dir.join(format!(
        "{}_{}_{}.deb",
        ctx.name,
        plain_version(&ctx.version),
        deb_arch()
    ));
    remove_if_present(&artifact)?;
    let mut cmd = Command::new("dpkg-deb");
    cmd.arg("--build")
        .arg("--root-owner-group")
        .arg(&stage)
        .arg(&artifact);
    run_tool(cmd, "dpkg-deb", DEB_HINT)?;
    Ok(artifact)
}

/// The `DEBIAN/control` text for this package.
fn deb_control(ctx: &DistContext) -> String {
    let d = &ctx.dist;
    let mut out = String::new();
    out.push_str(&format!("Package: {}\n", ctx.name));
    out.push_str(&format!("Version: {}\n", plain_version(&ctx.version)));
    out.push_str(&format!("Architecture: {}\n", deb_arch()));
    out.push_str(&format!("Maintainer: {}\n", d.maintainer));
    out.push_str(&format!("Section: {}\n", d.linux.section));
    out.push_str(&format!("Priority: {}\n", d.linux.priority));
    if !d.linux.depends.is_empty() {
        out.push_str(&format!("Depends: {}\n", d.linux.depends.join(", ")));
    }
    if !d.homepage.is_empty() {
        out.push_str(&format!("Homepage: {}\n", d.homepage));
    }
    out.push_str(&format!("Description: {}\n", d.description));
    out
}

// ---------------------------------------------------------------------------
// rpm
// ---------------------------------------------------------------------------

/// `rpm`: stage the install tree, generate a spec whose `%install` copies
/// it into the buildroot, and run `rpmbuild -bb`.
fn dist_rpm(ctx: &DistContext) -> Result<PathBuf, OpError> {
    let stage = ctx.work_dir.join("rpm-root");
    stage_tree(ctx, &stage, "usr/bin", "usr")?;
    let topdir = ctx.work_dir.join("rpm");
    create_dir_all(&topdir)?;
    let spec_path = ctx.work_dir.join(format!("{}.spec", ctx.name));
    write_file(&spec_path, &rpm_spec(ctx, &stage))?;

    let mut cmd = Command::new("rpmbuild");
    cmd.arg("-bb")
        .arg("--define")
        .arg(format!("_topdir {}", topdir.display()))
        .arg(&spec_path);
    run_tool(cmd, "rpmbuild", RPM_HINT)?;

    // rpmbuild names the artifact itself; find it under RPMS and move it
    // next to the other artifacts.
    let built = find_first_rpm(&topdir.join("RPMS"))?.ok_or_else(|| OpError::DistFailed {
        tool: "rpmbuild".to_string(),
        detail: "rpmbuild succeeded but produced no .rpm under RPMS".to_string(),
    })?;
    let artifact = ctx.out_dir.join(built.file_name().expect("rpm file name"));
    remove_if_present(&artifact)?;
    std::fs::rename(&built, &artifact).map_err(|e| io_err("move", &built, e))?;
    Ok(artifact)
}

/// The rpm spec for this package. `%install` copies the staged tree, so
/// the spec needs no build steps.
fn rpm_spec(ctx: &DistContext, staged_root: &Path) -> String {
    let d = &ctx.dist;
    let mut out = String::new();
    out.push_str(&format!("Name: {}\n", ctx.name));
    out.push_str(&format!("Version: {}\n", rpm_version(&ctx.version)));
    out.push_str("Release: 1\n");
    out.push_str(&format!("Summary: {}\n", d.description));
    let license = if d.license.is_empty() {
        "Unspecified"
    } else {
        &d.license
    };
    out.push_str(&format!("License: {}\n", license));
    if !d.homepage.is_empty() {
        out.push_str(&format!("URL: {}\n", d.homepage));
    }
    if !d.vendor.is_empty() {
        out.push_str(&format!("Vendor: {}\n", d.vendor));
    }
    if !d.linux.depends.is_empty() {
        for dep in &d.linux.depends {
            out.push_str(&format!("Requires: {}\n", dep));
        }
    }
    out.push_str(&format!("BuildArch: {}\n", host_arch()));
    out.push_str("\n%description\n");
    out.push_str(&format!("{}\n", d.description));
    out.push_str("\n%install\n");
    out.push_str("mkdir -p %{buildroot}\n");
    out.push_str(&format!(
        "cp -a {}/. %{{buildroot}}/\n",
        staged_root.display()
    ));
    out.push_str("\n%files\n");
    out.push_str(&format!("/usr/bin/{}\n", ctx.name));
    for asset in &d.assets {
        out.push_str(&format!("/usr/{}\n", asset.dest));
    }
    out
}

fn find_first_rpm(dir: &Path) -> Result<Option<PathBuf>, OpError> {
    if !dir.exists() {
        return Ok(None);
    }
    let entries = std::fs::read_dir(dir).map_err(|e| io_err("read", dir, e))?;
    for entry in entries {
        let entry = entry.map_err(|e| io_err("read", dir, e))?;
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_first_rpm(&path)? {
                return Ok(Some(found));
            }
        } else if path.extension().is_some_and(|e| e == "rpm") {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

// ---------------------------------------------------------------------------
// msi (WiX)
// ---------------------------------------------------------------------------

/// `msi`: generate a WiX source and compile it with WiX 3 (candle and
/// light) or the WiX 4+ CLI, whichever is installed. Returns the artifact
/// and an advisory note when the upgrade GUID was derived rather than
/// pinned in the manifest.
fn dist_msi(ctx: &DistContext) -> Result<(PathBuf, Option<String>), OpError> {
    let stage = ctx.work_dir.join("msi");
    stage_tree(ctx, &stage, "", "")?;

    let (upgrade_code, note) = if ctx.dist.windows.upgrade_code.is_empty() {
        let derived = derived_upgrade_code(&ctx.name);
        let note = format!(
            "note: derived msi upgrade code {} from the package name; pin it as [dist.windows].upgrade_code so future installers keep upgrading old ones",
            derived
        );
        (derived, Some(note))
    } else {
        (ctx.dist.windows.upgrade_code.clone(), None)
    };

    let wxs_path = ctx.work_dir.join(format!("{}.wxs", ctx.name));
    write_file(&wxs_path, &wix_source(ctx, &stage, &upgrade_code))?;
    let artifact = ctx.out_dir.join(format!(
        "{}-{}-{}.msi",
        ctx.name,
        plain_version(&ctx.version),
        host_arch()
    ));
    remove_if_present(&artifact)?;

    if command_exists("wix") {
        let mut cmd = Command::new("wix");
        cmd.arg("build")
            .arg(&wxs_path)
            .arg("-arch")
            .arg("x64")
            .arg("-o")
            .arg(&artifact);
        run_tool(cmd, "wix", WIX_HINT)?;
        return Ok((artifact, note));
    }

    let candle = find_wix3_tool("candle.exe").ok_or_else(|| OpError::DistTool {
        tool: "candle.exe".to_string(),
        hint: WIX_HINT.to_string(),
    })?;
    let light = find_wix3_tool("light.exe").ok_or_else(|| OpError::DistTool {
        tool: "light.exe".to_string(),
        hint: WIX_HINT.to_string(),
    })?;
    let wixobj = ctx.work_dir.join(format!("{}.wixobj", ctx.name));
    let mut compile = Command::new(&candle);
    compile
        .arg("-nologo")
        .arg("-arch")
        .arg("x64")
        .arg("-out")
        .arg(&wixobj)
        .arg(&wxs_path);
    run_tool(compile, "candle.exe", WIX_HINT)?;
    let mut link = Command::new(&light);
    link.arg("-nologo").arg("-out").arg(&artifact).arg(&wixobj);
    run_tool(link, "light.exe", WIX_HINT)?;
    Ok((artifact, note))
}

/// WiX 3 installs are found through PATH or the WIX environment variable
/// the toolset installer sets.
fn find_wix3_tool(exe: &str) -> Option<PathBuf> {
    if command_exists(exe.trim_end_matches(".exe")) {
        return Some(PathBuf::from(exe));
    }
    let wix_home = std::env::var_os("WIX")?;
    let candidate = PathBuf::from(wix_home).join("bin").join(exe);
    candidate.exists().then_some(candidate)
}

/// A stable GUID derived from the package name, used when the manifest
/// does not pin `[dist.windows].upgrade_code`. Stability is what matters:
/// the same name always derives the same code, so upgrades keep working.
fn derived_upgrade_code(name: &str) -> String {
    let digest = Sha256::digest(format!("rvpm-dist-upgrade-code:{}", name));
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        digest[0], digest[1], digest[2], digest[3],
        digest[4], digest[5],
        digest[6], digest[7],
        digest[8], digest[9],
        digest[10], digest[11], digest[12], digest[13], digest[14], digest[15],
    )
}

/// The WiX source: one per-file component per staged file under an
/// application folder in Program Files, with a major-upgrade rule.
fn wix_source(ctx: &DistContext, stage: &Path, upgrade_code: &str) -> String {
    let d = &ctx.dist;
    let mut components = String::new();
    let mut refs = String::new();
    let files = files_under(stage);
    for (i, file) in files.iter().enumerate() {
        let id = format!("File{}", i);
        components.push_str(&format!(
            "        <Component Id=\"{id}\" Guid=\"*\">\n          <File Id=\"{id}File\" Source=\"{}\" KeyPath=\"yes\" />\n        </Component>\n",
            xml_escape(&file.display().to_string()),
            id = id,
        ));
        refs.push_str(&format!("      <ComponentRef Id=\"{}\" />\n", id));
    }
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<Wix xmlns="http://schemas.microsoft.com/wix/2006/wi">
  <Product Id="*" Name="{name}" Language="1033" Version="{version}"
           Manufacturer="{vendor}" UpgradeCode="{upgrade}">
    <Package InstallerVersion="500" Compressed="yes" InstallScope="perMachine" />
    <MajorUpgrade DowngradeErrorMessage="A newer version of {name} is already installed." />
    <MediaTemplate EmbedCab="yes" />
    <Directory Id="TARGETDIR" Name="SourceDir">
      <Directory Id="ProgramFiles64Folder">
        <Directory Id="APPDIR" Name="{name}">
{components}        </Directory>
      </Directory>
    </Directory>
    <Feature Id="Main" Title="{name}" Level="1">
{refs}    </Feature>
  </Product>
</Wix>
"#,
        name = xml_escape(&d.display_name),
        version = msi_version(&ctx.version),
        vendor = xml_escape(if d.vendor.is_empty() {
            &d.maintainer
        } else {
            &d.vendor
        }),
        upgrade = upgrade_code,
        components = components,
        refs = refs,
    )
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Every file under `root`, depth-first, in a stable order.
fn files_under(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        paths.sort();
        for path in paths {
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

// ---------------------------------------------------------------------------
// inno
// ---------------------------------------------------------------------------

/// `inno`: generate an Inno Setup script and compile it with ISCC.
fn dist_inno(ctx: &DistContext) -> Result<PathBuf, OpError> {
    let stage = ctx.work_dir.join("inno");
    stage_tree(ctx, &stage, "", "")?;
    let iss_path = ctx.work_dir.join(format!("{}.iss", ctx.name));
    write_file(&iss_path, &inno_script(ctx, &stage))?;
    let base = format!("{}-{}-setup", ctx.name, plain_version(&ctx.version));
    let artifact = ctx.out_dir.join(format!("{}.exe", base));
    remove_if_present(&artifact)?;

    let iscc = find_iscc().ok_or_else(|| OpError::DistTool {
        tool: "ISCC.exe".to_string(),
        hint: INNO_HINT.to_string(),
    })?;
    let mut cmd = Command::new(iscc);
    cmd.arg("/Q").arg(&iss_path);
    run_tool(cmd, "ISCC.exe", INNO_HINT)?;
    Ok(artifact)
}

fn find_iscc() -> Option<PathBuf> {
    if command_exists("iscc") {
        return Some(PathBuf::from("iscc"));
    }
    for base in [
        "C:\\Program Files (x86)\\Inno Setup 6\\ISCC.exe",
        "C:\\Program Files\\Inno Setup 6\\ISCC.exe",
    ] {
        let p = PathBuf::from(base);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

/// The Inno Setup script for this package.
fn inno_script(ctx: &DistContext, stage: &Path) -> String {
    let d = &ctx.dist;
    let mut out = String::new();
    out.push_str("[Setup]\n");
    out.push_str(&format!("AppName={}\n", d.display_name));
    out.push_str(&format!("AppVersion={}\n", plain_version(&ctx.version)));
    if !d.vendor.is_empty() {
        out.push_str(&format!("AppPublisher={}\n", d.vendor));
    }
    if !d.homepage.is_empty() {
        out.push_str(&format!("AppPublisherURL={}\n", d.homepage));
    }
    out.push_str(&format!("DefaultDirName={{autopf}}\\{}\n", d.display_name));
    out.push_str("DisableProgramGroupPage=yes\n");
    out.push_str("ArchitecturesInstallIn64BitMode=x64compatible\n");
    if !d.windows.icon.is_empty() {
        let icon = ctx.project_dir.join(&d.windows.icon);
        out.push_str(&format!("SetupIconFile={}\n", icon.display()));
    }
    out.push_str(&format!(
        "OutputBaseFilename={}-{}-setup\n",
        ctx.name,
        plain_version(&ctx.version)
    ));
    out.push_str(&format!("OutputDir={}\n", ctx.out_dir.display()));
    out.push_str("\n[Files]\n");
    out.push_str(&format!(
        "Source: \"{}\\*\"; DestDir: \"{{app}}\"; Flags: recursesubdirs\n",
        stage.display()
    ));
    out
}

// ---------------------------------------------------------------------------
// Tool execution and small file helpers
// ---------------------------------------------------------------------------

const TAR_HINT: &str =
    "install tar; it ships with Windows 10 and later and every major Linux distribution";
const ZIP_HINT: &str =
    "no zip-capable tool found; install zip, or rely on bsdtar (the Windows tar), or use the tar target";
const DEB_HINT: &str = "install dpkg (for example 'apt install dpkg'); deb packages are normally built on a Debian-family system";
const RPM_HINT: &str = "install rpmbuild ('apt install rpm' on Debian-family, 'dnf install rpm-build' on Fedora-family)";
const WIX_HINT: &str = "install the WiX Toolset: 'choco install wixtoolset' (WiX 3, candle and light) or 'dotnet tool install --global wix' (WiX 4+)";
const INNO_HINT: &str = "install Inno Setup 6 and put ISCC.exe on PATH ('choco install innosetup')";

/// Run a packaging tool, mapping a missing executable to an install hint
/// and a non-zero exit to the tool's own diagnostics.
fn run_tool(mut cmd: Command, tool: &str, hint: &str) -> Result<(), OpError> {
    let output = match cmd.output() {
        Ok(o) => o,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(OpError::DistTool {
                tool: tool.to_string(),
                hint: hint.to_string(),
            })
        }
        Err(e) => {
            return Err(OpError::DistFailed {
                tool: tool.to_string(),
                detail: e.to_string(),
            })
        }
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = if stderr.trim().is_empty() {
            stdout.trim().to_string()
        } else {
            stderr.trim().to_string()
        };
        return Err(OpError::DistFailed {
            tool: tool.to_string(),
            detail,
        });
    }
    Ok(())
}

/// Whether `name` resolves to a runnable command.
fn command_exists(name: &str) -> bool {
    Command::new(name)
        .arg("--version")
        .output()
        .map(|_| true)
        .unwrap_or(false)
}

fn top_level_entries(dir: &Path) -> Result<Vec<String>, OpError> {
    let mut names = Vec::new();
    let entries = std::fs::read_dir(dir).map_err(|e| io_err("read", dir, e))?;
    for entry in entries {
        let entry = entry.map_err(|e| io_err("read", dir, e))?;
        names.push(entry.file_name().to_string_lossy().to_string());
    }
    names.sort();
    Ok(names)
}

fn create_dir_all(path: &Path) -> Result<(), OpError> {
    std::fs::create_dir_all(path).map_err(|e| io_err("create", path, e))
}

fn copy_file(from: &Path, to: &Path) -> Result<(), OpError> {
    std::fs::copy(from, to)
        .map(|_| ())
        .map_err(|e| io_err("copy", from, e))
}

fn write_file(path: &Path, text: &str) -> Result<(), OpError> {
    std::fs::write(path, text).map_err(|e| io_err("write", path, e))
}

fn remove_if_present(path: &Path) -> Result<(), OpError> {
    if path.exists() {
        std::fs::remove_file(path).map_err(|e| io_err("remove", path, e))?;
    }
    Ok(())
}

fn io_err(action: &str, path: &Path, source: std::io::Error) -> OpError {
    OpError::Io {
        action: action.to_string(),
        path: path.to_path_buf(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{Dist, DistAsset, Manifest};

    fn counter() -> u64 {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        COUNTER.fetch_add(1, Ordering::Relaxed)
    }

    /// A scratch package root holding a fake built binary and one asset,
    /// so backends can be exercised without compiling a program.
    struct FakeApp {
        root: PathBuf,
    }

    impl FakeApp {
        fn new(tag: &str) -> FakeApp {
            let mut root = std::env::temp_dir();
            root.push(format!(
                "rvpm-dist-{}-{}-{}",
                tag,
                std::process::id(),
                counter()
            ));
            std::fs::create_dir_all(&root).expect("create temp root");
            FakeApp { root }
        }

        fn context(&self, manifest_toml: &str) -> DistContext {
            let manifest = Manifest::from_toml_str(manifest_toml).expect("manifest parses");
            let dist = manifest
                .dist
                .clone()
                .unwrap_or_else(|| Dist::with_defaults(&manifest.package));
            let binary = self.root.join(binary_file_name(&manifest.package.name));
            std::fs::write(&binary, b"#!/bin/sh\necho fake\n").expect("write fake binary");
            std::fs::write(self.root.join("README.md"), "docs\n").expect("write asset");
            let out_dir = self.root.join("target/dist");
            DistContext {
                name: manifest.package.name.clone(),
                version: manifest.package.version.clone(),
                dist,
                binary,
                project_dir: self.root.clone(),
                out_dir: out_dir.clone(),
                work_dir: out_dir.join("work"),
            }
        }
    }

    impl Drop for FakeApp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    const BASIC: &str =
        "[package]\nname = \"demo\"\nversion = \"v1.2.0\"\nauthors = [\"Ada <ada@example.com>\"]\n";

    const WITH_ASSET: &str = r#"
[package]
name = "demo"
version = "1.2.0"
authors = ["Ada <ada@example.com>"]

[dist]
description = "A demo tool"
license = "MIT"
homepage = "https://example.com/demo"
vendor = "Acme"

[[dist.assets]]
source = "README.md"
dest = "share/doc/demo/README.md"

[dist.linux]
depends = ["libc6 (>= 2.31)", "zlib1g"]
"#;

    #[test]
    fn resolve_targets_precedence() {
        let over = vec!["deb".to_string()];
        let manifest = vec!["rpm".to_string(), "zip".to_string()];
        assert_eq!(resolve_targets(&over, &manifest), vec!["deb"]);
        assert_eq!(resolve_targets(&[], &manifest), vec!["rpm", "zip"]);
        let host = resolve_targets(&[], &[]);
        assert_eq!(host.len(), 1);
        assert!(host[0] == "zip" || host[0] == "tar");
    }

    #[test]
    fn version_normalization() {
        assert_eq!(plain_version("v1.2.3"), "1.2.3");
        assert_eq!(plain_version("1.2.3"), "1.2.3");
        assert_eq!(rpm_version("v1.2.3-beta.1"), "1.2.3~beta.1");
        assert_eq!(msi_version("v1.2.3-beta.1"), "1.2.3");
        assert_eq!(msi_version("nonsense"), "0.0.0");
    }

    #[test]
    fn deb_control_renders_the_manifest() {
        let app = FakeApp::new("control");
        let ctx = app.context(WITH_ASSET);
        let control = deb_control(&ctx);
        assert!(control.contains("Package: demo\n"));
        assert!(control.contains("Version: 1.2.0\n"));
        assert!(control.contains("Maintainer: Ada <ada@example.com>\n"));
        assert!(control.contains("Depends: libc6 (>= 2.31), zlib1g\n"));
        assert!(control.contains("Homepage: https://example.com/demo\n"));
        assert!(control.contains("Description: A demo tool\n"));
        assert!(control.contains("Section: utils\n"));
    }

    #[test]
    fn deb_control_minimal_has_no_optional_fields() {
        let app = FakeApp::new("control-min");
        let ctx = app.context(BASIC);
        let control = deb_control(&ctx);
        assert!(control.contains("Version: 1.2.0\n"), "v prefix is stripped");
        assert!(!control.contains("Depends:"));
        assert!(!control.contains("Homepage:"));
    }

    #[test]
    fn rpm_spec_lists_every_installed_file() {
        let app = FakeApp::new("spec");
        let ctx = app.context(WITH_ASSET);
        let spec = rpm_spec(&ctx, Path::new("/tmp/staged"));
        assert!(spec.contains("Name: demo\n"));
        assert!(spec.contains("License: MIT\n"));
        assert!(spec.contains("Requires: libc6 (>= 2.31)\n"));
        assert!(spec.contains("/usr/bin/demo\n"));
        assert!(spec.contains("/usr/share/doc/demo/README.md\n"));
        assert!(spec.contains("cp -a /tmp/staged/. %{buildroot}/\n"));
    }

    #[test]
    fn wix_source_holds_upgrade_code_and_files() {
        let app = FakeApp::new("wix");
        let ctx = app.context(WITH_ASSET);
        let stage = ctx.work_dir.join("msi");
        stage_tree(&ctx, &stage, "", "").expect("stage");
        let code = "9f0c86a1-2b3c-4d5e-8f90-112233445566";
        let wxs = wix_source(&ctx, &stage, code);
        assert!(wxs.contains(code));
        assert!(wxs.contains("Name=\"demo\""));
        assert!(wxs.contains("Manufacturer=\"Acme\""));
        assert!(wxs.contains("<MajorUpgrade"));
        assert!(wxs.contains("File0"));
        assert!(
            wxs.contains("File1"),
            "binary and asset each get a component"
        );
    }

    #[test]
    fn derived_upgrade_code_is_a_stable_guid() {
        let a = derived_upgrade_code("demo");
        let b = derived_upgrade_code("demo");
        let other = derived_upgrade_code("other");
        assert_eq!(a, b);
        assert_ne!(a, other);
        assert!(crate::manifest::is_safe_dist_path("x"));
        assert_eq!(a.len(), 36);
        assert!(a.as_bytes()[8] == b'-' && a.as_bytes()[13] == b'-');
    }

    #[test]
    fn inno_script_points_at_stage_and_out_dir() {
        let app = FakeApp::new("inno");
        let ctx = app.context(WITH_ASSET);
        let stage = ctx.work_dir.join("inno");
        let iss = inno_script(&ctx, &stage);
        assert!(iss.contains("AppName=demo\n"));
        assert!(iss.contains("AppVersion=1.2.0\n"));
        assert!(iss.contains("AppPublisher=Acme\n"));
        assert!(iss.contains("OutputBaseFilename=demo-1.2.0-setup\n"));
        assert!(iss.contains("recursesubdirs"));
    }

    #[test]
    fn stage_tree_places_binary_and_assets() {
        let app = FakeApp::new("stage");
        let ctx = app.context(WITH_ASSET);
        let root = ctx.work_dir.join("deb-stage");
        stage_tree(&ctx, &root, "usr/bin", "usr").expect("stage");
        assert!(root
            .join("usr/bin")
            .join(binary_file_name("demo"))
            .is_file());
        assert!(root.join("usr/share/doc/demo/README.md").is_file());
    }

    #[test]
    fn stage_tree_missing_asset_is_a_clear_error() {
        let app = FakeApp::new("missing-asset");
        let mut ctx = app.context(BASIC);
        ctx.dist.assets.push(DistAsset {
            source: "no-such-file".to_string(),
            dest: "share/x".to_string(),
        });
        let err = stage_tree(&ctx, &ctx.work_dir.join("s"), "", "").unwrap_err();
        assert!(err.to_string().contains("no-such-file"));
    }

    // The archive backends run the real tools, which exist on every
    // supported development host and CI runner. The deb backend runs only
    // where dpkg-deb exists (the ubuntu runner); elsewhere the test
    // confirms the missing-tool diagnostic instead.

    #[test]
    fn tar_backend_produces_an_archive() {
        let app = FakeApp::new("tar");
        let ctx = app.context(WITH_ASSET);
        let report = package_targets(&ctx, &["tar".to_string()]).expect("tar dist");
        assert_eq!(report.artifacts.len(), 1);
        let artifact = &report.artifacts[0];
        assert!(artifact.is_file(), "archive exists: {}", artifact.display());
        assert!(artifact
            .file_name()
            .unwrap()
            .to_string_lossy()
            .ends_with(".tar.gz"));
    }

    #[test]
    fn zip_backend_produces_an_archive_when_a_tool_exists() {
        let app = FakeApp::new("zip");
        let ctx = app.context(WITH_ASSET);
        match package_targets(&ctx, &["zip".to_string()]) {
            Ok(report) => {
                assert!(report.artifacts[0].is_file());
            }
            // A host with GNU tar and no zip has no zip-capable tool; the
            // diagnostic must say so rather than fail mysteriously.
            Err(OpError::DistTool { tool, .. }) => assert_eq!(tool, "zip"),
            Err(other) => panic!("unexpected error: {}", other),
        }
    }

    #[test]
    fn deb_backend_builds_where_dpkg_exists() {
        let app = FakeApp::new("deb");
        let ctx = app.context(WITH_ASSET);
        match package_targets(&ctx, &["deb".to_string()]) {
            Ok(report) => {
                let artifact = &report.artifacts[0];
                assert!(artifact.is_file());
                assert!(artifact
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .ends_with(".deb"));
            }
            Err(OpError::DistTool { tool, .. }) => assert_eq!(tool, "dpkg-deb"),
            Err(other) => panic!("unexpected error: {}", other),
        }
    }

    #[test]
    fn artifact_names_follow_each_format_convention() {
        let app = FakeApp::new("names");
        let ctx = app.context(BASIC);
        let report = package_targets(&ctx, &["tar".to_string()]).expect("tar dist");
        let name = report.artifacts[0]
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        assert_eq!(name, format!("demo-1.2.0-{}.tar.gz", host_arch()));
    }
}
