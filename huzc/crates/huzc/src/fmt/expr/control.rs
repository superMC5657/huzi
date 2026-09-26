//! 控制流表达式与内联语句:if/match/内联块。

use super::format_expr;
use huzi_ast::*;

pub(super) fn format_inline_block(b: &Block) -> String {
    if b.statements.len() == 1 {
        if let Stmt::Expr(e) = &b.statements[0].node {
            return format_expr(&e.expr);
        }
    }
    let stmts: Vec<_> = b
        .statements
        .iter()
        .map(|s| format_stmt_inline(&s.node))
        .collect();
    stmts.join("; ")
}

// 注:E0364 回避——`format_stmt_inline` 需经 `expr/mod.rs` 重导出给
// 同级 `block` 模块(`use super::expr::*`),`pub(super)` 重导出
// 私有项会触发 E0364,故此处放宽为 `pub(crate)`(行为零变化)。
pub(crate) fn format_stmt_inline(stmt: &Stmt) -> String {
    match stmt {
        Stmt::Let(l) => {
            let mut s = String::from("let ");
            if l.mutable {
                s.push_str("mut ");
            }
            s.push_str(&l.name);
            if let Some(t) = &l.type_annotation {
                s.push_str(&format!(": {}", t));
            }
            if let Some(v) = &l.value {
                s.push_str(&format!(" = {}", format_expr(v)));
            }
            s
        }
        Stmt::Expr(e) => format_expr(&e.expr),
        Stmt::Return(r) => match &r.value {
            Some(v) => format!("return {}", format_expr(v)),
            None => "return".to_string(),
        },
        Stmt::Break => "break".to_string(),
        Stmt::Continue => "continue".to_string(),
        Stmt::Defer(inner) => format!("defer {}", format_stmt_inline(&inner.node)),
        _ => "<complex_stmt>".to_string(),
    }
}

fn extract_if_expr(b: &Block) -> Option<&IfExpr> {
    if b.statements.len() == 1 {
        if let Stmt::Expr(ExprStmt {
            expr: Expr::If(ref nested),
        }) = b.statements[0].node
        {
            return Some(nested);
        }
    }
    None
}

pub(super) fn format_if_expr(i: &IfExpr) -> String {
    let mut out = format!(
        "if {} {{ {} }}",
        format_expr(&i.condition),
        format_inline_block(&i.then_branch)
    );
    let mut cur = i;
    while let Some(nested) = extract_if_expr(&cur.else_branch) {
        out.push_str(&format!(
            " elif {} {{ {} }}",
            format_expr(&nested.condition),
            format_inline_block(&nested.then_branch)
        ));
        cur = nested;
    }
    out.push_str(&format!(
        " else {{ {} }}",
        format_inline_block(&cur.else_branch)
    ));
    out
}

fn format_pattern(p: &Pattern) -> String {
    match p {
        Pattern::Wildcard => "_".to_string(),
        Pattern::Variable(name) => name.clone(),
        Pattern::Literal(lit) => super::literal::format_literal(lit),
        Pattern::Variant {
            enum_name,
            variant,
            bindings,
        } => {
            if bindings.is_empty() {
                format!("{}::{}", enum_name, variant)
            } else {
                format!("{}::{}({})", enum_name, variant, bindings.join(", "))
            }
        }
    }
}

pub(super) fn format_match_expr(m: &MatchExpr) -> String {
    let mut out = format!("match {} {{\n", format_expr(&m.scrutinee));
    for arm in &m.arms {
        let pat_str = format_pattern(&arm.pattern);
        let arm_head = if let Some(guard) = &arm.guard {
            format!("{} if {}", pat_str, format_expr(guard))
        } else {
            pat_str
        };
        if arm.body.statements.len() == 1 {
            if let Stmt::Expr(e) = &arm.body.statements[0].node {
                out.push_str(&format!("    {} => {},\n", arm_head, format_expr(&e.expr)));
                continue;
            }
        }
        out.push_str(&format!("    {} => {{\n", arm_head));
        for s in &arm.body.statements {
            out.push_str(&format!("        {}\n", format_stmt_inline(&s.node)));
        }
        out.push_str("    },\n");
    }
    out.push('}');
    out
}
