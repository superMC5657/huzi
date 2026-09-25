use super::Parser;
use huzi_ast::*;
use huzi_error::{HuziError, Result};
use huzi_lexer::Token;

impl Parser {
    pub(super) fn parse_optional_type_params(&mut self) -> Result<Vec<String>> {
        if !self.check(&Token::Less) {
            return Ok(Vec::new());
        }
        self.advance(); // consume '<'
        let mut params = Vec::new();
        if self.check(&Token::Greater) {
            return Err(HuziError::new(
                "Expected type parameter name between '<' and '>'",
                self.current_line(),
                self.current_col(),
            ));
        }
        while !self.check(&Token::Greater) && !self.is_at_end() {
            let p = self.expect_ident("Expected type parameter name")?;
            if params.contains(&p) {
                return Err(HuziError::new(
                    format!("Duplicate type parameter '{}'", p),
                    self.current_line(),
                    self.current_col(),
                ));
            }
            params.push(p);
            if self.check(&Token::Comma) {
                self.advance();
            } else {
                break;
            }
        }
        self.expect(&Token::Greater, "Expected '>' after type parameters")?;
        Ok(params)
    }

    pub(super) fn parse_struct_statement(&mut self) -> Result<Stmt> {
        self.advance();

        let name = self.expect_ident("Expected struct name")?;
        let type_params = self.parse_optional_type_params()?;
        let num_params = type_params.len();
        self.push_type_params(&type_params);

        self.expect(&Token::LBrace, "Expected '{' after struct name")?;

        let mut fields = Vec::new();
        while !self.check(&Token::RBrace) && !self.is_at_end() {
            let is_weak_prefix = if self.check(&Token::Weak) {
                self.advance();
                true
            } else {
                false
            };
            let field_name = self.expect_ident("Expected field name")?;

            self.expect(&Token::Colon, "Expected ':' after field name")?;
            let mut field_type = self.parse_type()?;
            if is_weak_prefix {
                match field_type {
                    Type::Box(_) => field_type = Type::Weak(Box::new(field_type)),
                    _ => {
                        return Err(HuziError::new(
                            format!(
                                "'weak' modifier can only be applied to 'Box<T>' (found '{}')",
                                field_type
                            ),
                            self.current_line(),
                            self.current_col(),
                        ));
                    }
                }
            }

            fields.push(StructField {
                name: field_name,
                field_type,
            });

            if self.check(&Token::Comma) {
                self.advance();
            } else {
                break;
            }
        }

        self.expect(&Token::RBrace, "Expected '}' after struct fields")?;
        self.pop_type_params(num_params);

        Ok(Stmt::Struct(StructDef {
            name,
            type_params,
            fields,
        }))
    }

    pub(super) fn parse_enum_statement(&mut self) -> Result<Stmt> {
        self.advance();

        let name = self.expect_ident("Expected enum name")?;
        let type_params = self.parse_optional_type_params()?;
        let num_params = type_params.len();
        self.push_type_params(&type_params);

        self.expect(&Token::LBrace, "Expected '{' after enum name")?;

        let mut variants = Vec::new();
        while !self.check(&Token::RBrace) && !self.is_at_end() {
            let variant_name = self.expect_ident("Expected variant name")?;

            let payloads = if self.check(&Token::LParen) {
                self.advance();
                let mut payloads = Vec::new();
                if !self.check(&Token::RParen) {
                    loop {
                        payloads.push(self.parse_type()?);
                        if self.check(&Token::Comma) {
                            self.advance();
                        } else {
                            break;
                        }
                    }
                }
                self.expect(&Token::RParen, "Expected ')' after variant payload types")?;
                payloads
            } else {
                Vec::new()
            };

            variants.push(EnumVariant {
                name: variant_name,
                payloads,
            });

            if self.check(&Token::Comma) {
                self.advance();
            } else {
                break;
            }
        }

        self.expect(&Token::RBrace, "Expected '}' after enum variants")?;
        self.pop_type_params(num_params);

        Ok(Stmt::Enum(EnumDef {
            name,
            type_params,
            variants,
        }))
    }

    pub(super) fn parse_fn_statement(&mut self) -> Result<Stmt> {
        self.advance();

        let name = self.expect_ident("Expected function name")?;
        let type_params = self.parse_optional_type_params()?;
        let num_params = type_params.len();
        self.push_type_params(&type_params);

        self.expect(&Token::LParen, "Expected '(' after function name")?;

        let mut params = Vec::new();
        while !self.check(&Token::RParen) {
            let param_name = self.expect_ident("Expected parameter name")?;

            self.expect(&Token::Colon, "Expected ':' after parameter name")?;
            let param_type = self.parse_type()?;

            params.push(FnParam {
                name: param_name,
                param_type,
            });

            if self.check(&Token::Comma) {
                self.advance();
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
        let body_res = self.parse_block();
        self.in_function = prev_in_fn;
        self.pop_type_params(num_params);
        let body = body_res?;

        Ok(Stmt::Fn(FnStmt {
            name,
            type_params,
            params,
            return_type,
            body,
        }))
    }
}
