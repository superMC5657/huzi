//! Backend 单测(自 `backend.rs` 纯搬移,零逻辑变化)。

use super::*;

const FACT: &str =
    "fn factorial(n: i32) -> i32 {\n    n\n}\nfn main() -> i32 {\n    factorial(1)\n}\n";

#[test]
fn hover_on_fn_name_shows_signature() {
    // Given: 递归函数定义的全文
    // When: hover 首行 fn 名(0-based 行 0,列 3)
    let hover = hover_for_text(FACT, Position { line: 0, character: 3 });
    // Then: 返回 markdown 签名(含 detail 与 kind)
    let hover = hover.expect("hover hits fn name");
    match hover.contents {
        HoverContents::Markup(markup) => {
            assert!(markup.value.contains("fn factorial"), "{markup:?}");
            assert!(markup.value.contains("Function"), "{markup:?}");
        }
        other => panic!("expected markup, got {other:?}"),
    }
}

#[test]
fn hover_miss_returns_none() {
    // Given: 同上全文与空文件
    // When: 落在缩进空白 / 关键字 `fn` 无符号处 / 空文件
    // Then: 一律 None,不抛错
    assert!(hover_for_text(FACT, Position { line: 1, character: 0 }).is_none());
    assert!(hover_for_text(FACT, Position { line: 3, character: 0 }).is_none());
    assert!(hover_for_text("", Position { line: 0, character: 0 }).is_none());
    assert!(hover_for_text("let = \n", Position { line: 0, character: 0 }).is_none());
}

#[test]
fn definition_range_is_never_inverted() {
    // Given: main 内调用 factorial 的全文
    // When: 在调用点(0-based 行 4,列 5)请求定义
    let range = definition_range_for_text(FACT, Position { line: 4, character: 5 })
        .expect("definition found");
    // Then: 区间非倒置,且落在首行 fn 定义处
    assert!(
        (range.end.line, range.end.character)
            >= (range.start.line, range.start.character),
        "{range:?}"
    );
    assert_eq!(range.start.line, 0);
}

#[test]
fn document_symbol_lists_outline() {
    // Given: 含 fn 与顶层 let 的全文
    let text = "fn factorial(n: i32) -> i32 {\n    n\n}\nlet x = 1\n";
    // When: 取 Outline
    let syms = symbols_for_text(text);
    // Then: 非空,fn 映射为 Function,变量映射为 Variable
    assert!(!syms.is_empty());
    let fun = syms.iter().find(|s| s.name == "factorial").expect("fn symbol");
    assert_eq!(fun.kind, SymbolKind::FUNCTION);
    assert!(fun.detail.as_deref().unwrap_or_default().contains("fn factorial"));
    let var = syms.iter().find(|s| s.name == "x").expect("let symbol");
    assert_eq!(var.kind, SymbolKind::VARIABLE);
    for s in &syms {
        assert!(
            (s.range.end.line, s.range.end.character)
                >= (s.range.start.line, s.range.start.character),
            "inverted range for {}",
            s.name
        );
    }
}

#[test]
fn hover_after_chinese_line_stays_aligned() {
    // Given: 首行为中文注释,次行定义函数
    let text = "# 递归函数示例中文行\nfn factorial(n: i32) -> i32 {\n    n\n}\n";
    // When: hover 次行 fn 名(0-based 行 1,列 3)
    let hover = hover_for_text(text, Position { line: 1, character: 3 });
    // Then: 命中签名,不因中文行错位
    let hover = hover.expect("hover aligned after chinese line");
    match hover.contents {
        HoverContents::Markup(markup) => {
            assert!(markup.value.contains("fn factorial"), "{markup:?}");
        }
        other => panic!("expected markup, got {other:?}"),
    }
    // 同行中文标识符同样可取词命中
    let text2 = "let 变量 = 1\n变量\n";
    let hover2 = hover_for_text(text2, Position { line: 1, character: 1 })
        .expect("chinese ident hover");
    match hover2.contents {
        HoverContents::Markup(markup) => {
            assert!(markup.value.contains("变量"), "{markup:?}");
        }
        other => panic!("expected markup, got {other:?}"),
    }
}

#[test]
fn formatting_returns_full_document_edit() {
    let text = "fn main()->i32{return 1}\n";
    let edits = format_for_text(text).expect("formatting succeeds");
    assert_eq!(edits.len(), 1);
    assert!(edits[0].new_text.contains("fn main() -> i32 {\n    return 1\n}"));
}
