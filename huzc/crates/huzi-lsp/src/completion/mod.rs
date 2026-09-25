//! Completion:关键字/符号/成员/`math::` 补全(坏文件仍可用)。
//!
//! 纯函数 [`completion_for_text`]:全文 + 光标 -> 补全项。
//! 解析失败时符号表为空,但关键字照常返回,保证坏文件可补。
//! `:`/`::` 与 `.` 上下文由本文件自行做 UTF-16 折算,不依赖
//! [`crate::mapping::word_at_position`](其要求光标落在词内,
//! 而补全光标常在词尾/点号后)。
//!
//! 子模块:`items`(各上下文补全项组装)、`cursor`(光标上下文与词边界)。

mod cursor;
mod items;

use std::collections::HashSet;

use tower_lsp_server::ls_types::{
    CompletionItem, Position,
};

use crate::analysis::parse_and_collect;
use cursor::cursor_context;
use items::{colon_items, dot_items, normal_items};

/// 补全关键字表(与语法关键字对齐)。
const KEYWORDS: &[&str] = &[
    "fn", "let", "mut", "if", "elif", "else", "for", "in", "while",
    "return", "break", "continue", "defer", "import", "export", "match", "struct", "enum",
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

/// 按 `label` 去重(保留首个,维持稳定顺序)。
fn dedup(items: Vec<CompletionItem>) -> Vec<CompletionItem> {
    let mut seen = HashSet::new();
    items
        .into_iter()
        .filter(|item| seen.insert(item.label.clone()))
        .collect()
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

    #[test]
    fn enum_double_colon_returns_variants() {
        // Given: 含 Color 枚举的文件(L1 锁定:存量 `::` 变体行为)
        let text = "enum Color { Red, Green }\nColor::";
        // When: 在 `Color::` 后取补全
        let items =
            completion_for_text(text, Position { line: 1, character: 7 });
        // Then: 含 Red/Green
        let got = labels(&items);
        assert!(got.contains(&"Red"), "{got:?}");
        assert!(got.contains(&"Green"), "{got:?}");
    }

    #[test]
    fn dot_prefix_filters_struct_fields() {
        // Given: 含 Point 结构体的文件(L1 锁定:前缀过滤行为)
        let text = "struct Point { x: i32, y: i32 }\nPoint.x";
        // When: 在 `Point.x` 后取补全
        let items =
            completion_for_text(text, Position { line: 1, character: 7 });
        // Then: 仅含 x,不含 y
        let got = labels(&items);
        assert!(got.contains(&"x"), "{got:?}");
        assert!(!got.contains(&"y"), "{got:?}");
    }

    #[test]
    fn out_of_range_position_returns_empty() {
        // Given: 普通文件(L1 锁定:越界永不 panic)
        let text = "fn f() -> i32 {\n return 1\n}\n";
        // When: 行号越界
        let items =
            completion_for_text(text, Position { line: 99, character: 0 });
        // Then: 空表
        assert!(items.is_empty());
    }

    #[test]
    fn dot_after_struct_includes_impl_methods() {
        // Given: Point 结构体 + Printable 实现(L4)
        let text = "struct Point { x: i32, y: i32 }\nimpl Printable for Point {\n fn show(self: Point) -> str {\n return \"p\"\n }\n}\nPoint.";
        // When: 在 `Point.` 后取补全
        let items =
            completion_for_text(text, Position { line: 6, character: 6 });
        // Then: 字段与 impl 方法并存
        let got = labels(&items);
        assert!(got.contains(&"x"), "{got:?}");
        assert!(got.contains(&"show"), "{got:?}");
    }

    #[test]
    fn trait_double_colon_returns_trait_methods() {
        // Given: Printable trait 定义(L4)
        let text =
            "trait Printable {\n fn show(self) -> str\n}\nPrintable::";
        // When: 在 `Printable::` 后取补全
        let items =
            completion_for_text(text, Position { line: 3, character: 11 });
        // Then: 含 trait 方法 show
        let got = labels(&items);
        assert!(got.contains(&"show"), "{got:?}");
    }
    #[test]
    fn dot_after_let_typed_returns_fields_and_impl() {
        for tail in ["let p: Point\np.", "let p: Point = Point { x: 1, y: 2 }\np.", "let p = Point { x: 1, y: 2 }\np."] {
            let text = format!("struct Point {{ x: i32, y: i32 }}\nimpl Printable for Point {{\n fn show(self: Point) -> str {{\n return \"p\"\n }}\n}}\n{tail}"); let items = completion_for_text(&text, Position { line: 7, character: 2 }); let got = labels(&items);
            assert!(got.contains(&"x") && got.contains(&"y") && got.contains(&"show"), "{got:?} {tail}");
        }
    }
    #[test]
    fn dot_after_unknown_var_falls_back_to_generic() {
        let items = completion_for_text("let q = 1\nq.", Position { line: 1, character: 2 }); let got = labels(&items);
        assert!(got.contains(&"len"), "{got:?}");
    }
}
