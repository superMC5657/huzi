//! `--dump-parse-stats` 九维向量：与 hzparse `summarize` 同口径。
//!
//! - `fns` 含顶层/嵌套 `fn` 与 `impl` 方法；`trait` 方法签名不计。
//! - `structs/enums/traits/impls/imports/exports/lets` 为对应语句计数（含嵌套块）。
//! - `depth` 为解析期 `parse_expression` 嵌套峰值（由调用方传入，对标 hzparse `pz_expr` 的 maxd）。
//! - `FString` 内联参数不计（hzparse 视 `f"..."` 为不透明串）。

use huzi_ast::{Block, Expr, Program, Stmt};

/// 九维统计向量。
pub struct ParseStats {
    pub fns: usize,
    pub structs: usize,
    pub enums: usize,
    pub traits: usize,
    pub impls: usize,
    pub imports: usize,
    pub exports: usize,
    pub lets: usize,
    pub depth: usize,
}

impl ParseStats {
    fn blank(depth: usize) -> Self {
        Self {
            fns: 0,
            structs: 0,
            enums: 0,
            traits: 0,
            impls: 0,
            imports: 0,
            exports: 0,
            lets: 0,
            depth,
        }
    }

    /// 与 hzparse `summarize` 逐字节一致的单行格式。
    pub fn format(&self) -> String {
        format!(
            "fns={} structs={} enums={} traits={} impls={} imports={} exports={} lets={} depth={}",
            self.fns,
            self.structs,
            self.enums,
            self.traits,
            self.impls,
            self.imports,
            self.exports,
            self.lets,
            self.depth
        )
    }
}

/// 收集整程序的九维统计（含嵌套块内语句）。
pub fn collect(program: &Program, depth: usize) -> ParseStats {
    let mut st = ParseStats::blank(depth);
    for s in &program.statements {
        count_stmt(&s.node, &mut st);
    }
    st
}

/// 分发单条语句计数（含嵌套块递归）。
fn count_stmt(stmt: &Stmt, st: &mut ParseStats) {
    match stmt {
        Stmt::Fn(f) => {
            st.fns += 1;
            count_block(&f.body, st);
        }
        Stmt::Struct(_) => st.structs += 1,
        Stmt::Enum(_) => st.enums += 1,
        Stmt::Trait(_) => st.traits += 1,
        Stmt::Impl(b) => count_impl(b, st),
        Stmt::Import(_) => st.imports += 1,
        Stmt::Export(_) => st.exports += 1,
        Stmt::Let(l) => {
            st.lets += 1;
            if let Some(v) = &l.value {
                count_expr(v, st);
            }
        }
        Stmt::Expr(e) => count_expr(&e.expr, st),
        Stmt::Return(r) => {
            if let Some(v) = &r.value {
                count_expr(v, st);
            }
        }
        Stmt::Block(b) => count_block(b, st),
        Stmt::If(i) => count_if(i, st),
        Stmt::For(f) => count_for(f, st),
        Stmt::While(w) => {
            count_expr(&w.condition, st);
            count_block(&w.body, st);
        }
        Stmt::Defer(inner) => count_stmt(&inner.node, st),
        Stmt::Break | Stmt::Continue => {}
    }
}

/// `impl` 块：块计数 + 方法按 `fn` 计数并递归方法体。
fn count_impl(b: &huzi_ast::ImplBlock, st: &mut ParseStats) {
    st.impls += 1;
    for m in &b.methods {
        st.fns += 1;
        count_block(&m.body, st);
    }
}

/// `if` 语句：条件表达式 + 各分支块。
fn count_if(i: &huzi_ast::IfStmt, st: &mut ParseStats) {
    count_expr(&i.condition, st);
    count_block(&i.then_branch, st);
    for (c, b) in &i.elif_branches {
        count_expr(c, st);
        count_block(b, st);
    }
    if let Some(b) = &i.else_branch {
        count_block(b, st);
    }
}

/// `for` 语句：迭代源表达式 + 循环体块。
fn count_for(f: &huzi_ast::ForStmt, st: &mut ParseStats) {
    match &f.source {
        huzi_ast::ForSource::Range { start, end } => {
            count_expr(start, st);
            count_expr(end, st);
        }
        huzi_ast::ForSource::Array(e) => count_expr(e, st),
    }
    count_block(&f.body, st);
}

/// 块内逐语句计数。
fn count_block(block: &Block, st: &mut ParseStats) {
    for s in &block.statements {
        count_stmt(&s.node, st);
    }
}

/// 表达式内嵌套块/语句的递归（找 `if/match/闭包` 体内的 `let/fn` 等）。
fn count_expr(expr: &Expr, st: &mut ParseStats) {
    match expr {
        Expr::Literal(_) | Expr::Ident(_) | Expr::Null | Expr::VecEmpty(_) => {}
        Expr::Binary(b) => {
            count_expr(&b.left, st);
            count_expr(&b.right, st);
        }
        Expr::Unary(u) => count_expr(&u.operand, st),
        Expr::Call(c) => count_call(&c.callee, &c.arguments, st),
        Expr::Assign(a) => {
            count_expr(&a.target, st);
            count_expr(&a.value, st);
        }
        Expr::ArrayIndex(a) => {
            count_expr(&a.array, st);
            count_expr(&a.index, st);
        }
        Expr::ArrayLiteral(es) | Expr::TupleLiteral(es) => count_expr_list(es, st),
        Expr::BoxAlloc(inner) => count_expr(inner, st),
        Expr::Try(t) => count_expr(&t.inner, st),
        Expr::FieldAccess(f) => count_expr(&f.base, st),
        Expr::If(i) => count_if_expr(i, st),
        Expr::StructLiteral(s) => {
            for (_, v) in &s.fields {
                count_expr(v, st);
            }
        }
        Expr::EnumConstruct(e) => count_expr_list(&e.args, st),
        Expr::Match(m) => count_match(m, st),
        Expr::MethodCall(m) => {
            count_expr(&m.receiver, st);
            count_expr_list(&m.arguments, st);
        }
        Expr::Closure(c) => match &c.body {
            huzi_ast::ClosureBody::Expr(e) => count_expr(e, st),
            huzi_ast::ClosureBody::Block(b) => count_block(b, st),
        },
        Expr::FString(_) => {}
    }
}

/// 调用形：被调方 + 实参表。
fn count_call(callee: &Expr, args: &[Expr], st: &mut ParseStats) {
    count_expr(callee, st);
    count_expr_list(args, st);
}

/// 表达式列表逐项计数。
fn count_expr_list(es: &[Expr], st: &mut ParseStats) {
    for e in es {
        count_expr(e, st);
    }
}

/// `if` 表达式：条件 + 两分支块。
fn count_if_expr(i: &huzi_ast::IfExpr, st: &mut ParseStats) {
    count_expr(&i.condition, st);
    count_block(&i.then_branch, st);
    count_block(&i.else_branch, st);
}

/// `match` 表达式：被测式 + 守卫 + 各臂块。
fn count_match(m: &huzi_ast::MatchExpr, st: &mut ParseStats) {
    count_expr(&m.scrutinee, st);
    for arm in &m.arms {
        if let Some(g) = &arm.guard {
            count_expr(g, st);
        }
        count_block(&arm.body, st);
    }
}
