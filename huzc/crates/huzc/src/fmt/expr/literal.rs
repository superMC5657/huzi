//! 字面量格式化:整数/浮点/布尔/字符串/字符。

use huzi_ast::*;

pub(super) fn format_literal(lit: &Literal) -> String {
    match lit {
        Literal::Int(n) => n.to_string(),
        Literal::Float(f) => {
            let s = f.to_string();
            if !s.contains('.') && !s.contains('e') && !s.contains('E') {
                format!("{}.0", s)
            } else {
                s
            }
        }
        Literal::Bool(b) => if *b { "true".to_string() } else { "false".to_string() },
        Literal::String(s) => format_string_lit(s),
        Literal::Char(c) => format_char_lit(*c),
    }
}

fn format_string_lit(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\0' => out.push_str("\\0"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

fn format_char_lit(c: char) -> String {
    let mut out = String::new();
    out.push('\'');
    match c {
        '\n' => out.push_str("\\n"),
        '\t' => out.push_str("\\t"),
        '\r' => out.push_str("\\r"),
        '\\' => out.push_str("\\\\"),
        '\'' => out.push_str("\\'"),
        '\0' => out.push_str("\\0"),
        other => out.push(other),
    }
    out.push('\'');
    out
}
