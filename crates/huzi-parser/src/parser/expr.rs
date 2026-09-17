use super::Parser;
use huzi_ast::*;
use huzi_error::HuziError;
use huzi_error::Result;
use huzi_lexer::Token;

impl Parser {
    pub(super) fn parse_expression(&mut self) -> Result<Expr> {
        // Assignment is the lowest-precedence expression: `x = ...`, `arr[i] = ...`
        let expr = self.parse_or_expression()?;

        if self.check(&Token::Equal) {
            let target = match &expr {
                Expr::Ident(_) | Expr::ArrayIndex(_) | Expr::FieldAccess(_) => expr,
                _ => {
                    return Err(HuziError::new(
                        "Invalid assignment target",
                        self.current_line(),
                        self.current_col(),
                    ))
                }
            };
            self.advance();
            let value = self.parse_expression()?;
            return Ok(Expr::Assign(AssignExpr {
                target: Box::new(target),
                value: Box::new(value),
            }));
        }

        Ok(expr)
    }

    fn parse_or_expression(&mut self) -> Result<Expr> {
        let mut left = self.parse_and_expression()?;

        while self.check(&Token::BarBar) {
            self.advance();
            let right = self.parse_and_expression()?;
            left = Expr::Binary(BinaryExpr {
                left: Box::new(left),
                operator: BinOp::Or,
                right: Box::new(right),
            });
        }

        Ok(left)
    }

    fn parse_and_expression(&mut self) -> Result<Expr> {
        let mut left = self.parse_equality_expression()?;

        while self.check(&Token::AmpAmp) {
            self.advance();
            let right = self.parse_equality_expression()?;
            left = Expr::Binary(BinaryExpr {
                left: Box::new(left),
                operator: BinOp::And,
                right: Box::new(right),
            });
        }

        Ok(left)
    }

    fn parse_equality_expression(&mut self) -> Result<Expr> {
        let mut left = self.parse_comparison_expression()?;

        while self.check(&Token::EqualEqual) || self.check(&Token::BangEqual) {
            let op = if self.check(&Token::EqualEqual) {
                BinOp::Eq
            } else {
                BinOp::Neq
            };
            self.advance();
            let right = self.parse_comparison_expression()?;
            left = Expr::Binary(BinaryExpr {
                left: Box::new(left),
                operator: op,
                right: Box::new(right),
            });
        }

        Ok(left)
    }

    fn parse_comparison_expression(&mut self) -> Result<Expr> {
        let mut left = self.parse_additive_expression()?;

        while self.check(&Token::Less)
            || self.check(&Token::LessEqual)
            || self.check(&Token::Greater)
            || self.check(&Token::GreaterEqual)
        {
            let op = if self.check(&Token::Less) {
                BinOp::Lt
            } else if self.check(&Token::LessEqual) {
                BinOp::Le
            } else if self.check(&Token::Greater) {
                BinOp::Gt
            } else {
                BinOp::Ge
            };
            self.advance();
            let right = self.parse_additive_expression()?;
            left = Expr::Binary(BinaryExpr {
                left: Box::new(left),
                operator: op,
                right: Box::new(right),
            });
        }

        Ok(left)
    }

    fn parse_additive_expression(&mut self) -> Result<Expr> {
        let mut left = self.parse_multiplicative_expression()?;

        while self.check(&Token::Plus) || self.check(&Token::Minus) {
            let op = if self.check(&Token::Plus) {
                BinOp::Add
            } else {
                BinOp::Sub
            };
            self.advance();
            let right = self.parse_multiplicative_expression()?;
            left = Expr::Binary(BinaryExpr {
                left: Box::new(left),
                operator: op,
                right: Box::new(right),
            });
        }

        Ok(left)
    }

    fn parse_multiplicative_expression(&mut self) -> Result<Expr> {
        let mut left = self.parse_unary_expression()?;

        while self.check(&Token::Star) || self.check(&Token::Slash) || self.check(&Token::Percent) {
            let op = if self.check(&Token::Star) {
                BinOp::Mul
            } else if self.check(&Token::Slash) {
                BinOp::Div
            } else {
                BinOp::Mod
            };
            self.advance();
            let right = self.parse_unary_expression()?;
            left = Expr::Binary(BinaryExpr {
                left: Box::new(left),
                operator: op,
                right: Box::new(right),
            });
        }

        Ok(left)
    }

    fn parse_unary_expression(&mut self) -> Result<Expr> {
        if self.check(&Token::Bang) {
            self.advance();
            let operand = self.parse_unary_expression()?;
            Ok(Expr::Unary(UnaryExpr {
                operator: UnOp::Not,
                operand: Box::new(operand),
            }))
        } else if self.check(&Token::Minus) {
            self.advance();
            let operand = self.parse_unary_expression()?;
            Ok(Expr::Unary(UnaryExpr {
                operator: UnOp::Neg,
                operand: Box::new(operand),
            }))
        } else {
            self.parse_call_expression()
        }
    }

    fn parse_call_expression(&mut self) -> Result<Expr> {
        let mut expr = self.parse_primary_expression()?;

        // Postfix operations can chain and interleave: points[1].x, p.vals[0],
        // f(a).field, ...
        loop {
            if self.check(&Token::LBracket) {
                self.advance(); // consume '['
                let index = self.parse_expression()?;
                self.expect(&Token::RBracket, "Expected ']' after index")?;
                expr = Expr::ArrayIndex(ArrayIndexExpr {
                    array: Box::new(expr),
                    index: Box::new(index),
                });
            } else if self.check(&Token::Dot) {
                self.advance();
                // Tuple element access: `t.0`, `t.1`, ... — a digit after the
                // dot is an element index, not a field name.
                let index = if self.is_at_end() {
                    None
                } else {
                    match self.peek().clone() {
                        Token::Int(n) => {
                            self.advance();
                            Some(n.to_string())
                        }
                        _ => None,
                    }
                };
                let field = match index {
                    Some(i) => i,
                    None => self.expect_ident("Expected field name after '.'")?,
                };
                expr = Expr::FieldAccess(FieldAccessExpr {
                    base: Box::new(expr),
                    field,
                });
            } else if self.check(&Token::LParen) {
                self.advance();

                let mut arguments = Vec::new();
                while !self.check(&Token::RParen) && !self.is_at_end() {
                    arguments.push(self.parse_expression()?);

                    if self.check(&Token::Comma) {
                        self.advance();
                    }
                }

                self.expect(&Token::RParen, "Expected ')' after arguments")?;

                expr = Expr::Call(CallExpr {
                    callee: Box::new(expr),
                    arguments,
                });
            } else {
                break;
            }
        }

        Ok(expr)
    }

    fn parse_primary_expression(&mut self) -> Result<Expr> {
        let token = self.peek().clone();

        match token {
            Token::True => {
                self.advance();
                Ok(Expr::Literal(Literal::Bool(true)))
            }
            Token::False => {
                self.advance();
                Ok(Expr::Literal(Literal::Bool(false)))
            }
            Token::Int(n) => {
                self.advance();
                Ok(Expr::Literal(Literal::Int(n)))
            }
            Token::Float(n) => {
                self.advance();
                Ok(Expr::Literal(Literal::Float(n)))
            }
            Token::String(s) => {
                self.advance();
                Ok(Expr::Literal(Literal::String(s)))
            }
            Token::Char(c) => {
                self.advance();
                Ok(Expr::Literal(Literal::Char(c)))
            }
            Token::Ident(name) => {
                self.advance();
                // `box(expr)` — 堆分配构造(上下文关键字,小写)。
                if name == "box" && self.check(&Token::LParen) {
                    return self.parse_box_alloc();
                }
                // `null` — 空指针字面量(上下文关键字)。
                if name == "null" {
                    return Ok(Expr::Null);
                }
                // `vec<T>()` — 空 vec 构造(零长,元素类型由尖括号指定)。
                // 仅当 `<` 后能完整解析为 `Type>()` 时才提交,避免把
                // `vec < x` 比较误解析为泛型构造。
                if name == "vec" && self.check(&Token::Less) {
                    if let Some(expr) = self.try_parse_vec_empty()? {
                        return Ok(expr);
                    }
                }
                // `Enum::Variant` / `Enum::Variant(args)` — variant construction.
                if self.check(&Token::PathSep) {
                    self.advance();
                    let variant = self.expect_ident("Expected variant name after '::'")?;
                    let args = if self.check(&Token::LParen) {
                        self.advance();
                        let mut args = Vec::new();
                        while !self.check(&Token::RParen) && !self.is_at_end() {
                            args.push(self.parse_expression()?);
                            if self.check(&Token::Comma) {
                                self.advance();
                            }
                        }
                        self.expect(&Token::RParen, "Expected ')' after variant arguments")?;
                        args
                    } else {
                        Vec::new()
                    };
                    return Ok(Expr::EnumConstruct(EnumConstructExpr {
                        enum_name: name,
                        variant,
                        args,
                    }));
                }
                // `Point { x: 1, ... }` — a struct literal, recognized only when
                // `{` is followed by `field:`, so bare blocks still parse.
                if self.check(&Token::LBrace) && self.looks_like_struct_literal() {
                    return self.parse_struct_literal(&name);
                }
                Ok(Expr::Ident(name))
            }
            Token::LParen => {
                self.advance();
                return self.parse_paren_or_tuple();
            }
            Token::LBracket => {
                // Array literal: [1, 2, 3]
                self.advance(); // consume '['
                let mut elements = Vec::new();
                if !self.check(&Token::RBracket) {
                    loop {
                        elements.push(self.parse_expression()?);
                        if self.check(&Token::Comma) {
                            self.advance();
                        } else {
                            break;
                        }
                    }
                }
                self.expect(&Token::RBracket, "Expected ']' after array literal")?;
                Ok(Expr::ArrayLiteral(elements))
            }
            Token::If => self.parse_if_expression(),
            Token::Match => self.parse_match_expression(),
            _ => Err(HuziError::new(
                format!("Unexpected token: {}", token),
                self.current_line(),
                self.current_col(),
            )),
        }
    }

    /// After `(` is consumed: `(a, b)` is a tuple literal, a bare `(expr)` is
    /// just grouping.
    fn parse_paren_or_tuple(&mut self) -> Result<Expr> {
        let first = self.parse_expression()?;

        if !self.check(&Token::Comma) {
            self.expect(&Token::RParen, "Expected ')' after expression")?;
            return Ok(first);
        }

        let mut elements = vec![first];
        while self.check(&Token::Comma) {
            self.advance();
            elements.push(self.parse_expression()?);
        }
        self.expect(&Token::RParen, "Expected ')' after tuple literal")?;
        Ok(Expr::TupleLiteral(elements))
    }

    /// True if the upcoming tokens look like `{ field: ... }` — the shape of a
    /// struct literal body (a bare block cannot start with `ident :`).
    fn looks_like_struct_literal(&self) -> bool {
        matches!(self.peek_at(1), Some(Token::Ident(_))) && matches!(self.peek_at(2), Some(Token::Colon))
    }

    /// Parse `{ field: expr, ... }` after the struct name was consumed.
    fn parse_struct_literal(&mut self, name: &str) -> Result<Expr> {
        self.expect(&Token::LBrace, "Expected '{' in struct literal")?;

        let mut fields = Vec::new();
        while !self.check(&Token::RBrace) && !self.is_at_end() {
            let field_name = self.expect_ident("Expected field name in struct literal")?;

            self.expect(&Token::Colon, "Expected ':' after field name in struct literal")?;
            let value = self.parse_expression()?;

            fields.push((field_name, value));

            if self.check(&Token::Comma) {
                self.advance();
            } else {
                break;
            }
        }

        self.expect(&Token::RBrace, "Expected '}' after struct literal fields")?;

        Ok(Expr::StructLiteral(StructLiteralExpr {
            name: name.to_string(),
            fields,
        }))
    }

    /// If used as an expression: `let m = if c { a } else { b }`, with
    /// `elif` chains folded into a nested expression.
    fn parse_if_expression(&mut self) -> Result<Expr> {
        self.advance();
        let condition = self.parse_expression()?;
        let then_branch = self.parse_block()?;

        let mut elif_branches: Vec<(Expr, Block, usize, usize)> = Vec::new();
        while self.check(&Token::Elif) {
            let (elif_line, elif_col) = (self.current_line(), self.current_col());
            self.advance();
            let elif_cond = self.parse_expression()?;
            let elif_block = self.parse_block()?;
            elif_branches.push((elif_cond, elif_block, elif_line, elif_col));
        }

        self.expect(&Token::Else, "Expected 'else' after if expression")?;

        let else_block = if self.check(&Token::If) {
            let (nested_line, nested_col) = (self.current_line(), self.current_col());
            let nested = self.parse_if_expression()?;
            self.expr_block(nested, nested_line, nested_col)
        } else {
            self.parse_block()?
        };

        let else_branch = Self::fold_elif_expr(&elif_branches, else_block);

        Ok(Expr::If(IfExpr {
            condition: Box::new(condition),
            then_branch,
            else_branch,
        }))
    }

    /// Fold elif branches into nested if expressions as the else block.
    /// 每层折叠出的合成语句继承对应 `elif` 关键字的位置。
    fn fold_elif_expr(elifs: &[(Expr, Block, usize, usize)], else_b: Block) -> Block {
        match elifs.split_first() {
            None => else_b,
            Some(((cond, block, line, col), rest)) => {
                let nested = Expr::If(IfExpr {
                    condition: Box::new(cond.clone()),
                    then_branch: block.clone(),
                    else_branch: Self::fold_elif_expr(rest, else_b),
                });
                Self::synth_block_at(nested, *line, *col)
            }
        }
    }

    /// 与 `Parser::expr_block` 等价的静态版本(fold 递归中没有 parser 可用)。
    fn synth_block_at(expr: Expr, line: usize, col: usize) -> Block {
        Block {
            statements: vec![Spanned::new(Stmt::Expr(ExprStmt { expr }), line, col)],
        }
    }

    /// 解析 `box` 后的 `(expr)`(调用时 `(` 尚未消费)。
    fn parse_box_alloc(&mut self) -> Result<Expr> {
        self.expect(&Token::LParen, "Expected '(' after 'box'")?;
        let inner = self.parse_expression()?;
        self.expect(&Token::RParen, "Expected ')' after box expression")?;
        Ok(Expr::BoxAlloc(Box::new(inner)))
    }

    /// 尝试解析 `vec` 后的 `<T>()`(调用时 `<` 尚未消费)。
    /// 完整匹配 `Type>()` 才返回 `Some`,否则回退调用点并返回
    /// `None`(外层按普通 `vec` 标识继续解析,不误伤 `vec < x` 比较)。
    fn try_parse_vec_empty(&mut self) -> Result<Option<Expr>> {
        let saved = self.pos;
        // `<` 已由调用方确认存在。
        self.advance();
        let elem_ty = match self.parse_type() {
            Ok(t) => t,
            Err(_) => {
                self.pos = saved;
                return Ok(None);
            }
        };
        if !self.check(&Token::Greater) {
            self.pos = saved;
            return Ok(None);
        }
        self.advance();
        if !self.check(&Token::LParen) {
            self.pos = saved;
            return Ok(None);
        }
        self.advance();
        if !self.check(&Token::RParen) {
            // `vec<T>(args)` 暂不支持:空 vec 必须无参(位置停在 `(` 处)。
            return Err(HuziError::new(
                "vec<T>() takes no arguments (empty vec has no elements)",
                self.current_line(),
                self.current_col(),
            ));
        }
        self.advance();
        Ok(Some(Expr::VecEmpty(elem_ty)))
    }
}
