use super::Parser;
use huzi_ast::*;
use huzi_error::HuziError;
use huzi_error::Result;
use huzi_lexer::Token;

impl Parser {
    pub(super) fn parse_expression(&mut self) -> Result<Expr> {
        // 赋值表达式优先级最低：`x = ...`, `arr[i] += ...`
        let expr = self.parse_or_expression()?;

        let op = if self.check(&Token::Equal) {
            Some(AssignOp::Assign)
        } else if self.check(&Token::PlusEq) {
            Some(AssignOp::AddAssign)
        } else if self.check(&Token::MinusEq) {
            Some(AssignOp::SubAssign)
        } else if self.check(&Token::StarEq) {
            Some(AssignOp::MulAssign)
        } else if self.check(&Token::SlashEq) {
            Some(AssignOp::DivAssign)
        } else if self.check(&Token::PercentEq) {
            Some(AssignOp::ModAssign)
        } else {
            None
        };

        if let Some(operator) = op {
            let target = match &expr {
                Expr::Ident(_) | Expr::ArrayIndex(_) | Expr::FieldAccess(_) => expr,
                Expr::Unary(u) if u.operator == UnOp::Deref => expr,
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
                operator,
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

        // 行首 `*` 优先为前缀解引用(不与上一行粘连成乘法);`/`/`%`
        // 不能起始新语句,保持原跨行合并行为不变。
        loop {
            let op = if self.check(&Token::Star) && self.current_line() == self.prev_line() {
                BinOp::Mul
            } else if self.check(&Token::Slash) {
                BinOp::Div
            } else if self.check(&Token::Percent) {
                BinOp::Mod
            } else {
                break;
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
        } else if self.check(&Token::Star) {
            // 前缀解引用 `*b`(与乘法 `*` 复用 Star token,此处为一元位置)。
            self.advance();
            let operand = self.parse_unary_expression()?;
            Ok(Expr::Unary(UnaryExpr {
                operator: UnOp::Deref,
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

        // 后缀操作可链式与交错调用：points[1].x, p.vals[0],
        // f(a).field, ...
        loop {
            if self.check(&Token::LBracket) {
                expr = self.parse_index_suffix(expr)?;
            } else if self.check(&Token::Dot) {
                expr = self.parse_dot_suffix(expr)?;
            } else if self.check(&Token::LParen) {
                expr = self.parse_call_suffix(expr)?;
            } else if self.check(&Token::Question) {
                self.advance();
                expr = Expr::Try(TryExpr {
                    inner: Box::new(expr),
                });
            } else {
                break;
            }
        }

        Ok(expr)
    }

    /// 后缀阶段之一：下标 `base[index]`。
    fn parse_index_suffix(&mut self, base: Expr) -> Result<Expr> {
        self.advance(); // consume '['
        let index = self.parse_expression()?;
        self.expect(&Token::RBracket, "Expected ']' after index")?;
        Ok(Expr::ArrayIndex(ArrayIndexExpr {
            array: Box::new(base),
            index: Box::new(index),
        }))
    }

    /// 后缀阶段之一：点号 `base.field` / `base.0` / `base.method(args)`。
    fn parse_dot_suffix(&mut self, base: Expr) -> Result<Expr> {
        self.advance();
        // 元组元素访问：`t.0`、`t.1`... — 点号后的数字
        // 是元素索引，而非字段名。
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
        let is_tuple_idx = index.is_some();
        let field = match index {
            Some(i) => i,
            None => self.expect_ident("Expected field name after '.'")?,
        };
        if !is_tuple_idx && self.check(&Token::LParen) {
            self.advance();
            let mut arguments = Vec::new();
            while !self.check(&Token::RParen) && !self.is_at_end() {
                arguments.push(self.parse_expression()?);
                if self.check(&Token::Comma) {
                    self.advance();
                }
            }
            self.expect(&Token::RParen, "Expected ')' after arguments")?;
            Ok(Expr::MethodCall(MethodCallExpr {
                receiver: Box::new(base),
                method: field,
                arguments,
            }))
        } else {
            Ok(Expr::FieldAccess(FieldAccessExpr {
                base: Box::new(base),
                field,
            }))
        }
    }

    /// 后缀阶段之一：直接调用 `callee(args)`。
    fn parse_call_suffix(&mut self, callee: Expr) -> Result<Expr> {
        self.advance();

        let mut arguments = Vec::new();
        while !self.check(&Token::RParen) && !self.is_at_end() {
            arguments.push(self.parse_expression()?);

            if self.check(&Token::Comma) {
                self.advance();
            }
        }

        self.expect(&Token::RParen, "Expected ')' after arguments")?;

        Ok(Expr::Call(CallExpr {
            callee: Box::new(callee),
            arguments,
            type_args: Vec::new(),
        }))
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
            Token::FString(s) => {
                self.advance();
                self.parse_fstring(&s)
            }
            Token::Char(c) => {
                self.advance();
                Ok(Expr::Literal(Literal::Char(c)))
            }
            Token::Ident(name) => self.parse_ident_primary(name),
            Token::LParen => {
                self.advance();
                self.parse_paren_or_tuple()
            }
            Token::LBracket => self.parse_array_literal(),
            Token::If => self.parse_if_expression(),
            Token::Match => self.parse_match_expression(),
            Token::BarBar => self.parse_closure_zero_args(),
            Token::Pipe => self.parse_closure(),
            _ => Err(HuziError::new(
                format!("Unexpected token: {}", token),
                self.current_line(),
                self.current_col(),
            )),
        }
    }

    /// 标识起始的主表达式分发：上下文关键字 / 泛型 / 路径 / 字面量。
    fn parse_ident_primary(&mut self, name: String) -> Result<Expr> {
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
        // 泛型调用 `id<i32>(42)` 或结构体字面量 `Pair<i32, str> { ... }`。
        if self.check(&Token::Less) {
            if let Some(expr) = self.try_parse_generic(&name)? {
                return Ok(expr);
            }
        }
        // 路径限定符号，如 `Enum::Variant` / `Enum::Variant(args)` / `pkg::sub::fn(args)`
        if self.check(&Token::PathSep) {
            return self.parse_path_suffix(name);
        }
        // `Point { x: 1, ... }` — 结构体字面量，仅在 `{` 后紧跟 `field:` 时识别，
        // 确保独立代码块仍能正常解析。
        if self.check(&Token::LBrace) && self.looks_like_struct_literal() {
            return self.parse_struct_literal(&name);
        }
        Ok(Expr::Ident(name))
    }

    /// 路径后缀阶段：`name(::seg)*[(args)]`(调用时首个 `::` 尚未消费)。
    fn parse_path_suffix(&mut self, name: String) -> Result<Expr> {
        self.advance();
        let mut variant = self.expect_ident("Expected variant or symbol name after '::'")?;
        while self.check(&Token::PathSep) {
            self.advance();
            let seg = self.expect_ident("Expected identifier after '::'")?;
            variant.push_str("::");
            variant.push_str(&seg);
        }
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
        Ok(Expr::EnumConstruct(EnumConstructExpr {
            enum_name: name,
            variant,
            args,
            type_args: Vec::new(),
        }))
    }

    /// 数组字面量阶段：`[e1, e2, ...]`(调用时 `[` 尚未消费)。
    fn parse_array_literal(&mut self) -> Result<Expr> {
        // 数组字面量: [1, 2, 3]
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

    /// 消费 `(` 之后：`(a, b)` 为元组字面量，单独的 `(expr)`
    /// 仅作为优先级括号。
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
}
