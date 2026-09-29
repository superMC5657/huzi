//! 传递闭包求解:合并根清单及传递依赖的全部版本约束,逐包选最高满足版本.
//!
//! 源优先级(M4):显式`path` > `registry=`声明 > `HUZI_REGISTRY` > 本地(vendor/缓存);
//! 同名跨源`path`胜(有可行path只看path);`Registry`经索引选版,落盘由fetch完成;
//! `std/core/alloc`命中注册表直接拒绝(走内置);`--offline`下读索引直接错.
use super::manifest::{Manifest, parse_manifest};
use super::source::{default_registry, is_builtin_pkg, read_index, vendor_dir_for};
use super::version::{SemVersion, VersionReq, parse_version_req};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
/// 传递闭包中单个包的选中结果:精确版本+来源源码目录+来源种类.
#[derive(Debug, Clone, PartialEq)]
pub struct SelectedDep {
    pub version: SemVersion,
    pub source_dir: PathBuf,
    pub source_kind: SourceKind,
}
/// 依赖来源种类:显式`path`/本地缓存(`vendor/`+全局缓存)/注册表索引.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SourceKind {
    Path,
    Local,
    Registry,
}
impl SourceKind {
    /// 锁文件`source`字段串值(`path`/`local`/`registry`).
    pub fn as_str(&self) -> &'static str {
        match self {
            SourceKind::Path => "path",
            SourceKind::Local => "local",
            SourceKind::Registry => "registry",
        }
    }
}
/// 求解传递闭包(联网默认;`--offline`/build请用`resolve_closure_with`).
pub fn resolve_closure(root: &Manifest, root_dir: &Path) -> Result<BTreeMap<String, SelectedDep>, String> {
    resolve_closure_with(root, root_dir, false)
}
/// 带离线开关求解:`offline=true`时读索引直接错,仅用path/本地.
pub fn resolve_closure_with(root: &Manifest, root_dir: &Path, offline: bool) -> Result<BTreeMap<String, SelectedDep>, String> {
    let mut solver = ClosureSolver::new(root_dir, offline);
    solver.seed_root(root)?;
    solver.run()?;
    Ok(solver.selected)
}
/// 传递闭包求解器:约束/来源单调累积,选中版本变化时才展开传递依赖.
struct ClosureSolver {
    root_dir: PathBuf,
    offline: bool,
    constraints: HashMap<String, Vec<String>>,
    path_sources: HashMap<String, Vec<PathBuf>>,
    registries: HashMap<String, Vec<String>>,
    needs_registry: HashSet<String>,
    selected: BTreeMap<String, SelectedDep>,
    expanded: BTreeSet<(String, SemVersion)>,
    queue: Vec<QueueItem>,
}
/// 队列项:一条版本约束原串及其声明位置(传递`path`相对声明方解析).
struct QueueItem {
    name: String,
    raw: String,
    declarer_dir: PathBuf,
    path: Option<String>,
    registry: Option<String>,
}
impl ClosureSolver {
    /// 以工程根目录为基准新建求解器.
    fn new(root_dir: &Path, offline: bool) -> Self {
        Self {
            root_dir: root_dir.to_path_buf(),
            offline,
            constraints: HashMap::new(),
            path_sources: HashMap::new(),
            registries: HashMap::new(),
            needs_registry: HashSet::new(),
            selected: BTreeMap::new(),
            expanded: BTreeSet::new(),
            queue: Vec::new(),
        }
    }
    /// 把根清单的直接依赖入队(非法版本需求在此直接报错).
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
                registry: dep.registry.clone(),
            });
        }
        Ok(())
    }
    /// 排空队列即得不动点(约束/来源只增不减,必终止).
    fn run(&mut self) -> Result<(), String> {
        while let Some(item) = self.queue.pop() {
            self.apply_requirement(item)?;
        }
        Ok(())
    }
    /// 累积一条约束并重选该包版本(path声明忽略同条registry).
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
            if let Some(r) = &item.registry {
                let regs = self.registries.entry(item.name.clone()).or_default();
                if !regs.contains(r) {
                    regs.push(r.clone());
                }
            }
            self.needs_registry.insert(item.name.clone());
        }
        self.reselect(&item.name)
    }
    /// 按优先级重选:有可行path只看path,否则registry(声明>环境),否则本地.
    fn reselect(&mut self, name: &str) -> Result<(), String> {
        let raws = self.constraints.get(name).cloned().unwrap_or_default();
        let reqs = parse_all_reqs(name, &raws)?;
        if is_builtin_pkg(name) && self.needs_registry.contains(name) {
            return Err(format!("包 '{}' 为内置标准库(std/core/alloc),请直接 import 无需在 [dependencies] 声明", name));
        }
        if self.has_path(name) {
            return self.reselect_path(name, &raws, &reqs);
        }
        if self.needs_registry.contains(name) {
            match self.reselect_registry(name, &reqs)? {
                Some(()) => return Ok(()),
                None => {}
            }
        }
        self.reselect_local(name, &raws, &reqs)
    }
    /// 是否有显式path来源.
    fn has_path(&self, name: &str) -> bool {
        self.path_sources.get(name).map(|v| !v.is_empty()).unwrap_or(false)
    }
    /// path胜:只看path候选,可行取最高,否则冲突(不回退registry).
    fn reselect_path(&mut self, name: &str, raws: &[String], reqs: &[VersionReq]) -> Result<(), String> {
        let mut cands = Vec::new();
        if let Some(srcs) = self.path_sources.get(name).cloned() {
            for dir in &srcs {
                cands.extend(candidates_from_path_dir(dir, raws).into_iter().map(|(v, p)| (v, p, SourceKind::Path)));
            }
        }
        let feasible = feasible_max(&cands, reqs);
        match feasible {
            Some((ver, src, kind)) => self.commit(name, ver, src, kind),
            None => Err(conflict_message(name, raws, &self.all_known(name, raws))),
        }
    }
    /// registry:声明源逐个试(错直接抛),环境源缺索引回退本地,预发布/offline硬错.
    /// offline且本地已有可行版则直接回退本地(供build离线重放);本地无可行才报offline错.
    fn reselect_registry(&mut self, name: &str, reqs: &[VersionReq]) -> Result<Option<()>, String> {
        if self.offline && has_local_feasible(name, &self.root_dir, reqs) {
            return Ok(None);
        }
        let decls = self.registries.get(name).cloned().unwrap_or_default();
        for reg in &decls {
            let entries = read_index(reg, name, self.offline)?;
            let cands = index_cands(&entries, &self.root_dir, name, reqs);
            if let Some((ver, src, kind)) = cands.last().cloned() {
                self.commit(name, ver, src, kind)?;
                return Ok(Some(()));
            }
        }
        if let Some(env) = default_registry() {
            if !decls.contains(&env) {
                match read_index(&env, name, self.offline) {
                    Ok(entries) => {
                        let cands = index_cands(&entries, &self.root_dir, name, reqs);
                        if let Some((ver, src, kind)) = cands.last().cloned() {
                            self.commit(name, ver, src, kind)?;
                            return Ok(Some(()));
                        }
                    }
                    Err(e) if is_env_fallback(&e) => {}
                    Err(e) => return Err(e),
                }
            }
        }
        Ok(None)
    }
    /// 本地兜底:全局缓存+vendor最高满足版.
    fn reselect_local(&mut self, name: &str, raws: &[String], reqs: &[VersionReq]) -> Result<(), String> {
        let cands: Vec<(SemVersion, PathBuf, SourceKind)> =
            candidates_from_registry(name, &self.root_dir).into_iter().map(|(v, p)| (v, p, SourceKind::Local)).collect();
        match feasible_max(&cands, reqs) {
            Some((ver, src, kind)) => self.commit(name, ver, src, kind),
            None => Err(conflict_message(name, raws, &self.all_known(name, raws))),
        }
    }
    /// 提交选中版本,变化时展开传递依赖.
    fn commit(&mut self, name: &str, ver: SemVersion, src: PathBuf, kind: SourceKind) -> Result<(), String> {
        let changed = self.selected.get(name).map(|s| s.version) != Some(ver);
        if changed {
            self.selected.insert(name.to_string(), SelectedDep { version: ver, source_dir: src.clone(), source_kind: kind });
            self.expand_transitive(name, ver, &src)?;
        }
        Ok(())
    }
    /// 全已知候选(报错用):path+registry(声明+环境,错忽略)+本地.
    fn all_known(&self, name: &str, raws: &[String]) -> Vec<(SemVersion, PathBuf, SourceKind)> {
        let mut out = Vec::new();
        if let Some(srcs) = self.path_sources.get(name) {
            for dir in srcs {
                out.extend(candidates_from_path_dir(dir, raws).into_iter().map(|(v, p)| (v, p, SourceKind::Path)));
            }
        }
        for reg in self.registries.get(name).cloned().unwrap_or_default() {
            if let Ok(entries) = read_index(&reg, name, true) {
                out.extend(entries.into_iter().filter_map(|e| SemVersion::parse(&e.version.to_string()).ok()).map(|v| (v, PathBuf::new(), SourceKind::Registry)));
            }
        }
        out.extend(candidates_from_registry(name, &self.root_dir).into_iter().map(|(v, p)| (v, p, SourceKind::Local)));
        out
    }
    /// 展开选中版本的传递依赖(同包同版本只展开一次,环依赖截断).
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
                registry: dep.registry.clone(),
            });
        }
        Ok(())
    }
}
/// 可行候选中取最高版本.
fn feasible_max(cands: &[(SemVersion, PathBuf, SourceKind)], reqs: &[VersionReq]) -> Option<(SemVersion, PathBuf, SourceKind)> {
    let mut feasible: Vec<(SemVersion, PathBuf, SourceKind)> =
        cands.iter().cloned().filter(|(v, _, _)| reqs.iter().all(|r| r.matches(v))).collect();
    feasible.sort();
    feasible.dedup();
    feasible.last().cloned()
}
/// 索引条目按约束过滤为候选(source_dir为预落盘vendor路径).
fn index_cands(entries: &[super::source::IndexEntry], root_dir: &Path, pkg: &str, reqs: &[VersionReq]) -> Vec<(SemVersion, PathBuf, SourceKind)> {
    let vendor_root = root_dir.join("vendor");
    let mut out: Vec<(SemVersion, PathBuf, SourceKind)> = entries
        .iter()
        .filter(|e| reqs.iter().all(|r| r.matches(&e.version)))
        .map(|e| (e.version, vendor_dir_for(&vendor_root, pkg, &e.version), SourceKind::Registry))
        .collect();
    out.sort();
    out.dedup();
    out
}
/// 本地是否有可行版(供offline回退判定).
fn has_local_feasible(name: &str, root_dir: &Path, reqs: &[VersionReq]) -> bool {
    candidates_from_registry(name, root_dir)
        .into_iter()
        .any(|(v, _)| reqs.iter().all(|r| r.matches(&v)))
}
/// 环境源读失败仅缺索引回退本地;offline/预发布/非法硬错.
fn is_env_fallback(e: &str) -> bool {
    if e.contains("--offline") || e.contains("暂不支持预发布") {
        return false;
    }
    if e.contains("索引版本非法") || e.contains("索引sha256非法") || e.contains("索引条目缺") || e.contains("索引行为非法") || e.contains("索引行非法") {
        return false;
    }
    e.contains("读索引") || e.contains("curl")
}
/// 逐条解析约束原串(报错带包名上下文).
fn parse_all_reqs(name: &str, raws: &[String]) -> Result<Vec<VersionReq>, String> {
    raws.iter()
        .map(|raw| {
            parse_version_req(raw)
                .map_err(|e| format!("依赖 '{}' 的版本需求 {:?} 非法: {}", name, raw, e))
        })
        .collect()
}
/// `path`源目录候选:版本子目录优先,其次目录`huzi.toml`包版本,最后精确原串兼容.
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
/// 全局缓存与工程`vendor/`下该包的版本子目录候选(含/嵌套).
fn candidates_from_registry(name: &str, root_dir: &Path) -> Vec<(SemVersion, PathBuf)> {
    let mut out = Vec::new();
    if let Some(dir) = global_packages_dir(name) {
        out.extend(version_subdirs(&dir));
    }
    out.extend(version_subdirs(&root_dir.join("vendor").join(name)));
    out
}
/// 全局缓存中该包目录(`~/.huzi/packages/<pkg>`,Windows优先`USERPROFILE`).
fn global_packages_dir(name: &str) -> Option<PathBuf> {
    let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).ok()?;
    Some(PathBuf::from(home).join(".huzi").join("packages").join(name))
}
/// 目录下可解析为语义版本的子目录(`(版本,子目录)`未排序).
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
/// 目录`huzi.toml`中`[package]version`解析出的包版本(无清单/非法则无).
fn package_version_of(dir: &Path) -> Option<SemVersion> {
    let content = fs::read_to_string(dir.join("huzi.toml")).ok()?;
    let manifest = parse_manifest(&content).ok()?;
    SemVersion::parse(&manifest.version).ok()
}
/// 冲突报错文本:点名包、全部约束与已知可用版本(沿用直接报错口径).
fn conflict_message(name: &str, raws: &[String], known: &[(SemVersion, PathBuf, SourceKind)]) -> String {
    let mut vers: Vec<SemVersion> = known.iter().map(|(v, _, _)| *v).collect();
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
        let (manifest, dir) = pkg_manifest("app_multi");
        let closure = resolve_closure(&manifest, &dir).expect("多版本应求解成功");
        let sel = closure.get("my_math").expect("应选中 my_math");
        assert_eq!(sel.version.to_string(), "1.2.0");
        assert!(sel.source_dir.ends_with(Path::new("1.2.0")), "来源应为 1.2.0 子目录,实际: {}", sel.source_dir.display());
        assert_eq!(sel.source_kind, SourceKind::Path, "显式 path 应记 Path");
    }
    #[test]
    fn test_closure_diamond_conflict_neg() {
        let (manifest, dir) = pkg_manifest("app_diamond");
        let err = resolve_closure(&manifest, &dir).expect_err("菱形冲突应求解失败");
        assert!(err.contains("依赖冲突"), "报错应点明依赖冲突,实际: {}", err);
        assert!(err.contains("shared"), "报错应点名 shared 包,实际: {}", err);
    }
    #[test]
    fn test_closure_legacy_single_path() {
        let (manifest, dir) = pkg_manifest("app");
        let closure = resolve_closure(&manifest, &dir).expect("旧夹具应求解成功");
        let sel = closure.get("my_math").expect("应选中 my_math");
        assert_eq!(sel.version.to_string(), "1.0.0");
        assert_eq!(sel.source_kind, SourceKind::Path, "显式 path 应记 Path");
    }
    #[test]
    fn test_source_kind_str() {
        assert_eq!(SourceKind::Registry.as_str(), "registry");
        assert_eq!(SourceKind::Path.as_str(), "path");
        assert_eq!(SourceKind::Local.as_str(), "local");
    }
    #[test]
    fn test_builtin_registry_rejected() {
        let m = parse_manifest("[package]\nname=\"a\"\nversion=\"0.1.0\"\n\n[dependencies]\nstd = { version = \"1.0.0\", registry = \"file:///tmp/reg\" }\n").expect("清单可解析");
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../test/pkg/app");
        let err = resolve_closure(&m, &dir).expect_err("std走注册表应拒绝");
        assert!(err.contains("内置"), "应提示走内置,实际: {}", err);
    }
    #[test]
    fn test_offline_registry_errors() {
        let m = parse_manifest("[package]\nname=\"a\"\nversion=\"0.1.0\"\n\n[dependencies]\nno_such_pkg_xyz = { version = \"1.0.0\", registry = \"file:///tmp/reg\" }\n").expect("清单可解析");
        let dir = std::env::temp_dir().join(format!("huzc_offline_{}_empty", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("空目录可建");
        let err = resolve_closure_with(&m, &dir, true).expect_err("--offline应直接错");
        assert!(err.contains("--offline") || err.contains("offline"), "应点明offline,实际: {}", err);
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    fn test_path_beats_registry() {
        let root = std::env::temp_dir().join(format!("huzc_prio_{}_path", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("pathpkg/1.0.0")).expect("建path包");
        std::fs::write(root.join("pathpkg/1.0.0/huzi.toml"), "[package]\nname=\"my_math\"\nversion=\"1.0.0\"\n").expect("写清单");
        std::fs::write(root.join("pathpkg/1.0.0/lib.hz"), "export x = 1\n").expect("写源码");
        let reg = root.join("reg/my_math");
        std::fs::create_dir_all(&reg).expect("建索引");
        std::fs::write(reg.join("index.toml"), "[[package]]\nversion=\"9.9.9\"\nsha256=\"0000000000000000000000000000000000000000000000000000000000000000\"\ntarball=\"file:///nonexistent.tgz\"\n").expect("写索引");
        let reg_url = format!("file://{}", root.join("reg").display());
        let toml = format!("[package]\nname=\"a\"\nversion=\"0.1.0\"\n\n[dependencies]\nmy_math = {{ version = \"*\", path = \"./pathpkg\", registry = \"{}\" }}\n", reg_url);
        let m = parse_manifest(&toml).expect("清单可解析");
        let closure = resolve_closure(&m, &root).expect("path胜应成功");
        let sel = closure.get("my_math").expect("应选中");
        assert_eq!(sel.source_kind, SourceKind::Path, "同名跨源path胜,实际:{:?}", sel.source_kind);
        assert_eq!(sel.version.to_string(), "1.0.0");
        let _ = std::fs::remove_dir_all(&root);
    }
}
