use super::Lexer;
use crate::token::Token;
use huzi_error::{HuziError, Result};

impl Lexer {
    pub(super) fn read_ident(&mut self) -> Result<Token> {
        let start = self.pos;
        while !self.is_at_end() && (self.peek().is_alphanumeric() || self.peek() == '_') {
            self.advance();
        }

        let ident: String = self.source[start..self.pos].iter().collect();

        let token = match ident.as_str() {
            "fn" => Token::Fn,
            "struct" => Token::Struct,
            "enum" => Token::Enum,
            "match" => Token::Match,
            "let" => Token::Let,
            "mut" => Token::Mut,
            "if" => Token::If,
            "else" => Token::Else,
            "elif" => Token::Elif,
            "for" => Token::For,
            "in" => Token::In,
            "while" => Token::While,
            "return" => Token::Return,
            "break" => Token::Break,
            "continue" => Token::Continue,
            "defer" => Token::Defer,
            "trait" => Token::Trait,
            "impl" => Token::Impl,
            "true" => Token::True,
            "false" => Token::False,
            "import" => Token::Import,
            _ => Token::Ident(ident),
        };

        Ok(token)
    }

    pub(super) fn read_number(&mut self) -> Result<Token> {
        let start = self.pos;
        let mut has_dot = false;

        while !self.is_at_end() {
            match self.peek() {
                '0'..='9' => {
                    self.advance();
                }
                '.' if !has_dot => {
                    // Don't consume the dot of a range expression: 1..5
                    if self.pos + 1 < self.source.len() && self.source[self.pos + 1] == '.' {
                        break;
                    }
                    // After `.` the number is a tuple index: `t.1.2` must lex
                    // as three tokens, not `t`, `.`, `1.2`.
                    if self.prev_was_dot {
                        break;
                    }
                    has_dot = true;
                    self.advance();
                }
                _ => break,
            }
        }

        let num_str: String = self.source[start..self.pos].iter().collect();

        if has_dot {
            let val: f64 = num_str
                .parse()
                .map_err(|_| HuziError::new("Invalid float", self.line, self.column))?;
            Ok(Token::Float(val))
        } else {
            let val: i64 = num_str
                .parse()
                .map_err(|_| HuziError::new("Invalid integer", self.line, self.column))?;
            Ok(Token::Int(val))
        }
    }

    pub(super) fn read_string(&mut self) -> Result<Token> {
        self.advance();
        let mut value = String::new();

        while !self.is_at_end() && self.peek() != '"' {
            if self.peek() == '\n' {
                return Err(HuziError::new(
                    "Unterminated string",
                    self.line,
                    self.column,
                ));
            }
            if self.peek() == '\\' {
                self.advance();
                if self.is_at_end() {
                    return Err(HuziError::new(
                        "Unterminated string",
                        self.line,
                        self.column,
                    ));
                }
                let escaped = match self.peek() {
                    'n' => '\n',
                    't' => '\t',
                    'r' => '\r',
                    '\\' => '\\',
                    '"' => '"',
                    '0' => '\0',
                    other => other,
                };
                value.push(escaped);
            } else {
                value.push(self.peek());
            }
            self.advance();
        }

        if self.is_at_end() {
            return Err(HuziError::new(
                "Unterminated string",
                self.line,
                self.column,
            ));
        }

        self.advance();

        Ok(Token::String(value))
    }

    pub(super) fn read_char(&mut self) -> Result<Token> {
        self.advance();

        if self.is_at_end() {
            return Err(HuziError::new(
                "Unterminated char",
                self.line,
                self.column,
            ));
        }

        let c = if self.peek() == '\\' {
            self.advance();
            if self.is_at_end() {
                return Err(HuziError::new(
                    "Unterminated char",
                    self.line,
                    self.column,
                ));
            }
            match self.peek() {
                'n' => '\n',
                't' => '\t',
                'r' => '\r',
                '\\' => '\\',
                '\'' => '\'',
                '0' => '\0',
                other => other,
            }
        } else {
            self.peek()
        };

        self.advance();

        if self.is_at_end() || self.peek() != '\'' {
            return Err(HuziError::new(
                "Unterminated char",
                self.line,
                self.column,
            ));
        }

        self.advance();

        Ok(Token::Char(c))
    }
}
