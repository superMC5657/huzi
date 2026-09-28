use super::Parser;
use huzi_ast::*;
use huzi_error::Result;
use huzi_lexer::Token;

impl Parser {
    /// 主表达式分发：字面量 / 标识 / 括号 / 数组 / if / match / 闭包。
    pub(super) fn parse_primary_expression(&mut self) -> Result<Expr> {
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
            _ => Err(self.error(format!("Unexpected token: {}", token))),
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
