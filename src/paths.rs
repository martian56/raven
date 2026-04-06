use std::path::{Path, PathBuf};

/// Resolve `import name` to a `.rv` file path.
///
/// Order:
/// 1. **Current working directory** — `./name.rv`, then `./src/name.rv` (common project layout).
/// 2. **Stdlib** — `RAVEN_LIB_PATH` if set, then install-relative `lib/` next to the executable,
///    then platform install locations (`/usr/share/raven/lib`, Program Files, etc.).
///
/// Raven does not depend on rvpm; resolution uses only `cwd` and install paths.
pub fn resolve_module_path(module_name: &str) -> String {
    if module_name.ends_with(".rv") {
        return module_name.to_string();
    }

    let filename = format!("{}.rv", module_name);

    if let Ok(cwd) = std::env::current_dir() {
        let flat: PathBuf = cwd.join(&filename);
        if flat.exists() {
            return flat.to_string_lossy().into_owned();
        }
        let under_src: PathBuf = cwd.join("src").join(&filename);
        if under_src.exists() {
            return under_src.to_string_lossy().into_owned();
        }
    }

    if let Ok(path) = std::env::var("RAVEN_LIB_PATH") {
        let p = Path::new(&path).join(&filename);
        if p.exists() {
            return p.to_string_lossy().into_owned();
        }
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for lib_dir in [dir.join("lib"), dir.join("..").join("lib")] {
                let p = lib_dir.join(&filename);
                if p.exists() {
                    return p.to_string_lossy().into_owned();
                }
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        for base in [
            Path::new("/usr/share/raven/lib"),
            Path::new("/usr/local/share/raven/lib"),
        ] {
            let p = base.join(&filename);
            if p.exists() {
                return p.to_string_lossy().into_owned();
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Ok(prog) = std::env::var("PROGRAMFILES") {
            let p = Path::new(&prog).join("raven").join("lib").join(&filename);
            if p.exists() {
                return p.to_string_lossy().into_owned();
            }
        }
        if let Ok(prog) = std::env::var("PROGRAMFILES(X86)") {
            let p = Path::new(&prog).join("raven").join("lib").join(&filename);
            if p.exists() {
                return p.to_string_lossy().into_owned();
            }
        }
    }

    // Fallback for error messages (file usually missing).
    filename
}
