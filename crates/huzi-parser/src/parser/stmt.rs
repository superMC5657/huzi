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
        while self.check(&Token::Dot) {
            self.advance();
            let seg = self.expect_ident("Expected identifier after '.' in import")?;
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

    pub(super) fn parse_struct_statement(&mut self) -> Result<Stmt> {
        self.advance();

        let name = self.expect_ident("Expected struct name")?;

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

        Ok(Stmt::Struct(StructDef { name, fields }))
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

    pub(super) fn parse_fn_statement(&mut self) -> Result<Stmt> {
        self.advance();

        let name = self.expect_ident("Expected function name")?;

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
        let body = body_res?;

        Ok(Stmt::Fn(FnStmt {
            name,
            params,
            return_type,
            body,
        }))
    }

    pub(super) fn parse_return_statement(&mut self) -> Result<Stmt> {
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
        self.advance(); // consume 'defer'
        let inner = self.parse_statement()?;
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
