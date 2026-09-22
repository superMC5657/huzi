//! 错误渲染:把 HuziError 与源码文本组合成带位置摘录的错误报告——
//! 重印出错行并在列位置放置 `^` 指示符,终端输出时带 ANSI 颜色。

use std::io::IsTerminal;

use super::HuziError;

/// 以阶段标签(如 "Lex error")渲染错误。stderr 为终端时自动启用颜色。
pub fn render(error: &HuziError, source: &str, label: &str) -> String {
    render_with_color(error, source, label, std::io::stderr().is_terminal())
}

/// 同 [`render`],但显式控制颜色(供测试与管道输出使用)。
pub fn render_with_color(error: &HuziError, source: &str, label: &str, color: bool) -> String {
    if error.line() == 0 {
        return format!("{}: {}", label, error.message());
    }
    let Some(source_line) = source.lines().nth(error.line() - 1) else {
        // 行号超出源码范围(不应发生),退回单行格式
        return format!("{}: {}", label, error);
    };

    let header = colorize(
        &format!(
            "{} at line {}, column {}: {}",
            label,
            error.line(),
            error.column(),
            error.message()
        ),
        color,
    );

    // lexer 的列号按字符计数(1-based),指示符需对齐到多字节字符之后
    let caret_offset = source_line
        .chars()
        .take(error.column().saturating_sub(1))
        .count();
    let caret: String = " ".repeat(caret_offset) + "^";
    let caret = colorize(&caret, color);

    let line_no = error.line().to_string();
    let gutter = " ".repeat(line_no.len());
    format!(
        "{}\n{} |\n{} | {}\n{} | {}",
        header, gutter, line_no, source_line, gutter, caret
    )
}

fn colorize(text: &str, color: bool) -> String {
    if color {
        format!("\x1b[1;31m{}\x1b[0m", text)
    } else {
        text.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_source_excerpt_with_caret() {
        let error = HuziError::new("Unexpected token", 2, 5);
        let output = render_with_color(&error, "let a = 1;\nlet b = 2;\n", "Parse error", false);
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(lines[0], "Parse error at line 2, column 5: Unexpected token");
        assert_eq!(lines[1], "  |");
        assert_eq!(lines[2], "2 | let b = 2;");
        assert_eq!(lines[3], "  |     ^");
    }

    #[test]
    fn colored_output_wraps_header_in_ansi() {
        let error = HuziError::new("boom", 1, 1);
        let output = render_with_color(&error, "x", "Lex error", true);
        assert!(output.contains("\x1b[1;31mLex error at line 1, column 1: boom\x1b[0m"));
        assert!(output.contains("\x1b[1;31m^\x1b[0m"));
    }

    #[test]
    fn global_errors_stay_single_line() {
        let error = HuziError::new_global("no position");
        let output = render_with_color(&error, "x", "Compile error", false);
        assert_eq!(output, "Compile error: no position");
    }

    #[test]
    fn out_of_range_line_falls_back_to_plain_format() {
        let error = HuziError::new("beyond eof", 9, 1);
        let output = render_with_color(&error, "x", "Parse error", false);
        assert_eq!(
            output,
            "Parse error: Error at line 9, column 1: beyond eof"
        );
    }
}
