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
            "export" => Token::Export,
            "weak" => Token::Weak,
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
                    // 仅当点号后紧随数字时才识别为浮点数,避免误吞方法调用点号(如 `10.foo()`)或范围 `1..5`。
                    if self.pos + 1 >= self.source.len() || !self.source[self.pos + 1].is_ascii_digit() {
                        break;
                    }
                    // 点号后的数字为元组索引：`t.1.2` 必须分词为
                    // 三个 Token，而不是 `t`、`.`、`1.2`。
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

    pub(super) fn read_fstring(&mut self) -> Result<Token> {
        self.advance(); // consume 'f' or 'F'
        self.advance(); // consume '"'
        let mut value = String::new();

        while !self.is_at_end() && self.peek() != '"' {
            if self.peek() == '\n' {
                return Err(HuziError::new(
                    "Unterminated format string",
                    self.line,
                    self.column,
                ));
            }
            if self.peek() == '\\' {
                self.advance();
                if self.is_at_end() {
                    return Err(HuziError::new(
                        "Unterminated format string",
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
                "Unterminated format string",
                self.line,
                self.column,
            ));
        }

        self.advance(); // consume closing '"'

        Ok(Token::FString(value))
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

/// 转义序列回归测试：`\r` 必须解码为 0x0D（CR），其余常用转义同步锁定。
/// 背景：`--dump-tokens` 对 `"\r"` 显示空文本曾被怀疑是解码丢失；
/// 实测解码正确，空显示是因为 dump 直接打印原始 CR 字节导致终端回车覆盖。
/// 本模块把正确行为锁死，防止未来回归。
#[cfg(test)]
mod escape_tests {
    use super::Lexer;
    use crate::token::Token;

    fn first_token(src: &str) -> Token {
        Lexer::new(src.to_string())
            .tokenize()
            .expect("escape sample must lex")
            .into_iter()
            .next()
            .expect("at least one token")
            .token
    }

    #[test]
    fn string_cr_escape_is_0x0d() {
        match first_token("\"\\r\"") {
            Token::String(s) => {
                assert_eq!(s, "\r");
                assert_eq!(s.as_bytes(), &[0x0D]);
            }
            other => panic!("expected String, got {:?}", other),
        }
    }

    #[test]
    fn string_all_common_escapes() {
        match first_token("\"\\n\\t\\r\\\\\\\"\\0\"") {
            Token::String(s) => {
                assert_eq!(s, "\n\t\r\\\"\0");
                assert_eq!(s.as_bytes(), &[0x0A, 0x09, 0x0D, 0x5C, 0x22, 0x00]);
            }
            other => panic!("expected String, got {:?}", other),
        }
    }

    #[test]
    fn fstring_and_char_cr_escape() {
        assert_eq!(first_token("f\"\\r\""), Token::FString("\r".to_string()));
        assert_eq!(first_token("'\\r'"), Token::Char('\r'));
    }
}
