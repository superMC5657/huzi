use std::fmt;

mod render;
mod suggest;

pub use render::{render, render_with_color};
pub use suggest::{did_you_mean, levenshtein};

#[derive(Debug, Clone)]
pub struct HuziError {
    message: String,
    line: usize,
    column: usize,
}

impl HuziError {
    pub fn new(message: impl Into<String>, line: usize, column: usize) -> Self {
        Self {
            message: message.into(),
            line,
            column,
        }
    }

    pub fn new_global(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            line: 0,
            column: 0,
        }
    }

    /// 仅当错误尚无位置(全局错误)时附加行列;已有位置则保持不变,
    /// 供泛型单态化/Trait 脱糖等按语句 span 回填调用点位置使用。
    pub fn with_position(mut self, line: usize, column: usize) -> Self {
        if self.line == 0 {
            self.line = line;
            self.column = column;
        }
        self
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    /// 1-based 行号;0 表示无位置信息(全局错误)。
    pub fn line(&self) -> usize {
        self.line
    }

    /// 1-based 列号(按字符计数);行号为 0 时无意义。
    pub fn column(&self) -> usize {
        self.column
    }
}

impl fmt::Display for HuziError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.line > 0 {
            write!(
                f,
                "Error at line {}, column {}: {}",
                self.line, self.column, self.message
            )
        } else {
            write!(f, "Error: {}", self.message)
        }
    }
}

impl std::error::Error for HuziError {}

pub type Result<T> = std::result::Result<T, HuziError>;
