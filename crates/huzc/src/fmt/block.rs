//! 块结构与普通语句的格式化:语句分派、let/defer/if/for/while,
//! 以及块语句序列的统一输出助手。

use super::Formatter;
use super::expr::*;
use huzi_ast::*;

impl Formatter {
    pub(super) fn format_stmt(&mut self, stmt: &Stmt, line: usize, end: usize) {
        match stmt {
            Stmt::Let(l) => self.format_let(l, line),
            Stmt::Expr(e) => self.line_at(&format_expr(&e.expr), line),
            Stmt::Return(r) => match &r.value {
                Some(v) => self.line_at(&format!("return {}", format_expr(v)), line),
                None => self.line_at("return", line),
            },
            Stmt::Break => self.line_at("break", line),
            Stmt::Continue => self.line_at("continue", line),
            Stmt::Defer(d) => self.format_defer(&d.node, line, end),
            Stmt::If(i) => self.format_if(i, line, end),
            Stmt::For(f) => self.format_for(f, line, end),
            Stmt::While(w) => self.format_while(w, line, end),
            Stmt::Block(b) => {
                self.line_at("{", line);
                self.indent += 1;
                self.format_block_stmts(&b.statements, end);
                self.indent -= 1;
                self.line("}");
            }
            Stmt::Import(imp) => self.line_at(&format!("import {}", imp.name), line),
            Stmt::Export(exp) => {
                if exp.is_wildcard {
                    self.line_at(&format!("export {}::*", exp.path), line);
                } else {
                    self.line_at(&format!("export {}", exp.path), line);
                }
            }
            Stmt::Struct(s) => self.format_struct(s, line, end),
            Stmt::Enum(e) => self.format_enum(e, line, end),
            Stmt::Trait(t) => self.format_trait(t, line, end),
            Stmt::Impl(i) => self.format_impl(i, line, end),
            Stmt::Fn(f) => self.format_fn(f, line, end),
        }
    }

    /// 块语句序列的统一输出:逐条按源行号回插注释,并以相邻语句
    /// 起始行作为每条语句的注释回收边界。
    pub(super) fn format_block_stmts(&mut self, stmts: &[Spanned<Stmt>], end: usize) {
        let lines: Vec<usize> = stmts.iter().map(|s| s.span.start_line()).collect();
        for (i, s) in stmts.iter().enumerate() {
            let bound = Self::child_bound(&lines, i, end);
            self.format_stmt(&s.node, lines[i], bound);
        }
    }

    fn format_let(&mut self, l: &LetStmt, line: usize) {
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
        self.line_at(&s, line);
    }

    fn format_defer(&mut self, inner: &Stmt, line: usize, end: usize) {
        match inner {
            Stmt::Block(b) => {
                self.line_at("defer {", line);
                self.indent += 1;
                self.format_block_stmts(&b.statements, end);
                self.indent -= 1;
                self.line("}");
            }
            other => {
                self.line_at(&format!("defer {}", format_stmt_inline(other)), line);
            }
        }
    }

    fn format_if(&mut self, i: &IfStmt, line: usize, end: usize) {
        self.line_at(&format!("if {} {{", format_expr(&i.condition)), line);
        self.indent += 1;
        self.format_block_stmts(&i.then_branch.statements, end);
        self.indent -= 1;

        for (cond, block) in &i.elif_branches {
            self.line(&format!("}} elif {} {{", format_expr(cond)));
            self.indent += 1;
            self.format_block_stmts(&block.statements, end);
            self.indent -= 1;
        }

        if let Some(else_branch) = &i.else_branch {
            self.line("} else {");
            self.indent += 1;
            self.format_block_stmts(&else_branch.statements, end);
            self.indent -= 1;
        }
        self.line("}");
    }

    fn format_for(&mut self, f: &ForStmt, line: usize, end: usize) {
        let source_str = match &f.source {
            ForSource::Range { start, end } => {
                format!("{}..{}", format_expr(start), format_expr(end))
            }
            ForSource::Array(arr) => format_expr(arr),
        };
        self.line_at(&format!("for {} in {} {{", f.var_name, source_str), line);
        self.indent += 1;
        self.format_block_stmts(&f.body.statements, end);
        self.indent -= 1;
        self.line("}");
    }

    fn format_while(&mut self, w: &WhileStmt, line: usize, end: usize) {
        self.line_at(&format!("while {} {{", format_expr(&w.condition)), line);
        self.indent += 1;
        self.format_block_stmts(&w.body.statements, end);
        self.indent -= 1;
        self.line("}");
    }
}
