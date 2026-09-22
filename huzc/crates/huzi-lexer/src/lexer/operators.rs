use super::Lexer;
use crate::token::Token;
use huzi_error::{HuziError, Result};

impl Lexer {
    pub(super) fn read_dot(&mut self) -> Result<Token> {
        self.advance();

        if !self.is_at_end() && self.peek() == '.' {
            self.advance();
            Ok(Token::DotDot)
        } else {
            Ok(Token::Dot)
        }
    }

    pub(super) fn read_colon(&mut self) -> Result<Token> {
        self.advance();

        if !self.is_at_end() && self.peek() == ':' {
            self.advance();
            Ok(Token::PathSep)
        } else {
            Ok(Token::Colon)
        }
    }

    pub(super) fn read_slash(&mut self) -> Result<Token> {
        self.advance();
        // `//` comments at token start are already handled by skip_whitespace.
        Ok(Token::Slash)
    }

    pub(super) fn read_minus(&mut self) -> Result<Token> {
        self.advance();

        if !self.is_at_end() && self.peek() == '>' {
            self.advance();
            Ok(Token::Arrow)
        } else {
            Ok(Token::Minus)
        }
    }

    pub(super) fn read_equal(&mut self) -> Result<Token> {
        self.advance();

        if !self.is_at_end() && self.peek() == '=' {
            self.advance();
            Ok(Token::EqualEqual)
        } else if !self.is_at_end() && self.peek() == '>' {
            self.advance();
            Ok(Token::FatArrow)
        } else {
            Ok(Token::Equal)
        }
    }

    pub(super) fn read_bang(&mut self) -> Result<Token> {
        self.advance();

        if !self.is_at_end() && self.peek() == '=' {
            self.advance();
            Ok(Token::BangEqual)
        } else {
            Ok(Token::Bang)
        }
    }

    pub(super) fn read_less(&mut self) -> Result<Token> {
        self.advance();

        if !self.is_at_end() && self.peek() == '=' {
            self.advance();
            Ok(Token::LessEqual)
        } else {
            Ok(Token::Less)
        }
    }

    pub(super) fn read_greater(&mut self) -> Result<Token> {
        self.advance();

        if !self.is_at_end() && self.peek() == '=' {
            self.advance();
            Ok(Token::GreaterEqual)
        } else {
            Ok(Token::Greater)
        }
    }

    pub(super) fn read_amp(&mut self) -> Result<Token> {
        self.advance();

        if !self.is_at_end() && self.peek() == '&' {
            self.advance();
            Ok(Token::AmpAmp)
        } else {
            Err(HuziError::new(
                "Unexpected character '&' (did you mean '&&'?)",
                self.line,
                self.column,
            ))
        }
    }

    pub(super) fn read_bar(&mut self) -> Result<Token> {
        self.advance();

        if !self.is_at_end() && self.peek() == '|' {
            self.advance();
            Ok(Token::BarBar)
        } else {
            Err(HuziError::new(
                "Unexpected character '|' (did you mean '||'?)",
                self.line,
                self.column,
            ))
        }
    }
}
