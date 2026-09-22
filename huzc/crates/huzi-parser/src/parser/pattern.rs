use super::Parser;
use huzi_ast::*;
use huzi_error::Result;
use huzi_lexer::Token;

impl Parser {
    /// `match expr { pattern => body, ... }` — 每个分支体为一个语句块或
    /// 单个表达式。
    pub(super) fn parse_match_expression(&mut self) -> Result<Expr> {
        self.advance();
        let scrutinee = self.parse_expression()?;

        self.expect(&Token::LBrace, "Expected '{' after match scrutinee")?;

        let mut arms = Vec::new();
        while !self.check(&Token::RBrace) && !self.is_at_end() {
            let pattern = self.parse_pattern()?;

            self.expect(&Token::FatArrow, "Expected '=>' after match pattern")?;

            let body = if self.check(&Token::LBrace) {
                self.parse_block()?
            } else {
                let (line, col) = (self.current_line(), self.current_col());
                let expr = self.parse_expression()?;
                self.expr_block(expr, line, col)
            };

            arms.push(MatchArm { pattern, body });

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

    /// `Enum::Variant`、`Enum::Variant(x)`、`Enum::Variant(x, y)` 或 `_`。
    fn parse_pattern(&mut self) -> Result<Pattern> {
        // 注意：`check` 会将任意 Ident 匹配为 Token::Ident，因此
        // 通配符必须通过比对实际名称 `_` 来识别。
        if let Token::Ident(name) = self.peek() {
            if name == "_" {
                self.advance();
                return Ok(Pattern::Wildcard);
            }
        }

        let enum_name = self.expect_ident("Expected pattern (variant or '_')")?;

        self.expect(&Token::PathSep, "Expected '::' after enum name in pattern")?;
        let variant = self.expect_ident("Expected variant name after '::'")?;

        let bindings = if self.check(&Token::LParen) {
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
            bindings
        } else {
            Vec::new()
        };

        Ok(Pattern::Variant {
            enum_name,
            variant,
            bindings,
        })
    }
}
