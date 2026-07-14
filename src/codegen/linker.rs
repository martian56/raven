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

use sha2::{Digest, Sha256};
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
    // The UCRT inlines the `printf` family in its headers, so the bare
    // symbols are not in `msvcrt.lib`. This compatibility lib provides them
    // as real functions, so an FFI call to `printf`/`sprintf`/... resolves.
    "/defaultlib:legacy_stdio_definitions",
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

/// Native inputs to link into the program: object files compiled from bundled
/// C sources, a Windows resource when configured, system library names (`-l`),
/// and raw linker arguments.
#[derive(Debug, Clone, Default)]
pub struct NativeLink {
    /// Object and resource files to link in.
    pub objects: Vec<PathBuf>,
    /// System library names to link, passed as `-l<name>` (or `<name>.lib`).
    pub libs: Vec<String>,
    /// Raw linker arguments passed through verbatim.
    pub link_args: Vec<String>,
}

/// The actionable error shown on a windows-msvc host when no MSVC C compiler
/// can be located. A package such as raven-sqlite bundles C that must be
/// compiled, and the MSVC build is the supported Windows toolchain.
const NO_MSVC_C_COMPILER: &str = "no C compiler found to build this package's bundled C \
    ([ffi]) sources, which a package such as raven-sqlite ships. Install the Visual Studio \
    C++ Build Tools, which provide cl.exe: run\n  \
    winget install Microsoft.VisualStudio.2022.BuildTools --override \"--quiet --wait --add \
    Microsoft.VisualStudio.Workload.VCTools --includeRecommended\"\nthen open a new terminal \
    and try again. (A MinGW gcc cannot be used here: it produces GNU-ABI objects that do not \
    link into the MSVC-targeted Raven build.)";

/// The actionable error shown on a non-msvc host when no C compiler is found.
const NO_C_COMPILER: &str = "no C compiler found to build this package's bundled C ([ffi]) \
    sources. Install a C toolchain (gcc or clang) and put it on PATH, or set the CC environment \
    variable to the compiler's path.";

/// Locate the C compiler for the `[ffi]` sources, with an actionable error when
/// none is installed.
///
/// On a windows-msvc host this mirrors the linker's `link.exe` lookup: it
/// resolves `cl.exe` through the Windows registry (vswhere), which also presets
/// the SDK and CRT include/lib environment, so no Developer Command Prompt is
/// needed. cc's own detection instead hands back a bare `cl.exe` that only
/// fails to launch later (a cryptic "program not found"), so the registry
/// result is checked directly to report the missing toolchain up front. A `CC`
/// override or a non-msvc host defers to cc's normal detection.
fn locate_c_compiler(triple: &str, build: &cc::Build) -> Result<cc::Tool, CodegenError> {
    let host = Triple::host();
    let cc_override = std::env::var_os("CC").is_some();
    if is_windows_msvc(&host) && !cc_override {
        return cc::windows_registry::find_tool(triple, "cl.exe")
            .ok_or_else(|| CodegenError::Target(NO_MSVC_C_COMPILER.to_string()));
    }
    build
        .try_get_compiler()
        .map_err(|_| CodegenError::Target(NO_C_COMPILER.to_string()))
}

/// Compile each bundled C source into an object file under `out_dir`,
/// returning their paths to link directly. Uses the `cc` crate's compiler
/// detection (cl.exe on windows-msvc, `cc`/`gcc` elsewhere), configured
/// explicitly so it needs no build-script environment. The compiler's stdout
/// is discarded so chatter (cl.exe echoes the source name) never reaches a
/// program's output under `rvpm run`.
pub fn compile_c_sources(
    sources: &[PathBuf],
    out_dir: &Path,
) -> Result<Vec<PathBuf>, CodegenError> {
    std::fs::create_dir_all(out_dir)
        .map_err(|e| CodegenError::Target(format!("create ffi build dir: {}", e)))?;
    let triple = host_triple_string();
    let mut build = cc::Build::new();
    build
        .cargo_metadata(false)
        // Suppress cc's `cargo:warning=...` chatter (for example "Compiler
        // family detection failed"): it leaks to the console of a user running
        // `rvpm`, who is not running cargo at all.
        .cargo_warnings(false)
        .opt_level(2)
        .debug(false)
        .warnings(false)
        .host(&triple)
        .target(&triple)
        .out_dir(out_dir);
    let compiler = locate_c_compiler(&triple, &build)?;
    let msvc = compiler.is_like_msvc();

    let mut objects = Vec::with_capacity(sources.len());
    for (index, source) in sources.iter().enumerate() {
        let stem = source.file_stem().and_then(|s| s.to_str()).unwrap_or("src");
        let object = out_dir.join(format!(
            "{}_{}.{}",
            stem,
            index,
            if msvc { "obj" } else { "o" }
        ));
        // Reuse a previously compiled object when the source has not changed
        // since, so a large bundled source (a 9 MB sqlite3.c) is not recompiled
        // on every build.
        if object_is_fresh(source, &object) {
            objects.push(object);
            continue;
        }
        let mut cmd = compiler.to_command();
        if msvc {
            // `/MD` selects the dynamic CRT to match the Rust/Raven objects,
            // which use it too. Without it cl.exe defaults to the static CRT
            // (LIBCMT) and the linker reports a defaultlib conflict (LNK4098).
            cmd.arg("/nologo")
                .arg("/MD")
                .arg("/c")
                .arg(source)
                .arg(format!("/Fo{}", object.display()));
        } else {
            cmd.arg("-c").arg(source).arg("-o").arg(&object);
        }
        let out = cmd
            .stdout(std::process::Stdio::null())
            .output()
            .map_err(|e| CodegenError::Target(format!("C compiler failed to launch: {}", e)))?;
        if !out.status.success() {
            return Err(CodegenError::Target(format!(
                "compiling {} failed:\n{}",
                source.display(),
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        // Record the source hash beside the fresh object so a later build reuses
        // it only while the source is byte-for-byte unchanged.
        if let Some(hash) = source_content_hash(source) {
            let _ = std::fs::write(object_hash_sidecar(&object), hash);
        }
        objects.push(object);
    }
    Ok(objects)
}

/// Compile an `.ico` file into a native Windows resource that the linker can
/// embed in the executable. MSVC targets use the Windows SDK's `rc.exe`;
/// GNU targets use MinGW-w64's `windres`.
pub fn compile_windows_icon(icon: &Path, out_dir: &Path) -> Result<PathBuf, CodegenError> {
    if !icon.is_file() {
        return Err(CodegenError::Target(format!(
            "Windows icon '{}' does not exist or is not a file",
            icon.display()
        )));
    }
    std::fs::create_dir_all(out_dir)
        .map_err(|e| CodegenError::Target(format!("create native build dir: {}", e)))?;

    // Give the resource script a fixed local filename. This avoids quoting and
    // escaping an arbitrary absolute Windows path in RC syntax.
    let local_icon = out_dir.join("raven_app.ico");
    std::fs::copy(icon, &local_icon)
        .map_err(|e| CodegenError::Target(format!("copy Windows icon: {}", e)))?;
    let script = out_dir.join("raven_app.rc");
    std::fs::write(&script, "1 ICON \"raven_app.ico\"\n")
        .map_err(|e| CodegenError::Target(format!("write Windows resource script: {}", e)))?;

    let triple = Triple::host();
    if is_windows_msvc(&triple) {
        let resource = out_dir.join("raven_app.res");
        let mut cmd = cc::windows_registry::find_tool(&host_triple_string(), "rc.exe")
            .ok_or_else(|| {
                CodegenError::Target(
                    "cannot embed the Windows icon because rc.exe was not found; install the Windows SDK through the Visual Studio C++ Build Tools"
                        .to_string(),
                )
            })?
            .to_command();
        cmd.current_dir(out_dir)
            .arg("/nologo")
            .arg("/fo")
            .arg("raven_app.res")
            .arg("raven_app.rc");
        run_resource_compiler(cmd, "rc.exe", &resource)?;
        Ok(resource)
    } else if is_windows_gnu(&triple) {
        let resource = out_dir.join("raven_app.o");
        let windres = find_on_path(&["windres.exe", "windres"]).ok_or_else(|| {
            CodegenError::Target(
                "cannot embed the Windows icon because windres was not found; install a MinGW-w64 toolchain and put windres on PATH"
                    .to_string(),
            )
        })?;
        let mut cmd = Command::new(windres);
        cmd.current_dir(out_dir).args([
            "raven_app.rc",
            "--output-format=coff",
            "-o",
            "raven_app.o",
        ]);
        run_resource_compiler(cmd, "windres", &resource)?;
        Ok(resource)
    } else {
        Err(CodegenError::Target(
            "Windows executable icons can only be compiled for a Windows target".to_string(),
        ))
    }
}

fn run_resource_compiler(mut cmd: Command, name: &str, output: &Path) -> Result<(), CodegenError> {
    let result = cmd
        .output()
        .map_err(|e| CodegenError::Target(format!("{} failed to launch: {}", name, e)))?;
    if !result.status.success() {
        return Err(CodegenError::Target(format!(
            "{} failed while compiling the Windows icon:\n{}",
            name,
            String::from_utf8_lossy(&result.stderr).trim()
        )));
    }
    if !output.is_file() {
        return Err(CodegenError::Target(format!(
            "{} did not produce '{}'",
            name,
            output.display()
        )));
    }
    Ok(())
}

fn find_on_path(names: &[&str]) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        for name in names {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// True when `object` exists and was built from the current contents of
/// `source`, so the cached object can be reused. Freshness is keyed on a content
/// hash recorded in a `<object>.hash` sidecar, not the modification time: a
/// source edited with its mtime preserved (a checkout, a restore) would
/// otherwise be skipped. A missing object, missing sidecar, or hash mismatch
/// returns false, so the safe default is to recompile.
fn object_is_fresh(source: &Path, object: &Path) -> bool {
    if !object.exists() {
        return false;
    }
    let Some(hash) = source_content_hash(source) else {
        return false;
    };
    match std::fs::read_to_string(object_hash_sidecar(object)) {
        Ok(stored) => stored.trim() == hash,
        Err(_) => false,
    }
}

/// The SHA-256 of `source`'s bytes as a hex string, or `None` when it cannot be
/// read.
fn source_content_hash(source: &Path) -> Option<String> {
    let bytes = std::fs::read(source).ok()?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Some(format!("{:x}", hasher.finalize()))
}

/// The sidecar path holding the source hash a cached object was built from.
fn object_hash_sidecar(object: &Path) -> PathBuf {
    let mut name = object.as_os_str().to_os_string();
    name.push(".hash");
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn missing_compiler_errors_are_actionable() {
        // The Windows message must name the toolchain and the command that
        // installs it, so a user who hits the no-compiler path knows exactly
        // what to do rather than seeing a raw "program not found".
        assert!(NO_MSVC_C_COMPILER.contains("Build Tools"));
        assert!(NO_MSVC_C_COMPILER.contains("winget"));
        assert!(NO_MSVC_C_COMPILER.contains("cl.exe"));
        // The non-Windows message must point at the fix (PATH or CC).
        assert!(NO_C_COMPILER.contains("PATH"));
        assert!(NO_C_COMPILER.contains("CC"));
    }

    #[test]
    fn compile_reuses_an_unchanged_object() {
        // Needs a C compiler; skip cleanly where none is available.
        if !cc_available() {
            return;
        }
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "raven-ffi-cache-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let out = dir.join("out");
        std::fs::create_dir_all(&dir).expect("mkdir");
        let source = dir.join("t.c");
        std::fs::write(
            &source,
            "#include <stdint.h>\nint64_t t(int64_t x){return x;}\n",
        )
        .expect("write source");

        let first = compile_c_sources(&[source.clone()], &out).expect("first compile");
        assert_eq!(first.len(), 1);
        let object = first[0].clone();
        let mtime = std::fs::metadata(&object).unwrap().modified().unwrap();

        // A second compile with the source unchanged reuses the object: the
        // file is not rewritten, so its mtime is identical.
        let second = compile_c_sources(&[source.clone()], &out).expect("second compile");
        assert_eq!(second[0], object);
        assert_eq!(
            std::fs::metadata(&object).unwrap().modified().unwrap(),
            mtime,
            "an unchanged source should reuse the cached object"
        );

        std::fs::remove_dir_all(&dir).ok();
    }
}

/// Link `object_path` with `runtime` and any `native` FFI inputs into `output`.
///
/// Selects the linker by host target triple. See the module docs for
/// the per-platform behavior.
pub fn link(
    object_path: &Path,
    runtime: &RuntimeStaticLib,
    native: &NativeLink,
    output: &Path,
) -> Result<LinkOutput, CodegenError> {
    let triple = Triple::host();
    if is_windows_msvc(&triple) {
        link_msvc(object_path, runtime, native, output)
    } else if is_windows_gnu(&triple) {
        link_gnu(object_path, runtime, native, output)
    } else {
        link_unix(object_path, runtime, native, output)
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
    native: &NativeLink,
    output: &Path,
) -> Result<LinkOutput, CodegenError> {
    let triple = host_triple_string();
    if let Some(mut cmd) = msvc_link_tool(&triple) {
        push_msvc_args(&mut cmd, object_path, runtime, native, output);
        run_linker(cmd, "link.exe", output)
    } else if let Some(lld) = locate_rust_lld() {
        // Best-effort fallback. `rust-lld` in lld-link flavor speaks the
        // same command line as `link.exe`, but it does not bring the SDK
        // library search paths, so the LIB environment from a developer
        // shell (or a prior `link.exe` discovery) must already be set.
        let mut cmd = Command::new(&lld);
        cmd.arg("-flavor").arg("link");
        push_msvc_args(&mut cmd, object_path, runtime, native, output);
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
    native: &NativeLink,
    output: &Path,
) {
    cmd.arg("/NOLOGO");
    cmd.arg(format!("/OUT:{}", output.display()));
    cmd.arg(object_path);
    cmd.arg(&runtime.path);
    // FFI inputs from `[ffi]`: compiled C objects, then named libraries
    // (`name` links `name.lib`), then any raw linker arguments.
    for object in &native.objects {
        cmd.arg(object);
    }
    for lib in &native.libs {
        cmd.arg(format!("{}.lib", lib));
    }
    for arg in &native.link_args {
        cmd.arg(arg);
    }
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
    native: &NativeLink,
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
    push_native_unix(&mut cmd, native);
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
    native: &NativeLink,
    output: &Path,
) -> Result<LinkOutput, CodegenError> {
    let cc =
        which_cc().ok_or_else(|| CodegenError::Target("no `cc` driver on PATH".to_string()))?;
    let mut cmd = Command::new(&cc);
    cmd.arg(object_path).arg(&runtime.path);
    push_native_unix(&mut cmd, native);
    cmd.arg("-o").arg(output);
    if cfg!(target_os = "linux") {
        cmd.args(["-lpthread", "-ldl", "-lm", "-lrt", "-lgcc_s", "-lutil"]);
    } else if cfg!(target_os = "macos") {
        // Apple's new linker (ld-prime) asserts on some relocation
        // patterns in the Cranelift object (Relocations.cpp
        // addFixupFromRelocations). The classic linker handles them, so
        // select it explicitly.
        //
        // CoreFoundation is required by a runtime dependency
        // (iana_time_zone, reached through the time stdlib). Rust's
        // `#[link(kind = "framework")]` directives are not applied when
        // the staticlib is handed to `cc` directly, so name the
        // framework explicitly, the same way the MSVC path names its
        // native libs.
        cmd.args([
            "-Wl,-ld_classic",
            "-lpthread",
            "-ldl",
            "-lm",
            "-framework",
            "CoreFoundation",
        ]);
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

/// Append the FFI inputs to a `cc`/`gcc` command line, after the program and
/// runtime objects: compiled C archives (linked in full), then `-l` libraries,
/// then raw linker arguments.
fn push_native_unix(cmd: &mut Command, native: &NativeLink) {
    for object in &native.objects {
        cmd.arg(object);
    }
    for lib in &native.libs {
        cmd.arg(format!("-l{}", lib));
    }
    for arg in &native.link_args {
        cmd.arg(arg);
    }
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
