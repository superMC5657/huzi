//! 全文诊断纯函数:源码文本 -> 原始诊断列表。
//!
//! 与 IO/LSP 无关,便于单测。位置沿用 Huzi 约定
//! (1-based 行 / 1-based 字符列),由 [`crate::mapping`] 转 LSP 坐标。

use huzi_ast::{Program, Symbol, collect_symbols};

/// 单条原始诊断:(起始行, 起始列, 结束行, 结束列, 消息)。
/// 行列均为 1-based;`line == 0` 表示词法/语法层给出的全局错误。
pub type RawDiagnostic = (usize, usize, usize, usize, String);

/// 检查全文,返回全部诊断(词法失败即单错短路返回)。
pub fn check_text(text: &str) -> Vec<RawDiagnostic> {
    let mut lexer = huzi_lexer::Lexer::new(text.to_string());
    let tokens = match lexer.tokenize() {
        Ok(tokens) => tokens,
        Err(err) => return vec![single_spot(&err)],
    };
    let mut parser = huzi_parser::Parser::new(tokens);
    let (_program, errors) = parser.parse_recoverable();
    errors.iter().map(single_spot).collect()
}

/// 解析全文并收集符号(hover/跳转/Outline 共用)。
///
/// 词法失败时返回空程序与空符号表,保证坏文件不崩;
/// 语法层走 `parse_recoverable`,残缺程序仍保留已成功语句的符号。
pub fn parse_and_collect(text: &str) -> (Program, Vec<Symbol>) {
    let mut lexer = huzi_lexer::Lexer::new(text.to_string());
    let tokens = match lexer.tokenize() {
        Ok(tokens) => tokens,
        Err(_) => return (Program { statements: Vec::new() }, Vec::new()),
    };
    let mut parser = huzi_parser::Parser::new(tokens);
    let (program, _errors) = parser.parse_recoverable();
    let symbols = collect_symbols(&program);
    (program, symbols)
}

/// 单个 [`huzi_error::HuziError`] 转单字符宽度的点区间。
fn single_spot(err: &huzi_error::HuziError) -> RawDiagnostic {
    let line = normalize_line(err.line());
    let col = normalize_col(err.column());
    (line, col, line, col + 1, err.message().to_string())
}

/// 行号归一化:0(全局)与哨兵 `usize::MAX` 均钳到首行。
fn normalize_line(line: usize) -> usize {
    if line == 0 || line == usize::MAX {
        1
    } else {
        line
    }
}

/// 列号归一化:0 与哨兵钳到首列。
fn normalize_col(col: usize) -> usize {
    if col == 0 || col == usize::MAX {
        1
    } else {
        col
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_file_has_no_diagnostics() {
        // Given: 空文件与纯空白文件
        // When: 全文检查
        // Then: 零诊断
        assert!(check_text("").is_empty());
        assert!(check_text("   \n  \n").is_empty());
    }

    #[test]
    fn good_file_has_no_diagnostics() {
        // Given: 合法 Huzi 程序
        let text = "fn main() -> i32 {\n    let x = 10\n    x\n}\n";
        // When: 全文检查
        // Then: 零诊断
        assert!(check_text(text).is_empty());
    }

    #[test]
    fn two_bad_lines_report_two_errors() {
        // Given: 两行各缺变量名的 let 语句(错误点行内闭合)
        let text = "let = 1\nlet = 2\n";
        // When: 全文检查(错误恢复应跨行继续)
        let diags = check_text(text);
        // Then: 两行各一错,且行号分别为 1、2
        assert_eq!(diags.len(), 2);
        assert_eq!(diags[0].0, 1);
        assert_eq!(diags[1].0, 2);
    }

    #[test]
    fn lexer_failure_short_circuits_to_single_error() {
        // Given: 首行含非法字符(词法层即失败)
        let text = "let x = @\nlet y = 1\n";
        // When: 全文检查
        let diags = check_text(text);
        // Then: 单错短路,定位首行
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].0, 1);
    }
}
