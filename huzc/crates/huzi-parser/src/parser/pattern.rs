use super::Parser;
use huzi_ast::*;
use huzi_error::{HuziError, Result};
use huzi_lexer::Token;

impl Parser {
    /// `match expr { pattern [if guard] => body, ... }` — 每个分支体为一个语句块或单个表达式。
    pub(super) fn parse_match_expression(&mut self) -> Result<Expr> {
        self.advance();
        let scrutinee = self.parse_expression()?;

        self.expect(&Token::LBrace, "Expected '{' after match scrutinee")?;

        let mut arms = Vec::new();
        while !self.check(&Token::RBrace) && !self.is_at_end() {
            let pattern = self.parse_pattern()?;

            let guard = if self.check(&Token::If) {
                self.advance();
                Some(self.parse_expression()?)
            } else {
                None
            };

            self.expect(&Token::FatArrow, "Expected '=>' after match pattern")?;

            let body = if self.check(&Token::LBrace) {
                self.parse_block()?
            } else {
                let (line, col) = (self.current_line(), self.current_col());
                let expr = self.parse_expression()?;
                self.expr_block(expr, line, col)
            };

            arms.push(MatchArm {
                pattern,
                guard,
                body,
            });

            if self.check(&Token::Comma) {
                self.advance();
            }
        }

        self.expect(&Token::RBrace, "Expected '}' after match arms")?;

        Ok(Expr::Match(MatchExpr {
            scrutinee: Box::new(scrutinee),
            arms,
        }))
    }

    /// 解析模式：枚举变体、字面量、变量绑定或通配符。
    fn parse_pattern(&mut self) -> Result<Pattern> {
        match self.peek().clone() {
            Token::Ident(name) => {
                if name == "_" {
                    self.advance();
                    Ok(Pattern::Wildcard)
                } else if self.peek_at(1) == Some(&Token::PathSep) {
                    self.advance();
                    self.expect(&Token::PathSep, "Expected '::' after enum name in pattern")?;
                    let variant = self.expect_ident("Expected variant name after '::'")?;
                    let bindings = self.parse_pattern_bindings()?;
                    Ok(Pattern::Variant {
                        enum_name: name,
                        variant,
                        bindings,
                    })
                } else {
                    self.advance();
                    Ok(Pattern::Variable(name))
                }
            }
            Token::Minus => self.parse_signed_pattern(),
            Token::Int(n) => {
                self.advance();
                Ok(Pattern::Literal(Literal::Int(n)))
            }
            Token::Float(f) => {
                self.advance();
                Ok(Pattern::Literal(Literal::Float(f)))
            }
            Token::True => {
                self.advance();
                Ok(Pattern::Literal(Literal::Bool(true)))
            }
            Token::False => {
                self.advance();
                Ok(Pattern::Literal(Literal::Bool(false)))
            }
            Token::String(s) => {
                self.advance();
                Ok(Pattern::Literal(Literal::String(s)))
            }
            Token::Char(c) => {
                self.advance();
                Ok(Pattern::Literal(Literal::Char(c)))
            }
            _ => Err(HuziError::new(
                "Expected pattern (variant, literal, variable, or '_')",
                self.current_line(),
                self.current_col(),
            )),
        }
    }

    /// 带符号数字模式阶段：`-` 后的整数/浮点字面量(调用时 `-` 尚未消费)。
    fn parse_signed_pattern(&mut self) -> Result<Pattern> {
        self.advance();
        match self.peek() {
            Token::Int(n) => {
                let n = *n;
                self.advance();
                Ok(Pattern::Literal(Literal::Int(-n)))
            }
            Token::Float(f) => {
                let f = *f;
                self.advance();
                Ok(Pattern::Literal(Literal::Float(-f)))
            }
            _ => Err(HuziError::new(
                "Expected number after '-' in pattern",
                self.current_line(),
                self.current_col(),
            )),
        }
    }

    /// 解析变体模式的负载变量列表 `(x, y)`。
    fn parse_pattern_bindings(&mut self) -> Result<Vec<String>> {
        if !self.check(&Token::LParen) {
            return Ok(Vec::new());
        }
        self.advance();
        let mut bindings = Vec::new();
        if !self.check(&Token::RParen) {
            loop {
                bindings.push(self.expect_ident("Expected binding name in pattern")?);
                if self.check(&Token::Comma) {
                    self.advance();
                } else {
                    break;
                }
            }
        }
        self.expect(&Token::RParen, "Expected ')' after pattern bindings")?;
        Ok(bindings)
    }
}
