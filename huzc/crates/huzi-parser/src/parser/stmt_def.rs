use super::Parser;
use huzi_ast::*;
use huzi_error::Result;
use huzi_lexer::Token;

impl Parser {
    pub(super) fn parse_optional_type_params(&mut self) -> Result<Vec<String>> {
        if !self.check(&Token::Less) {
            return Ok(Vec::new());
        }
        self.advance(); // consume '<'
        if self.check(&Token::Greater) {
            return Err(self.error("Expected type parameter name between '<' and '>'"));
        }
        let params = self.parse_comma_separated(&Token::Greater, true, |p, prev| {
            let name = p.expect_ident("Expected type parameter name")?;
            if prev.contains(&name) {
                return Err(p.error(format!("Duplicate type parameter '{}'", name)));
            }
            Ok(name)
        })?;
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

        let fields = self.parse_comma_separated(&Token::RBrace, true, |p, _| {
            let is_weak_prefix = if p.check(&Token::Weak) {
                p.advance();
                true
            } else {
                false
            };
            let field_name = p.expect_ident("Expected field name")?;
            p.expect(&Token::Colon, "Expected ':' after field name")?;
            let mut field_type = p.parse_type()?;
            if is_weak_prefix {
                match field_type {
                    Type::Box(_) => field_type = Type::Weak(Box::new(field_type)),
                    _ => {
                        return Err(p.error(format!(
                            "'weak' modifier can only be applied to 'Box<T>' (found '{}')",
                            field_type
                        )));
                    }
                }
            }
            Ok(StructField {
                name: field_name,
                field_type,
            })
        })?;

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

        let variants = self.parse_comma_separated(&Token::RBrace, true, |p, _| {
            let variant_name = p.expect_ident("Expected variant name")?;
            let payloads = if p.check(&Token::LParen) {
                p.advance();
                let payloads =
                    p.parse_comma_separated(&Token::RParen, false, |q, _| q.parse_type())?;
                p.expect(&Token::RParen, "Expected ')' after variant payload types")?;
                payloads
            } else {
                Vec::new()
            };
            Ok(EnumVariant {
                name: variant_name,
                payloads,
            })
        })?;

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
