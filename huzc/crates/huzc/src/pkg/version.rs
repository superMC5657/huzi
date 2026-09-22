//! 版本需求 (VersionReq) 解析与匹配。
//!
//! 支持 `huzi.toml` 依赖声明中的版本范围写法,解析与“是否满足”判定见
//! `VersionReq::matches`,最高满足求解见 `select_max_satisfying`
//! (fetch/build 传递闭包共用,不写 `huzi.lock`、不做传递合并):
//!
//! | 写法 | 含义 |
//! |------|------|
//! | `1.2.3` | 精确相等(默认;裸版本号缺省分量补 0,`1.2` 即 `1.2.0`) |
//! | `^1.2.3` | 兼容上限(`>=1.2.3, <2.0.0`;`^0.2.3` → `<0.3.0`;`^0.0.3` → `<0.0.4`) |
//! | `~1.2.3` | 小版本上限(`>=1.2.3, <1.3.0`;`~1` → `>=1.0.0, <2.0.0`) |
//! | `>=1.0.0, <2.0.0` | 范围:逗号/空格分隔的比较符(`>`/`>=`/`<`/`<=`/`=`/`==`)需全部满足 |
//! | `*` / `1.*` | 任意版本 / 前缀通配(`1.*` 即 `>=1.0.0, <2.0.0`) |
//!
//! 说明:裸版本号取精确语义,以保持 `find`/`fetch` 按
//! `vendor/<pkg>/<version>/` 精确路径行为不变。

use std::fmt;

/// 语义版本三元组,缺省分量补 0(`1` → `1.0.0`,`1.2` → `1.2.0`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SemVersion {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl SemVersion {
    /// 解析 `1[.2[.3]]`,允许前导 `v`;拒绝预发布/通配等非纯数字写法。
    pub fn parse(raw: &str) -> Result<Self, String> {
        let t = raw.trim();
        let s = t
            .strip_prefix('v')
            .or_else(|| t.strip_prefix('V'))
            .unwrap_or(t);
        if s.is_empty() {
            return Err(format!("版本号为空: {:?}", raw));
        }
        let parts: Vec<&str> = s.split('.').collect();
        if parts.len() > 3 {
            return Err(format!("非法版本号 {:?}: 至多 3 个分量", raw));
        }
        let mut nums = [0u64; 3];
        for (i, p) in parts.iter().enumerate() {
            if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
                return Err(format!("非法版本号 {:?}: 分量 {:?} 非纯数字", raw, p));
            }
            nums[i] = p
                .parse::<u64>()
                .map_err(|_| format!("版本号 {:?} 数值越界", raw))?;
        }
        Ok(Self {
            major: nums[0],
            minor: nums[1],
            patch: nums[2],
        })
    }

    fn bump_major(&self) -> Self {
        Self {
            major: self.major + 1,
            minor: 0,
            patch: 0,
        }
    }

    fn bump_minor(&self) -> Self {
        Self {
            major: self.major,
            minor: self.minor + 1,
            patch: 0,
        }
    }
}

impl fmt::Display for SemVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// 范围比较符。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComparatorOp {
    Eq,
    Gt,
    Ge,
    Lt,
    Le,
}

/// 单个比较条件(如 `>=1.0.0`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Comparator {
    pub op: ComparatorOp,
    pub version: SemVersion,
}

impl Comparator {
    /// 判定版本是否满足本比较条件。
    pub fn matches(&self, ver: &SemVersion) -> bool {
        match self.op {
            ComparatorOp::Eq => ver == &self.version,
            ComparatorOp::Gt => ver > &self.version,
            ComparatorOp::Ge => ver >= &self.version,
            ComparatorOp::Lt => ver < &self.version,
            ComparatorOp::Le => ver <= &self.version,
        }
    }
}

/// 版本需求:依赖 `version` 字符串的解析视角(原串保留在 `Dependency::version`)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionReq {
    /// `*`:接受任意版本。
    Any,
    /// 裸版本号:精确相等。
    Exact(SemVersion),
    /// `^`:兼容上限。
    Caret(SemVersion),
    /// `~`:小版本上限(解析时已按精度折算 `upper`)。
    Tilde { lower: SemVersion, upper: SemVersion },
    /// 比较符范围:全部满足。
    Range(Vec<Comparator>),
}

impl VersionReq {
    /// 判定版本是否满足本需求。
    pub fn matches(&self, ver: &SemVersion) -> bool {
        match self {
            VersionReq::Any => true,
            VersionReq::Exact(v) => ver == v,
            VersionReq::Caret(base) => ver >= base && *ver < caret_upper(base),
            VersionReq::Tilde { lower, upper } => ver >= lower && ver < upper,
            VersionReq::Range(comps) => comps.iter().all(|c| c.matches(ver)),
        }
    }

    /// 解析版本串后判定是否满足,方便调用方单行使用。
    pub fn matches_str(&self, raw: &str) -> Result<bool, String> {
        Ok(self.matches(&SemVersion::parse(raw)?))
    }
}

/// 在候选版本中按需求选出最高满足版本(复用 `VersionReq::matches`)。
///
/// 无满足版本时返回 `None`,调用方按依赖冲突直接报错(不做自动升级):
/// ```rust
/// use huzc::pkg::{parse_version_req, select_max_satisfying, SemVersion};
/// let req = parse_version_req("^1.0.0").unwrap();
/// let cands = vec![
///     SemVersion::parse("1.0.0").unwrap(),
///     SemVersion::parse("1.2.0").unwrap(),
///     SemVersion::parse("2.0.0").unwrap(),
/// ];
/// assert_eq!(select_max_satisfying(&req, &cands).unwrap().to_string(), "1.2.0");
/// ```
pub fn select_max_satisfying(req: &VersionReq, candidates: &[SemVersion]) -> Option<SemVersion> {
    candidates.iter().copied().filter(|v| req.matches(v)).max()
}

/// 解析版本需求串(`^`/`~`/`>=`范围/`*`/裸版本精确)。
pub fn parse_version_req(raw: &str) -> Result<VersionReq, String> {    let s = raw.trim();
    if s.is_empty() {
        return Err("版本需求为空".to_string());
    }
    if s == "*" {
        return Ok(VersionReq::Any);
    }
    if let Some(rest) = s.strip_prefix('^') {
        return Ok(VersionReq::Caret(SemVersion::parse(rest)?));
    }
    if let Some(rest) = s.strip_prefix('~') {
        return parse_tilde(rest);
    }
    if has_comparator_prefix(s) || s.contains(',') || has_whitespace(s) {
        return parse_range(s);
    }
    if s.contains('*') {
        return parse_wildcard(s);
    }
    Ok(VersionReq::Exact(SemVersion::parse(s)?))
}

/// `^` 上限:`major>0` 进 major;`major==0` 时进 minor/patch。
fn caret_upper(base: &SemVersion) -> SemVersion {
    if base.major > 0 {
        base.bump_major()
    } else if base.minor > 0 {
        SemVersion {
            major: 0,
            minor: base.minor + 1,
            patch: 0,
        }
    } else {
        SemVersion {
            major: 0,
            minor: 0,
            patch: base.patch + 1,
        }
    }
}

/// 解析 `~` 需求:带 minor(精度到 `.`) 进 minor,否则进 major。
fn parse_tilde(rest: &str) -> Result<VersionReq, String> {
    let t = rest.trim();
    if t.is_empty() {
        return Err("`~` 后缺少版本号".to_string());
    }
    let lower = SemVersion::parse(t)?;
    let shaped = t
        .strip_prefix('v')
        .or_else(|| t.strip_prefix('V'))
        .unwrap_or(t);
    let upper = if shaped.contains('.') {
        lower.bump_minor()
    } else {
        lower.bump_major()
    };
    Ok(VersionReq::Tilde { lower, upper })
}

/// 解析比较符范围(逗号/空格分隔,需全部满足)。
fn parse_range(s: &str) -> Result<VersionReq, String> {
    let mut comps = Vec::new();
    for tok in s.replace(',', " ").split_whitespace() {
        comps.push(parse_comparator(tok)?);
    }
    if comps.is_empty() {
        return Err(format!("非法版本范围 {:?}", s));
    }
    Ok(VersionReq::Range(comps))
}

/// 解析单个比较符(无前缀视为 `=`)。
fn parse_comparator(tok: &str) -> Result<Comparator, String> {
    let (op, rest) = if let Some(r) = tok.strip_prefix(">=") {
        (ComparatorOp::Ge, r)
    } else if let Some(r) = tok.strip_prefix("<=") {
        (ComparatorOp::Le, r)
    } else if let Some(r) = tok.strip_prefix("==") {
        (ComparatorOp::Eq, r)
    } else if let Some(r) = tok.strip_prefix('>') {
        (ComparatorOp::Gt, r)
    } else if let Some(r) = tok.strip_prefix('<') {
        (ComparatorOp::Lt, r)
    } else if let Some(r) = tok.strip_prefix('=') {
        (ComparatorOp::Eq, r)
    } else {
        (ComparatorOp::Eq, tok)
    };
    if rest.trim().is_empty() {
        return Err(format!("比较符 {:?} 缺少版本号", tok));
    }
    if rest.contains('*') {
        return Err(format!("范围比较符不支持通配符 {:?}", tok));
    }
    Ok(Comparator {
        op,
        version: SemVersion::parse(rest)?,
    })
}

/// 解析前缀通配(`1.*` → `>=1.0.0, <2.0.0`;`1.2.*` → `>=1.2.0, <1.3.0`)。
fn parse_wildcard(s: &str) -> Result<VersionReq, String> {
    let core = s
        .strip_suffix(".*")
        .ok_or_else(|| format!("非法版本通配 {:?}", s))?;
    if core.is_empty() || core.contains('*') {
        return Err(format!("非法版本通配 {:?}", s));
    }
    let parts = core.split('.').count();
    if parts > 2 {
        return Err(format!("非法版本通配 {:?}", s));
    }
    let base = SemVersion::parse(core)?;
    let upper = if parts == 1 {
        base.bump_major()
    } else {
        base.bump_minor()
    };
    Ok(VersionReq::Range(vec![
        Comparator {
            op: ComparatorOp::Ge,
            version: base,
        },
        Comparator {
            op: ComparatorOp::Lt,
            version: upper,
        },
    ]))
}

fn has_comparator_prefix(s: &str) -> bool {
    s.starts_with(['>', '<', '=', '!'])
}

fn has_whitespace(s: &str) -> bool {
    s.chars().any(|c| c.is_whitespace())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(s: &str) -> VersionReq {
        parse_version_req(s).expect("版本需求应解析成功")
    }

    #[test]
    fn test_version_req_exact() {
        let r = req("1.2.3");
        assert_eq!(
            r,
            VersionReq::Exact(SemVersion {
                major: 1,
                minor: 2,
                patch: 3
            })
        );
        assert!(r.matches_str("1.2.3").unwrap());
        assert!(!r.matches_str("1.2.4").unwrap());
    }

    #[test]
    fn test_version_req_caret() {
        let r = req("^1.2.3");
        assert!(r.matches_str("1.9.0").unwrap());
        assert!(!r.matches_str("2.0.0").unwrap());
        assert!(!r.matches_str("1.2.2").unwrap());
        let zero = req("^0.2.3");
        assert!(zero.matches_str("0.2.9").unwrap());
        assert!(!zero.matches_str("0.3.0").unwrap());
    }

    #[test]
    fn test_version_req_tilde() {
        let r = req("~1.2.3");
        assert!(r.matches_str("1.2.9").unwrap());
        assert!(!r.matches_str("1.3.0").unwrap());
        let major_only = req("~1");
        assert!(major_only.matches_str("1.9.9").unwrap());
        assert!(!major_only.matches_str("2.0.0").unwrap());
    }

    #[test]
    fn test_version_req_range() {
        let r = req(">=1.0.0, <2.0.0");
        assert!(r.matches_str("1.5.0").unwrap());
        assert!(!r.matches_str("2.0.0").unwrap());
        assert!(!r.matches_str("0.9.9").unwrap());
    }

    #[test]
    fn test_version_req_any() {        assert_eq!(req("*"), VersionReq::Any);
        assert!(req("*").matches_str("9.9.9").unwrap());
        let prefixed = req("1.*");
        assert!(prefixed.matches_str("1.7.0").unwrap());
        assert!(!prefixed.matches_str("2.0.0").unwrap());
    }

    fn cands(raws: &[&str]) -> Vec<SemVersion> {
        raws.iter().map(|s| SemVersion::parse(s).unwrap()).collect()
    }

    #[test]
    fn test_select_max_caret_picks_highest() {
        // 多版本夹具口径:1.0.0/1.2.0 + ^1.0 → 1.2.0
        let r = req("^1.0.0");
        let got = select_max_satisfying(&r, &cands(&["1.0.0", "1.2.0"]));
        assert_eq!(got.unwrap().to_string(), "1.2.0");
    }

    #[test]
    fn test_select_max_range_skips_major_bump() {
        let r = req("^1.0.0");
        let got = select_max_satisfying(&r, &cands(&["1.0.0", "1.2.0", "2.0.0"])).unwrap();
        assert_eq!(got.to_string(), "1.2.0");
    }

    #[test]
    fn test_select_max_none_when_conflict() {
        let r = req("1.0.0");
        assert!(select_max_satisfying(&r, &cands(&["1.2.0", "2.0.0"])).is_none());
        let empty: Vec<SemVersion> = vec![];
        assert!(select_max_satisfying(&req("*"), &empty).is_none());
    }
}
