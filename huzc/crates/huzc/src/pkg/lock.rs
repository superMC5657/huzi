//! `huzi.lock` 读写:fetch 求解结果的精确版本快照(手写解析,无三方依赖)。
//!
//! 文件位置:工程根 `huzi.lock`(与 `huzi.toml` 同目录),fetch 生成,
//! build 校验(与清单约束/求解结果不一致直接报错,不做自动升级)。
//!
//! M1 扩展:`LockEntry` 新增 `source`/`checksum`/`registry` 可选字段,
//! 旧锁缺字段解析为 `None`(兼容),新锁按 `name,version,source,checksum,registry`
//! 键序写出;`checksum` 非空须为 `sha256:<64位小写hex>` 否则解析报错。

use super::solve::SelectedDep;
use super::version::SemVersion;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

/// 锁定条目:包名 + 精确版本 + 来源/校验和(可选,M1 新增)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockEntry {
    pub name: String,
    pub version: SemVersion,
    pub source: Option<String>,
    pub checksum: Option<String>,
    pub registry: Option<String>,
}

/// 锁文件:按包名排序的精确版本表。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HuziLock {
    pub packages: Vec<LockEntry>,
}

impl HuziLock {
    /// 由求解闭包生成(按包名排序,精确记录)。
    ///
    /// M1 回填:`source` 由 `SelectedDep::source_kind` 映射;
    /// `checksum`/`registry` 保持 `None`(点亮留给 M2)。
    pub fn from_closure(closure: &BTreeMap<String, SelectedDep>) -> Self {
        let mut packages: Vec<LockEntry> = closure
            .iter()
            .map(|(name, sel)| LockEntry {
                name: name.clone(),
                version: sel.version,
                source: Some(sel.source_kind.as_str().to_string()),
                checksum: None,
                registry: None,
            })
            .collect();
        packages.sort_by(|a, b| a.name.cmp(&b.name));
        Self { packages }
    }

    /// 查包的锁定版本。
    pub fn get(&self, name: &str) -> Option<SemVersion> {
        self.packages.iter().find(|e| e.name == name).map(|e| e.version)
    }

    /// 查包的整条锁定条目(M1 新增)。
    pub fn get_entry(&self, name: &str) -> Option<&LockEntry> {
        self.packages.iter().find(|e| e.name == name)
    }
}

/// 解析锁文件文本(`[[package]]` 节,name/version/source/checksum/registry 键)。
pub fn parse_lock(content: &str) -> Result<HuziLock, String> {
    let mut packages: Vec<LockEntry> = Vec::new();
    let mut cur_name: Option<String> = None;
    let mut cur_version: Option<String> = None;
    let mut cur_source: Option<String> = None;
    let mut cur_checksum: Option<String> = None;
    let mut cur_registry: Option<String> = None;
    let mut in_pkg = false;
    for (idx, raw_line) in content.lines().enumerate() {
        let line_no = idx + 1;
        let line = raw_line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if line == "[[package]]" {
            flush_entry(
                &mut packages,
                cur_name.take(),
                cur_version.take(),
                cur_source.take(),
                cur_checksum.take(),
                cur_registry.take(),
                line_no,
            )?;
            in_pkg = true;
            continue;
        }
        if !in_pkg {
            return Err(format!("锁文件第 {} 行非法: 期望 `[[package]]`,实际 {:?}", line_no, raw_line.trim()));
        }
        let Some((key, val)) = line.split_once('=') else {
            return Err(format!("锁文件第 {} 行非法: 缺 `=`", line_no));
        };
        match key.trim() {
            "name" => cur_name = Some(trim_quotes(val)),
            "version" => cur_version = Some(trim_quotes(val)),
            "source" => cur_source = Some(trim_quotes(val)),
            "checksum" => cur_checksum = Some(trim_quotes(val)),
            "registry" => cur_registry = Some(trim_quotes(val)),
            _ => {}
        }
    }
    flush_entry(
        &mut packages,
        cur_name,
        cur_version,
        cur_source,
        cur_checksum,
        cur_registry,
        content.lines().count() + 1,
    )?;
    check_duplicate(&packages)?;
    packages.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(HuziLock { packages })
}

/// 收拢一个 `[[package]]` 节(缺字段直接报错)。
fn flush_entry(
    out: &mut Vec<LockEntry>,
    name: Option<String>,
    version: Option<String>,
    source: Option<String>,
    checksum: Option<String>,
    registry: Option<String>,
    line_no: usize,
) -> Result<(), String> {
    let source = normalize_optional(source);
    let registry = normalize_optional(registry);
    let checksum = normalize_optional(checksum);
    match (name, version) {
        (None, None) => {
            if source.is_some() || checksum.is_some() || registry.is_some() {
                return Err(format!("锁文件第 {} 行: [[package]] 缺少 name/version", line_no));
            }
            Ok(())
        }
        (Some(n), Some(v)) => {
            let ver = SemVersion::parse(&v)
                .map_err(|e| format!("锁文件第 {} 行版本非法: {}", line_no, e))?;
            if let Some(ref c) = checksum {
                if !is_valid_checksum(c) {
                    return Err(format!("锁文件第 {} 行 checksum 非法: {:?}", line_no, c));
                }
            }
            out.push(LockEntry {
                name: n,
                version: ver,
                source,
                checksum,
                registry,
            });
            Ok(())
        }
        _ => Err(format!("锁文件第 {} 行: [[package]] 缺少 name/version", line_no)),
    }
}

/// 空串视为缺省(`None`),其余原样保留。
fn normalize_optional(raw: Option<String>) -> Option<String> {
    match raw {
        None => None,
        Some(s) if s.trim().is_empty() => None,
        Some(s) => Some(s),
    }
}

/// 校验 `sha256:<64位小写hex>`(无三方正则,手写判定)。
fn is_valid_checksum(s: &str) -> bool {
    let Some(hex) = s.strip_prefix("sha256:") else {
        return false;
    };
    if hex.len() != 64 {
        return false;
    }
    hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// 同包多条直接报错(沿用直接报错口径)。
fn check_duplicate(packages: &[LockEntry]) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for e in packages {
        if !seen.insert(e.name.clone()) {
            return Err(format!("锁文件包 '{}' 重复", e.name));
        }
    }
    Ok(())
}

/// 格式化锁文件(包名排序,确定性输出)。
///
/// 键序固定 `name,version,source,checksum,registry`,有值才写。
pub fn format_lock(lock: &HuziLock) -> String {
    let mut out = String::from("# huzi.lock 由 `huzc fetch` 生成,请勿手动编辑。\n");
    let mut entries = lock.packages.clone();
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    for e in &entries {
        out.push_str("[[package]]\n");
        out.push_str(&format!("name = \"{}\"\n", e.name));
        out.push_str(&format!("version = \"{}\"\n", e.version));
        if let Some(ref s) = e.source {
            out.push_str(&format!("source = \"{}\"\n", s));
        }
        if let Some(ref c) = e.checksum {
            out.push_str(&format!("checksum = \"{}\"\n", c));
        }
        if let Some(ref r) = e.registry {
            out.push_str(&format!("registry = \"{}\"\n", r));
        }
    }
    out
}

/// 读锁文件:不存在返回 Ok(None),内容非法返回 Err。
pub fn read_lock_file(path: &Path) -> Result<Option<HuziLock>, String> {
    if !path.is_file() {
        return Ok(None);
    }
    let content = fs::read_to_string(path)
        .map_err(|e| format!("读取 {} 失败: {}", path.display(), e))?;
    parse_lock(&content).map(Some)
}

/// 写锁文件(覆盖)。
pub fn write_lock_file(path: &Path, lock: &HuziLock) -> Result<(), String> {
    fs::write(path, format_lock(lock))
        .map_err(|e| format!("写入 {} 失败: {}", path.display(), e))
}

/// 清单同目录的锁文件路径。
pub fn lock_path_for(proj_dir: &Path) -> PathBuf {
    proj_dir.join("huzi.lock")
}

fn trim_quotes(s: &str) -> String {
    s.trim().trim_matches('"').trim_matches('\'').to_string()
}

#[cfg(test)]
mod tests {
    use super::super::solve::SourceKind;
    use super::*;

    fn sample_closure() -> BTreeMap<String, SelectedDep> {
        let mut m = BTreeMap::new();
        m.insert("shared".to_string(), SelectedDep {
            version: SemVersion::parse("2.0.0").unwrap(),
            source_dir: PathBuf::from("vendor/shared/2.0.0"),
            source_kind: SourceKind::Local,
        });
        m.insert("my_math".to_string(), SelectedDep {
            version: SemVersion::parse("1.2.0").unwrap(),
            source_dir: PathBuf::from("vendor/my_math/1.2.0"),
            source_kind: SourceKind::Path,
        });
        m
    }

    #[test]
    fn test_lock_roundtrip_sorted() {
        let lock = HuziLock::from_closure(&sample_closure());
        assert_eq!(lock.packages[0].name, "my_math");
        assert_eq!(lock.packages[1].name, "shared");
        let text = format_lock(&lock);
        assert!(text.contains("name = \"my_math\""));
        assert!(text.contains("version = \"1.2.0\""));
        let back = parse_lock(&text).expect("锁文件应解析成功");
        assert_eq!(back, lock);
        assert_eq!(back.get("shared").unwrap().to_string(), "2.0.0");
        assert!(back.get("missing").is_none());
    }

    #[test]
    fn test_lock_old_file_yields_none() {
        let old = "[[package]]\nname = \"my_math\"\nversion = \"1.0.0\"\n";
        let lock = parse_lock(old).expect("旧锁应解析成功");
        let entry = lock.get_entry("my_math").expect("应命中整条");
        assert_eq!(entry.version.to_string(), "1.0.0");
        assert!(entry.source.is_none(), "旧锁 source 应为 None");
        assert!(entry.checksum.is_none(), "旧锁 checksum 应为 None");
        assert!(entry.registry.is_none(), "旧锁 registry 应为 None");
        assert!(lock.get_entry("missing").is_none());
    }

    #[test]
    fn test_lock_new_roundtrip_with_all_fields() {
        let hex = "a".repeat(64);
        let checksum = format!("sha256:{}", hex);
        let lock = HuziLock {
            packages: vec![LockEntry {
                name: "my_math".to_string(),
                version: SemVersion::parse("1.2.0").unwrap(),
                source: Some("path".to_string()),
                checksum: Some(checksum.clone()),
                registry: None,
            }],
        };
        let text = format_lock(&lock);
        assert_key_order(&text);
        let back = parse_lock(&text).expect("新锁应往返成功");
        assert_eq!(back, lock);
        assert_eq!(back.get_entry("my_math").unwrap().checksum.as_deref(), Some(checksum.as_str()));
    }

    #[test]
    fn test_lock_rejects_bad_checksum() {
        for bad in [
            "sha256:xyz",
            "md5:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "sha256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
            "sha256:aaaa",
            "",
        ] {
            // 空串视为缺省,不报错;其余非法须拒绝
            if bad.is_empty() {
                continue;
            }
            let text = format!("[[package]]\nname = \"x\"\nversion = \"1.0.0\"\nchecksum = \"{}\"\n", bad);
            assert!(parse_lock(&text).is_err(), "非法 checksum 应拒绝: {}", bad);
        }
        // 非空非法(含大写/长度不足)整体拒绝
        let dup_bad = "[[package]]\nname = \"x\"\nversion = \"1.0.0\"\nchecksum = \"sha256:ABC\"\n";
        assert!(parse_lock(dup_bad).is_err());
        // 合法 64 位小写 hex 通过
        let good = format!("[[package]]\nname = \"x\"\nversion = \"1.0.0\"\nchecksum = \"sha256:{}\"\n", "b".repeat(64));
        assert!(parse_lock(&good).is_ok());
        // 未知键保持忽略
        let unknown = "[[package]]\nname = \"x\"\nversion = \"1.0.0\"\nfoo = \"bar\"\n";
        assert!(parse_lock(unknown).is_ok());
    }

    #[test]
    fn test_lock_rejects_bad_input() {
        assert!(parse_lock("name = \"x\"").is_err());
        let missing_ver = "[[package]]\nname = \"x\"\n";
        assert!(parse_lock(missing_ver).is_err());
        let bad_ver = "[[package]]\nname = \"x\"\nversion = \"abc\"\n";
        assert!(parse_lock(bad_ver).is_err());
        let dup = "[[package]]\nname = \"x\"\nversion = \"1.0.0\"\n[[package]]\nname = \"x\"\nversion = \"1.0.0\"\n";
        assert!(parse_lock(dup).is_err());
        assert!(parse_lock("").is_ok());
    }

    /// 断言新锁键序 name,version,source,checksum,registry。
    fn assert_key_order(text: &str) {
        let name = text.find("name =").expect("应含 name");
        let version = text.find("version =").expect("应含 version");
        assert!(name < version, "name 应在 version 之前");
        if let (Some(s), Some(c)) = (text.find("source ="), text.find("checksum =")) {
            assert!(s < c, "source 应在 checksum 之前");
            assert!(version < s, "version 应在 source 之前");
        }
    }
}
