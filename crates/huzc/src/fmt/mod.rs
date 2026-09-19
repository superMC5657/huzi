mod comments;
mod expr;
#[cfg(test)]
mod tests;

use comments::{collect_comments, CommentInfo};
use expr::*;
use huzi_ast::*;
use huzi_error::HuziError;
use huzi_lexer::Lexer;
use huzi_parser::Parser as HuziParser;
use std::fs;
use std::path::{Path, PathBuf};

pub fn format_source(source: &str) -> Result<String, HuziError> {
    let comments = collect_comments(source);
    let mut lexer = Lexer::new(source.to_string());
    let tokens = lexer.tokenize()?;
    let mut parser = HuziParser::new(tokens);
    let program = parser.parse()?;
    Ok(format_program(&program, comments))
}

/// `end_bound[i]` = 下一条顶层语句的起始行(末条为 usize::MAX),
/// 作为复合结构(结构体等)内部注释的回收边界。
pub fn format_program(program: &Program, comments: Vec<CommentInfo>) -> String {
    let mut f = Formatter::new(comments);
    let mut prev_was_import = false;

    let n = program.statements.len();
    let start_lines: Vec<usize> = program
        .statements
        .iter()
        .map(|s| s.span.start_line())
        .collect();
    for (i, stmt) in program.statements.iter().enumerate() {
        let is_import = matches!(stmt.node, Stmt::Import(_) | Stmt::Export(_));
        if i > 0 && (!is_import || !prev_was_import) {
            f.buf.push('\n');
        }
        prev_was_import = is_import;
        let end_bound = if i + 1 < n {
            start_lines[i + 1]
        } else {
            usize::MAX
        };
        f.format_top_stmt(&stmt.node, start_lines[i], end_bound);
    }
    f.emit_pending_before(usize::MAX);
    if !f.buf.ends_with('\n') {
        f.buf.push('\n');
    }
    f.buf
}

struct Formatter {
    indent: usize,
    buf: String,
    comments: Vec<CommentInfo>,
    next_comment: usize,
}

impl Formatter {
    fn new(comments: Vec<CommentInfo>) -> Self {
        Self {
            indent: 0,
            buf: String::new(),
            comments,
            next_comment: 0,
        }
    }

    fn pad(&self) -> String {
        "    ".repeat(self.indent)
    }

    fn line(&mut self, s: &str) {
        self.buf.push_str(&self.pad());
        self.buf.push_str(s);
        self.buf.push('\n');
    }

    /// 把行号早于 `before_line` 的整行注释按当前缩进吐出。
    fn emit_pending_before(&mut self, before_line: usize) {
        while self.next_comment < self.comments.len() {
            let c = &self.comments[self.next_comment];
            if c.line >= before_line {
                break;
            }
            let text = c.text.clone();
            self.line(&text);
            self.next_comment += 1;
        }
    }

    /// 若指定行有行尾注释则消费并返回其文本。
    fn take_trailing(&mut self, line: usize) -> Option<String> {
        let c = self.comments.get(self.next_comment)?;
        if c.line == line && !c.full_line {
            self.next_comment += 1;
            return Some(c.text.clone());
        }
        None
    }

    /// 语句行输出:先吐前置整行注释,再拼接可能的行尾注释。
    fn line_at(&mut self, s: &str, line: usize) {
        self.emit_pending_before(line);
        let mut out = s.to_string();
        if let Some(c) = self.take_trailing(line) {
            out.push(' ');
            out.push_str(&c);
        }
        self.line(&out);
    }

    /// 块内语句的边界:下一条语句起始行,末条用外层 end_bound。
    fn child_bound(lines: &[usize], i: usize, end: usize) -> usize {
        if i + 1 < lines.len() {
            lines[i + 1]
        } else {
            end
        }
    }

    fn format_top_stmt(&mut self, stmt: &Stmt, line: usize, end: usize) {
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

    fn format_struct(&mut self, s: &StructDef, line: usize, end: usize) {
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
            self.emit_pending_before(end);
            self.line(&format!("{}: {},", field.name, field.field_type));
        }
        self.emit_pending_before(end);
        self.indent -= 1;
        self.line("}");
    }

    fn format_enum(&mut self, e: &EnumDef, line: usize, end: usize) {
        if e.variants.is_empty() {
            self.line_at(&format!("enum {} {{}}", e.name), line);
            return;
        }
        self.line_at(&format!("enum {} {{", e.name), line);
        self.indent += 1;
        for v in &e.variants {
            self.emit_pending_before(end);
            if v.payloads.is_empty() {
                self.line(&format!("{},", v.name));
            } else {
                let payloads: Vec<_> = v.payloads.iter().map(|p| p.to_string()).collect();
                self.line(&format!("{}({}),", v.name, payloads.join(", ")));
            }
        }
        self.emit_pending_before(end);
        self.indent -= 1;
        self.line("}");
    }

    fn format_trait(&mut self, t: &TraitDef, line: usize, _end: usize) {
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

    fn format_impl(&mut self, i: &ImplBlock, line: usize, end: usize) {
        self.line_at(&format!("impl {} for {} {{", i.trait_name, i.target_type), line);
        self.indent += 1;
        for m in &i.methods {
            self.format_fn(m, 0, end);
        }
        self.emit_pending_before(end);
        self.indent -= 1;
        self.line("}");
    }

    fn format_fn(&mut self, f: &FnStmt, line: usize, end: usize) {
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
        let body_lines: Vec<usize> = f
            .body
            .statements
            .iter()
            .map(|st| st.span.start_line())
            .collect();
        for (i, stmt) in f.body.statements.iter().enumerate() {
            let bound = Self::child_bound(&body_lines, i, end);
            self.format_stmt(&stmt.node, body_lines[i], bound);
        }
        self.indent -= 1;
        self.line("}");
    }

    fn format_stmt(&mut self, stmt: &Stmt, line: usize, end: usize) {
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
                let lines: Vec<usize> =
                    b.statements.iter().map(|s| s.span.start_line()).collect();
                for (i, s) in b.statements.iter().enumerate() {
                    let bound = Self::child_bound(&lines, i, end);
                    self.format_stmt(&s.node, lines[i], bound);
                }
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
                let lines: Vec<usize> =
                    b.statements.iter().map(|s| s.span.start_line()).collect();
                for (i, s) in b.statements.iter().enumerate() {
                    let bound = Self::child_bound(&lines, i, end);
                    self.format_stmt(&s.node, lines[i], bound);
                }
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
        let then_lines: Vec<usize> = i
            .then_branch
            .statements
            .iter()
            .map(|s| s.span.start_line())
            .collect();
        for (k, s) in i.then_branch.statements.iter().enumerate() {
            let bound = Self::child_bound(&then_lines, k, end);
            self.format_stmt(&s.node, then_lines[k], bound);
        }
        self.indent -= 1;

        for (cond, block) in &i.elif_branches {
            self.line(&format!("}} elif {} {{", format_expr(cond)));
            self.indent += 1;
            let lines: Vec<usize> =
                block.statements.iter().map(|s| s.span.start_line()).collect();
            for (k, s) in block.statements.iter().enumerate() {
                let bound = Self::child_bound(&lines, k, end);
                self.format_stmt(&s.node, lines[k], bound);
            }
            self.indent -= 1;
        }

        if let Some(else_branch) = &i.else_branch {
            self.line("} else {");
            self.indent += 1;
            let lines: Vec<usize> = else_branch
                .statements
                .iter()
                .map(|s| s.span.start_line())
                .collect();
            for (k, s) in else_branch.statements.iter().enumerate() {
                let bound = Self::child_bound(&lines, k, end);
                self.format_stmt(&s.node, lines[k], bound);
            }
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
        let body_lines: Vec<usize> = f
            .body
            .statements
            .iter()
            .map(|s| s.span.start_line())
            .collect();
        for (i, s) in f.body.statements.iter().enumerate() {
            let bound = Self::child_bound(&body_lines, i, end);
            self.format_stmt(&s.node, body_lines[i], bound);
        }
        self.indent -= 1;
        self.line("}");
    }

    fn format_while(&mut self, w: &WhileStmt, line: usize, end: usize) {
        self.line_at(&format!("while {} {{", format_expr(&w.condition)), line);
        self.indent += 1;
        let body_lines: Vec<usize> = w
            .body
            .statements
            .iter()
            .map(|s| s.span.start_line())
            .collect();
        for (i, s) in w.body.statements.iter().enumerate() {
            let bound = Self::child_bound(&body_lines, i, end);
            self.format_stmt(&s.node, body_lines[i], bound);
        }
        self.indent -= 1;
        self.line("}");
    }
}

pub fn run_fmt(path: &str, check: bool) -> bool {
    let p = Path::new(path);
    if !p.exists() {
        eprintln!("Error: path '{}' does not exist", path);
        return false;
    }
    let mut files = Vec::new();
    collect_hz_files(p, &mut files);
    if files.is_empty() {
        if p.is_file() {
            files.push(p.to_path_buf());
        } else {
            eprintln!("No .hz files found in '{}'", path);
            return true;
        }
    }

    files.sort();
    let mut any_diff = false;
    let mut checked_count = 0;

    for file_path in files {
        let content = match fs::read_to_string(&file_path) {
            Ok(c) => c,
            Err(err) => {
                eprintln!("Error reading {}: {}", file_path.display(), err);
                any_diff = true;
                continue;
            }
        };

        let formatted = match format_source(&content) {
            Ok(f) => f,
            Err(err) => {
                eprintln!("Error parsing {}: {}", file_path.display(), err);
                any_diff = true;
                continue;
            }
        };

        let formatted = if content.contains("\r\n") {
            formatted.replace("\r\n", "\n").replace('\n', "\r\n")
        } else {
            formatted
        };

        checked_count += 1;
        if content != formatted {
            any_diff = true;
            if check {
                println!("Diff in {}", file_path.display());
            } else {
                if let Err(err) = fs::write(&file_path, &formatted) {
                    eprintln!("Error writing {}: {}", file_path.display(), err);
                } else {
                    println!("Formatted {}", file_path.display());
                }
            }
        }
    }

    if check {
        if any_diff {
            eprintln!("Some files are not formatted.");
            return false;
        } else {
            println!("All {} file(s) are properly formatted.", checked_count);
            return true;
        }
    }

    true
}

fn collect_hz_files(path: &Path, out: &mut Vec<PathBuf>) {
    if path.is_file() {
        if path.extension().is_some_and(|ext| ext == "hz") {
            out.push(path.to_path_buf());
        }
        return;
    }
    if path.is_dir() {
        let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if file_name == "target" || file_name == ".git" || file_name == ".omo" {
            return;
        }
        if let Ok(entries) = fs::read_dir(path) {
            for entry in entries.flatten() {
                collect_hz_files(&entry.path(), out);
            }
        }
    }
}
