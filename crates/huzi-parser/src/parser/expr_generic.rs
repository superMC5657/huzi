use super::Parser;
use huzi_ast::*;
use huzi_error::{HuziError, Result};
use huzi_lexer::Token;

impl Parser {
    pub(super) fn looks_like_struct_literal(&self) -> bool {
        matches!(self.peek_at(1), Some(Token::Ident(_))) && matches!(self.peek_at(2), Some(Token::Colon))
    }

    /// Parse `{ field: expr, ... }` after the struct name was consumed.
    pub(super) fn parse_struct_literal(&mut self, name: &str) -> Result<Expr> {
        let fields = self.parse_struct_fields()?;
        Ok(Expr::StructLiteral(StructLiteralExpr {
            name: name.to_string(),
            fields,
            type_args: Vec::new(),
        }))
    }

    pub(super) fn parse_struct_fields(&mut self) -> Result<Vec<(String, Expr)>> {
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
        Ok(fields)
    }

    /// 尝试解析泛型调用 `name<T1, T2>(args)` 或泛型结构体字面量 `name<T1, T2> { ... }`。
    /// 未匹配或后续非 `(` / `{` 时安全回退并返回 `None`。
    pub(super) fn try_parse_generic(&mut self, name: &str) -> Result<Option<Expr>> {
        let saved = self.pos;
        if !self.check(&Token::Less) {
            return Ok(None);
        }
        self.advance(); // consume '<'
        let mut type_args = Vec::new();
        while !self.check(&Token::Greater) && !self.is_at_end() {
            match self.parse_type() {
                Ok(ty) => type_args.push(ty),
                Err(_) => {
                    self.pos = saved;
                    return Ok(None);
                }
            }
            if self.check(&Token::Comma) {
                self.advance();
            } else {
                break;
            }
        }
        if type_args.is_empty() || !self.check(&Token::Greater) {
            self.pos = saved;
            return Ok(None);
        }
        self.advance(); // consume '>'

        // Generic function call: name<T1, T2>(args)
        if self.check(&Token::LParen) {
            self.advance(); // consume '('
            let mut arguments = Vec::new();
            while !self.check(&Token::RParen) && !self.is_at_end() {
                arguments.push(self.parse_expression()?);
                if self.check(&Token::Comma) {
                    self.advance();
                }
            }
            self.expect(&Token::RParen, "Expected ')' after arguments")?;
            return Ok(Some(Expr::Call(CallExpr {
                callee: Box::new(Expr::Ident(name.to_string())),
                arguments,
                type_args,
            })));
        }

        // Generic struct literal: name<T1, T2> { field: value }
        if self.check(&Token::LBrace) && self.looks_like_struct_literal() {
            let fields = self.parse_struct_fields()?;
            return Ok(Some(Expr::StructLiteral(StructLiteralExpr {
                name: name.to_string(),
                fields,
                type_args,
            })));
        }

        self.pos = saved;
        Ok(None)
    }

    /// 解析 `box` 后的 `(expr)`(调用时 `(` 尚未消费)。
    pub(super) fn parse_box_alloc(&mut self) -> Result<Expr> {
        self.expect(&Token::LParen, "Expected '(' after 'box'")?;
        let inner = self.parse_expression()?;
        self.expect(&Token::RParen, "Expected ')' after box expression")?;
        Ok(Expr::BoxAlloc(Box::new(inner)))
    }

    /// 尝试解析 `vec` 后的 `<T>()`(调用时 `<` 尚未消费)。
    /// 完整匹配 `Type>()` 才返回 `Some`,否则回退调用点并返回
    /// `None`(外层按普通 `vec` 标识继续解析,不误伤 `vec < x` 比较)。
    pub(super) fn try_parse_vec_empty(&mut self) -> Result<Option<Expr>> {
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
