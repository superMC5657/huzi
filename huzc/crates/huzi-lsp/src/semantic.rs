//! semanticTokens:lexer [`Token`] -> LSP 5 整数编码。
//!
//! 图例顺序固定:`0=keyword, 1=variable, 2=function, 3=type`,
//! 修饰位全 0。映射规则:关键字全归 keyword;`fn` 后标识符归
//! function;`struct`/`enum`/`:`/`->` 后标识符归 type;其余标识符
//! 归 variable;`::` 后标识符归 function(跨模块调用名)。
//! 字面量/运算符/分隔符跳过,不编码。

use huzi_lexer::{Lexer, Token};
use ropey::Rope;
use tower_lsp_server::ls_types::*;

/// 图例索引:关键字。
pub const KIND_KEYWORD: u32 = 0;
/// 图例索引:变量。
pub const KIND_VARIABLE: u32 = 1;
/// 图例索引:函数名。
pub const KIND_FUNCTION: u32 = 2;
/// 图例索引:类型名。
pub const KIND_TYPE: u32 = 3;

/// 服务端图例:tokenTypes 按 keyword/variable/function/type 排序。
pub fn legend() -> SemanticTokensLegend {
    SemanticTokensLegend {
        token_types: vec![
            SemanticTokenType::KEYWORD,
            SemanticTokenType::VARIABLE,
            SemanticTokenType::FUNCTION,
            SemanticTokenType::TYPE,
        ],
        token_modifiers: vec![],
    }
}

/// 全文 -> 语义 token 表(按行列递增,delta 编码,坏文件返回空表)。
pub fn tokens_for_text(text: &str) -> Vec<SemanticToken> {
    let mut lexer = Lexer::new(text.to_string());
    let spanned = match lexer.tokenize() {
        Ok(tokens) => tokens,
        Err(_) => return Vec::new(),
    };
    let rope = Rope::from_str(text);
    let mut out = Vec::new();
    let mut prev_kind: Option<&Token> = None;
    let mut prev_line = 0u32;
    let mut prev_start = 0u32;
    let mut first = true;
    for item in &spanned {
        if let Some((kind, length)) = classify(&item.token, prev_kind) {
            if let Some((line, start)) =
                token_pos(&rope, item.line, item.column)
            {
                let delta_line = line - prev_line;
                let delta_start = if first || delta_line > 0 {
                    start
                } else {
                    start - prev_start
                };
                out.push(SemanticToken {
                    delta_line,
                    delta_start,
                    length,
                    token_type: kind,
                    token_modifiers_bitset: 0,
                });
                prev_line = line;
                prev_start = start;
                first = false;
            }
        }
        prev_kind = Some(&item.token);
    }
    out
}

/// Token 位置(1-based 行/列) -> 0-based 行与 UTF-16 起始列。
fn token_pos(
    rope: &Rope,
    line_1b: usize,
    col_1b: usize,
) -> Option<(u32, u32)> {
    if line_1b == 0 {
        return None;
    }
    let line_idx = line_1b - 1;
    if line_idx >= rope.len_lines() {
        return None;
    }
    let line: String = rope
        .line(line_idx)
        .chars()
        .take_while(|c| *c != '\n' && *c != '\r')
        .collect();
    let chars: Vec<char> = line.chars().collect();
    let col = col_1b.saturating_sub(1);
    if col > chars.len() {
        return None;
    }
    let start: u32 =
        chars[..col].iter().map(|c| c.len_utf16() as u32).sum();
    Some((line_idx as u32, start))
}

/// Token 分类 -> (图例索引, UTF-16 长度);不编码返回 `None`。
fn classify(token: &Token, prev: Option<&Token>) -> Option<(u32, u32)> {
    match token {
        Token::Ident(name) => Some((ident_kind(prev), utf16_len(name))),
        Token::Fn => Some((KIND_KEYWORD, 2)),
        Token::Struct => Some((KIND_KEYWORD, 6)),
        Token::Enum => Some((KIND_KEYWORD, 4)),
        Token::Match => Some((KIND_KEYWORD, 5)),
        Token::Let => Some((KIND_KEYWORD, 3)),
        Token::Mut => Some((KIND_KEYWORD, 3)),
        Token::If => Some((KIND_KEYWORD, 2)),
        Token::Else => Some((KIND_KEYWORD, 4)),
        Token::Elif => Some((KIND_KEYWORD, 4)),
        Token::For => Some((KIND_KEYWORD, 3)),
        Token::In => Some((KIND_KEYWORD, 2)),
        Token::While => Some((KIND_KEYWORD, 5)),
        Token::Return => Some((KIND_KEYWORD, 6)),
        Token::Import => Some((KIND_KEYWORD, 6)),
        Token::Break => Some((KIND_KEYWORD, 5)),
        Token::Continue => Some((KIND_KEYWORD, 8)),
        Token::Defer => Some((KIND_KEYWORD, 5)),
        Token::Trait => Some((KIND_KEYWORD, 5)),
        Token::Impl => Some((KIND_KEYWORD, 4)),
        Token::True => Some((KIND_KEYWORD, 4)),
        Token::False => Some((KIND_KEYWORD, 5)),
        _ => None,
    }
}

/// 标识符子分类:前一有效 Token 决定 function/type/variable。
fn ident_kind(prev: Option<&Token>) -> u32 {
    match prev {
        Some(Token::Fn | Token::PathSep) => KIND_FUNCTION,
        Some(Token::Struct | Token::Enum | Token::Trait | Token::Colon | Token::Arrow) => {
            KIND_TYPE
        }
        _ => KIND_VARIABLE,
    }
}

/// 字符串的 UTF-16 长度。
fn utf16_len(s: &str) -> u32 {
    s.chars().map(|c| c.len_utf16() as u32).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// token 表拍平为 u32 序列(与线上传输形状一致)。
    fn flatten(tokens: &[SemanticToken]) -> Vec<u32> {
        let mut out = Vec::with_capacity(tokens.len() * 5);
        for t in tokens {
            out.extend([
                t.delta_line,
                t.delta_start,
                t.length,
                t.token_type,
                t.token_modifiers_bitset,
            ]);
        }
        out
    }

    #[test]
    fn legend_order_is_keyword_variable_function_type() {
        // Given: 服务端图例
        // When: 取 tokenTypes
        let got = legend();
        let kinds: Vec<&str> =
            got.token_types.iter().map(|t| t.as_str()).collect();
        // Then: 顺序固定为 keyword/variable/function/type,修饰为空
        assert_eq!(kinds, vec!["keyword", "variable", "function", "type"]);
        assert!(got.token_modifiers.is_empty());
    }

    #[test]
    fn flat_encoding_length_is_multiple_of_five() {
        // Given: 含 fn 定义与调用的程序
        let text = "fn add(a: i32, b: i32) -> i32 {\n return a + b\n}\nlet s = add(1, 2)\n";
        // When: 编码并拍平
        let flat = flatten(&tokens_for_text(text));
        // Then: 非空且长度为 5 倍数
        assert!(!flat.is_empty());
        assert_eq!(flat.len() % 5, 0);
    }

    #[test]
    fn fn_name_is_function_and_keywords_mapped() {
        // Given: `fn add` 开头的文件
        let text = "fn add(a: i32) -> i32 {\n return a\n}\n";
        // When: 编码
        let tokens = tokens_for_text(text);
        // Then: 首 token 为 keyword(fn),次 token 为 function(add)
        assert!(tokens.len() >= 2);
        assert_eq!(tokens[0].token_type, KIND_KEYWORD);
        assert_eq!(tokens[0].length, 2);
        assert_eq!(tokens[1].token_type, KIND_FUNCTION);
        assert_eq!(tokens[1].length, 3);
        // import 关键字同样归 keyword
        let imp = tokens_for_text("import mods.helpers\n");
        assert!(!imp.is_empty());
        assert_eq!(imp[0].token_type, KIND_KEYWORD);
    }

    #[test]
    fn bad_file_yields_empty_tokens() {
        // Given: 词法失败的文件
        // When: 编码
        // Then: 空表,不抛错
        assert!(tokens_for_text("let x = @\n").is_empty());
        assert!(tokens_for_text("").is_empty());
    }

    #[test]
    fn colon_type_annotation_is_type_kind() {
        // Given: 含类型标注的文件(L1 锁定:`:` 后标识符归 type)
        let text = "fn add(a: i32) -> i32 {\n return a\n}\n";
        // When: 编码
        let tokens = tokens_for_text(text);
        // Then: 含 type 类 token
        assert!(
            tokens.iter().any(|t| t.token_type == KIND_TYPE),
            "{tokens:?}"
        );
    }

    #[test]
    fn pathsep_ident_is_function_kind() {
        // Given: 含跨模块调用的文件(L1 锁定:`::` 后标识符归 function)
        let text = "import mods.helpers\nlet s = helpers::add(3, 4)\n";
        // When: 编码
        let tokens = tokens_for_text(text);
        // Then: 含 function 类 token(调用名 add)
        assert!(
            tokens.iter().any(|t| t.token_type == KIND_FUNCTION),
            "{tokens:?}"
        );
    }
}
