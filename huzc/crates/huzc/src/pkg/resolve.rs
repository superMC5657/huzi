//! 模块解析:内置 -> 相对路径 -> `vendor/` -> `~/.huzi/packages/`。
//!
//! 说明:模块文件查找仍按 `vendor/<pkg>/<version>/` 精确目录行为,
//! 版本“选哪一份”由 `resolve_closure` 最高满足求解 + `fetch` 落盘决定
//! (逐包取最高满足版本,冲突直接报错,不做自动升级)。
//! M2:有锁时优先读锁选定版本目录,无锁/锁目录缺失回退降序(兼容旧工程)。

use super::lock::{lock_path_for, read_lock_file};
use super::manifest::parse_manifest;
use super::version::SemVersion;
use std::fs;
use std::path::{Path, PathBuf};

pub fn find_manifest_file(dir: &Path) -> Option<PathBuf> {
    let mut cur = if dir.is_file() {
        dir.parent()?.to_path_buf()
    } else {
        dir.to_path_buf()
    };
    loop {
        let toml = cur.join("huzi.toml");
        if toml.is_file() {
            return Some(toml);
        }
        if !cur.pop() {
            break;
        }
    }
    None
}

/// 复制目录 (递归)。
pub fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    if !dst.exists() {
        fs::create_dir_all(dst)?;
    }
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let dst_path = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &dst_path)?;
        } else {
            fs::copy(entry.path(), dst_path)?;
        }
    }
    Ok(())
}

/// 在 `vendor/` 或 `~/.huzi/packages/` 中查找包模块文件。
pub fn resolve_package_module(
    pkg_name: &str,
    sub_segs: &[&str],
    base_dir: &Path,
) -> Option<PathBuf> {
    let mut search_dirs = Vec::new();
    let mut locked_ver: Option<SemVersion> = None;
    if let Some(m_file) = find_manifest_file(base_dir) {
        if let Some(p) = m_file.parent() {
            locked_ver = read_locked_version(p, pkg_name);
            search_dirs.push(p.join("vendor"));
            // 若 manifest 中声明了显式 path 依赖，支持直接从本地源码路径解析
            if let Ok(content) = fs::read_to_string(&m_file) {
                if let Ok(manifest) = parse_manifest(&content) {
                    if let Some(dep) = manifest.dependencies.get(pkg_name) {
                        if let Some(dep_path) = &dep.path {
                            let direct_path = p.join(dep_path);
                            if direct_path.is_dir() {
                                if let Some(hit) =
                                    find_in_package_dir(&direct_path, sub_segs, locked_ver.as_ref())
                                {
                                    return Some(hit);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    search_dirs.push(base_dir.join("vendor"));
    search_dirs.push(PathBuf::from("vendor"));

    for vendor in &search_dirs {
        let pkg_root = vendor.join(pkg_name);
        if !pkg_root.is_dir() {
            continue;
        }
        if let Some(hit) = find_in_package_dir(&pkg_root, sub_segs, locked_ver.as_ref()) {
            return Some(hit);
        }
    }

    // 全局缓存 ~/.huzi/packages/
    let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).ok()?;
    let global_cache = PathBuf::from(home).join(".huzi").join("packages").join(pkg_name);
    if global_cache.is_dir() {
        if let Some(hit) = find_in_package_dir(&global_cache, sub_segs, locked_ver.as_ref()) {
            return Some(hit);
        }
    }

    None
}

/// 读工程锁中该包的选定版本(无锁/非法锁/无条目返回 None,调用方回退降序)。
fn read_locked_version(proj_dir: &Path, pkg_name: &str) -> Option<SemVersion> {
    read_lock_file(&lock_path_for(proj_dir)).ok()??.get(pkg_name)
}

fn find_in_package_dir(
    pkg_dir: &Path,
    sub_segs: &[&str],
    locked: Option<&SemVersion>,
) -> Option<PathBuf> {
    // 0. 锁优先:锁选定版本目录存在则先试,未命中回退降序
    if let Some(ver) = locked {
        let vdir = pkg_dir.join(ver.to_string());
        if vdir.is_dir() {
            if let Some(hit) = find_in_version_dir(&vdir, pkg_dir, sub_segs) {
                return Some(hit);
            }
        }
    }
    // 1. 如果 pkg_dir 下有版本子目录 (例如 1.0.0/)
    if let Ok(entries) = fs::read_dir(pkg_dir) {
        let mut ver_dirs: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        ver_dirs.sort();
        ver_dirs.reverse(); // 优先尝试最高版本
        for vdir in ver_dirs {
            if let Some(hit) = find_in_version_dir(&vdir, pkg_dir, sub_segs) {
                return Some(hit);
            }
        }
    }

    // 2. pkg_dir 自身即为包源码根目录
    match_module_file(pkg_dir, sub_segs)
}

/// 单个版本子目录内找模块(先规范入口,再包名同名文件;原逐版本块的纯搬移)。
fn find_in_version_dir(vdir: &Path, pkg_dir: &Path, sub_segs: &[&str]) -> Option<PathBuf> {
    if let Some(hit) = match_module_file(vdir, sub_segs) {
        return Some(hit);
    }
    if sub_segs.is_empty() {
        if let Some(pkg_stem) = pkg_dir.file_name().and_then(|n| n.to_str()) {
            for candidate in &[format!("src/{}.hz", pkg_stem), format!("{}.hz", pkg_stem)] {
                let p = vdir.join(candidate);
                if p.is_file() {
                    return Some(p);
                }
            }
        }
    }
    None
}

fn match_module_file(root: &Path, sub_segs: &[&str]) -> Option<PathBuf> {
    if !sub_segs.is_empty() {
        let file_path: PathBuf = sub_segs.iter().collect();
        let target = root.join(&file_path).with_extension("hz");
        if target.is_file() {
            return Some(target);
        }
        let src_target = root.join("src").join(&file_path).with_extension("hz");
        if src_target.is_file() {
            return Some(src_target);
        }
    } else {
        // 0. 优先检查 huzi.toml 中指定的 lib_entry / lib
        let toml_path = root.join("huzi.toml");
        if toml_path.is_file() {
            if let Ok(content) = fs::read_to_string(&toml_path) {
                if let Ok(m) = parse_manifest(&content) {
                    if let Some(lib_entry) = m.lib_entry {
                        let p = root.join(lib_entry);
                        if p.is_file() {
                            return Some(p);
                        }
                    }
                }
            }
        }
        // 1. 规范库入口优先级: src/lib.hz -> lib.hz -> src/mod.hz -> mod.hz
        for candidate in &["src/lib.hz", "lib.hz", "src/mod.hz", "mod.hz"] {
            let p = root.join(candidate);
            if p.is_file() {
                return Some(p);
            }
        }
        // 2. 包名同名文件: src/<pkg>.hz -> <pkg>.hz
        if let Some(pkg_stem) = root.file_name().and_then(|n| n.to_str()) {
            for candidate in &[
                format!("src/{}.hz", pkg_stem),
                format!("{}.hz", pkg_stem),
            ] {
                let p = root.join(candidate);
                if p.is_file() {
                    return Some(p);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_manifest() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../test/pkg/app");
        let manifest = find_manifest_file(&root);
        assert!(manifest.is_some());
    }

    #[test]
    fn test_resolve_package_library_entry() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../test/pkg/app");
        // 测试解析顶层库入口：sub_segs 为空 (&[])
        let hit = resolve_package_module("my_math", &[], &root);
        assert!(hit.is_some(), "Expected library entry to be resolved");
        let path = hit.unwrap();
        assert!(path.ends_with("lib.hz"), "Expected resolved path to end with lib.hz, got: {}", path.display());
    }

    #[test]
    fn test_resolve_package_submodule() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../test/pkg/app");
        // 测试解析子模块：sub_segs 为 ["calc"]
        let hit = resolve_package_module("my_math", &["calc"], &root);
        assert!(hit.is_some(), "Expected submodule calc to be resolved");
        let path = hit.unwrap();
        assert!(path.ends_with("calc.hz"), "Expected resolved path to end with calc.hz, got: {}", path.display());
    }

}
