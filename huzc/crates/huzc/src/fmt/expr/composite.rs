//! 复合表达式:数组/元组/结构体/枚举构造/方法调用/闭包/f-string。

use super::control::format_inline_block;
use super::format_expr;
use huzi_ast::*;

pub(super) fn format_array_literal(elems: &[Expr]) -> String {
    let formatted: Vec<_> = elems.iter().map(format_expr).collect();
    format!("[{}]", formatted.join(", "))
}

pub(super) fn format_tuple_literal(elems: &[Expr]) -> String {
    if elems.len() == 1 {
        format!("({},)", format_expr(&elems[0]))
    } else {
        let formatted: Vec<_> = elems.iter().map(format_expr).collect();
        format!("({})", formatted.join(", "))
    }
}

pub(super) fn format_struct_literal(s: &StructLiteralExpr) -> String {
    let type_args = if s.type_args.is_empty() {
        String::new()
    } else {
        let ts: Vec<_> = s.type_args.iter().map(|t| t.to_string()).collect();
        format!("<{}>", ts.join(", "))
    };
    let fields: Vec<_> = s
        .fields
        .iter()
        .map(|(name, val)| format!("{}: {}", name, format_expr(val)))
        .collect();
    format!("{}{} {{ {} }}", s.name, type_args, fields.join(", "))
}

pub(super) fn format_enum_construct(e: &EnumConstructExpr) -> String {
    let type_args_str = if e.type_args.is_empty() {
        String::new()
    } else {
        let targs: Vec<_> = e.type_args.iter().map(|t| t.to_string()).collect();
        format!("<{}>", targs.join(", "))
    };
    if e.args.is_empty() {
        format!("{}{}::{}", e.enum_name, type_args_str, e.variant)
    } else {
        let args: Vec<_> = e.args.iter().map(format_expr).collect();
        format!("{}{}::{}({})", e.enum_name, type_args_str, e.variant, args.join(", "))
    }
}

pub(super) fn format_method_call(m: &MethodCallExpr) -> String {
    let receiver = format_expr(&m.receiver);
    let args: Vec<_> = m.arguments.iter().map(format_expr).collect();
    format!("{}.{}({})", receiver, m.method, args.join(", "))
}

pub(super) fn format_closure(c: &ClosureExpr) -> String {
    let params: Vec<String> = c
        .params
        .iter()
        .map(|p| {
            if let Some(t) = &p.param_type {
                format!("{}: {}", p.name, t)
            } else {
                p.name.clone()
            }
        })
        .collect();
    let pipe_part = if params.is_empty() {
        "||".to_string()
    } else {
        format!("|{}|", params.join(", "))
    };
    let ret_part = if let Some(ret) = &c.return_type {
        format!(" -> {}", ret)
    } else {
        String::new()
    };
    match &c.body {
        ClosureBody::Expr(e) => format!("{}{} {}", pipe_part, ret_part, format_expr(e)),
        ClosureBody::Block(b) => {
            let inner = format_inline_block(b);
            if inner.is_empty() {
                format!("{}{} {{}}", pipe_part, ret_part)
            } else {
                format!("{}{} {{ {} }}", pipe_part, ret_part, inner)
            }
        }
    }
}

pub(super) fn format_fstring(f: &FStringExpr) -> String {
    let mut out = String::from("f\"");
    let chars: Vec<char> = f.template.chars().collect();
    let n = chars.len();
    let mut i = 0;
    let mut arg_idx = 0;
    while i < n {
        if chars[i] == '{' {
            if i + 1 < n && chars[i + 1] == '{' {
                out.push_str("{{");
                i += 2;
            } else if i + 1 < n && chars[i + 1] == '}' {
                out.push('{');
                if arg_idx < f.args.len() {
                    out.push_str(&format_expr(&f.args[arg_idx]));
                    arg_idx += 1;
                }
                out.push('}');
                i += 2;
            } else {
                out.push(chars[i]);
                i += 1;
            }
        } else if chars[i] == '}' {
            if i + 1 < n && chars[i + 1] == '}' {
                out.push_str("}}");
                i += 2;
            } else {
                out.push(chars[i]);
                i += 1;
            }
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out.push('"');
    out
}
