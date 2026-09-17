//! Huzi 源码位置(1-based 行 / 1-based 字符列)与
//! LSP 位置(0-based 行 / 0-based UTF-16 列)之间的纯函数映射。
//!
//! Huzi 的 [`huzi_error::HuziError`] 列号按字符计数,而 LSP 列按
//! UTF-16 代码单元计数:中文字符两者一致,但增补平面字符(如 emoji,
//! 1 字符 = 2 UTF-16 单元)必须折算,否则红线会随行内 emoji 漂移。

use ropey::Rope;
use tower_lsp_server::ls_types::{Position, Range};

/// Huzi 位置转 LSP [`Position`]。
///
/// `line_1b` / `col_1b` 均为 1-based;`line_1b == 0` 表示全局错误,
/// 此时钳制到文档首行首列。越界行列一律钳制到合法范围,不 panic。
pub fn huzi_pos_to_lsp(rope: &Rope, line_1b: usize, col_1b: usize) -> Position {
    let total_lines = rope.len_lines().max(1);
    let line_idx = line_1b.saturating_sub(1).min(total_lines - 1);
    let line_start = rope.line_to_char(line_idx);
    let line_len = rope.line(line_idx).len_chars();
    let char_off = col_1b.saturating_sub(1).min(line_len);
    let target = line_start + char_off;
    Position {
        line: line_idx as u32,
        character: utf16_units(rope, line_start, target),
    }
}

/// Huzi 起止区间转 LSP [`Range`];保证 `end >= start`。
pub fn huzi_range_to_lsp(
    rope: &Rope,
    start_line: usize,
    start_col: usize,
    end_line: usize,
    end_col: usize,
) -> Range {
    let start = huzi_pos_to_lsp(rope, start_line, start_col);
    let mut end = huzi_pos_to_lsp(rope, end_line, end_col);
    if (end.line, end.character) < (start.line, start.character) {
        end = start;
    }
    Range { start, end }
}

/// `[from, to)` 字符区间对应的 UTF-16 代码单元数。
fn utf16_units(rope: &Rope, from: usize, to: usize) -> u32 {
    rope.chars_at(from)
        .take(to.saturating_sub(from))
        .map(|c| c.len_utf16() as u32)
        .sum()
}

/// LSP [`Position`] 转行内字符偏移(0-based char index)。
///
/// LSP 列按 UTF-16 代码单元计数,逐字符累加 `len_utf16` 折算;
/// 落在代理对中间或越界时返回 `None`(调用方一律转 `Ok(None)`,不抛错)。
pub fn lsp_pos_to_char_col(rope: &Rope, pos: Position) -> Option<usize> {
    let line_idx = pos.line as usize;
    if line_idx >= rope.len_lines() {
        return None;
    }
    let target = pos.character as usize;
    let mut used_utf16 = 0usize;
    let mut char_col = 0usize;
    for c in rope.line(line_idx).chars() {
        if c == '\n' || c == '\r' {
            break;
        }
        if used_utf16 == target {
            return Some(char_col);
        }
        if used_utf16 > target {
            return None;
        }
        used_utf16 += c.len_utf16();
        char_col += 1;
    }
    if used_utf16 == target {
        Some(char_col)
    } else {
        None
    }
}

/// LSP [`Position`] 转 Huzi 坐标(1-based 行 / 1-based 字符列)。
///
/// 行号越界或列折算失败时返回 `None`。
pub fn lsp_pos_to_huzi(rope: &Rope, pos: Position) -> Option<(usize, usize)> {
    let line_idx = pos.line as usize;
    if line_idx >= rope.len_lines() {
        return None;
    }
    let char_col = lsp_pos_to_char_col(rope, pos)?;
    Some((line_idx + 1, char_col + 1))
}

/// 取光标处的词(`alnum` + `_`,中文标识符归入词内)及其 LSP [`Range`]。
///
/// 词边界按字符扩展,起止列经 UTF-16 折算;光标落在空白/符号/行尾时返回 `None`。
pub fn word_at_position(rope: &Rope, pos: Position) -> Option<(String, Range)> {
    let line_idx = pos.line as usize;
    if line_idx >= rope.len_lines() {
        return None;
    }
    let char_col = lsp_pos_to_char_col(rope, pos)?;
    let line_text: String = rope
        .line(line_idx)
        .chars()
        .take_while(|c| *c != '\n' && *c != '\r')
        .collect();
    let chars: Vec<char> = line_text.chars().collect();
    if char_col >= chars.len() || !is_word_char(chars[char_col]) {
        return None;
    }
    let mut start = char_col;
    while start > 0 && is_word_char(chars[start - 1]) {
        start -= 1;
    }
    let mut end = char_col;
    while end < chars.len() && is_word_char(chars[end]) {
        end += 1;
    }
    let word: String = chars[start..end].iter().collect();
    let start_u16: u32 =
        chars[..start].iter().map(|c| c.len_utf16() as u32).sum();
    let end_u16: u32 =
        chars[..end].iter().map(|c| c.len_utf16() as u32).sum();
    let line = line_idx as u32;
    Some((
        word,
        Range {
            start: Position {
                line,
                character: start_u16,
            },
            end: Position {
                line,
                character: end_u16,
            },
        },
    ))
}

/// 词字符:字母/数字/下划线(中文标识符按 `alphanumeric` 归入词内)。
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chinese_line_maps_by_char_index() {
        // Given: 全中文行(每字 1 字符 = 1 UTF-16 单元)
        let rope = Rope::from_str("变量名 = 1\n第二行\n");
        // When: 取第 1 行第 4 个字符(1-based 列 4)
        let pos = huzi_pos_to_lsp(&rope, 1, 4);
        // Then: LSP 行 0,UTF-16 列 3
        assert_eq!(
            pos,
            Position {
                line: 0,
                character: 3
            }
        );
        // 中文第二行行首同样对齐
        let pos2 = huzi_pos_to_lsp(&rope, 2, 1);
        assert_eq!(
            pos2,
            Position {
                line: 1,
                character: 0
            }
        );
    }

    #[test]
    fn emoji_counts_as_two_utf16_units() {
        // Given: 行内含增补平面字符(1 字符 = 2 UTF-16 单元)
        let rope = Rope::from_str("a\u{1F600}b\n");
        // When: 取 'b'(第 3 个字符,1-based 列 3)
        let pos = huzi_pos_to_lsp(&rope, 1, 3);
        // Then: UTF-16 列为 1 + 2 = 3,而非字符数的 2
        assert_eq!(
            pos,
            Position {
                line: 0,
                character: 3
            }
        );
    }

    #[test]
    fn global_and_out_of_range_positions_are_clamped() {
        // Given: 单行文档
        let rope = Rope::from_str("let x = 1\n");
        // When: 全局错误(0,0)与超大行列
        let global = huzi_pos_to_lsp(&rope, 0, 0);
        let huge = huzi_pos_to_lsp(&rope, 99, 99);
        // Then: 钳制到首行首列与末行内,不 panic
        assert_eq!(
            global,
            Position {
                line: 0,
                character: 0
            }
        );
        assert_eq!(huge.line, 1);
    }

    #[test]
    fn inverted_range_collapses_to_start() {
        // Given: 起止颠倒的区间
        let rope = Rope::from_str("let x = 1\n");
        // When: end 在 start 之前
        let range = huzi_range_to_lsp(&rope, 1, 5, 1, 2);
        // Then: end 被拉回到 start
        assert_eq!(range.start, range.end);
    }

    #[test]
    fn word_range_folds_emoji_utf16() {
        // Given: 行首含增补平面字符(a=1 单元,emoji=2 单元,b/空格各 1 单元)
        let rope = Rope::from_str("a\u{1F600}b foo\n");
        // When: 取 "foo"(字符列 4..7,UTF-16 列 5..8)
        let (word, range) =
            word_at_position(&rope, Position { line: 0, character: 5 })
                .expect("word foo");
        // Then: 词与起止列均按 UTF-16 折算
        assert_eq!(word, "foo");
        assert_eq!(range.start.character, 5);
        assert_eq!(range.end.character, 8);
    }

    #[test]
    fn word_miss_on_blank_and_out_of_range() {
        // Given: 普通单行文档
        let rope = Rope::from_str("let x = 1\n");
        // When: 落在空白处与越界行列
        // Then: 一律返回 None,不抛错
        assert!(word_at_position(&rope, Position { line: 0, character: 3 }).is_none());
        assert!(word_at_position(&rope, Position { line: 9, character: 0 }).is_none());
        assert!(lsp_pos_to_huzi(&rope, Position { line: 9, character: 0 }).is_none());
    }
}
