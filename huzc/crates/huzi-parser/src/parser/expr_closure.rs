use super::Parser;
use huzi_ast::*;
use huzi_error::Result;
use huzi_lexer::Token;

impl Parser {
    /// 解析 0 实参闭包: `|| expr` 或 `|| { ... }`
    pub(super) fn parse_closure_zero_args(&mut self) -> Result<Expr> {
        self.advance(); // consume Token::BarBar
        let return_type = if self.check(&Token::Arrow) {
            self.advance();
            Some(self.parse_type()?)
        } else {
            None
        };
        let prev_in_fn = self.in_function;
        self.in_function = true;
        let body = if self.check(&Token::LBrace) {
            ClosureBody::Block(self.parse_block()?)
        } else {
            ClosureBody::Expr(Box::new(self.parse_expression()?))
        };
        self.in_function = prev_in_fn;

        Ok(Expr::Closure(ClosureExpr {
            params: Vec::new(),
            return_type,
            body,
        }))
    }

    /// 解析 1 个或多个实参的闭包: `|x, y| expr` 或 `|x: i32| -> i32 { ... }`
    pub(super) fn parse_closure(&mut self) -> Result<Expr> {
        self.advance(); // consume leading Token::Pipe
        let mut params = Vec::new();
        while !self.check(&Token::Pipe) && !self.is_at_end() {
            let name = self.expect_ident("Expected parameter name in closure")?;
            let param_type = if self.check(&Token::Colon) {
                self.advance();
                Some(self.parse_type()?)
            } else {
                None
            };
            params.push(ClosureParam { name, param_type });
            if self.check(&Token::Comma) {
                self.advance();
            } else {
                break;
            }
        }
        self.expect(&Token::Pipe, "Expected '|' after closure parameters")?;

        let return_type = if self.check(&Token::Arrow) {
            self.advance();
            Some(self.parse_type()?)
        } else {
            None
        };

        let prev_in_fn = self.in_function;
        self.in_function = true;
        let body = if self.check(&Token::LBrace) {
            ClosureBody::Block(self.parse_block()?)
        } else {
            ClosureBody::Expr(Box::new(self.parse_expression()?))
        };
        self.in_function = prev_in_fn;

        Ok(Expr::Closure(ClosureExpr {
            params,
            return_type,
            body,
        }))
    }
}
