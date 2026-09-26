//! 模块入口探查与候选根(纯磁盘探查,失败返回 `None`/空表,不抛错)。

use std::path::{Path, PathBuf};

use tower_lsp_server::ls_types::Uri;

use super::{HUZI_SRC_SUBS, STEM_CANDIDATES};

/// 指定根集合内解析模块(供单测注入临时目录,纯磁盘探查)。
pub(crate) fn resolve_in_roots(
    import_name: &str,
    roots: &[PathBuf],
) -> Option<PathBuf> {
    let rel: PathBuf = import_name.split('.').collect();
    let file = rel.with_extension("hz");
    for root in roots {
        if let Some(hit) = probe_entry_file(root, &file) {
            return Some(hit);
        }
    }
    None
}

/// 探查单个根下的模块入口文件(直连 → `huzi.toml lib_entry` → 常规候选)。
pub(crate) fn probe_entry_file(root: &Path, file: &Path) -> Option<PathBuf> {
    let direct = root.join(file);
    if direct.is_file() {
        return Some(direct);
    }
    let stem_dir = root.join(file.with_extension(""));
    if stem_dir.is_dir() {
        let toml_path = stem_dir.join("huzi.toml");
        if toml_path.is_file() {
            if let Ok(content) = std::fs::read_to_string(&toml_path) {
                if let Some(entry) = parse_lib_entry(&content) {
                    let candidate = stem_dir.join(entry);
                    if candidate.is_file() {
                        return Some(candidate);
                    }
                }
            }
        }
        for candidate in STEM_CANDIDATES {
            let p = stem_dir.join(candidate);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// 候选根集合:当前文件目录 → 进程 cwd → 标准库根(环境/可执行文件旁/家目录)。
pub(crate) fn module_file_roots(current_uri: Option<&Uri>) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(uri) = current_uri {
        if let Some(path) = uri.to_file_path() {
            if let Some(dir) = path.parent() {
                roots.push(dir.to_path_buf());
            }
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd);
    }
    roots.extend(std_roots());
    let snapshot = roots.clone();
    for base in snapshot {
        for sub in HUZI_SRC_SUBS {
            roots.push(base.join(sub));
        }
    }
    roots
}

/// 标准库根:HUZI_LIB → 可执行文件旁 → 家目录(与存量跳转顺序一致)。
fn std_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(lib) = std::env::var("HUZI_LIB") {
        roots.push(PathBuf::from(lib));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            for sub in ["../huzi-src", "../../huzi-src", "../../../huzi-src", "../lib/huzi-src", "huzi-src"] {
                roots.push(exe_dir.join(sub));
            }
        }
    }
    if let Ok(home) = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")) {
        roots.push(PathBuf::from(&home).join(".huzi").join("huzi-src"));
        roots.push(PathBuf::from(&home).join(".huzi").join("std"));
    }
    roots
}

/// 根集合内找首个 `std/lib.hz`。
pub(super) fn find_std_lib(roots: &[PathBuf]) -> Option<PathBuf> {
    for root in roots {
        for rel in ["std/lib.hz", "huzi-src/std/lib.hz"] {
            let p = root.join(rel);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// `huzi.toml` 文本 -> `lib_entry`/`lib` 值(无依赖的最小解析,失败为 `None`)。
fn parse_lib_entry(content: &str) -> Option<String> {
    for line in content.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("lib_entry").or_else(|| line.strip_prefix("lib")) else {
            continue;
        };
        let Some(value) = rest.trim_start().strip_prefix('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"').trim_matches('\'');
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}
