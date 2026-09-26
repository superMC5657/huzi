//! 表达式格式化入口:分派与优先级。
//!
//! 子模块:`literal`(字面量)、`ops`(一/二元/调用/索引/字段访问)、
//! `composite`(数组/元组/结构体/枚举/方法/闭包/f-string)、
//! `control`(if/match/内联块与语句)。

mod composite;
mod control;
mod literal;
mod ops;

use composite::{
    format_array_literal, format_closure, format_enum_construct, format_fstring,
    format_method_call, format_struct_literal, format_tuple_literal,
};
use control::{format_if_expr, format_match_expr};
pub(super) use control::format_stmt_inline;
use huzi_ast::*;
use literal::format_literal;
use ops::{
    format_array_index, format_binary, format_call, format_field_access, format_try,
    format_unary,
};

pub(super) fn format_expr(expr: &Expr) -> String {
    match expr {
        Expr::Literal(lit) => format_literal(lit),
        Expr::Ident(s) => s.clone(),
        Expr::Binary(b) => format_binary(b),
        Expr::Unary(u) => format_unary(u),
        Expr::Call(c) => format_call(c),
        Expr::Assign(a) => format!("{} {} {}", format_expr(&a.target), a.operator.as_str(), format_expr(&a.value)),
        Expr::ArrayIndex(a) => format_array_index(a),
        Expr::ArrayLiteral(elems) => format_array_literal(elems),
        Expr::TupleLiteral(elems) => format_tuple_literal(elems),
        Expr::VecEmpty(ty) => format!("vec<{}>()", ty),
        Expr::BoxAlloc(inner) => format!("box({})", format_expr(inner)),
        Expr::Null => "null".to_string(),
        Expr::If(i) => format_if_expr(i),
        Expr::FieldAccess(f) => format_field_access(f),
        Expr::StructLiteral(s) => format_struct_literal(s),
        Expr::EnumConstruct(e) => format_enum_construct(e),
        Expr::Match(m) => format_match_expr(m),
        Expr::MethodCall(m) => format_method_call(m),
        Expr::Try(t) => format_try(t),
        Expr::Closure(c) => format_closure(c),
        Expr::FString(f) => format_fstring(f),
    }
}

pub(super) fn op_precedence(op: &BinOp) -> u8 {
    match op {
        BinOp::Or => 1,
        BinOp::And => 2,
        BinOp::Eq | BinOp::Neq => 3,
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => 4,
        BinOp::Add | BinOp::Sub => 5,
        BinOp::Mul | BinOp::Div | BinOp::Mod => 6,
    }
}

pub(super) fn expr_precedence(expr: &Expr) -> u8 {
    match expr {
        Expr::Binary(b) => op_precedence(&b.operator),
        Expr::Assign(_) => 0,
        Expr::Unary(_) => 7,
        Expr::Try(_) => 8,
        _ => 8,
    }
}
