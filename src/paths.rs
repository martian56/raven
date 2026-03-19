use std::path::Path;

pub fn resolve_module_path(module_name: &str) -> String {
    if module_name.ends_with(".rv") {
        return module_name.to_string();
    }

    let filename = format!("{}.rv", module_name);

    let cwd_lib = format!("lib/{}", filename);
    if Path::new(&cwd_lib).exists() {
        return cwd_lib;
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
        let p = Path::new("/usr/share/raven/lib").join(&filename);
        if p.exists() {
            return p.to_string_lossy().into_owned();
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

    if Path::new(&filename).exists() {
        return filename;
    }

    cwd_lib
}
