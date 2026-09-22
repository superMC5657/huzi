//! `huzi.lock` 读写:fetch 求解结果的精确版本快照(手写解析,无三方依赖)。
//!
//! 文件位置:工程根 `huzi.lock`(与 `huzi.toml` 同目录),fetch 生成,
//! build 校验(与清单约束/求解结果不一致直接报错,不做自动升级)。

use super::solve::SelectedDep;
use super::version::SemVersion;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

/// 锁定条目:包名 + 精确版本。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockEntry {
    pub name: String,
    pub version: SemVersion,
}

/// 锁文件:按包名排序的精确版本表。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HuziLock {
    pub packages: Vec<LockEntry>,
}

impl HuziLock {
    /// 由求解闭包生成(按包名排序,精确记录)。
    pub fn from_closure(closure: &BTreeMap<String, SelectedDep>) -> Self {
        let mut packages: Vec<LockEntry> = closure
            .iter()
            .map(|(name, sel)| LockEntry { name: name.clone(), version: sel.version })
            .collect();
        packages.sort_by(|a, b| a.name.cmp(&b.name));
        Self { packages }
    }

    /// 查包的锁定版本。
    pub fn get(&self, name: &str) -> Option<SemVersion> {
        self.packages.iter().find(|e| e.name == name).map(|e| e.version)
    }
}

/// 解析锁文件文本(`[[package]]` 节,name/version 键)。
pub fn parse_lock(content: &str) -> Result<HuziLock, String> {
    let mut packages: Vec<LockEntry> = Vec::new();
    let mut cur_name: Option<String> = None;
    let mut cur_version: Option<String> = None;
    let mut in_pkg = false;
    for (idx, raw_line) in content.lines().enumerate() {
        let line_no = idx + 1;
        let line = raw_line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if line == "[[package]]" {
            flush_entry(&mut packages, cur_name.take(), cur_version.take(), line_no)?;
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
            _ => {}
        }
    }
    flush_entry(&mut packages, cur_name, cur_version, content.lines().count() + 1)?;
    check_duplicate(&packages)?;
    packages.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(HuziLock { packages })
}

/// 收拢一个 `[[package]]` 节(缺字段直接报错)。
fn flush_entry(out: &mut Vec<LockEntry>, name: Option<String>, version: Option<String>, line_no: usize) -> Result<(), String> {
    match (name, version) {
        (None, None) => Ok(()),
        (Some(n), Some(v)) => {
            let ver = SemVersion::parse(&v)
                .map_err(|e| format!("锁文件第 {} 行版本非法: {}", line_no, e))?;
            out.push(LockEntry { name: n, version: ver });
            Ok(())
        }
        _ => Err(format!("锁文件第 {} 行: [[package]] 缺少 name/version", line_no)),
    }
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
pub fn format_lock(lock: &HuziLock) -> String {
    let mut out = String::from("# huzi.lock 由 `huzc fetch` 生成,请勿手动编辑。
");
    let mut entries = lock.packages.clone();
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    for e in &entries {
        out.push_str("[[package]]
");
        out.push_str(&format!("name = \"{}\"
", e.name));
        out.push_str(&format!("version = \"{}\"
", e.version));
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
    use super::*;

    fn sample_closure() -> BTreeMap<String, SelectedDep> {
        let mut m = BTreeMap::new();
        m.insert("shared".to_string(), SelectedDep {
            version: SemVersion::parse("2.0.0").unwrap(),
            source_dir: PathBuf::from("vendor/shared/2.0.0"),
        });
        m.insert("my_math".to_string(), SelectedDep {
            version: SemVersion::parse("1.2.0").unwrap(),
            source_dir: PathBuf::from("vendor/my_math/1.2.0"),
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
    fn test_lock_rejects_bad_input() {
        assert!(parse_lock("name = \"x\"").is_err());
        let missing_ver = "[[package]]
name = \"x\"
";
        assert!(parse_lock(missing_ver).is_err());
        let bad_ver = "[[package]]
name = \"x\"
version = \"abc\"
";
        assert!(parse_lock(bad_ver).is_err());
        let dup = "[[package]]
name = \"x\"
version = \"1.0.0\"
[[package]]
name = \"x\"
version = \"1.0.0\"
";
        assert!(parse_lock(dup).is_err());
        assert!(parse_lock("").is_ok());
    }
}
