use super::Parser;
use huzi_ast::*;
use huzi_error::Result;
use huzi_lexer::Token;

impl Parser {
    pub(super) fn parse_trait_statement(&mut self) -> Result<Stmt> {
        self.advance();
        let name = self.expect_ident("Expected trait name after 'trait'")?;
        self.expect(&Token::LBrace, "Expected '{' after trait name")?;

        let mut methods = Vec::new();
        while !self.check(&Token::RBrace) && !self.is_at_end() {
            methods.push(self.parse_trait_method()?);
        }

        self.expect(&Token::RBrace, "Expected '}' after trait methods")?;
        Ok(Stmt::Trait(TraitDef { name, methods }))
    }

    /// 解析 trait 声明内的单个方法签名。
    fn parse_trait_method(&mut self) -> Result<TraitMethodDef> {
        self.expect(&Token::Fn, "Expected 'fn' in trait definition")?;
        let method_name = self.expect_ident("Expected method name in trait definition")?;
        self.expect(&Token::LParen, "Expected '(' after method name")?;

        let mut has_self = false;
        let mut params = Vec::new();

        if !self.check(&Token::RParen) {
            let is_self = matches!(self.peek(), Token::Ident(pname) if pname == "self");
            if is_self {
                self.advance();
                has_self = true;
                if self.check(&Token::Colon) {
                    self.advance();
                    self.parse_type()?;
                }
                if self.check(&Token::Comma) {
                    self.advance();
                }
            }

            while !self.check(&Token::RParen) && !self.is_at_end() {
                let pname = self.expect_ident("Expected parameter name")?;
                self.expect(&Token::Colon, "Expected ':' after parameter name")?;
                let ptype = self.parse_type()?;
                params.push(FnParam {
                    name: pname,
                    param_type: ptype,
                });
                if self.check(&Token::Comma) {
                    self.advance();
                } else {
                    break;
                }
            }
        }
        self.expect(&Token::RParen, "Expected ')' after trait method parameters")?;

        let return_type = if self.check(&Token::Arrow) {
            self.advance();
            Some(self.parse_type()?)
        } else {
            None
        };

        if self.check(&Token::Semi) {
            self.advance();
        }

        Ok(TraitMethodDef {
            name: method_name,
            has_self,
            params,
            return_type,
        })
    }

    pub(super) fn parse_impl_statement(&mut self) -> Result<Stmt> {
        self.advance();
        let trait_name = self.expect_ident("Expected trait name after 'impl'")?;
        self.expect(&Token::For, "Expected 'for' after trait name in impl")?;
        let target_type = self.expect_ident("Expected target type after 'for'")?;
        self.expect(&Token::LBrace, "Expected '{' after target type in impl")?;

        let mut methods = Vec::new();
        while !self.check(&Token::RBrace) && !self.is_at_end() {
            methods.push(self.parse_impl_method(&target_type)?);
        }

        self.expect(&Token::RBrace, "Expected '}' after impl block")?;
        Ok(Stmt::Impl(ImplBlock {
            trait_name,
            target_type,
            methods,
        }))
    }

    /// 解析 impl 块内的单个方法定义。
    fn parse_impl_method(&mut self, target_type: &str) -> Result<FnStmt> {
        self.expect(&Token::Fn, "Expected 'fn' in impl block")?;
        let method_name = self.expect_ident("Expected method name in impl block")?;
        self.expect(&Token::LParen, "Expected '(' after method name")?;

        let mut params = Vec::new();
        if !self.check(&Token::RParen) {
            let is_self = matches!(self.peek(), Token::Ident(pname) if pname == "self");
            if is_self {
                self.advance();
                let self_type = if self.check(&Token::Colon) {
                    self.advance();
                    self.parse_type()?
                } else {
                    Type::Named(target_type.to_string())
                };
                params.push(FnParam {
                    name: "self".to_string(),
                    param_type: self_type,
                });
                if self.check(&Token::Comma) {
                    self.advance();
                }
            }

            while !self.check(&Token::RParen) && !self.is_at_end() {
                let pname = self.expect_ident("Expected parameter name")?;
                self.expect(&Token::Colon, "Expected ':' after parameter name")?;
                let ptype = self.parse_type()?;
                params.push(FnParam {
                    name: pname,
                    param_type: ptype,
                });
                if self.check(&Token::Comma) {
                    self.advance();
                } else {
                    break;
                }
            }
        }
        self.expect(&Token::RParen, "Expected ')' after parameters")?;

        let return_type = if self.check(&Token::Arrow) {
            self.advance();
            Some(self.parse_type()?)
        } else {
            None
        };

        let prev_in_fn = self.in_function;
        self.in_function = true;
        let body = self.parse_block()?;
        self.in_function = prev_in_fn;

        Ok(FnStmt {
            name: method_name,
            type_params: Vec::new(),
            params,
            return_type,
            body,
        })
    }
}
