//! 运算符与调用类表达式:二元/一元/`?`/函数调用/索引/字段访问。

use super::{expr_precedence, format_expr, op_precedence};
use huzi_ast::*;

/// 后缀 `?`:内层为低优先级表达式时加括号,如 `(a + b)?`。
pub(super) fn format_try(t: &TryExpr) -> String {
    let inner_str = format_expr(&t.inner);
    if expr_precedence(&t.inner) < 8 {
        format!("({})?", inner_str)
    } else {
        format!("{}?", inner_str)
    }
}

pub(super) fn format_binary(b: &BinaryExpr) -> String {
    let parent_prec = op_precedence(&b.operator);
    let left_str = format_expr(&b.left);
    let left = if expr_precedence(&b.left) < parent_prec {
        format!("({})", left_str)
    } else {
        left_str
    };
    let right_str = format_expr(&b.right);
    let right = if expr_precedence(&b.right) <= parent_prec {
        format!("({})", right_str)
    } else {
        right_str
    };
    let op_str = match b.operator {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Mod => "%",
        BinOp::Eq => "==",
        BinOp::Neq => "!=",
        BinOp::Lt => "<",
        BinOp::Le => "<=",
        BinOp::Gt => ">",
        BinOp::Ge => ">=",
        BinOp::And => "&&",
        BinOp::Or => "||",
    };
    format!("{} {} {}", left, op_str, right)
}

pub(super) fn format_unary(u: &UnaryExpr) -> String {
    let op_str = match u.operator {
        UnOp::Neg => "-",
        UnOp::Not => "!",
        UnOp::Deref => "*",
    };
    let inner_str = format_expr(&u.operand);
    if expr_precedence(&u.operand) < 7 {
        format!("{}({})", op_str, inner_str)
    } else {
        format!("{}{}", op_str, inner_str)
    }
}

pub(super) fn format_call(c: &CallExpr) -> String {
    let callee = format_expr(&c.callee);
    let type_args = if c.type_args.is_empty() {
        String::new()
    } else {
        let ts: Vec<_> = c.type_args.iter().map(|t| t.to_string()).collect();
        format!("<{}>", ts.join(", "))
    };
    let args = c
        .arguments
        .iter()
        .map(format_expr)
        .collect::<Vec<_>>()
        .join(", ");
    format!("{}{}({})", callee, type_args, args)
}

pub(super) fn format_array_index(a: &ArrayIndexExpr) -> String {
    let base = format_expr(&a.array);
    let base = if expr_precedence(&a.array) < 8 {
        format!("({})", base)
    } else {
        base
    };
    format!("{}[{}]", base, format_expr(&a.index))
}

pub(super) fn format_field_access(f: &FieldAccessExpr) -> String {
    let base = format_expr(&f.base);
    let base = if expr_precedence(&f.base) < 8 {
        format!("({})", base)
    } else {
        base
    };
    format!("{}.{}", base, f.field)
}
