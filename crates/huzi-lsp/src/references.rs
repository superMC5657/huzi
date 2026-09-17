//! 同文件 references:光标词全文匹配,返回全部出现处 [`Location`]。
//!
//! 纯函数层:经 [`crate::mapping`] 做 UTF-16 折算逐行取词,
//! 中文标识符同样对齐;失败返回空表,不抛错。

use ropey::Rope;
use tower_lsp_server::ls_types::*;

use crate::mapping::word_at_position;

/// 全文 + 光标 -> 该词全部出现处(同文件,含声明本身)。
pub fn references_for_text(
    text: &str,
    uri: &Uri,
    pos: Position,
) -> Vec<Location> {
    let rope = Rope::from_str(text);
    let (word, _) = match word_at_position(&rope, pos) {
        Some(found) => found,
        None => return Vec::new(),
    };
    let mut out = Vec::new();
    for line_idx in 0..rope.len_lines() {
        out.extend(line_hits(&rope, uri, line_idx, &word));
    }
    out
}

/// 单行内全部词匹配(词边界两侧须为非词字符)。
fn line_hits(
    rope: &Rope,
    uri: &Uri,
    line_idx: usize,
    word: &str,
) -> Vec<Location> {
    let line: String = rope
        .line(line_idx)
        .chars()
        .take_while(|c| *c != '\n' && *c != '\r')
        .collect();
    let chars: Vec<char> = line.chars().collect();
    let want: Vec<char> = word.chars().collect();
    if want.is_empty() || want.len() > chars.len() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i + want.len() <= chars.len() {
        if chars[i..i + want.len()] == want[..]
            && boundary_before(&chars, i)
            && boundary_after(&chars, i + want.len())
        {
            out.push(Location {
                uri: uri.clone(),
                range: char_range(&chars, line_idx, i, i + want.len()),
            });
            i += want.len();
        } else {
            i += 1;
        }
    }
    out
}

/// 左边界:行首或前一字符为非词字符。
fn boundary_before(chars: &[char], start: usize) -> bool {
    start == 0 || !is_word_char(chars[start - 1])
}

/// 右边界:行尾或后一字符为非词字符。
fn boundary_after(chars: &[char], end: usize) -> bool {
    end >= chars.len() || !is_word_char(chars[end])
}

/// 字符列区间 -> LSP [`Range`](UTF-16 折算)。
fn char_range(
    chars: &[char],
    line_idx: usize,
    start: usize,
    end: usize,
) -> Range {
    let line = line_idx as u32;
    Range {
        start: Position {
            line,
            character: utf16_len(&chars[..start]),
        },
        end: Position {
            line,
            character: utf16_len(&chars[..end]),
        },
    }
}

/// 字符序列对应的 UTF-16 代码单元数。
fn utf16_len(chars: &[char]) -> u32 {
    chars.iter().map(|c| c.len_utf16() as u32).sum()
}

/// 词字符:字母/数字/下划线(与 [`crate::mapping`] 一致)。
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uri_of(path: &str) -> Uri {
        path.parse().expect("uri")
    }

    #[test]
    fn same_word_lists_all_occurrences() {
        // Given: factorial 出现 3 次(定义 + 体内 + 调用)
        let text = "fn factorial(n: i32) -> i32 {\n    n\n}\nfn main() -> i32 {\n    factorial(1)\n}\n";
        let uri = uri_of("file:///proj/main.hz");
        // When: 在调用点(行 4,列 5)请求 references
        let hits = references_for_text(
            text,
            &uri,
            Position {
                line: 4,
                character: 5,
            },
        );
        // Then: 非空,且含定义行(0)与调用行(4)
        assert!(!hits.is_empty());
        let lines: Vec<u32> =
            hits.iter().map(|h| h.range.start.line).collect();
        assert!(lines.contains(&0), "{lines:?}");
        assert!(lines.contains(&4), "{lines:?}");
        for h in &hits {
            assert_eq!(h.uri, uri);
        }
    }

    #[test]
    fn prefix_word_does_not_match_longer_ident() {
        // Given: `add` 与 `adder` 并存
        let text = "fn add() -> i32 {\n return 1\n}\nfn adder() -> i32 {\n return 2\n}\n";
        let uri = uri_of("file:///proj/main.hz");
        // When: 在 `add` 定义处(行 0,列 3)请求 references
        let hits = references_for_text(
            text,
            &uri,
            Position {
                line: 0,
                character: 3,
            },
        );
        // Then: 仅命中 `add` 本身,不含 `adder` 行
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].range.start.line, 0);
    }

    #[test]
    fn chinese_ident_hits_stay_aligned() {
        // Given: 中文标识符出现两次
        let text = "let 变量 = 1\n变量\n";
        let uri = uri_of("file:///proj/main.hz");
        // When: 在第二行中文词处请求 references
        let hits = references_for_text(
            text,
            &uri,
            Position {
                line: 1,
                character: 1,
            },
        );
        // Then: 两行各一处,且列按 UTF-16 对齐(中文 1 字 = 1 单元)
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].range.start.line, 0);
        assert_eq!(hits[0].range.start.character, 4);
        assert_eq!(hits[1].range.start.line, 1);
    }

    #[test]
    fn blank_position_returns_empty() {
        // Given: 普通文件
        let text = "let x = 1\n";
        let uri = uri_of("file:///proj/main.hz");
        // When: 落在空白/越界处
        // Then: 空表,不抛错
        assert!(references_for_text(
            text,
            &uri,
            Position {
                line: 0,
                character: 3,
            }
        )
        .is_empty());
        assert!(references_for_text(
            text,
            &uri,
            Position {
                line: 9,
                character: 0,
            }
        )
        .is_empty());
    }
}
