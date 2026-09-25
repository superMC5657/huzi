//! 光标上下文:前缀词 + 点/双冒号基名(含 UTF-16 折算与词边界)。

use super::CursorCtx;
use ropey::Rope;
use tower_lsp_server::ls_types::Position;

/// 取光标上下文:前缀词 + 点/双冒号基名(越界返回 `None`)。
pub(super) fn cursor_context(text: &str, pos: Position) -> Option<CursorCtx> {
    let rope = Rope::from_str(text);
    let line_idx = pos.line as usize;
    if line_idx >= rope.len_lines() {
        return None;
    }
    let line: String = rope
        .line(line_idx)
        .chars()
        .take_while(|c| *c != '\n' && *c != '\r')
        .collect();
    let chars: Vec<char> = line.chars().collect();
    let col = utf16_to_char_col(&chars, pos.character as usize)?;
    let prefix_start = word_start_before(&chars, col);
    let prefix: String = chars[prefix_start..col].iter().collect();
    if let Some(module) = dbl_colon_base_before(&chars, prefix_start) {
        return Some(CursorCtx {
            prefix,
            dot_base: None,
            dbl_colon_base: Some(module),
        });
    }
    let dot_base = dot_base_before(&chars, prefix_start, col);
    Some(CursorCtx {
        prefix,
        dot_base,
        dbl_colon_base: None,
    })
}

/// `::` 前的模块名(`word_start` 为前缀词起点)。
fn dbl_colon_base_before(
    chars: &[char],
    word_start: usize,
) -> Option<String> {
    if word_start < 2 || chars[word_start - 1] != ':' || chars[word_start - 2] != ':' {
        return None;
    }
    let end = word_start - 2;
    let start = word_start_before(chars, end);
    if start == end {
        return None;
    }
    Some(chars[start..end].iter().collect())
}

/// `.` 前的基名(前缀词为空时看光标前一字符)。
fn dot_base_before(
    chars: &[char],
    word_start: usize,
    col: usize,
) -> Option<String> {
    let dot_idx = if word_start > 0 && chars[word_start - 1] == '.' {
        word_start - 1
    } else if word_start == col && col > 0 && chars[col - 1] == '.' {
        col - 1
    } else {
        return None;
    };
    let start = word_start_before(chars, dot_idx);
    if start == dot_idx {
        return None;
    }
    Some(chars[start..dot_idx].iter().collect())
}

/// 从 `col` 向前取连续词字符的起点。
fn word_start_before(chars: &[char], col: usize) -> usize {
    let mut start = col.min(chars.len());
    while start > 0 && is_word_char(chars[start - 1]) {
        start -= 1;
    }
    start
}

/// UTF-16 列 -> 字符列(行尾钳制,代理对中间返回 `None`)。
fn utf16_to_char_col(chars: &[char], target: usize) -> Option<usize> {
    let mut used = 0usize;
    for (idx, c) in chars.iter().enumerate() {
        if used == target {
            return Some(idx);
        }
        if used > target {
            return None;
        }
        used += c.len_utf16();
    }
    if target >= used {
        Some(chars.len())
    } else {
        None
    }
}

/// 词字符:字母/数字/下划线(与 [`crate::mapping`] 一致,中文归入词内)。
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}
