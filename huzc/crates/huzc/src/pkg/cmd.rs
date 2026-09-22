//! 子命令编排:`huzc add/fetch/build` 离线流程。
//!
//! 说明:`fetch` 按传递闭包逐包落盘最高满足版本
//! (`vendor/<pkg>/<version>/`),冲突直接报错,不做自动升级。

use super::manifest::{Dependency, Manifest, format_manifest, parse_manifest};
use super::resolve::{copy_dir_all, find_manifest_file};
use super::solve::{SelectedDep, resolve_closure};
use crate::cli::{AddArgs, BuildArgs, FetchArgs};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

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
    let (manifest, proj_dir) = load_project_manifest(&args.path);
    let closure = must_resolve_closure(&manifest, &proj_dir);

    let vendor_dir = proj_dir.join("vendor");
    for (pkg, sel) in &closure {
        let target_dir = vendor_dir.join(pkg).join(sel.version.to_string());
        vendor_selected(&sel.source_dir, &target_dir);
        prune_stale_versions(&vendor_dir.join(pkg), &sel.version.to_string());
    }

    println!("Fetched {} dependency(ies) to vendor/", closure.len());
}

/// 按 `--path` 定位清单并解析,返回(清单,工程根);缺清单直接报错。
fn load_project_manifest(path_arg: &str) -> (Manifest, PathBuf) {
    let manifest_path = find_manifest_file(Path::new(path_arg))
        .unwrap_or_else(|| Path::new(path_arg).join("huzi.toml"));
    if !manifest_path.is_file() {
        eprintln!("huzi.toml not found in {}", path_arg);
        std::process::exit(1);
    }
    let content = fs::read_to_string(&manifest_path).unwrap_or_default();
    let manifest = parse_manifest(&content).unwrap_or_default();
    let proj_dir = manifest_path.parent().unwrap_or(Path::new(".")).to_path_buf();
    (manifest, proj_dir)
}

/// 求解传递闭包,冲突直接报错退出(沿用直接报错口径)。
fn must_resolve_closure(manifest: &Manifest, proj_dir: &Path) -> BTreeMap<String, SelectedDep> {
    match resolve_closure(manifest, proj_dir) {
        Ok(closure) => closure,
        Err(e) => {
            eprintln!("依赖解析失败: {}", e);
            std::process::exit(1);
        }
    }
}

/// 落盘单个选中依赖:目录递归拷贝,单文件则建目录后拷贝(兼容旧单文件 `path`)。
fn vendor_selected(src: &Path, target: &Path) {
    if same_dir(src, target) {
        return;
    }
    if src.is_file() {
        let _ = fs::create_dir_all(target);
        if let Some(fname) = src.file_name() {
            let _ = fs::copy(src, target.join(fname));
        }
    } else if src.is_dir() {
        let _ = copy_dir_all(src, target);
    }
}

/// 同目录判定(结构相等或规范化后相等,避免 vendor 自拷贝截断文件)。
fn same_dir(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

/// 清理该包下落选的版本子目录,使 `vendor/` 与求解结果一致。
fn prune_stale_versions(pkg_dir: &Path, keep: &str) {
    let Ok(entries) = fs::read_dir(pkg_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() && p.file_name().and_then(|n| n.to_str()) != Some(keep) {
            let _ = fs::remove_dir_all(&p);
        }
    }
}

/// 已选依赖必须已落盘 `vendor/<pkg>/<version>/`,缺失提示先 fetch。
fn ensure_vendored(closure: &BTreeMap<String, SelectedDep>, proj_dir: &Path, path_arg: &str) {
    for (pkg, sel) in closure {
        let vdir = proj_dir.join("vendor").join(pkg).join(sel.version.to_string());
        if !vdir.is_dir() {
            eprintln!(
                "缺少已选依赖 vendor/{}/{}:请先运行 `huzc fetch --path {}`",
                pkg, sel.version, path_arg
            );
            std::process::exit(1);
        }
    }
}

pub fn run_build(args: &BuildArgs) {
    let (manifest, proj_dir) = load_project_manifest(&args.path);
    let closure = must_resolve_closure(&manifest, &proj_dir);
    ensure_vendored(&closure, &proj_dir, &args.path);

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
