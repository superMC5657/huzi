//! 注释回收:格式化前按行扫描源码,记录每条注释的行号、原文与
//! 是否独占一行。扫描器跳过字符串/字符字面量内部,注释语义与
//! 词法器一致(`//` 与 `#` 均为行注释)。
//!
//! 打印阶段由 Formatter 按语句的 span 行号把这些注释插回输出:
//! 整行注释插在语句前,行尾注释拼在语句后。任何注释都不会丢失;
//! 个别位置(如结构体字段之间)可能整体偏移。

#[derive(Debug, Clone)]
pub struct CommentInfo {
    /// 1-based 源码行号。
    pub line: usize,
    /// 注释原文(含 `//` 或 `#` 前缀,已去行尾空白)。
    pub text: String,
    /// 行首到注释之间只有空白时为 true。
    pub full_line: bool,
}

pub fn collect_comments(source: &str) -> Vec<CommentInfo> {
    let chars: Vec<char> = source.chars().collect();
    let n = chars.len();
    let mut out: Vec<CommentInfo> = Vec::new();
    let mut i = 0usize;
    let mut line = 1usize;
    let mut col = 0usize;
    let mut in_string = false;
    let mut in_char = false;

    while i < n {
        let c = chars[i];
        if in_string {
            let (ni, nline, ncol, still) = advance_in_string(&chars, i, line, col);
            i = ni;
            line = nline;
            col = ncol;
            in_string = still;
            continue;
        }
        if in_char {
            let (ni, ncol, still) = advance_in_char(&chars, i, col);
            i = ni;
            col = ncol;
            in_char = still;
            continue;
        }
        if c == '"' {
            in_string = true;
            col += 1;
            i += 1;
            continue;
        }
        if c == '\'' {
            in_char = true;
            col += 1;
            i += 1;
            continue;
        }
        let is_comment = (c == '/' && i + 1 < n && chars[i + 1] == '/') || c == '#';
        if is_comment {
            let mut text = String::new();
            while i < n && chars[i] != '\n' {
                text.push(chars[i]);
                i += 1;
            }
            out.push(CommentInfo {
                line,
                text: text.trim_end().to_string(),
                full_line: col == 0,
            });
            continue;
        }
        if c == '\n' {
            line += 1;
            col = 0;
            i += 1;
            continue;
        }
        col += 1;
        i += 1;
    }
    out
}

/// 字符串字面量内推进一步:转义跳两格,结束引号退出字面量,
/// 换行累计行号。返回新的 (i, line, col, 是否仍在字符串内)。
fn advance_in_string(chars: &[char], i: usize, line: usize, col: usize) -> (usize, usize, usize, bool) {
    let c = chars[i];
    if c == '\\' && i + 1 < chars.len() {
        return (i + 2, line, col + 2, true);
    }
    if c == '"' {
        return (i + 1, line, col + 1, false);
    }
    if c == '\n' {
        (i + 1, line + 1, 0, true)
    } else {
        (i + 1, line, col + 1, true)
    }
}

/// 字符字面量内推进一步(字面量内无换行):转义跳两格,结束引号
/// 退出字面量。返回新的 (i, col, 是否仍在字符字面量内)。
fn advance_in_char(chars: &[char], i: usize, col: usize) -> (usize, usize, bool) {
    let c = chars[i];
    if c == '\\' && i + 1 < chars.len() {
        return (i + 2, col + 2, true);
    }
    if c == '\'' {
        return (i + 1, col + 1, false);
    }
    (i + 1, col + 1, true)
}
