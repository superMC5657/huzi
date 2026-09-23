use std::path::{Path, PathBuf};

/// 探查目录下的模块入口文件:
/// 1. `<root>/<file>` 直接存在 (如 `core/assert.hz`)
/// 2. `<root>/<file_stem>/huzi.toml` 中的 `lib_entry`
/// 3. 候选入口 `<root>/<file_stem>/{src/lib.hz, lib.hz, src/mod.hz, mod.hz}`
pub(super) fn probe_entry_file(root: &Path, file: &Path) -> Option<PathBuf> {
    let p = root.join(file);
    if p.is_file() {
        return Some(p);
    }
    let stem_path = file.with_extension("");
    let stem_dir = root.join(&stem_path);
    if stem_dir.is_dir() {
        let toml_path = stem_dir.join("huzi.toml");
        if toml_path.is_file() {
            if let Ok(content) = std::fs::read_to_string(&toml_path) {
                if let Ok(m) = crate::pkg::parse_manifest(&content) {
                    if let Some(lib_entry) = m.lib_entry {
                        let p = stem_dir.join(lib_entry);
                        if p.is_file() {
                            return Some(p);
                        }
                    }
                }
            }
        }
        for candidate in &["src/lib.hz", "lib.hz", "src/mod.hz", "mod.hz"] {
            let p = stem_dir.join(candidate);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// 模块名 -> 文件:点分段转子路径加 `.hz`,先找导入文件同目录,再找当前工作目录。
/// 纯函数版:找不到返回 `Err`,不退出进程。
pub(super) fn resolve_module_file_result(
    import_name: &str,
    base_dir: &Path,
) -> Result<PathBuf, String> {
    let segments: PathBuf = import_name.split('.').collect();
    let file = segments.with_extension("hz");
    for dir in [base_dir, Path::new(".")] {
        if let Some(hit) = probe_entry_file(dir, &file) {
            return Ok(hit);
        }
    }

    // 标准库根解析 (HUZI_LIB -> 可执行文件旁 ../huzi-src -> 相对路径 -> ~/.huzi)
    if let Some(hit) = resolve_std_module(&file, base_dir) {
        return Ok(hit);
    }

    // 尝试在 vendor/ 或 ~/.huzi/packages/ 中按包解析
    let segs: Vec<&str> = import_name.split('.').collect();
    if !segs.is_empty() {
        let pkg_name = segs[0];
        let sub_segs = &segs[1..];
        if let Some(hit) = crate::pkg::resolve_package_module(pkg_name, sub_segs, base_dir) {
            return Ok(hit);
        }
    }

    Err(format!(
        "Cannot find module '{}': tried {} and {}",
        import_name,
        base_dir.join(&file).display(),
        Path::new(".").join(&file).display()
    ))
}

/// 探查标准库模块文件:
/// 优先级: HUZI_LIB 环境变量 -> 可执行文件相对路径 -> base_dir/工作目录相对路径 -> ~/.huzi/
fn resolve_std_module(file: &Path, base_dir: &Path) -> Option<PathBuf> {
    if let Ok(lib) = std::env::var("HUZI_LIB") {
        if let Some(hit) = probe_entry_file(&PathBuf::from(lib), file) {
            return Some(hit);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            for sub in [
                "../huzi-src",
                "../../huzi-src",
                "../../../huzi-src",
                "../lib/huzi-src",
                "huzi-src",
            ] {
                if let Some(hit) = probe_entry_file(&exe_dir.join(sub), file) {
                    return Some(hit);
                }
            }
        }
    }
    for parent in [base_dir, Path::new(".")] {
        for sub in [
            "huzi-src",
            "../huzi-src",
            "../../huzi-src",
            "../../../huzi-src",
        ] {
            if let Some(hit) = probe_entry_file(&parent.join(sub), file) {
                return Some(hit);
            }
        }
    }
    if let Ok(home) = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")) {
        for sub in ["huzi-src", "std"] {
            if let Some(hit) =
                probe_entry_file(&PathBuf::from(&home).join(".huzi").join(sub), file)
            {
                return Some(hit);
            }
        }
    }
    None
}
