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

    /// 解析导出语句：`export calc`、`export calc::*`、`export calc::add`
    pub(super) fn parse_export_statement(&mut self) -> Result<Stmt> {
        self.advance();
        let mut path = self.expect_ident("Expected identifier or module name after 'export'")?;
        let mut is_wildcard = false;
        while self.check(&Token::Dot) || self.check(&Token::PathSep) {
            self.advance();
            if self.check(&Token::Star) {
                self.advance();
                is_wildcard = true;
                break;
            }
            let seg = self.expect_ident("Expected identifier or '*' after '.' or '::' in export")?;
            path.push_str("::");
            path.push_str(&seg);
        }
        if self.check(&Token::Semi) {
            self.advance();
        }
        Ok(Stmt::Export(ExportStmt { path, is_wildcard }))
    }

    pub(super) fn parse_let_statement(&mut self) -> Result<Stmt> {
        self.advance();

        // `let mut name` 或 `let name` 或 `let (a, b)` 或 `let mut (a, b)`
        let mutable = self.check(&Token::Mut);
        if mutable {
            self.advance();
        }

        let (name, tuple_pattern) = if self.check(&Token::LParen) {
            let items = self.parse_tuple_pattern(mutable)?;
            (String::new(), Some(items))
        } else {
            let name = self.expect_ident("Expected variable name")?;
            (name, None)
        };

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
            tuple_pattern,
            type_annotation,
            value,
        }))
    }

    /// 解析元组解构模式：`(a, b)` / `(mut a, b)` / `(a, (b, c))`
    fn parse_tuple_pattern(&mut self, inherited_mut: bool) -> Result<Vec<LetPatternItem>> {
        self.expect(&Token::LParen, "Expected '(' for tuple pattern")?;
        let mut items = Vec::new();
        while !self.check(&Token::RParen) && !self.is_at_end() {
            let item_mut = if self.check(&Token::Mut) {
                self.advance();
                true
            } else {
                inherited_mut
            };
            if self.check(&Token::LParen) {
                let sub_items = self.parse_tuple_pattern(item_mut)?;
                items.push(LetPatternItem::Tuple(sub_items));
            } else {
                let name = self.expect_ident("Expected variable name in tuple pattern")?;
                items.push(LetPatternItem::Ident {
                    name,
                    mutable: item_mut,
                });
            }
            if self.check(&Token::Comma) {
                self.advance();
            } else {
                break;
            }
        }
        self.expect(&Token::RParen, "Expected ')' to close tuple pattern")?;
        Ok(items)
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
        self.advance(); // 消费 'defer'
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
