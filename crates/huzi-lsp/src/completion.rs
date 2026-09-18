//! Completion:关键字/符号/成员/`math::` 补全(坏文件仍可用)。
//!
//! 纯函数 [`completion_for_text`]:全文 + 光标 -> 补全项。
//! 解析失败时符号表为空,但关键字照常返回,保证坏文件可补。
//! `:`/`::` 与 `.` 上下文由本文件自行做 UTF-16 折算,不依赖
//! [`crate::mapping::word_at_position`](其要求光标落在词内,
//! 而补全光标常在词尾/点号后)。

use std::collections::HashSet;

use huzi_ast::symbols::SymbolKind as HuziSymbolKind;
use ropey::Rope;
use tower_lsp_server::ls_types::{
    CompletionItem, CompletionItemKind, Position,
};

use crate::analysis::parse_and_collect;

/// 补全关键字表(与语法关键字对齐)。
const KEYWORDS: &[&str] = &[
    "fn", "let", "mut", "if", "elif", "else", "for", "in", "while",
    "return", "break", "continue", "defer", "import", "match", "struct", "enum",
    "trait", "impl",
];

/// `math::` 模块函数(与 codegen libm/libc 声明对齐)。
const MATH_FNS: &[&str] = &[
    "abs", "sqrt", "pow", "sin", "cos", "tan", "floor", "ceil", "round",
];

/// 未知接收者的通用成员(保证 `.` 后永不落空)。
const GENERIC_MEMBERS: &[&str] =
    &["len", "to_string", "push", "pop", "clear"];

/// 光标上下文:当前前缀词 + 点/双冒号前的基名。
struct CursorCtx {
    prefix: String,
    dot_base: Option<String>,
    dbl_colon_base: Option<String>,
}

/// 全文 + 光标 -> 补全项(纯函数,永不 panic,失败返回空表)。
pub fn completion_for_text(
    text: &str,
    pos: Position,
) -> Vec<CompletionItem> {
    let ctx = match cursor_context(text, pos) {
        Some(ctx) => ctx,
        None => return Vec::new(),
    };
    if let Some(module) = ctx.dbl_colon_base {
        return dedup(colon_items(text, &module, &ctx.prefix));
    }
    if let Some(base) = ctx.dot_base {
        return dedup(dot_items(text, &base, &ctx.prefix));
    }
    let (_program, symbols) = parse_and_collect(text);
    dedup(normal_items(&symbols, &ctx.prefix))
}

/// 空白/词内上下文:关键字(前缀匹配) + 同文件符号(前缀匹配)。
fn normal_items(
    symbols: &[huzi_ast::symbols::Symbol],
    prefix: &str,
) -> Vec<CompletionItem> {
    let mut out = Vec::new();
    for kw in KEYWORDS.iter().filter(|kw| kw.starts_with(prefix)) {
        out.push(CompletionItem {
            label: (*kw).to_string(),
            kind: Some(CompletionItemKind::KEYWORD),
            detail: Some("keyword".to_string()),
            ..Default::default()
        });
    }
    for sym in symbols.iter().filter(|s| s.name.starts_with(prefix)) {
        out.push(CompletionItem {
            label: sym.name.clone(),
            kind: Some(to_completion_kind(sym.kind)),
            detail: Some(sym.detail.clone()),
            ..Default::default()
        });
    }
    out
}

/// `.` 上下文:基名为 struct 补字段,为 enum 补变体,否则补通用成员。
fn dot_items(
    text: &str,
    base: &str,
    prefix: &str,
) -> Vec<CompletionItem> {
    let (program, _symbols) = parse_and_collect(text);
    for stmt in &program.statements {
        if let huzi_ast::Stmt::Struct(d) = &stmt.node {
            if d.name == base {
                return d
                    .fields
                    .iter()
                    .filter(|f| f.name.starts_with(prefix))
                    .map(|f| CompletionItem {
                        label: f.name.clone(),
                        kind: Some(CompletionItemKind::FIELD),
                        detail: Some(format!(
                            "{}::{}: {}",
                            base, f.name, f.field_type
                        )),
                        ..Default::default()
                    })
                    .collect();
            }
        }
        if let huzi_ast::Stmt::Enum(d) = &stmt.node {
            if d.name == base {
                return variant_items(&d.variants, base, prefix);
            }
        }
    }
    generic_member_items(prefix)
}

/// `::` 上下文:`math` 补数学函数,枚举名补变体,其他返回空表。
fn colon_items(
    text: &str,
    module: &str,
    prefix: &str,
) -> Vec<CompletionItem> {
    if module == "math" {
        return MATH_FNS
            .iter()
            .filter(|name| name.starts_with(prefix))
            .map(|name| CompletionItem {
                label: (*name).to_string(),
                kind: Some(CompletionItemKind::FUNCTION),
                detail: Some(format!("math::{name}")),
                ..Default::default()
            })
            .collect();
    }
    let (program, _symbols) = parse_and_collect(text);
    for stmt in &program.statements {
        if let huzi_ast::Stmt::Enum(d) = &stmt.node {
            if d.name == module {
                return variant_items(&d.variants, module, prefix);
            }
        }
    }
    Vec::new()
}

/// 枚举变体列表(前缀匹配,供 `.` 与 `::` 共用)。
fn variant_items(
    variants: &[huzi_ast::EnumVariant],
    enum_name: &str,
    prefix: &str,
) -> Vec<CompletionItem> {
    variants
        .iter()
        .filter(|v| v.name.starts_with(prefix))
        .map(|v| CompletionItem {
            label: v.name.clone(),
            kind: Some(CompletionItemKind::ENUM_MEMBER),
            detail: Some(format!("{enum_name}::{}", v.name)),
            ..Default::default()
        })
        .collect()
}

/// 通用成员列表(前缀匹配)。
fn generic_member_items(prefix: &str) -> Vec<CompletionItem> {
    GENERIC_MEMBERS
        .iter()
        .filter(|name| name.starts_with(prefix))
        .map(|name| CompletionItem {
            label: (*name).to_string(),
            kind: Some(CompletionItemKind::FIELD),
            detail: Some("member".to_string()),
            ..Default::default()
        })
        .collect()
}

/// Huzi 符号种类 -> 补全项种类。
fn to_completion_kind(kind: HuziSymbolKind) -> CompletionItemKind {
    match kind {
        HuziSymbolKind::Function => CompletionItemKind::FUNCTION,
        HuziSymbolKind::Struct => CompletionItemKind::STRUCT,
        HuziSymbolKind::Enum => CompletionItemKind::ENUM,
        HuziSymbolKind::Variant => CompletionItemKind::ENUM_MEMBER,
        HuziSymbolKind::Module => CompletionItemKind::MODULE,
        HuziSymbolKind::Trait => CompletionItemKind::INTERFACE,
        HuziSymbolKind::Variable | HuziSymbolKind::Param => {
            CompletionItemKind::VARIABLE
        }
    }
}

/// 按 `label` 去重(保留首个,维持稳定顺序)。
fn dedup(items: Vec<CompletionItem>) -> Vec<CompletionItem> {
    let mut seen = HashSet::new();
    items
        .into_iter()
        .filter(|item| seen.insert(item.label.clone()))
        .collect()
}

/// 取光标上下文:前缀词 + 点/双冒号基名(越界返回 `None`)。
fn cursor_context(text: &str, pos: Position) -> Option<CursorCtx> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(items: &[CompletionItem]) -> Vec<&str> {
        items.iter().map(|i| i.label.as_str()).collect()
    }

    #[test]
    fn empty_prefix_offers_keywords() {
        // Given: 空文件,光标在首行首列
        // When: 取补全(空前缀)
        let items = completion_for_text("", Position { line: 0, character: 0 });
        // Then: 含 fn/let 等关键字
        let got = labels(&items);
        assert!(got.contains(&"fn"), "{got:?}");
        assert!(got.contains(&"let"), "{got:?}");
    }

    #[test]
    fn symbol_prefix_filters() {
        // Given: 定义了 factorial 的文件
        let text =
            "fn factorial(n: i32) -> i32 {\n    n\n}\nfac";
        // When: 在末行 `fac` 后取补全
        let items =
            completion_for_text(text, Position { line: 3, character: 3 });
        // Then: 命中 factorial,不含不相關的 let
        let got = labels(&items);
        assert!(got.contains(&"factorial"), "{got:?}");
        assert!(!got.contains(&"let"), "{got:?}");
    }

    #[test]
    fn dot_after_struct_returns_fields() {
        // Given: 含 Point 结构体的文件
        let text = "struct Point { x: i32, y: i32 }\nPoint.";
        // When: 在 `Point.` 后取补全
        let items =
            completion_for_text(text, Position { line: 1, character: 6 });
        // Then: 含字段 x/y
        let got = labels(&items);
        assert!(got.contains(&"x"), "{got:?}");
        assert!(got.contains(&"y"), "{got:?}");
    }

    #[test]
    fn dot_unknown_base_returns_generic_members() {
        // Given: 未知接收者的成员访问
        let text = "let v = 1\nv.";
        // When: 在 `v.` 后取补全
        let items =
            completion_for_text(text, Position { line: 1, character: 2 });
        // Then: 非空(通用成员兜底)
        assert!(!items.is_empty());
    }

    #[test]
    fn bad_file_still_offers_keywords() {
        // Given: 坏文件(残缺 let)
        let text = "let = \n";
        // When: 在首行行首取补全
        let items =
            completion_for_text(text, Position { line: 0, character: 0 });
        // Then: 仍有关键字,不崩
        let got = labels(&items);
        assert!(got.contains(&"fn"), "{got:?}");
        assert!(got.contains(&"let"), "{got:?}");
    }

    #[test]
    fn math_module_returns_trig_fns() {
        // Given: `math::` 前缀
        let text = "math::";
        // When: 在双冒号后取补全
        let items =
            completion_for_text(text, Position { line: 0, character: 6 });
        // Then: 含 sin/cos/sqrt
        let got = labels(&items);
        assert!(got.contains(&"sin"), "{got:?}");
        assert!(got.contains(&"cos"), "{got:?}");
        assert!(got.contains(&"sqrt"), "{got:?}");
    }
}
