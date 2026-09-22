//! 传递闭包求解:合并根清单及传递依赖的全部版本约束,逐包选最高满足版本。
//!
//! 候选来源见 `candidates_from_path_dir` / `candidates_from_registry`;
//! 同一包多约束无共同满足版本时返回冲突错误,调用方(fetch/build)直接报错退出;
//! 环依赖按“同包同版本只展开一次”截断,不会无限循环。

use super::manifest::{Manifest, parse_manifest};
use super::version::{SemVersion, VersionReq, parse_version_req};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

/// 传递闭包中单个包的选中结果:精确版本 + 来源源码目录。
#[derive(Debug, Clone, PartialEq)]
pub struct SelectedDep {
    pub version: SemVersion,
    pub source_dir: PathBuf,
}

/// 求解传递闭包:合并根清单及传递依赖的全部版本约束,逐包选最高满足版本。
///
/// 候选来源见 `candidates_from_path_dir` / `candidates_from_registry`;
/// 同一包多约束无共同满足版本时返回冲突错误,调用方(fetch/build)直接报错退出;
/// 环依赖按“同包同版本只展开一次”截断,不会无限循环。
pub fn resolve_closure(root: &Manifest, root_dir: &Path) -> Result<BTreeMap<String, SelectedDep>, String> {
    let mut solver = ClosureSolver::new(root_dir);
    solver.seed_root(root)?;
    solver.run()?;
    Ok(solver.selected)
}

/// 传递闭包求解器:约束/来源单调累积,选中版本变化时才展开传递依赖。
struct ClosureSolver {
    root_dir: PathBuf,
    constraints: HashMap<String, Vec<String>>,
    path_sources: HashMap<String, Vec<PathBuf>>,
    needs_registry: HashSet<String>,
    selected: BTreeMap<String, SelectedDep>,
    expanded: BTreeSet<(String, SemVersion)>,
    queue: Vec<QueueItem>,
}

/// 队列项:一条版本约束原串及其声明位置(传递依赖的相对 `path` 相对声明方解析)。
struct QueueItem {
    name: String,
    raw: String,
    declarer_dir: PathBuf,
    path: Option<String>,
}

impl ClosureSolver {
    /// 以工程根目录为基准新建求解器。
    fn new(root_dir: &Path) -> Self {
        Self {
            root_dir: root_dir.to_path_buf(),
            constraints: HashMap::new(),
            path_sources: HashMap::new(),
            needs_registry: HashSet::new(),
            selected: BTreeMap::new(),
            expanded: BTreeSet::new(),
            queue: Vec::new(),
        }
    }

    /// 把根清单的直接依赖入队(非法版本需求在此直接报错)。
    fn seed_root(&mut self, root: &Manifest) -> Result<(), String> {
        let mut names: Vec<&String> = root.dependencies.keys().collect();
        names.sort();
        for name in names {
            let dep = &root.dependencies[name];
            parse_version_req(&dep.version).map_err(|e| {
                format!("依赖 '{}' 的版本需求 {:?} 非法: {}", name, dep.version, e)
            })?;
            self.queue.push(QueueItem {
                name: name.clone(),
                raw: dep.version.clone(),
                declarer_dir: self.root_dir.clone(),
                path: dep.path.clone(),
            });
        }
        Ok(())
    }

    /// 排空队列即得不动点(约束/来源只增不减,必终止)。
    fn run(&mut self) -> Result<(), String> {
        while let Some(item) = self.queue.pop() {
            self.apply_requirement(item)?;
        }
        Ok(())
    }

    /// 累积一条约束并重选该包版本。
    fn apply_requirement(&mut self, item: QueueItem) -> Result<(), String> {
        let entry = self.constraints.entry(item.name.clone()).or_default();
        if !entry.contains(&item.raw) {
            entry.push(item.raw.clone());
        }
        if let Some(p) = &item.path {
            let dir = item.declarer_dir.join(p);
            let srcs = self.path_sources.entry(item.name.clone()).or_default();
            if !srcs.contains(&dir) {
                srcs.push(dir);
            }
        } else {
            self.needs_registry.insert(item.name.clone());
        }
        self.reselect(&item.name)
    }

    /// 按该包全部约束重选最高满足版本,版本变化时展开其传递依赖。
    fn reselect(&mut self, name: &str) -> Result<(), String> {
        let raws = self.constraints.get(name).cloned().unwrap_or_default();
        let reqs = parse_all_reqs(name, &raws)?;
        let mut cands = self.collect_candidates(name, &raws);
        cands.sort();
        cands.dedup();
        let feasible: Vec<(SemVersion, PathBuf)> = cands
            .into_iter()
            .filter(|(v, _)| reqs.iter().all(|r| r.matches(v)))
            .collect();
        let Some((ver, src)) = feasible.last().cloned() else {
            return Err(conflict_message(name, &raws, &self.collect_candidates(name, &raws)));
        };
        let changed = self.selected.get(name).map(|s| s.version) != Some(ver);
        if changed {
            self.selected.insert(
                name.to_string(),
                SelectedDep { version: ver, source_dir: src.clone() },
            );
            self.expand_transitive(name, ver, &src)?;
        }
        Ok(())
    }

    /// 收集该包全部候选(版本,来源):`path` 源目录 + 按需全局缓存/`vendor/`。
    fn collect_candidates(&self, name: &str, raws: &[String]) -> Vec<(SemVersion, PathBuf)> {
        let mut out = Vec::new();
        if let Some(srcs) = self.path_sources.get(name) {
            for dir in srcs {
                out.extend(candidates_from_path_dir(dir, raws));
            }
        }
        if self.needs_registry.contains(name) {
            out.extend(candidates_from_registry(name, &self.root_dir));
        }
        out
    }

    /// 展开选中版本的传递依赖(同包同版本只展开一次,环依赖截断)。
    fn expand_transitive(&mut self, name: &str, ver: SemVersion, src: &Path) -> Result<(), String> {
        if !self.expanded.insert((name.to_string(), ver)) {
            return Ok(());
        }
        let toml_path = src.join("huzi.toml");
        if !toml_path.is_file() {
            return Ok(());
        }
        let content = fs::read_to_string(&toml_path)
            .map_err(|e| format!("读取 {} 失败: {}", toml_path.display(), e))?;
        let manifest = parse_manifest(&content)
            .map_err(|e| format!("解析 {} 失败: {}", toml_path.display(), e))?;
        let mut dep_names: Vec<&String> = manifest.dependencies.keys().collect();
        dep_names.sort();
        for dep_name in dep_names {
            let dep = &manifest.dependencies[dep_name];
            parse_version_req(&dep.version).map_err(|e| {
                format!("依赖 '{}' 的版本需求 {:?} 非法: {}", dep_name, dep.version, e)
            })?;
            self.queue.push(QueueItem {
                name: dep_name.clone(),
                raw: dep.version.clone(),
                declarer_dir: src.to_path_buf(),
                path: dep.path.clone(),
            });
        }
        Ok(())
    }
}

/// 逐条解析约束原串(报错带包名上下文)。
fn parse_all_reqs(name: &str, raws: &[String]) -> Result<Vec<VersionReq>, String> {
    raws.iter()
        .map(|raw| {
            parse_version_req(raw)
                .map_err(|e| format!("依赖 '{}' 的版本需求 {:?} 非法: {}", name, raw, e))
        })
        .collect()
}

/// `path` 源目录的候选:版本子目录优先,其次目录 `huzi.toml` 包版本,
/// 最后回退“精确原串声明 + 单目录源码”(兼容旧单版本 `path` 夹具)。
fn candidates_from_path_dir(dir: &Path, raws: &[String]) -> Vec<(SemVersion, PathBuf)> {
    let mut out = version_subdirs(dir);
    if !out.is_empty() {
        return out;
    }
    if let Some(ver) = package_version_of(dir) {
        out.push((ver, dir.to_path_buf()));
        return out;
    }
    for raw in raws {
        if let Ok(ver) = SemVersion::parse(raw) {
            out.push((ver, dir.to_path_buf()));
        }
    }
    out
}

/// 全局缓存与工程 `vendor/` 下该包的版本子目录候选。
fn candidates_from_registry(name: &str, root_dir: &Path) -> Vec<(SemVersion, PathBuf)> {
    let mut out = Vec::new();
    if let Some(dir) = global_packages_dir(name) {
        out.extend(version_subdirs(&dir));
    }
    out.extend(version_subdirs(&root_dir.join("vendor").join(name)));
    out
}

/// 全局缓存中该包目录(`~/.huzi/packages/<pkg>`,Windows 优先 `USERPROFILE`)。
fn global_packages_dir(name: &str) -> Option<PathBuf> {
    let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).ok()?;
    Some(PathBuf::from(home).join(".huzi").join("packages").join(name))
}

/// 目录下可解析为语义版本的子目录(`(版本, 子目录)` 未排序)。
fn version_subdirs(dir: &Path) -> Vec<(SemVersion, PathBuf)> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .filter_map(|p| {
            let ver = p.file_name()?.to_str().and_then(|n| SemVersion::parse(n).ok())?;
            Some((ver, p))
        })
        .collect()
}

/// 目录 `huzi.toml` 中 `[package] version` 解析出的包版本(无清单/非法则无)。
fn package_version_of(dir: &Path) -> Option<SemVersion> {
    let content = fs::read_to_string(dir.join("huzi.toml")).ok()?;
    let manifest = parse_manifest(&content).ok()?;
    SemVersion::parse(&manifest.version).ok()
}

/// 冲突报错文本:点名包、全部约束与已知可用版本(沿用直接报错口径)。
fn conflict_message(name: &str, raws: &[String], known: &[(SemVersion, PathBuf)]) -> String {
    let mut vers: Vec<SemVersion> = known.iter().map(|(v, _)| *v).collect();
    vers.sort();
    vers.dedup();
    let known_str = if vers.is_empty() {
        "无".to_string()
    } else {
        vers.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(", ")
    };
    format!(
        "依赖冲突: 包 '{}' 的版本约束 [{}] 无共同满足版本(已知可用版本: {});请统一各 huzi.toml 中的版本需求后重试",
        name,
        raws.join(", "),
        known_str
    )
}

#[cfg(test)]
mod tests {
    use super::super::manifest::Manifest;
    use super::super::manifest::parse_manifest;
    use super::*;
    use std::path::Path;

    fn pkg_manifest(app: &str) -> (Manifest, PathBuf) {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../test/pkg").join(app);
        let content = std::fs::read_to_string(dir.join("huzi.toml")).expect("夹具 huzi.toml 可读");
        let manifest = parse_manifest(&content).expect("夹具 huzi.toml 可解析");
        (manifest, dir)
    }

    #[test]
    fn test_closure_multi_picks_highest() {
        // 1.0.0/1.2.0 + ^1.0.0 → 选 1.2.0
        let (manifest, dir) = pkg_manifest("app_multi");
        let closure = resolve_closure(&manifest, &dir).expect("多版本应求解成功");
        let sel = closure.get("my_math").expect("应选中 my_math");
        assert_eq!(sel.version.to_string(), "1.2.0");
        assert!(sel.source_dir.ends_with(Path::new("1.2.0")), "来源应为 1.2.0 子目录,实际: {}", sel.source_dir.display());
    }

    #[test]
    fn test_closure_diamond_conflict_neg() {
        // 菱形冲突:shared 被同时要求 1.0.0 与 2.0.0 → 直接报错
        let (manifest, dir) = pkg_manifest("app_diamond");
        let err = resolve_closure(&manifest, &dir).expect_err("菱形冲突应求解失败");
        assert!(err.contains("依赖冲突"), "报错应点明依赖冲突,实际: {}", err);
        assert!(err.contains("shared"), "报错应点名 shared 包,实际: {}", err);
    }

    #[test]
    fn test_closure_legacy_single_path() {
        // 旧单目录 path 夹具:精确 1.0.0 保持可用
        let (manifest, dir) = pkg_manifest("app");
        let closure = resolve_closure(&manifest, &dir).expect("旧夹具应求解成功");
        let sel = closure.get("my_math").expect("应选中 my_math");
        assert_eq!(sel.version.to_string(), "1.0.0");
    }
}
