//! 包管理与依赖解析 (Package management and dependency resolution)。
//!
//! 支持 `huzi.toml` 清单、`huzc add/fetch/build` 离线子命令,
//! 模块解析优先级:内置 -> 相对路径 -> `vendor/` -> `~/.huzi/packages/`。

use crate::cli::{AddArgs, BuildArgs, FetchArgs};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    pub entry: Option<String>,
    pub lib_entry: Option<String>,
    pub dependencies: HashMap<String, Dependency>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Dependency {
    pub version: String,
    pub path: Option<String>,
}

impl Default for Manifest {
    fn default() -> Self {
        Self {
            name: "app".to_string(),
            version: "0.1.0".to_string(),
            entry: None,
            lib_entry: None,
            dependencies: HashMap::new(),
        }
    }
}

/// 解析简单的 `huzi.toml` 文本 (无三方依赖,离线即用)。
pub fn parse_manifest(content: &str) -> Result<Manifest, String> {
    let mut manifest = Manifest::default();
    let mut current_section = "";

    for line in content.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }

        if line.starts_with('[') && line.ends_with(']') {
            current_section = line[1..line.len() - 1].trim();
            continue;
        }

        let Some((key, val)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let val = val.trim();

        match current_section {
            "package" => match key {
                "name" => manifest.name = trim_quotes(val),
                "version" => manifest.version = trim_quotes(val),
                "entry" => manifest.entry = Some(trim_quotes(val)),
                "lib_entry" | "lib" => manifest.lib_entry = Some(trim_quotes(val)),
                _ => {}
            },
            "dependencies" => {
                let dep = parse_dependency_val(val);
                manifest.dependencies.insert(key.to_string(), dep);
            }
            _ => {}
        }
    }

    Ok(manifest)
}

fn trim_quotes(s: &str) -> String {
    s.trim().trim_matches('"').trim_matches('\'').to_string()
}

fn parse_dependency_val(val: &str) -> Dependency {
    let val = val.trim();
    if val.starts_with('{') && val.ends_with('}') {
        let inner = &val[1..val.len() - 1];
        let mut version = "0.1.0".to_string();
        let mut path = None;
        for pair in inner.split(',') {
            if let Some((k, v)) = pair.split_once(':') {
                let k = k.trim();
                let v = trim_quotes(v);
                if k == "version" {
                    version = v;
                } else if k == "path" {
                    path = Some(v);
                }
            } else if let Some((k, v)) = pair.split_once('=') {
                let k = k.trim();
                let v = trim_quotes(v);
                if k == "version" {
                    version = v;
                } else if k == "path" {
                    path = Some(v);
                }
            }
        }
        Dependency { version, path }
    } else {
        Dependency {
            version: trim_quotes(val),
            path: None,
        }
    }
}

pub fn format_manifest(manifest: &Manifest) -> String {
    let mut out = String::new();
    out.push_str("[package]\n");
    out.push_str(&format!("name = \"{}\"\n", manifest.name));
    out.push_str(&format!("version = \"{}\"\n", manifest.version));
    if let Some(entry) = &manifest.entry {
        out.push_str(&format!("entry = \"{}\"\n", entry));
    }
    if let Some(lib_entry) = &manifest.lib_entry {
        out.push_str(&format!("lib_entry = \"{}\"\n", lib_entry));
    }
    out.push('\n');

    out.push_str("[dependencies]\n");
    let mut dep_keys: Vec<_> = manifest.dependencies.keys().collect();
    dep_keys.sort();
    for k in dep_keys {
        let dep = &manifest.dependencies[k];
        if let Some(path) = &dep.path {
            out.push_str(&format!(
                "{} = {{ version = \"{}\", path = \"{}\" }}\n",
                k, dep.version, path
            ));
        } else {
            out.push_str(&format!("{} = \"{}\"\n", k, dep.version));
        }
    }
    out
}

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
    if let Some(m_file) = find_manifest_file(base_dir) {
        if let Some(p) = m_file.parent() {
            search_dirs.push(p.join("vendor"));
            // 若 manifest 中声明了显式 path 依赖，支持直接从本地源码路径解析
            if let Ok(content) = fs::read_to_string(&m_file) {
                if let Ok(manifest) = parse_manifest(&content) {
                    if let Some(dep) = manifest.dependencies.get(pkg_name) {
                        if let Some(dep_path) = &dep.path {
                            let direct_path = p.join(dep_path);
                            if direct_path.is_dir() {
                                if let Some(hit) = find_in_package_dir(&direct_path, sub_segs) {
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
        if let Some(hit) = find_in_package_dir(&pkg_root, sub_segs) {
            return Some(hit);
        }
    }

    // 全局缓存 ~/.huzi/packages/
    let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).ok()?;
    let global_cache = PathBuf::from(home).join(".huzi").join("packages").join(pkg_name);
    if global_cache.is_dir() {
        if let Some(hit) = find_in_package_dir(&global_cache, sub_segs) {
            return Some(hit);
        }
    }

    None
}

fn find_in_package_dir(pkg_dir: &Path, sub_segs: &[&str]) -> Option<PathBuf> {
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
            if let Some(hit) = match_module_file(&vdir, sub_segs) {
                return Some(hit);
            }
            if sub_segs.is_empty() {
                if let Some(pkg_stem) = pkg_dir.file_name().and_then(|n| n.to_str()) {
                    for candidate in &[
                        format!("src/{}.hz", pkg_stem),
                        format!("{}.hz", pkg_stem),
                    ] {
                        let p = vdir.join(candidate);
                        if p.is_file() {
                            return Some(p);
                        }
                    }
                }
            }
        }
    }

    // 2. pkg_dir 自身即为包源码根目录
    match_module_file(pkg_dir, sub_segs)
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

pub fn run_add(args: &AddArgs) {
    let manifest_path = find_manifest_file(Path::new(".")).unwrap_or_else(|| PathBuf::from("huzi.toml"));
    let mut manifest = if manifest_path.is_file() {
        let content = fs::read_to_string(&manifest_path).unwrap_or_default();
        parse_manifest(&content).unwrap_or_default()
    } else {
        Manifest::default()
    };

    manifest.dependencies.insert(
        args.package.clone(),
        Dependency {
            version: args.version.clone(),
            path: args.path.clone(),
        },
    );

    let formatted = format_manifest(&manifest);
    if let Err(e) = fs::write(&manifest_path, formatted) {
        eprintln!("Error saving {}: {}", manifest_path.display(), e);
        std::process::exit(1);
    }

    // 若指定了 --path,直接 vendor
    if let Some(src_path) = &args.path {
        let dest = Path::new("vendor").join(&args.package).join(&args.version);
        let src = Path::new(src_path);
        if src.is_dir() {
            let _ = copy_dir_all(src, &dest);
        } else if src.is_file() {
            let _ = fs::create_dir_all(&dest);
            let fname = src.file_name().unwrap();
            let _ = fs::copy(src, dest.join(fname));
        }
    }

    println!("Added dependency: {} v{}", args.package, args.version);
}

pub fn run_fetch(args: &FetchArgs) {
    let manifest_path = find_manifest_file(Path::new(&args.path))
        .unwrap_or_else(|| Path::new(&args.path).join("huzi.toml"));
    if !manifest_path.is_file() {
        eprintln!("huzi.toml not found in {}", args.path);
        std::process::exit(1);
    }

    let content = fs::read_to_string(&manifest_path).unwrap_or_default();
    let manifest = parse_manifest(&content).unwrap_or_default();

    let vendor_dir = manifest_path.parent().unwrap_or(Path::new(".")).join("vendor");
    let mut fetched = 0;

    for (pkg, dep) in &manifest.dependencies {
        let target_dir = vendor_dir.join(pkg).join(&dep.version);
        if let Some(src_path) = &dep.path {
            let src = manifest_path.parent().unwrap_or(Path::new(".")).join(src_path);
            if src.is_dir() {
                let _ = copy_dir_all(&src, &target_dir);
                fetched += 1;
            } else if src.is_file() {
                let _ = fs::create_dir_all(&target_dir);
                let fname = src.file_name().unwrap();
                let _ = fs::copy(&src, target_dir.join(fname));
                fetched += 1;
            }
        } else {
            // 从全局缓存拷贝
            if let Ok(home) = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")) {
                let cached = PathBuf::from(home)
                    .join(".huzi")
                    .join("packages")
                    .join(pkg)
                    .join(&dep.version);
                if cached.is_dir() {
                    let _ = copy_dir_all(&cached, &target_dir);
                    fetched += 1;
                }
            }
        }
    }

    println!("Fetched {} dependency(ies) to vendor/", fetched);
}

pub fn run_build(args: &BuildArgs) {
    let manifest_path = find_manifest_file(Path::new(&args.path))
        .unwrap_or_else(|| Path::new(&args.path).join("huzi.toml"));
    if !manifest_path.is_file() {
        eprintln!("huzi.toml not found in {}", args.path);
        std::process::exit(1);
    }

    let content = fs::read_to_string(&manifest_path).unwrap_or_default();
    let manifest = parse_manifest(&content).unwrap_or_default();
    let proj_dir = manifest_path.parent().unwrap_or(Path::new("."));

    let entry = if let Some(e) = &manifest.entry {
        proj_dir.join(e)
    } else if proj_dir.join("src/main.hz").is_file() {
        proj_dir.join("src/main.hz")
    } else if proj_dir.join("main.hz").is_file() {
        proj_dir.join("main.hz")
    } else if proj_dir.join(format!("{}.hz", manifest.name)).is_file() {
        proj_dir.join(format!("{}.hz", manifest.name))
    } else {
        eprintln!("No entry file found for package '{}'", manifest.name);
        std::process::exit(1);
    };

    let output = args.output.clone().unwrap_or_else(|| {
        proj_dir.join(&manifest.name).to_string_lossy().to_string()
    });

    let huzc_bin = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("huzc"));
    let mut cmd = std::process::Command::new(huzc_bin);
    cmd.arg("-i")
        .arg(&entry)
        .arg("-o")
        .arg(&output)
        .arg("--linker")
        .arg(args.linker.to_string());

    if args.release {
        cmd.arg("-r");
    }

    let status = cmd.status().unwrap_or_else(|e| {
        eprintln!("Failed to execute compiler: {}", e);
        std::process::exit(1);
    });

    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manifest_parse_and_format() {
        let toml = r#"
[package]
name = "my_app"
version = "1.2.0"
entry = "src/app.hz"
lib_entry = "src/lib.hz"

[dependencies]
foo = "0.1.0"
bar = { version = "2.0.0", path = "../bar" }
"#;
        let m = parse_manifest(toml).expect("parse ok");
        assert_eq!(m.name, "my_app");
        assert_eq!(m.version, "1.2.0");
        assert_eq!(m.entry.as_deref(), Some("src/app.hz"));
        assert_eq!(m.lib_entry.as_deref(), Some("src/lib.hz"));
        assert_eq!(m.dependencies.len(), 2);
        assert_eq!(m.dependencies["foo"].version, "0.1.0");
        assert_eq!(m.dependencies["foo"].path, None);
        assert_eq!(m.dependencies["bar"].version, "2.0.0");
        assert_eq!(m.dependencies["bar"].path.as_deref(), Some("../bar"));

        let formatted = format_manifest(&m);
        assert!(formatted.contains("name = \"my_app\""));
        assert!(formatted.contains("lib_entry = \"src/lib.hz\""));
        assert!(formatted.contains("bar = { version = \"2.0.0\", path = \"../bar\" }"));
        assert!(formatted.contains("foo = \"0.1.0\""));
    }

    #[test]
    fn test_find_manifest() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../test/pkg/app");
        let manifest = find_manifest_file(&root);
        assert!(manifest.is_some());
    }

    #[test]
    fn test_resolve_package_library_entry() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../test/pkg/app");
        // Test resolving top-level library entry: sub_segs is empty (&[])
        let hit = resolve_package_module("my_math", &[], &root);
        assert!(hit.is_some(), "Expected library entry to be resolved");
        let path = hit.unwrap();
        assert!(path.ends_with("lib.hz"), "Expected resolved path to end with lib.hz, got: {}", path.display());
    }

    #[test]
    fn test_resolve_package_submodule() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../test/pkg/app");
        // Test resolving submodule: sub_segs is ["calc"]
        let hit = resolve_package_module("my_math", &["calc"], &root);
        assert!(hit.is_some(), "Expected submodule calc to be resolved");
        let path = hit.unwrap();
        assert!(path.ends_with("calc.hz"), "Expected resolved path to end with calc.hz, got: {}", path.display());
    }
}

