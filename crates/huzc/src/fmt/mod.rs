//! Huzi 格式化器:AST pretty-printer,注释按语句 span 行号回插。
//!
//! 子模块:`comments`(源码注释回收)、`decl`(顶层与声明类语句)、
//! `block`(块结构与普通语句)、`run`(`huzc fmt` CLI 驱动)。

mod block;
mod comments;
mod decl;
mod expr;
#[cfg(test)]
mod tests;
mod run;

pub use comments::{collect_comments, CommentInfo};
pub use run::run_fmt;
use huzi_ast::*;
use huzi_error::HuziError;
use huzi_lexer::Lexer;
use huzi_parser::Parser as HuziParser;

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
}
