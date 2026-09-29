//! `huzc update`:显式重解闭包 + 只读矩阵 + 显式重锁(P3-M3)。
//!
//! 语义:默认不动(无变化直接 `all up to date`,exit 0);`--dry-run` 全程只读
//! (重解闭包 + 与锁 diff + 打印矩阵,不写 `vendor/`/`huzi.lock`,不改 `huzi.toml`);
//! 非 dry-run 且有变化时落盘 + 写锁(复用 M2 时序:完整求解成功后再写,
//! 禁止边解边写);菱形冲突沿用求解原文案直接报错,不写盘。

use super::hash;
use super::lock::{HuziLock, lock_path_for, read_lock_file, write_lock_file};
use super::manifest::Manifest;
use super::resolve::{copy_dir_all, find_manifest_file};
use super::solve::{SelectedDep, resolve_closure};
use crate::cli::UpdateArgs;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// 矩阵单行:包名 + 约束 + 锁版本/候选版本 + 来源 + 新旧校验和。
#[derive(Debug, Clone)]
pub struct UpdateRow {
    pub name: String,
    pub constraint: String,
    pub locked: Option<super::version::SemVersion>,
    pub candidate: super::version::SemVersion,
    pub source: String,
    pub old_checksum: Option<String>,
    pub new_checksum: String,
}

impl UpdateRow {
    /// 锁版本与候选不一致即需更新(校验和差异不触发更新,交由 build 硬错)。
    pub fn is_changed(&self) -> bool {
        self.locked != Some(self.candidate)
    }
}

/// 只读规划结果:展示行 + 已求解闭包(供显式 apply 复用,避免二次求解写穿)。
#[derive(Debug, Clone)]
pub struct UpdatePlan {
    pub rows: Vec<UpdateRow>,
    pub closure: BTreeMap<String, SelectedDep>,
}

impl UpdatePlan {
    /// 任一行版本漂移即有变化。
    pub fn has_changes(&self) -> bool {
        self.rows.iter().any(|r| r.is_changed())
    }
}

/// 只读规划:重解全闭包 + 与锁 diff,不触碰磁盘写(调用方负责展示/显式落盘)。
pub fn plan_update(
    root: &Manifest,
    root_dir: &Path,
    lock: Option<&HuziLock>,
    filter: Option<&str>,
) -> Result<UpdatePlan, String> {
    let closure = resolve_closure(root, root_dir)?;
    if let Some(want) = filter {
        if !closure.contains_key(want) {
            return Err(format!("包 '{}' 不在依赖闭包中", want));
        }
    }
    let mut rows = build_rows(root, &closure, lock);
    if let Some(want) = filter {
        rows.retain(|r| r.name == want);
    }
    Ok(UpdatePlan { rows, closure })
}

/// 由闭包 + 锁逐包拼行(按包名排序,新校验和为候选源目录哈希,只读计算)。
fn build_rows(
    root: &Manifest,
    closure: &BTreeMap<String, SelectedDep>,
    lock: Option<&HuziLock>,
) -> Vec<UpdateRow> {
    let mut rows = Vec::new();
    for (name, sel) in closure {
        let locked = lock.and_then(|l| l.get(name));
        let old_checksum = lock
            .and_then(|l| l.get_entry(name))
            .and_then(|e| e.checksum.clone());
        rows.push(UpdateRow {
            name: name.clone(),
            constraint: constraint_of(root, name),
            locked,
            candidate: sel.version,
            source: sel.source_kind.as_str().to_string(),
            old_checksum,
            new_checksum: hash::dir_checksum(&sel.source_dir),
        });
    }
    rows
}

/// 约束列:根清单直接依赖取原串,传递依赖记 `(传递)`。
fn constraint_of(root: &Manifest, name: &str) -> String {
    root.dependencies
        .get(name)
        .map(|d| d.version.clone())
        .unwrap_or_else(|| "(传递)".to_string())
}

/// 渲染只读矩阵;无变化返回 `all up to date`(调用方打印,exit 0)。
pub fn render_matrix(plan: &UpdatePlan) -> String {
    let mut changed: Vec<&UpdateRow> = plan.rows.iter().filter(|r| r.is_changed()).collect();
    changed.sort_by(|a, b| a.name.cmp(&b.name));
    if changed.is_empty() {
        return "all up to date\n".to_string();
    }
    let mut out = String::from("package | constraint | locked -> candidate | source | checksum(old -> new)\n");
    for r in changed {
        let locked = r.locked.map(|v| v.to_string()).unwrap_or_else(|| "-".to_string());
        out.push_str(&format!(
            "{} | {} | {} -> {} | {} | {} -> {}\n",
            r.name,
            r.constraint,
            locked,
            r.candidate,
            r.source,
            short_hash(r.old_checksum.as_deref()),
            short_hash(Some(&r.new_checksum)),
        ));
    }
    out
}

/// 短哈希:取 `sha256:` 后 8 位 hex,缺省记 `-`。
fn short_hash(sum: Option<&str>) -> String {
    match sum {
        None => "-".to_string(),
        Some(s) => {
            let hex = s.strip_prefix("sha256:").unwrap_or(s);
            hex.chars().take(8).collect()
        }
    }
}

/// 显式落盘 + 写锁(复用 M2 时序:逐包拷贝算哈希全成功后才清理旧版并写锁)。
pub fn apply_update(
    proj_dir: &Path,
    closure: &BTreeMap<String, SelectedDep>,
) -> Result<HuziLock, String> {
    let mut lock = HuziLock::from_closure(closure);
    let vendor_dir = proj_dir.join("vendor");
    for (pkg, sel) in closure {
        let target = vendor_dir.join(pkg).join(sel.version.to_string());
        vendor_one(&sel.source_dir, &target)?;
        let sum = hash::dir_checksum(&target);
        if let Some(e) = lock.packages.iter_mut().find(|e| &e.name == pkg) {
            e.checksum = Some(sum);
        }
    }
    for (pkg, sel) in closure {
        prune_one(&vendor_dir.join(pkg), &sel.version.to_string());
    }
    write_lock_file(&lock_path_for(proj_dir), &lock)?;
    Ok(lock)
}

/// 落盘单个选中依赖(同目录自拷贝跳过,缺源/拷贝失败返回 Err,不删旧版)。
fn vendor_one(src: &Path, target: &Path) -> Result<(), String> {
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
        return Ok(());
    }
    if src.is_dir() {
        return copy_dir_all(src, target)
            .map_err(|e| format!("拷贝 {} 失败: {}", src.display(), e));
    }
    Err(format!("依赖来源 {} 不存在,无法落盘", src.display()))
}

/// 同目录判定(结构相等或规范化后相等,避免 vendor 自拷贝截断)。
fn same_dir(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

/// 清理该包下落选版本子目录(与 fetch 同口径)。
fn prune_one(pkg_dir: &Path, keep: &str) {
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

/// `huzc update` 编排:先完整求解 + 只读矩阵,非 dry-run 有变化才显式重锁。
pub fn run_update(args: &UpdateArgs) {
    let (manifest, proj_dir) = load_proj(&args.path);
    let lock = match read_lock_file(&lock_path_for(&proj_dir)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    };
    let plan = match plan_update(&manifest, &proj_dir, lock.as_ref(), args.package.as_deref()) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("依赖解析失败: {}", e);
            std::process::exit(1);
        }
    };
    print!("{}", render_matrix(&plan));
    if args.dry_run || !plan.has_changes() {
        return;
    }
    if let Err(e) = apply_update(&proj_dir, &plan.closure) {
        eprintln!("{}", e);
        std::process::exit(1);
    }
}

/// 按 `--path` 定位清单并解析(缺清单直接报错,与 fetch/build 同口径)。
fn load_proj(path_arg: &str) -> (Manifest, PathBuf) {
    let manifest_path =
        find_manifest_file(Path::new(path_arg)).unwrap_or_else(|| Path::new(path_arg).join("huzi.toml"));
    if !manifest_path.is_file() {
        eprintln!("huzi.toml not found in {}", path_arg);
        std::process::exit(1);
    }
    let content = fs::read_to_string(&manifest_path).unwrap_or_default();
    let manifest = super::manifest::parse_manifest(&content).unwrap_or_default();
    let proj_dir = manifest_path.parent().unwrap_or(Path::new(".")).to_path_buf();
    (manifest, proj_dir)
}

#[cfg(test)]
mod tests {
    use super::super::manifest::parse_manifest;
    use super::super::resolve::copy_dir_all;
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    fn tmp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "huzc_update_{}_{}_{}",
            std::process::id(),
            tag,
            SEQ.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("临时目录可建");
        dir
    }

    fn fixture_pkg_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../test/pkg")
    }

    /// 搭隔离工程:复制 my_math_multi(1.0.0/1.2.0)+自造 1.3.0,锁/ vendor 停在 1.2.0。
    fn setup_multi_with_130(tag: &str) -> (PathBuf, PathBuf) {
        let root = tmp_root(tag);
        let src_base = fixture_pkg_dir();
        let fix = root.join("fixtures/my_math_multi");
        for ver in ["1.0.0", "1.2.0"] {
            let src = src_base.join("fixtures/my_math_multi").join(ver);
            copy_dir_all(&src, &fix.join(ver)).expect("夹具可拷");
        }
        // 1.3.0 由 1.2.0 复制并 bump 版本/api_level(内容必变,哈希必变)。
        copy_dir_all(&fix.join("1.2.0"), &fix.join("1.3.0")).expect("1.3.0 可建");
        let toml_130 = "[package]\nname = \"my_math\"\nversion = \"1.3.0\"\n";
        fs::write(fix.join("1.3.0/huzi.toml"), toml_130).expect("1.3.0 清单可写");
        let calc = fs::read_to_string(fix.join("1.3.0/calc.hz")).expect("calc 可读");
        fs::write(fix.join("1.3.0/calc.hz"), calc.replace("return 12", "return 13"))
            .expect("calc 可写");
        let proj = root.join("app");
        fs::create_dir_all(proj.join("vendor/my_math/1.2.0")).expect("vendor 可建");
        copy_dir_all(&fix.join("1.2.0"), &proj.join("vendor/my_math/1.2.0")).expect("vendor 可拷");
        let manifest = "[package]\nname = \"pkg_app_multi\"\nversion = \"0.1.0\"\n\n[dependencies]\nmy_math = { version = \"^1.0.0\", path = \"../fixtures/my_math_multi\" }\n";
        fs::write(proj.join("huzi.toml"), manifest).expect("清单可写");
        let lock = "# huzi.lock 由 `huzc fetch` 生成,请勿手动编辑。\n[[package]]\nname = \"my_math\"\nversion = \"1.2.0\"\nsource = \"path\"\n";
        fs::write(proj.join("huzi.lock"), lock).expect("锁可写");
        (root, proj)
    }

    fn load_manifest(proj: &Path) -> Manifest {
        let content = fs::read_to_string(proj.join("huzi.toml")).expect("清单可读");
        parse_manifest(&content).expect("清单可解析")
    }

    #[test]
    fn test_update_multi_dry_run_readonly_matrix() {
        let (root, proj) = setup_multi_with_130("dry");
        let manifest = load_manifest(&proj);
        let lock_text_before = fs::read_to_string(proj.join("huzi.lock")).expect("锁可读");
        let lock = super::super::lock::parse_lock(&lock_text_before).expect("锁可解析");
        let mtime_before = fs::metadata(proj.join("huzi.lock")).expect("mtime 可读").modified().ok();
        let plan = plan_update(&manifest, &proj, Some(&lock), None).expect("应规划成功");
        assert_eq!(plan.rows.len(), 1, "单包工程应单行");
        let row = &plan.rows[0];
        assert_eq!(row.name, "my_math");
        assert_eq!(row.constraint, "^1.0.0");
        assert_eq!(row.locked.unwrap().to_string(), "1.2.0");
        assert_eq!(row.candidate.to_string(), "1.3.0");
        assert_eq!(row.source, "path");
        assert!(row.is_changed());
        let text = render_matrix(&plan);
        assert!(text.contains("my_math"), "矩阵应含包名,实际:\n{}", text);
        assert!(text.contains("1.2.0 -> 1.3.0"), "矩阵应显旧->新,实际:\n{}", text);
        assert!(text.contains("^1.0.0"), "矩阵应显约束,实际:\n{}", text);
        // dry-run 只读:锁文件内容与 mtime 不变,无 1.3.0 落盘。
        let lock_text_after = fs::read_to_string(proj.join("huzi.lock")).expect("锁可读");
        assert_eq!(lock_text_before, lock_text_after, "dry-run 不得改锁");
        let mtime_after = fs::metadata(proj.join("huzi.lock")).expect("mtime 可读").modified().ok();
        assert_eq!(mtime_before, mtime_after, "dry-run 不得碰 mtime");
        assert!(!proj.join("vendor/my_math/1.3.0").exists(), "dry-run 不得落盘");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn test_update_multi_apply_writes_lock_130() {
        let (root, proj) = setup_multi_with_130("apply");
        let manifest = load_manifest(&proj);
        let toml_before = fs::read_to_string(proj.join("huzi.toml")).expect("清单可读");
        let lock = super::super::lock::parse_lock(
            &fs::read_to_string(proj.join("huzi.lock")).expect("锁可读"),
        )
        .expect("锁可解析");
        let plan = plan_update(&manifest, &proj, Some(&lock), None).expect("应规划成功");
        assert!(plan.has_changes());
        apply_update(&proj, &plan.closure).expect("显式重锁应成功");
        let back = super::super::lock::parse_lock(
            &fs::read_to_string(proj.join("huzi.lock")).expect("锁可读"),
        )
        .expect("新锁可解析");
        assert_eq!(back.get("my_math").unwrap().to_string(), "1.3.0");
        assert!(proj.join("vendor/my_math/1.3.0").is_dir(), "应落盘 1.3.0");
        let toml_after = fs::read_to_string(proj.join("huzi.toml")).expect("清单可读");
        assert_eq!(toml_before, toml_after, "不得改写 huzi.toml 版本串");
        // 二次规划应 all up to date(幂等)。
        let plan2 = plan_update(&manifest, &proj, Some(&back), None).expect("二次应成功");
        assert!(!plan2.has_changes());
        assert_eq!(render_matrix(&plan2), "all up to date\n");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn test_update_diamond_conflict_same_text_no_side_effects() {
        let base = fixture_pkg_dir().join("app_diamond");
        let content = fs::read_to_string(base.join("huzi.toml")).expect("菱形清单可读");
        let manifest = parse_manifest(&content).expect("菱形清单可解析");
        let expect = resolve_closure(&manifest, &base).expect_err("菱形应冲突");
        assert!(expect.contains("依赖冲突"), "应点明冲突,实际: {}", expect);
        assert!(expect.contains("shared"), "应点名 shared,实际: {}", expect);
        // 双路径(不过滤/按包过滤)同文案,且零副作用(不建锁/vendor)。
        let e1 = plan_update(&manifest, &base, None, None).expect_err("dry 全量应冲突");
        let e2 = plan_update(&manifest, &base, None, Some("shared")).expect_err("按包仍冲突");
        assert_eq!(e1, expect, "dry 文案须与求解一致");
        assert_eq!(e2, expect, "按包文案须与求解一致");
        assert!(!base.join("huzi.lock").exists(), "冲突不得建锁");
        assert!(!base.join("vendor").exists(), "冲突不得建 vendor");
    }
}
