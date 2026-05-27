//! Linker invocation.
//!
//! After codegen has produced a relocatable object, the driver hands
//! it here together with the path to the runtime staticlib and the
//! desired output path. The linker uses the system `cc` driver so
//! that the system linker and its default lib paths come along for
//! free.
//!
//! On Windows, MinGW or clang must be on PATH because MSVC's
//! `link.exe` does not have a `cc`-style command line. Cross platform
//! installer packaging will smooth this out later (issue #92).

use std::path::{Path, PathBuf};
use std::process::Command;

use super::CodegenError;

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

/// Detect whether `cc` is available on PATH. Used by tests that need
/// to short circuit if the toolchain is missing.
pub fn cc_available() -> bool {
    which_cc().is_some()
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

/// Link `object_path` with `runtime` into `output`.
///
/// On non Windows targets, the resulting executable is also linked
/// against `-lpthread`, `-ldl`, and `-lm` which the Rust standard
/// library that the runtime depends on requires. On Windows, the
/// equivalent system libraries are added explicitly.
pub fn link(
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
    } else if cfg!(windows) {
        // Rust's std on MSVC and MinGW pulls in these system libs. The
        // MinGW driver knows about them by default for libstd, but
        // listing them here keeps the link line consistent.
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
