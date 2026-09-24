//! "did you mean" 修复建议:用 Levenshtein 编辑距离在候选名中找最接近项。

/// 两个字符串之间的 Levenshtein 编辑距离(插入/删除/替换,代价均为 1)。
pub fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr: Vec<usize> = vec![0; b.len() + 1];
    for i in 1..=a.len() {
        curr[0] = i;
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()]
}

/// 建议距离上限:名称越长允许的容错越多,但封顶 3。
const MAX_DISTANCE: usize = 3;

/// 按输入名长度归一化的最大允许距离:min(3, max(1, len/3))。
/// `len` 取输入名的字符数(非字节数):短名容错小以避免误报,长名最多容忍 3。
fn allowed_distance(name_len: usize) -> usize {
    (name_len / 3).clamp(1, MAX_DISTANCE)
}

/// 公共前缀长度(按字符计,大小写敏感),越大表示越可能是前缀补全型笔误。
fn common_prefix_len(a: &str, b: &str) -> usize {
    a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count()
}

/// 首字符是否相同(忽略 ASCII 大小写;任一为空串视为不同)。
fn same_first_char(a: &str, b: &str) -> bool {
    match (a.chars().next(), b.chars().next()) {
        (Some(x), Some(y)) => x.to_ascii_lowercase() == y.to_ascii_lowercase(),
        _ => false,
    }
}

/// 在候选名中寻找与 `name` 最接近的一项(大小写不敏感的完全匹配优先),
/// 返回可直接追加到错误信息的提示文本,如 ``Some("did you mean `count`?")``。
///
/// 排名规则(比较键,越小越优,前缀取反后比较):
/// `(distance, 首字母不同惩罚, MAX-公共前缀长, 纯增删外惩罚, 长度差, 字典序)`。
/// 其中字典序作为最终兜底,保证结果与候选迭代顺序(如 HashMap 随机顺序)无关。
pub fn did_you_mean<'a, I>(name: &str, candidates: I) -> Option<String>
where
    I: IntoIterator<Item = &'a str>,
{
    let name_len = name.chars().count();
    let max_dist = allowed_distance(name_len);
    // 短名防 spam:单字符(及空)输入仅允许大小写不敏感的精确匹配。
    let short = name_len <= 1;

    let mut best: Option<(usize, usize, usize, usize, usize, &str)> = None;
    for candidate in candidates {
        if candidate == name {
            continue;
        }
        let distance = if candidate.eq_ignore_ascii_case(name) {
            0
        } else {
            levenshtein(name, candidate)
        };
        if short && distance != 0 {
            continue;
        }
        if distance > max_dist {
            continue;
        }
        let candidate_len = candidate.chars().count();
        let len_diff = name_len.abs_diff(candidate_len);
        let first_penalty = usize::from(!same_first_char(name, candidate));
        let prefix = common_prefix_len(name, candidate);
        let rev_prefix = usize::MAX - prefix;
        // 纯增删(距离恰等于长度差,如缺字母的补全)优于替换;distance==0 恒最优。
        let insertion_penalty = usize::from(distance != len_diff && distance != 0);
        let key = (
            distance,
            first_penalty,
            rev_prefix,
            insertion_penalty,
            len_diff,
            candidate,
        );
        if best.is_none_or(|b| key < b) {
            best = Some(key);
        }
    }
    best.map(|(_, _, _, _, _, candidate)| format!("did you mean `{}`?", candidate))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levenshtein_counts_edits() {
        assert_eq!(levenshtein("", ""), 0);
        assert_eq!(levenshtein("abc", "abc"), 0);
        assert_eq!(levenshtein("cont", "count"), 1);
        assert_eq!(levenshtein("helo", "hello"), 1);
        assert_eq!(levenshtein("kitten", "sitting"), 3);
    }

    #[test]
    fn suggests_the_closest_candidate() {
        let candidates = ["count", "cost", "main"];
        assert_eq!(
            did_you_mean("cont", candidates),
            Some("did you mean `count`?".to_string())
        );
    }

    #[test]
    fn case_insensitive_match_wins() {
        assert_eq!(
            did_you_mean("PRINT", ["print", "printer"]),
            Some("did you mean `print`?".to_string())
        );
    }

    #[test]
    fn distant_names_get_no_suggestion() {
        assert_eq!(did_you_mean("zzzqqq", ["count", "main"]), None);
        assert_eq!(did_you_mean("count", ["count", "main"]), None);
    }

    #[test]
    fn tie_break_deterministic() {
        // "cout" 到 "count"(插入 n)与到 "cost"(替换 u->s)距离均为 1,
        // 公共前缀更长者("cou" vs "co")胜出,且与候选顺序无关。
        let expected = Some("did you mean `count`?".to_string());
        assert_eq!(did_you_mean("cout", ["count", "cost"]), expected);
        assert_eq!(did_you_mean("cout", ["cost", "count"]), expected);
    }

    #[test]
    fn short_name_no_spam() {
        // 单字符输入仅允许大小写不敏感精确匹配,避免误报。
        assert_eq!(did_you_mean("b", ["a"]), None);
        assert_eq!(
            did_you_mean("b", ["B"]),
            Some("did you mean `B`?".to_string())
        );
    }

    #[test]
    fn long_name_tolerance() {
        // "kitten"->"sitting" 距离 3:短名(len6,阈值2)无建议,长名(len9,阈值3)有建议。
        assert_eq!(did_you_mean("kitten", ["sitting"]), None);
        assert_eq!(
            did_you_mean("kittenxxx", ["sittingxxx"]),
            Some("did you mean `sittingxxx`?".to_string())
        );
    }

    #[test]
    fn prefix_bonus() {
        // 距离同为 1 时公共前缀更长者胜出(覆盖字典序)。
        // "abcde" vs "abcXe"(前缀3) vs "abYde"(前缀2):后者字典序更小,但前者胜出。
        let expected = Some("did you mean `abcXe`?".to_string());
        assert_eq!(did_you_mean("abcde", ["abYde", "abcXe"]), expected);
        assert_eq!(did_you_mean("abcde", ["abcXe", "abYde"]), expected);
        // 首字母相同加权:"tent"(首字母同)优于 "best"(首字母异)。
        assert_eq!(
            did_you_mean("test", ["best", "tent"]),
            Some("did you mean `tent`?".to_string())
        );
    }
}
