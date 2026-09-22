//! 子命令编排:`huzc add/fetch/build` 离线流程。
//!
//! 说明:`fetch` 拷贝目标仍按依赖 `version` 原串精确落盘
//! (`vendor/<pkg>/<version>/`),不受 `VersionReq` 范围语义影响。

use super::manifest::{Dependency, Manifest, format_manifest, parse_manifest};
use super::resolve::{copy_dir_all, find_manifest_file};
use crate::cli::{AddArgs, BuildArgs, FetchArgs};
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
