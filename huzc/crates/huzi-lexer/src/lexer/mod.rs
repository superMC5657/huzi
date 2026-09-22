mod literals;
mod operators;
#[cfg(test)]
mod tests;

use crate::token::{SpannedToken, Token};
use huzi_error::Result;

pub struct Lexer {
    source: Vec<char>,
    pos: usize,
    line: usize,
    column: usize,
    /// True when the previously emitted token was `.`. A number right after a
    /// dot is a tuple element index (`t.0`, `t.1.2`), so it must not swallow
    /// the following `.digit` as a float.
    prev_was_dot: bool,
}

impl Lexer {
    pub fn new(source: String) -> Self {
        Self {
            source: source.chars().collect(),
            pos: 0,
            line: 1,
            column: 1,
            prev_was_dot: false,
        }
    }

    pub fn tokenize(&mut self) -> Result<Vec<SpannedToken>> {
        let mut tokens = Vec::new();
        loop {
            let token = self.next_token()?;
            let is_eof = token.token == Token::Eof;
            self.prev_was_dot = token.token == Token::Dot;
            tokens.push(token);
            if is_eof {
                break;
            }
        }
        Ok(tokens)
    }

    fn next_token(&mut self) -> Result<SpannedToken> {
        self.skip_whitespace();

        let (line, column) = (self.line, self.column);

        if self.is_at_end() {
            return Ok(SpannedToken { token: Token::Eof, line, column });
        }

        let c = self.peek();

        let token = if c.is_alphabetic() || c == '_' {
            self.read_ident()?
        } else if c.is_numeric() {
            self.read_number()?
        } else {
            match c {
                '"' => self.read_string()?,
                '\'' => self.read_char()?,
                '(' => self.single(Token::LParen),
                ')' => self.single(Token::RParen),
                '{' => self.single(Token::LBrace),
                '}' => self.single(Token::RBrace),
                '[' => self.single(Token::LBracket),
                ']' => self.single(Token::RBracket),
                ',' => self.single(Token::Comma),
                ':' => self.read_colon()?,
                ';' => self.single(Token::Semi),
                '/' => self.read_slash()?,
                '+' => self.single(Token::Plus),
                '-' => self.read_minus()?,
                '*' => self.single(Token::Star),
                '%' => self.single(Token::Percent),
                '.' => self.read_dot()?,
                '=' => self.read_equal()?,
                '!' => self.read_bang()?,
                '<' => self.read_less()?,
                '>' => self.read_greater()?,
                '&' => self.read_amp()?,
                '|' => self.read_bar()?,
                '?' => self.single(Token::Question),
                _ => {
                    return Err(huzi_error::HuziError::new(
                        format!("Unexpected character '{}'", c),
                        line,
                        column,
                    ))
                }
            }
        };

        Ok(SpannedToken { token, line, column })
    }

    fn single(&mut self, token: Token) -> Token {
        self.advance();
        token
    }

    fn skip_whitespace(&mut self) {
        while !self.is_at_end() {
            match self.peek() {
                ' ' | '\t' | '\r' => {
                    self.advance();
                }
                '\n' => {
                    self.advance();
                    self.line += 1;
                    self.column = 1;
                }
                // Support both // and # comments
                '/' => {
                    // Check for // comment
                    if self.pos + 1 < self.source.len() && self.source[self.pos + 1] == '/' {
                        self.skip_line_comment();
                    } else {
                        break;
                    }
                }
                '#' => {
                    self.skip_line_comment();
                }
                _ => break,
            }
        }
    }

    fn skip_line_comment(&mut self) {
        while !self.is_at_end() && self.peek() != '\n' {
            self.advance();
        }
        // Skip the newline too
        if !self.is_at_end() && self.peek() == '\n' {
            self.advance();
            self.line += 1;
            self.column = 1;
        }
    }

    fn peek(&self) -> char {
        self.source[self.pos]
    }

    fn advance(&mut self) {
        if !self.is_at_end() {
            self.pos += 1;
            self.column += 1;
        }
    }

    fn is_at_end(&self) -> bool {
        self.pos >= self.source.len()
    }
}
