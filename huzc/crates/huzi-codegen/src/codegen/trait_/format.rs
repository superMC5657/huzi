use super::TraitDesugarer;
use huzi_ast::*;
use huzi_error::{HuziError, Result};
use std::collections::HashMap;

enum FormatPart {
    Text(String),
    Placeholder,
}

impl TraitDesugarer {
    /// 将 `format("...", args...)` 内置调用编译期脱糖展开为高效的 `concat(...)` 与 `to_string(...)` 调用。
    pub(super) fn desugar_format_call(
        &self,
        c: &CallExpr,
        env: &HashMap<String, Type>,
    ) -> Result<Expr> {
        if c.arguments.is_empty() {
            return Err(HuziError::new_global(
                "format() 期望至少 1 个参数 (模板字符串)",
            ));
        }

        let tpl = match &c.arguments[0] {
            Expr::Literal(Literal::String(s)) => s,
            _ => {
                return Err(HuziError::new_global(
                    "format() 的首个参数必须为字符串字面量模板",
                ));
            }
        };

        let (static_parts, placeholder_count) = parse_format_template(tpl)?;
        let provided_args = c.arguments.len() - 1;
        if placeholder_count != provided_args {
            return Err(HuziError::new_global(format!(
                "format() 模板包含 {} 个占位符, 实际提供了 {} 个参数",
                placeholder_count, provided_args
            )));
        }

        let mut chunks: Vec<Expr> = Vec::new();
        let mut arg_idx = 1;

        for part in static_parts {
            match part {
                FormatPart::Text(t) => {
                    if !t.is_empty() {
                        chunks.push(Expr::Literal(Literal::String(t)));
                    }
                }
                FormatPart::Placeholder => {
                    let mut arg = c.arguments[arg_idx].clone();
                    self.resolve_expr(&mut arg, env)?;
                    let arg_ty = self.infer_expr_type(&arg, env);
                    let formatted_arg = if arg_ty == Some(Type::Str) {
                        arg
                    } else {
                        Expr::Call(CallExpr {
                            callee: Box::new(Expr::Ident("to_string".to_string())),
                            arguments: vec![arg],
                            type_args: Vec::new(),
                        })
                    };
                    chunks.push(formatted_arg);
                    arg_idx += 1;
                }
            }
        }

        if chunks.is_empty() {
            Ok(Expr::Literal(Literal::String(String::new())))
        } else if chunks.len() == 1 {
            Ok(chunks.remove(0))
        } else {
            Ok(Expr::Call(CallExpr {
                callee: Box::new(Expr::Ident("concat".to_string())),
                arguments: chunks,
                type_args: Vec::new(),
            }))
        }
    }

    /// 将 `f"..."` 插值表达式脱糖展开。
    pub(super) fn desugar_fstring(
        &self,
        fs: &FStringExpr,
        env: &HashMap<String, Type>,
    ) -> Result<Expr> {
        let mut call_args = vec![Expr::Literal(Literal::String(fs.template.clone()))];
        call_args.extend(fs.args.clone());
        let call = CallExpr {
            callee: Box::new(Expr::Ident("format".to_string())),
            arguments: call_args,
            type_args: Vec::new(),
        };
        self.desugar_format_call(&call, env)
    }
}

/// 解析模板中的静态片段与占位符，支持 `{{` 和 `}}` 转义。
fn parse_format_template(tpl: &str) -> Result<(Vec<FormatPart>, usize)> {
    let mut parts = Vec::new();
    let mut current_text = String::new();
    let mut count = 0;
    let chars: Vec<char> = tpl.chars().collect();
    let n = chars.len();
    let mut i = 0;

    while i < n {
        if chars[i] == '{' {
            if i + 1 < n && chars[i + 1] == '{' {
                current_text.push('{');
                i += 2;
            } else if i + 1 < n && chars[i + 1] == '}' {
                if !current_text.is_empty() {
                    parts.push(FormatPart::Text(std::mem::take(&mut current_text)));
                }
                parts.push(FormatPart::Placeholder);
                count += 1;
                i += 2;
            } else {
                return Err(HuziError::new_global(
                    "format() 模板中的占位符语法错误: 期望 '{}' 或 '{{'",
                ));
            }
        } else if chars[i] == '}' {
            if i + 1 < n && chars[i + 1] == '}' {
                current_text.push('}');
                i += 2;
            } else {
                return Err(HuziError::new_global(
                    "format() 模板中发现未配对的 '}'; 若要输出字面量 '}', 请使用 '}}'",
                ));
            }
        } else {
            current_text.push(chars[i]);
            i += 1;
        }
    }

    if !current_text.is_empty() {
        parts.push(FormatPart::Text(current_text));
    }

    Ok((parts, count))
}
