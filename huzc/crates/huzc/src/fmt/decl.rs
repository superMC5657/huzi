//! 声明类语句的格式化:顶层语句分派、结构体/枚举/trait/impl/fn。

use super::Formatter;
use huzi_ast::*;

impl Formatter {
    pub(super) fn format_top_stmt(&mut self, stmt: &Stmt, line: usize, end: usize) {
        match stmt {
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
            Stmt::Fn(func) => self.format_fn(func, line, end),
            other => self.format_stmt(other, line, end),
        }
    }

    pub(super) fn format_struct(&mut self, s: &StructDef, line: usize, end: usize) {
        let type_params = if s.type_params.is_empty() {
            String::new()
        } else {
            format!("<{}>", s.type_params.join(", "))
        };
        if s.fields.is_empty() {
            self.line_at(&format!("struct {}{} {{}}", s.name, type_params), line);
            return;
        }
        self.line_at(&format!("struct {}{} {{", s.name, type_params), line);
        self.indent += 1;
        for field in &s.fields {
            self.line(&format!("{}: {},", field.name, field.field_type));
        }
        // 字段没有独立 span,字段间注释无法逐位回插:统一在收尾
        // 花括号前回收(提交口径一致,注释不丢失)。
        self.emit_pending_before(end);
        self.indent -= 1;
        self.line("}");
    }

    pub(super) fn format_enum(&mut self, e: &EnumDef, line: usize, end: usize) {
        if e.variants.is_empty() {
            self.line_at(&format!("enum {} {{}}", e.name), line);
            return;
        }
        self.line_at(&format!("enum {} {{", e.name), line);
        self.indent += 1;
        for v in &e.variants {
            if v.payloads.is_empty() {
                self.line(&format!("{},", v.name));
            } else {
                let payloads: Vec<_> = v.payloads.iter().map(|p| p.to_string()).collect();
                self.line(&format!("{}({}),", v.name, payloads.join(", ")));
            }
        }
        // 同 format_struct:变体间注释统一在收尾花括号前回收。
        self.emit_pending_before(end);
        self.indent -= 1;
        self.line("}");
    }

    pub(super) fn format_trait(&mut self, t: &TraitDef, line: usize, _end: usize) {
        self.line_at(&format!("trait {} {{", t.name), line);
        self.indent += 1;
        for m in &t.methods {
            let mut params = Vec::new();
            if m.has_self {
                params.push("self".to_string());
            }
            for p in &m.params {
                params.push(format!("{}: {}", p.name, p.param_type));
            }
            let sig = match &m.return_type {
                Some(ret) => format!("fn {}({}) -> {}", m.name, params.join(", "), ret),
                None => format!("fn {}({})", m.name, params.join(", ")),
            };
            self.line(&sig);
        }
        self.indent -= 1;
        self.line("}");
    }

    pub(super) fn format_impl(&mut self, i: &ImplBlock, line: usize, end: usize) {
        self.line_at(&format!("impl {} for {} {{", i.trait_name, i.target_type), line);
        self.indent += 1;
        for m in &i.methods {
            // FnStmt 不携带 span,方法头行号不可知:取首条 body 语句
            // 行号作注释回收上界,方法前的整行注释在方法头前吐出
            // (行尾注释可能整体偏移,但不再丢失/失效)。
            let bound = m
                .body
                .statements
                .first()
                .map(|s| s.span.start_line())
                .unwrap_or(end);
            self.emit_pending_before(bound);
            self.format_fn(m, 0, end);
        }
        self.emit_pending_before(end);
        self.indent -= 1;
        self.line("}");
    }

    pub(super) fn format_fn(&mut self, f: &FnStmt, line: usize, end: usize) {
        let type_params = if f.type_params.is_empty() {
            String::new()
        } else {
            format!("<{}>", f.type_params.join(", "))
        };
        let params: Vec<_> = f
            .params
            .iter()
            .map(|p| format!("{}: {}", p.name, p.param_type))
            .collect();
        let header = match &f.return_type {
            Some(ret) => format!(
                "fn {}{}({}) -> {} {{",
                f.name,
                type_params,
                params.join(", "),
                ret
            ),
            None => format!("fn {}{}({}) {{", f.name, type_params, params.join(", ")),
        };
        self.line_at(&header, line);
        self.indent += 1;
        self.format_block_stmts(&f.body.statements, end);
        self.indent -= 1;
        self.line("}");
    }
}
