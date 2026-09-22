use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // Keywords
    Fn,
    Struct,
    Enum,
    Match,
    Let,
    Mut,
    If,
    Else,
    Elif,
    For,
    In,
    While,
    Return,
    Import,
    Export,
    Break,
    Continue,
    Defer,
    Trait,
    Impl,
    True,
    False,

    // Literals
    Ident(String),
    Int(i64),
    Float(f64),
    String(String),
    Char(char),

    // Operators
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Equal,
    EqualEqual,
    Bang,
    BangEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    AmpAmp,
    BarBar,
    Question,

    // Delimiters
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Comma,
    Colon,
    PathSep,
    Semi,
    Arrow,
    FatArrow,
    Dot,
    DotDot,

    // Special
    Eof,
}

/// 携带源码位置（基于 1 的行列号）的词法单元。
#[derive(Debug, Clone, PartialEq)]
pub struct SpannedToken {
    pub token: Token,
    pub line: usize,
    pub column: usize,
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Token::Ident(s) => write!(f, "{}", s),
            Token::Int(n) => write!(f, "{}", n),
            Token::Float(n) => write!(f, "{}", n),
            Token::String(s) => write!(f, "\"{}\"", s),
            Token::Char(c) => write!(f, "'{}'", c),
            Token::Eof => write!(f, "EOF"),
            Token::Question => write!(f, "?"),
            _ => write!(f, "{:?}", self),
        }
    }
}
