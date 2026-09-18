use super::Parser;
use huzi_ast::*;
use huzi_error::{HuziError, Result};
use huzi_lexer::Token;

impl Parser {
    /// `import math` 或 `import mods.helpers`(点分路径,对应文件
    /// `mods/helpers.hz`,符号绑定为末段名:`helpers::函数`)。
    pub(super) fn parse_import_statement(&mut self) -> Result<Stmt> {
        self.advance();
        let mut name = self.expect_ident("Expected module name after 'import'")?;
        while self.check(&Token::Dot) || self.check(&Token::PathSep) {
            self.advance();
            let seg = self.expect_ident("Expected identifier after '.' or '::' in import")?;
            name.push('.');
            name.push_str(&seg);
        }
        Ok(Stmt::Import(ImportStmt { name }))
    }

    pub(super) fn parse_let_statement(&mut self) -> Result<Stmt> {
        self.advance();

        // `let mut name` or `let name`
        let mutable = self.check(&Token::Mut);
        if mutable {
            self.advance();
        }

        let name = self.expect_ident("Expected variable name")?;

        let type_annotation = if self.check(&Token::Colon) {
            self.advance();
            Some(self.parse_type()?)
        } else {
            None
        };

        let value = if self.check(&Token::Equal) {
            self.advance();
            Some(self.parse_expression()?)
        } else {
            None
        };

        Ok(Stmt::Let(LetStmt {
            name,
            mutable,
            type_annotation,
            value,
        }))
    }

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
            let field_name = self.expect_ident("Expected field name")?;

            self.expect(&Token::Colon, "Expected ':' after field name")?;
            let field_type = self.parse_type()?;

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

        Ok(Stmt::Enum(EnumDef { name, variants }))
    }

    pub(super) fn parse_trait_statement(&mut self) -> Result<Stmt> {
        self.advance();
        let name = self.expect_ident("Expected trait name after 'trait'")?;
        self.expect(&Token::LBrace, "Expected '{' after trait name")?;

        let mut methods = Vec::new();
        while !self.check(&Token::RBrace) && !self.is_at_end() {
            self.expect(&Token::Fn, "Expected 'fn' in trait definition")?;
            let method_name = self.expect_ident("Expected method name in trait definition")?;
            self.expect(&Token::LParen, "Expected '(' after method name")?;

            let mut has_self = false;
            let mut params = Vec::new();

            if !self.check(&Token::RParen) {
                let is_self = match self.peek() {
                    Token::Ident(pname) if pname == "self" => true,
                    _ => false,
                };
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

            methods.push(TraitMethodDef {
                name: method_name,
                has_self,
                params,
                return_type,
            });
        }

        self.expect(&Token::RBrace, "Expected '}' after trait methods")?;
        Ok(Stmt::Trait(TraitDef { name, methods }))
    }

    pub(super) fn parse_impl_statement(&mut self) -> Result<Stmt> {
        self.advance();
        let trait_name = self.expect_ident("Expected trait name after 'impl'")?;
        self.expect(&Token::For, "Expected 'for' after trait name in impl")?;
        let target_type = self.expect_ident("Expected target type after 'for'")?;
        self.expect(&Token::LBrace, "Expected '{' after target type in impl")?;

        let mut methods = Vec::new();
        while !self.check(&Token::RBrace) && !self.is_at_end() {
            self.expect(&Token::Fn, "Expected 'fn' in impl block")?;
            let method_name = self.expect_ident("Expected method name in impl block")?;
            self.expect(&Token::LParen, "Expected '(' after method name")?;

            let mut params = Vec::new();
            if !self.check(&Token::RParen) {
                let is_self = match self.peek() {
                    Token::Ident(pname) if pname == "self" => true,
                    _ => false,
                };
                if is_self {
                    self.advance();
                    let self_type = if self.check(&Token::Colon) {
                        self.advance();
                        self.parse_type()?
                    } else {
                        Type::Named(target_type.clone())
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

            methods.push(FnStmt {
                name: method_name,
                type_params: Vec::new(),
                params,
                return_type,
                body,
            });
        }

        self.expect(&Token::RBrace, "Expected '}' after impl block")?;
        Ok(Stmt::Impl(ImplBlock {
            trait_name,
            target_type,
            methods,
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

    pub(super) fn parse_return_statement(&mut self) -> Result<Stmt> {
        let line = self.current_line();
        let col = self.current_col();
        if self.in_defer {
            return Err(HuziError::new(
                "return is not allowed inside defer",
                line,
                col,
            ));
        }
        self.advance();

        let value = if self.is_expr_start() {
            Some(self.parse_expression()?)
        } else {
            None
        };

        Ok(Stmt::Return(ReturnStmt { value }))
    }

    pub(super) fn parse_defer_statement(&mut self) -> Result<Stmt> {
        let line = self.current_line();
        let col = self.current_col();
        if !self.in_function {
            return Err(HuziError::new(
                "defer is only allowed inside a function",
                line,
                col,
            ));
        }
        if self.in_defer {
            return Err(HuziError::new(
                "nested defer is not allowed",
                line,
                col,
            ));
        }
        self.advance(); // consume 'defer'
        self.in_defer = true;
        let inner = self.parse_statement();
        self.in_defer = false;
        let inner = inner?;
        Ok(Stmt::Defer(Box::new(inner)))
    }

    pub(super) fn parse_if_statement(&mut self) -> Result<Stmt> {
        self.advance();

        let condition = self.parse_expression()?;

        let then_branch = self.parse_block()?;

        let mut elif_branches = Vec::new();
        while self.check_keyword(&[Token::Elif]) {
            self.advance();
            let elif_cond = self.parse_expression()?;
            let elif_block = self.parse_block()?;
            elif_branches.push((elif_cond, elif_block));
        }

        let else_branch = if self.check_keyword(&[Token::Else]) {
            self.advance();
            Some(self.parse_block()?)
        } else {
            None
        };

        Ok(Stmt::If(IfStmt {
            condition,
            then_branch,
            elif_branches,
            else_branch,
        }))
    }

    pub(super) fn parse_for_statement(&mut self) -> Result<Stmt> {
        self.advance();

        let var_name = self.expect_ident("Expected loop variable name")?;

        self.expect(&Token::In, "Expected 'in' after loop variable")?;

        let first = self.parse_expression()?;

        // `for i in start..end` 范围循环,或 `for x in arr` 数组遍历。
        let source = if self.check(&Token::DotDot) {
            self.advance();
            let end = self.parse_expression()?;
            ForSource::Range { start: first, end }
        } else {
            ForSource::Array(first)
        };

        let body = self.parse_block()?;

        Ok(Stmt::For(ForStmt {
            var_name,
            source,
            body,
        }))
    }

    pub(super) fn parse_while_statement(&mut self) -> Result<Stmt> {
        self.advance();

        let condition = self.parse_expression()?;

        let body = self.parse_block()?;

        Ok(Stmt::While(WhileStmt { condition, body }))
    }

    pub(super) fn parse_block(&mut self) -> Result<Block> {
        self.expect(&Token::LBrace, "Expected '{'")?;

        let mut statements = Vec::new();
        while !self.check(&Token::RBrace) && !self.is_at_end() {
            statements.push(self.parse_statement()?);
        }

        self.expect(&Token::RBrace, "Expected '}'")?;

        Ok(Block { statements })
    }
}
