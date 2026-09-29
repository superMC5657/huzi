//! 子命令编排:`huzc add/fetch/build` 离线流程。
//!
//! 说明:`fetch` 按传递闭包逐包落盘最高满足版本
//! (`vendor/<pkg>/<version>/`,含/全名即`vendor/<scope>/<name>/<ver>/`)并写 `huzi.lock`;
//! 注册表包走索引→下载→验哈希→落盘,复用 M2 原子时序;`build` 纯离线只认锁+vendor.
//!
//! M2:`fetch` 落盘后回填目录校验和,全部成功后才清理旧版本并写锁
//! (失败路径不删 `vendor`);`fetch --frozen` 只比锁不动盘.
//! M4:源优先级 path > registry=声明 > HUZI_REGISTRY > 本地;`--offline`下读索引直接错.

use super::hash;
use super::lock::{HuziLock, lock_path_for, read_lock_file, write_lock_file};
use super::manifest::{Dependency, Manifest, format_manifest, parse_manifest};
use super::resolve::{copy_dir_all, find_manifest_file};
use super::solve::{SelectedDep, SourceKind, resolve_closure_with};
use super::source::{default_registry, fetch_tarball, read_index, vendor_dir_for};
use super::version::SemVersion;
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
            registry: None,
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
    if args.offline && args.frozen {
        eprintln!("`--offline` 与 `--frozen` 不能同时使用,请只保留其一");
        std::process::exit(1);
    }
    let (manifest, proj_dir) = load_project_manifest(&args.path);
    let closure = must_resolve_closure(&manifest, &proj_dir, args.offline);
    if args.frozen {
        check_fetch_frozen(&closure, &proj_dir, &args.path);
        println!("Lockfile is up to date (frozen)");
        return;
    }
    // 逐包落盘并回填目录哈希;全部成功后才清理旧版本并写锁,
    // 任一步失败直接退出(不删 vendor,不写锁).注册表包先下载验哈希再落盘.
    let mut lock = HuziLock::from_closure(&closure);
    let vendor_dir = proj_dir.join("vendor");
    for (pkg, sel) in &closure {
        let target_dir = vendor_dir_for(&vendor_dir, pkg, &sel.version);
        if sel.source_kind == SourceKind::Registry {
            let decl = manifest.dependencies.get(pkg).and_then(|d| d.registry.as_deref());
            if let Err(e) = fetch_registry_package(pkg, &sel.version, decl, &target_dir, args.offline) {
                eprintln!("{}", e);
                std::process::exit(1);
            }
        } else if let Err(e) = vendor_selected(&sel.source_dir, &target_dir) {
            eprintln!("{}", e);
            std::process::exit(1);
        }
        let sum = hash::dir_checksum(&target_dir);
        if let Some(entry) = lock.packages.iter_mut().find(|e| &e.name == pkg) {
            entry.checksum = Some(sum);
            if sel.source_kind == SourceKind::Registry {
                let decl = manifest.dependencies.get(pkg).and_then(|d| d.registry.as_deref());
                entry.registry = decl.map(str::to_string).or_else(default_registry);
            }
        }
    }
    for (pkg, sel) in &closure {
        prune_stale_versions(&vendor_dir_for(&vendor_dir, pkg, &sel.version).parent().unwrap_or(&vendor_dir.join(pkg)), &sel.version.to_string());
    }
    if let Err(e) = write_lock_file(&lock_path_for(&proj_dir), &lock) {
        eprintln!("{}", e);
        std::process::exit(1);
    }

    println!("Fetched {} dependency(ies) to vendor/", closure.len());
}

/// 注册表包:读索引→下载tarball→验哈希→落盘vendor(目录直拷或tgz解包).
fn fetch_registry_package(pkg: &str, ver: &SemVersion, decl: Option<&str>, target: &Path, offline: bool) -> Result<(), String> {
    let (registry, entry) = find_registry_entry(pkg, ver, decl, offline)?;
    let tar_url = resolve_tar_url(&registry, pkg, &entry.tarball);
    if let Some(dir) = tar_url.strip_prefix("file://").map(Path::new).filter(|p| p.is_dir()) {
        if same_dir(dir, target) {
            return Ok(());
        }
        copy_dir_all(dir, target).map_err(|e| format!("拷贝 {} 失败: {}", dir.display(), e))?;
        return Ok(());
    }
    let tmp = std::env::temp_dir().join(format!("huzc_reg_{}_{}_{}.tgz", std::process::id(), pkg.replace('/', "_"), ver));
    fetch_tarball(&tar_url, &registry, &tmp, &entry.sha256, offline)?;
    extract_package(&tmp, target)?;
    let _ = fs::remove_file(&tmp);
    Ok(())
}

/// 按声明>环境顺序找索引中精确版本条目.
fn find_registry_entry(pkg: &str, ver: &SemVersion, decl: Option<&str>, offline: bool) -> Result<(String, super::source::IndexEntry), String> {
    let mut regs = Vec::new();
    if let Some(d) = decl {
        regs.push(d.to_string());
    }
    if let Some(env) = default_registry() {
        if !regs.contains(&env) {
            regs.push(env);
        }
    }
    if regs.is_empty() {
        return Err(format!("包 '{}' 需注册表但未声明 registry 且 HUZI_REGISTRY 未设", pkg));
    }
    for reg in regs {
        let entries = read_index(&reg, pkg, offline)?;
        if let Some(e) = entries.into_iter().find(|e| e.version == *ver) {
            return Ok((reg, e));
        }
    }
    Err(format!("注册表无包 '{}' 版本 {}", pkg, ver))
}

/// tarball相对路径相对`<registry>/<pkg>/`解析,其余原样.
fn resolve_tar_url(registry: &str, pkg: &str, tar: &str) -> String {
    if tar.contains("://") {
        return tar.to_string();
    }
    format!("{}/{}/{}", registry.trim_end_matches('/'), pkg, tar.trim_start_matches('/'))
}

/// 解包tgz到目标目录(经系统tar);非tgz单文件则直拷入内.
fn extract_package(tgz: &Path, target: &Path) -> Result<(), String> {
    fs::create_dir_all(target).map_err(|e| format!("创建 {} 失败: {}", target.display(), e))?;
    let name = tgz.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if name.ends_with(".tar.gz") || name.ends_with(".tgz") || name.ends_with(".tar") {
        let st = std::process::Command::new("tar").arg("-xzf").arg(tgz).arg("-C").arg(target).status()
            .map_err(|e| format!("tar 未安装或执行失败: {}", e))?;
        if !st.success() {
            return Err(format!("解包 {} 失败", tgz.display()));
        }
        strip_single_top_dir(target)?;
        return Ok(());
    }
    let fname = tgz.file_name().ok_or_else(|| "下载文件无名".to_string())?;
    fs::copy(tgz, target.join(fname)).map_err(|e| format!("拷贝单文件失败: {}", e))?;
    Ok(())
}

/// tgz若含唯一顶层目录则下沉一层(兼容`tar -czf -C pkg .`与整包目录两种打法).
fn strip_single_top_dir(target: &Path) -> Result<(), String> {
    let entries: Vec<PathBuf> = fs::read_dir(target).map_err(|e| format!("读 {} 失败: {}", target.display(), e))?
        .flatten().map(|e| e.path()).collect();
    if entries.len() == 1 && entries[0].is_dir() && entries[0].join("huzi.toml").is_file() {
        let inner = entries[0].clone();
        let tmp = target.join("__strip_tmp");
        fs::rename(&inner, &tmp).map_err(|e| format!("解包整理失败: {}", e))?;
        for e in fs::read_dir(&tmp).map_err(|e| format!("读 {} 失败: {}", tmp.display(), e))?.flatten() {
            fs::rename(e.path(), target.join(e.file_name())).map_err(|e| format!("解包整理失败: {}", e))?;
        }
        let _ = fs::remove_dir_all(&tmp);
    }
    Ok(())
}

/// `--frozen` 检查:锁必须已存在且与求解精确一致,一致直接成功(不动盘),
/// 缺锁/漂移均直接报错,不写锁不动 vendor(沿用直接报错口径)。
fn check_fetch_frozen(closure: &BTreeMap<String, SelectedDep>, proj_dir: &Path, path_arg: &str) {
    let lock = match read_lock_file(&lock_path_for(proj_dir)) {
        Ok(Some(l)) => l,
        Ok(None) => {
            eprintln!(
                "huzi.lock 缺失,冻结模式拒绝更新;请先运行 `huzc fetch --path {}`",
                path_arg
            );
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    };
    if let Some(pkg) = find_lock_mismatch(closure, &lock) {
        eprintln!(
            "huzi.lock 与 huzi.toml 不一致:包 '{}' 的锁定版本已漂移,冻结模式拒绝更新;请去掉 `--frozen` 后重 fetch",
            pkg
        );
        std::process::exit(1);
    }
}

/// 按 `--path` 定位清单并解析,返回(清单,工程根);缺清单/非法registry直接报错。
fn load_project_manifest(path_arg: &str) -> (Manifest, PathBuf) {
    let manifest_path = find_manifest_file(Path::new(path_arg))
        .unwrap_or_else(|| Path::new(path_arg).join("huzi.toml"));
    if !manifest_path.is_file() {
        eprintln!("huzi.toml not found in {}", path_arg);
        std::process::exit(1);
    }
    let content = fs::read_to_string(&manifest_path).unwrap_or_default();
    let manifest = match parse_manifest(&content) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("解析 {} 失败: {}", manifest_path.display(), e);
            std::process::exit(1);
        }
    };
    let proj_dir = manifest_path.parent().unwrap_or(Path::new(".")).to_path_buf();
    (manifest, proj_dir)
}

/// 求解传递闭包,冲突直接报错退出(沿用直接报错口径)。
fn must_resolve_closure(manifest: &Manifest, proj_dir: &Path, offline: bool) -> BTreeMap<String, SelectedDep> {
    match resolve_closure_with(manifest, proj_dir, offline) {
        Ok(closure) => closure,
        Err(e) => {
            eprintln!("依赖解析失败: {}", e);
            std::process::exit(1);
        }
    }
}

/// 落盘单个选中依赖:目录递归拷贝,单文件则建目录后拷贝(兼容旧单文件 `path`)。
///
/// 来源缺失或拷贝失败返回 Err,调用方直接退出(不删 vendor,不写锁)。
fn vendor_selected(src: &Path, target: &Path) -> Result<(), String> {
    if same_dir(src, target) {
        return Ok(());
    }
    if src.is_file() {
        fs::create_dir_all(target)
            .map_err(|e| format!("创建 {} 失败: {}", target.display(), e))?;
        if let Some(fname) = src.file_name() {
            fs::copy(src, target.join(fname))
                .map_err(|e| format!("拷贝 {} 失败: {}", src.display(), e))?;
        }
        Ok(())
    } else if src.is_dir() {
        copy_dir_all(src, target).map_err(|e| format!("拷贝 {} 失败: {}", src.display(), e))
    } else {
        Err(format!("依赖来源 {} 不存在,无法落盘", src.display()))
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

/// 已选依赖必须已落盘 `vendor/<pkg>/<version>/`(含/嵌套),缺失提示先 fetch。
fn ensure_vendored(closure: &BTreeMap<String, SelectedDep>, proj_dir: &Path, path_arg: &str) {
    for (pkg, sel) in closure {
        let vdir = vendor_dir_for(&proj_dir.join("vendor"), pkg, &sel.version);
        if !vdir.is_dir() {
            eprintln!(
                "缺少已选依赖 vendor/{}/{}:请先运行 `huzc fetch --path {}`",
                pkg, sel.version, path_arg
            );
            std::process::exit(1);
        }
    }
}

/// 锁一致性校验:有锁时必须与本次求解精确一致(漂移/缺失/多余均提示重 fetch)。
fn verify_lock_consistent(closure: &BTreeMap<String, SelectedDep>, proj_dir: &Path, path_arg: &str) {
    let lock = match read_lock_file(&lock_path_for(proj_dir)) {
        Ok(None) => return,
        Ok(Some(l)) => l,
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    };
    if let Some(pkg) = find_lock_mismatch(closure, &lock) {
        report_lock_mismatch(&pkg, path_arg);
    }
}

/// 锁与求解逐包比对:返回首个漂移/缺失/多余的包名,全一致返回 None。
fn find_lock_mismatch(closure: &BTreeMap<String, SelectedDep>, lock: &HuziLock) -> Option<String> {
    for (pkg, sel) in closure {
        if lock.get(pkg) != Some(sel.version) {
            return Some(pkg.clone());
        }
    }
    for entry in &lock.packages {
        if !closure.contains_key(&entry.name) {
            return Some(entry.name.clone());
        }
    }
    None
}

/// 锁不一致直接报错(沿用直接报错口径)。
fn report_lock_mismatch(pkg: &str, path_arg: &str) -> ! {
    eprintln!(
        "huzi.lock 与 huzi.toml 不一致:包 '{}' 的锁定版本已漂移;请重新运行 `huzc fetch --path {}`",
        pkg, path_arg
    );
    std::process::exit(1);
}

/// vendor 内容校验(M2 离线验):锁中有 checksum 的包重算目录哈希,失配硬错。
///
/// 无锁/旧锁无 checksum 直接放行(兼容旧工程);篡改 vendor 一字节即失配。
fn verify_vendor_checksums(
    closure: &BTreeMap<String, SelectedDep>,
    proj_dir: &Path,
    path_arg: &str,
) {
    let lock = match read_lock_file(&lock_path_for(proj_dir)) {
        Ok(None) => return,
        Ok(Some(l)) => l,
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    };
    for (pkg, sel) in closure {
        let expected = lock.get_entry(pkg).and_then(|e| e.checksum.as_deref());
        let Some(expected) = expected else {
            continue;
        };
        let vdir = vendor_dir_for(&proj_dir.join("vendor"), pkg, &sel.version);
        if !hash::verify_dir(&vdir, expected) {
            eprintln!(
                "vendor内容与huzi.lock不一致:包 '{}' 的 vendor 目录哈希已漂移,请重新运行 `huzc fetch --path {}`",
                pkg, path_arg
            );
            std::process::exit(1);
        }
    }
}

pub fn build_and_get_output(args: &BuildArgs) -> PathBuf {
    // build纯离线:只认锁+vendor,不读索引不联网.
    let (manifest, proj_dir) = load_project_manifest(&args.path);
    let closure = must_resolve_closure(&manifest, &proj_dir, true);
    verify_lock_consistent(&closure, &proj_dir, &args.path);
    ensure_vendored(&closure, &proj_dir, &args.path);
    verify_vendor_checksums(&closure, &proj_dir, &args.path);

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

    let mut exe_path = PathBuf::from(&output);
    if cfg!(target_os = "windows") && !exe_path.extension().map(|e| e == "exe").unwrap_or(false) {
        exe_path.set_extension("exe");
    }
    exe_path
}

pub fn run_build(args: &BuildArgs) {
    let _ = build_and_get_output(args);
}
