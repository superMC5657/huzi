mod expr;
#[cfg(test)]
mod tests;

use expr::*;
use huzi_ast::*;
use huzi_error::HuziError;
use huzi_lexer::Lexer;
use huzi_parser::Parser as HuziParser;
use std::fs;
use std::path::{Path, PathBuf};

pub fn format_source(source: &str) -> Result<String, HuziError> {
    let mut lexer = Lexer::new(source.to_string());
    let tokens = lexer.tokenize()?;
    let mut parser = HuziParser::new(tokens);
    let program = parser.parse()?;
    Ok(format_program(&program))
}

pub fn format_program(program: &Program) -> String {
    let mut f = Formatter::new();
    let mut prev_was_import = false;

    for (i, stmt) in program.statements.iter().enumerate() {
        let is_import = matches!(stmt.node, Stmt::Import(_) | Stmt::Export(_));
        if i > 0 && (!is_import || !prev_was_import) {
            f.buf.push('\n');
        }
        prev_was_import = is_import;
        f.format_top_stmt(&stmt.node);
    }
    if !f.buf.ends_with('\n') {
        f.buf.push('\n');
    }
    f.buf
}

struct Formatter {
    indent: usize,
    buf: String,
}

impl Formatter {
    fn new() -> Self {
        Self {
            indent: 0,
            buf: String::new(),
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

    fn format_top_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Import(imp) => self.line(&format!("import {}", imp.name)),
            Stmt::Export(exp) => {
                if exp.is_wildcard {
                    self.line(&format!("export {}::*", exp.path));
                } else {
                    self.line(&format!("export {}", exp.path));
                }
            }
            Stmt::Struct(s) => self.format_struct(s),
            Stmt::Enum(e) => self.format_enum(e),
            Stmt::Fn(func) => self.format_fn(func),
            other => self.format_stmt(other),
        }
    }

    fn format_struct(&mut self, s: &StructDef) {
        let type_params = if s.type_params.is_empty() {
            String::new()
        } else {
            format!("<{}>", s.type_params.join(", "))
        };
        if s.fields.is_empty() {
            self.line(&format!("struct {}{} {{}}", s.name, type_params));
            return;
        }
        self.line(&format!("struct {}{} {{", s.name, type_params));
        self.indent += 1;
        for field in &s.fields {
            self.line(&format!("{}: {},", field.name, field.field_type));
        }
        self.indent -= 1;
        self.line("}");
    }

    fn format_enum(&mut self, e: &EnumDef) {
        if e.variants.is_empty() {
            self.line(&format!("enum {} {{}}", e.name));
            return;
        }
        self.line(&format!("enum {} {{", e.name));
        self.indent += 1;
        for v in &e.variants {
            if v.payloads.is_empty() {
                self.line(&format!("{},", v.name));
            } else {
                let payloads: Vec<_> = v.payloads.iter().map(|p| p.to_string()).collect();
                self.line(&format!("{}({}),", v.name, payloads.join(", ")));
            }
        }
        self.indent -= 1;
        self.line("}");
    }

    fn format_trait(&mut self, t: &TraitDef) {
        self.line(&format!("trait {} {{", t.name));
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

    fn format_impl(&mut self, i: &ImplBlock) {
        self.line(&format!("impl {} for {} {{", i.trait_name, i.target_type));
        self.indent += 1;
        for m in &i.methods {
            self.format_fn(m);
        }
        self.indent -= 1;
        self.line("}");
    }

    fn format_fn(&mut self, f: &FnStmt) {
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
        self.line(&header);
        self.indent += 1;
        for stmt in &f.body.statements {
            self.format_stmt(&stmt.node);
        }
        self.indent -= 1;
        self.line("}");
    }

    fn format_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Let(l) => self.format_let(l),
            Stmt::Expr(e) => self.line(&format_expr(&e.expr)),
            Stmt::Return(r) => match &r.value {
                Some(v) => self.line(&format!("return {}", format_expr(v))),
                None => self.line("return"),
            },
            Stmt::Break => self.line("break"),
            Stmt::Continue => self.line("continue"),
            Stmt::Defer(d) => self.format_defer(&d.node),
            Stmt::If(i) => self.format_if(i),
            Stmt::For(f) => self.format_for(f),
            Stmt::While(w) => self.format_while(w),
            Stmt::Block(b) => {
                self.line("{");
                self.indent += 1;
                for s in &b.statements {
                    self.format_stmt(&s.node);
                }
                self.indent -= 1;
                self.line("}");
            }
            Stmt::Import(imp) => self.line(&format!("import {}", imp.name)),
            Stmt::Export(exp) => {
                if exp.is_wildcard {
                    self.line(&format!("export {}::*", exp.path));
                } else {
                    self.line(&format!("export {}", exp.path));
                }
            }
            Stmt::Struct(s) => self.format_struct(s),
            Stmt::Enum(e) => self.format_enum(e),
            Stmt::Trait(t) => self.format_trait(t),
            Stmt::Impl(i) => self.format_impl(i),
            Stmt::Fn(f) => self.format_fn(f),
        }
    }

    fn format_let(&mut self, l: &LetStmt) {
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
        self.line(&s);
    }

    fn format_defer(&mut self, inner: &Stmt) {
        match inner {
            Stmt::Block(b) => {
                self.line("defer {");
                self.indent += 1;
                for s in &b.statements {
                    self.format_stmt(&s.node);
                }
                self.indent -= 1;
                self.line("}");
            }
            other => {
                self.line(&format!("defer {}", format_stmt_inline(other)));
            }
        }
    }

    fn format_if(&mut self, i: &IfStmt) {
        self.line(&format!("if {} {{", format_expr(&i.condition)));
        self.indent += 1;
        for s in &i.then_branch.statements {
            self.format_stmt(&s.node);
        }
        self.indent -= 1;

        for (cond, block) in &i.elif_branches {
            self.line(&format!("}} elif {} {{", format_expr(cond)));
            self.indent += 1;
            for s in &block.statements {
                self.format_stmt(&s.node);
            }
            self.indent -= 1;
        }

        if let Some(else_branch) = &i.else_branch {
            self.line("} else {");
            self.indent += 1;
            for s in &else_branch.statements {
                self.format_stmt(&s.node);
            }
            self.indent -= 1;
        }
        self.line("}");
    }

    fn format_for(&mut self, f: &ForStmt) {
        let source_str = match &f.source {
            ForSource::Range { start, end } => {
                format!("{}..{}", format_expr(start), format_expr(end))
            }
            ForSource::Array(arr) => format_expr(arr),
        };
        self.line(&format!("for {} in {} {{", f.var_name, source_str));
        self.indent += 1;
        for s in &f.body.statements {
            self.format_stmt(&s.node);
        }
        self.indent -= 1;
        self.line("}");
    }

    fn format_while(&mut self, w: &WhileStmt) {
        self.line(&format!("while {} {{", format_expr(&w.condition)));
        self.indent += 1;
        for s in &w.body.statements {
            self.format_stmt(&s.node);
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
