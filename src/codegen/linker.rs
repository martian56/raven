//! Linker invocation.
//!
//! After codegen has produced a relocatable object, the driver hands
//! it here together with the path to the runtime staticlib and the
//! desired output path. The link step is toolchain aware: it selects a
//! linker based on the host target triple so that the object Cranelift
//! emitted (an MSVC-flavor COFF on windows-msvc, ELF on Linux, Mach-O
//! on macOS) is handed to a linker that understands that format.
//!
//! * `*-windows-msvc`: the MSVC `link.exe`, located through the Windows
//!   registry by the `cc` crate. `link.exe` is the linker rustc itself
//!   uses on this host, and it understands the 64-bit COFF objects
//!   Cranelift produces. A fallback to the Rust toolchain's bundled
//!   `rust-lld` (in `lld-link` flavor) keeps the build working when the
//!   registry lookup fails.
//! * `*-windows-gnu`: a `cc`/`gcc` driver, which must be 64-bit
//!   MinGW-w64. A 32-bit MinGW.org `gcc` cannot read the 64-bit object,
//!   so a failure here surfaces a hint about the architecture mismatch.
//! * everything else (Linux, macOS): the system `cc` driver, which
//!   brings the system linker and its default library search paths.

use std::path::{Path, PathBuf};
use std::process::Command;

use target_lexicon::{OperatingSystem, Triple};

use super::CodegenError;

/// Rust standard library native system libraries that the staticlib
/// references on windows-msvc. These must appear on the MSVC link line
/// because the staticlib does not carry them itself.
///
/// Captured from
/// `cargo rustc -p raven-runtime --crate-type staticlib -- --print native-static-libs`
/// against rustc 1.85.0 (x86_64-pc-windows-msvc). Refresh the same way
/// if the runtime crate gains native dependencies. The trailing
/// `/defaultlib:msvcrt` selects the C runtime and is passed verbatim.
const MSVC_NATIVE_STATIC_LIBS: &[&str] = &[
    "bcrypt.lib",
    "kernel32.lib",
    "advapi32.lib",
    "ntdll.lib",
    "userenv.lib",
    "ws2_32.lib",
    "dbghelp.lib",
    "/defaultlib:msvcrt",
];

/// Where to find the runtime staticlib produced by Cargo.
#[derive(Debug, Clone)]
pub struct RuntimeStaticLib {
    pub path: PathBuf,
}

/// Result of the linker invocation.
#[derive(Debug)]
pub struct LinkOutput {
    pub binary: PathBuf,
}

/// Whether a usable linker is available for the host target.
///
/// On windows-msvc this means the MSVC `link.exe` or the bundled
/// `rust-lld` can be located. On other hosts it means a `cc` driver is
/// on PATH. Tests use this to skip only when no linker exists at all.
pub fn linker_available() -> bool {
    let triple = Triple::host();
    if is_windows_msvc(&triple) {
        msvc_link_tool(&host_triple_string()).is_some() || locate_rust_lld().is_some()
    } else {
        which_cc().is_some()
    }
}

/// Back-compat alias: the older name reported `cc` availability. The
/// smoke test and any external callers keep working through the new
/// toolchain-aware check.
pub fn cc_available() -> bool {
    linker_available()
}

fn which_cc() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let candidates = if cfg!(windows) {
        vec!["cc.exe", "gcc.exe", "clang.exe"]
    } else {
        vec!["cc", "gcc", "clang"]
    };
    for dir in std::env::split_paths(&path) {
        for name in &candidates {
            let p = dir.join(name);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

fn is_windows_msvc(triple: &Triple) -> bool {
    matches!(triple.operating_system, OperatingSystem::Windows)
        && triple.environment == target_lexicon::Environment::Msvc
}

fn is_windows_gnu(triple: &Triple) -> bool {
    matches!(triple.operating_system, OperatingSystem::Windows)
        && matches!(
            triple.environment,
            target_lexicon::Environment::Gnu | target_lexicon::Environment::GnuLlvm
        )
}

/// The host triple as the `&str` form `cc::windows_registry::find_tool`
/// expects (for example `x86_64-pc-windows-msvc`).
fn host_triple_string() -> String {
    Triple::host().to_string()
}

/// Link `object_path` with `runtime` into `output`.
///
/// Selects the linker by host target triple. See the module docs for
/// the per-platform behavior.
pub fn link(
    object_path: &Path,
    runtime: &RuntimeStaticLib,
    output: &Path,
) -> Result<LinkOutput, CodegenError> {
    let triple = Triple::host();
    if is_windows_msvc(&triple) {
        link_msvc(object_path, runtime, output)
    } else if is_windows_gnu(&triple) {
        link_gnu(object_path, runtime, output)
    } else {
        link_unix(object_path, runtime, output)
    }
}

/// Link an MSVC COFF object with the MSVC toolchain.
///
/// Prefers `link.exe` located through the Windows registry, because the
/// `cc` crate hands back a `Command` preloaded with the SDK and CRT
/// `LIB`/`PATH` environment. Falls back to the Rust toolchain's bundled
/// `rust-lld` (best effort) when the registry lookup fails.
fn link_msvc(
    object_path: &Path,
    runtime: &RuntimeStaticLib,
    output: &Path,
) -> Result<LinkOutput, CodegenError> {
    let triple = host_triple_string();
    if let Some(mut cmd) = msvc_link_tool(&triple) {
        push_msvc_args(&mut cmd, object_path, runtime, output);
        run_linker(cmd, "link.exe", output)
    } else if let Some(lld) = locate_rust_lld() {
        // Best-effort fallback. `rust-lld` in lld-link flavor speaks the
        // same command line as `link.exe`, but it does not bring the SDK
        // library search paths, so the LIB environment from a developer
        // shell (or a prior `link.exe` discovery) must already be set.
        let mut cmd = Command::new(&lld);
        cmd.arg("-flavor").arg("link");
        push_msvc_args(&mut cmd, object_path, runtime, output);
        run_linker(cmd, "rust-lld", output)
    } else {
        Err(CodegenError::Target(
            "no MSVC linker found: install the Visual Studio C++ build tools \
             (for link.exe) or ensure rust-lld ships with the active toolchain"
                .to_string(),
        ))
    }
}

/// Locate `link.exe` through the Windows registry and return a
/// `Command` preloaded with the SDK and CRT environment.
fn msvc_link_tool(triple: &str) -> Option<Command> {
    let tool = cc::windows_registry::find_tool(triple, "link.exe")?;
    Some(tool.to_command())
}

/// Append the common MSVC link arguments to `cmd`.
fn push_msvc_args(
    cmd: &mut Command,
    object_path: &Path,
    runtime: &RuntimeStaticLib,
    output: &Path,
) {
    cmd.arg("/NOLOGO");
    cmd.arg(format!("/OUT:{}", output.display()));
    cmd.arg(object_path);
    cmd.arg(&runtime.path);
    for lib in MSVC_NATIVE_STATIC_LIBS {
        cmd.arg(lib);
    }
    cmd.arg("/SUBSYSTEM:CONSOLE");
}

/// Locate `rust-lld.exe` under the active Rust toolchain sysroot.
fn locate_rust_lld() -> Option<PathBuf> {
    let sysroot = rustc_sysroot()?;
    let triple = host_triple_string();
    let p = sysroot
        .join("lib")
        .join("rustlib")
        .join(&triple)
        .join("bin")
        .join(if cfg!(windows) {
            "rust-lld.exe"
        } else {
            "rust-lld"
        });
    if p.is_file() {
        Some(p)
    } else {
        None
    }
}

/// Query `rustc --print sysroot`.
fn rustc_sysroot() -> Option<PathBuf> {
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let out = Command::new(rustc)
        .arg("--print")
        .arg("sysroot")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?;
    let trimmed = s.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(PathBuf::from(trimmed))
    }
}

/// Link via a MinGW-w64 `gcc`/`cc` driver on windows-gnu.
fn link_gnu(
    object_path: &Path,
    runtime: &RuntimeStaticLib,
    output: &Path,
) -> Result<LinkOutput, CodegenError> {
    let cc = which_cc().ok_or_else(|| {
        CodegenError::Target(
            "no `cc`/`gcc` driver on PATH for windows-gnu linking; a 64-bit \
             MinGW-w64 toolchain is required"
                .to_string(),
        )
    })?;
    let mut cmd = Command::new(&cc);
    cmd.arg(object_path).arg(&runtime.path);
    cmd.arg("-o").arg(output);
    // Rust's std on MinGW pulls in these system libs.
    cmd.args([
        "-luserenv",
        "-lbcrypt",
        "-lws2_32",
        "-ladvapi32",
        "-lkernel32",
        "-luser32",
        "-lntdll",
        "-lsynchronization",
    ]);
    let status = cmd
        .status()
        .map_err(|e| CodegenError::Target(format!("cc failed to launch: {}", e)))?;
    if !status.success() {
        return Err(CodegenError::Target(format!(
            "cc exited with status {}. On windows-gnu the linker must be a \
             64-bit MinGW-w64 gcc; a 32-bit MinGW.org gcc cannot read the \
             64-bit object Cranelift emits.",
            status
        )));
    }
    Ok(LinkOutput {
        binary: output.to_path_buf(),
    })
}

/// Link via the system `cc` driver on Linux, macOS, and other Unix
/// hosts. Adds the Rust std system libraries the runtime depends on.
fn link_unix(
    object_path: &Path,
    runtime: &RuntimeStaticLib,
    output: &Path,
) -> Result<LinkOutput, CodegenError> {
    let cc =
        which_cc().ok_or_else(|| CodegenError::Target("no `cc` driver on PATH".to_string()))?;
    let mut cmd = Command::new(&cc);
    cmd.arg(object_path).arg(&runtime.path);
    cmd.arg("-o").arg(output);
    if cfg!(target_os = "linux") {
        cmd.args(["-lpthread", "-ldl", "-lm", "-lrt", "-lgcc_s", "-lutil"]);
    } else if cfg!(target_os = "macos") {
        cmd.args(["-lpthread", "-ldl", "-lm"]);
    }
    let status = cmd
        .status()
        .map_err(|e| CodegenError::Target(format!("cc failed to launch: {}", e)))?;
    if !status.success() {
        return Err(CodegenError::Target(format!(
            "cc exited with status {}",
            status
        )));
    }
    Ok(LinkOutput {
        binary: output.to_path_buf(),
    })
}

/// Run a configured linker command and map its result.
fn run_linker(mut cmd: Command, name: &str, output: &Path) -> Result<LinkOutput, CodegenError> {
    let status = cmd
        .status()
        .map_err(|e| CodegenError::Target(format!("{} failed to launch: {}", name, e)))?;
    if !status.success() {
        return Err(CodegenError::Target(format!(
            "{} exited with status {}",
            name, status
        )));
    }
    Ok(LinkOutput {
        binary: output.to_path_buf(),
    })
}
