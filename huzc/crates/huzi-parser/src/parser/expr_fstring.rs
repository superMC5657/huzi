use super::Parser;
use huzi_ast::*;
use huzi_error::{HuziError, Result};

impl Parser {
    /// 解析插值字符串 `f"..."` 并直接解糖为对内置函数 `format("...", args...)` 的调用。
    pub(super) fn parse_fstring(&mut self, s: &str) -> Result<Expr> {
        let mut template = String::new();
        let mut args = Vec::new();
        let chars: Vec<char> = s.chars().collect();
        let n = chars.len();
        let mut i = 0;

        while i < n {
            if chars[i] == '{' {
                if i + 1 < n && chars[i + 1] == '{' {
                    template.push_str("{{");
                    i += 2;
                } else {
                    i += 1;
                    let (expr_str, next_i) = extract_placeholder_expr(&chars, i, self.current_line(), self.current_col())?;
                    i = next_i;
                    let expr = parse_sub_expression(&expr_str, self.current_line(), self.current_col())?;
                    template.push_str("{}");
                    args.push(expr);
                }
            } else if chars[i] == '}' {
                if i + 1 < n && chars[i + 1] == '}' {
                    template.push_str("}}");
                    i += 2;
                } else {
                    return Err(HuziError::new(
                        "Unmatched '}' in format string; use '}}' to escape",
                        self.current_line(),
                        self.current_col(),
                    ));
                }
            } else {
                template.push(chars[i]);
                i += 1;
            }
        }

        Ok(Expr::FString(FStringExpr { template, args }))
    }
}

/// 提取花括号中的表达式文本，支持花括号配对嵌套。
fn extract_placeholder_expr(
    chars: &[char],
    start: usize,
    line: usize,
    col: usize,
) -> Result<(String, usize)> {
    let mut depth = 1;
    let mut expr_str = String::new();
    let mut i = start;
    let n = chars.len();

    while i < n && depth > 0 {
        if chars[i] == '{' {
            depth += 1;
            expr_str.push('{');
        } else if chars[i] == '}' {
            depth -= 1;
            if depth > 0 {
                expr_str.push('}');
            }
        } else {
            expr_str.push(chars[i]);
        }
        i += 1;
    }

    if depth != 0 {
        return Err(HuziError::new("Unclosed '{' in format string", line, col));
    }

    let trimmed = expr_str.trim();
    if trimmed.is_empty() {
        return Err(HuziError::new(
            "Empty expression in format string; use '{{}}' to escape",
            line,
            col,
        ));
    }

    Ok((trimmed.to_string(), i))
}

/// 解析内联表达式字符串为 AST 表达式。
fn parse_sub_expression(code: &str, line: usize, col: usize) -> Result<Expr> {
    let mut lexer = huzi_lexer::Lexer::new(code.to_string());
    let tokens = lexer
        .tokenize()
        .map_err(|e| HuziError::new(format!("In format string: {}", e.message()), line, col))?;
    let mut parser = Parser::new(tokens);
    parser
        .parse_expression()
        .map_err(|e| HuziError::new(format!("In format string: {}", e.message()), line, col))
}
