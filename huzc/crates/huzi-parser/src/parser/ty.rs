use super::Parser;
use huzi_ast::*;
use huzi_error::Result;
use huzi_lexer::Token;

impl Parser {
    pub(super) fn parse_type(&mut self) -> Result<Type> {
        if self.check(&Token::LBracket) {
            return self.parse_array_type();
        }
        if self.check(&Token::LParen) {
            return self.parse_tuple_type();
        }
        if self.check(&Token::Fn) {
            return self.parse_fn_type();
        }
        if self.check(&Token::Weak) {
            self.advance();
            let inner = self.parse_type()?;
            match &inner {
                Type::Box(_) => return Ok(Type::Weak(Box::new(inner))),
                _ => {
                    return Err(self.error(format!(
                        "'weak' modifier can only be applied to 'Box<T>' (found '{}')",
                        inner
                    )));
                }
            }
        }
        self.parse_named_or_generic_type()
    }

    fn parse_array_type(&mut self) -> Result<Type> {
        self.advance(); // consume '['
        let elem_type = self.parse_type()?;
        self.expect(&Token::Semi, "Expected ';' in array type")?;
        let size = self.expect_integer("Expected array size")? as usize;
        self.expect(&Token::RBracket, "Expected ']' in array type")?;
        Ok(Type::Array(Box::new(elem_type), size))
    }

    fn parse_tuple_type(&mut self) -> Result<Type> {
        self.advance(); // consume '('
        if self.check(&Token::RParen) {
            self.advance();
            return Ok(Type::Unit);
        }
        let elems = self.parse_comma_separated(&Token::RParen, false, |p, _| p.parse_type())?;
        self.expect(&Token::RParen, "Expected ')' in tuple type")?;
        Ok(Type::Tuple(elems))
    }

    /// 解析函数/闭包类型：`fn(T1, T2) -> Ret`
    fn parse_fn_type(&mut self) -> Result<Type> {
        self.advance(); // consume 'fn'
        self.expect(&Token::LParen, "Expected '(' after 'fn' in function type")?;
        let param_types =
            self.parse_comma_separated(&Token::RParen, false, |p, _| p.parse_type())?;
        self.expect(&Token::RParen, "Expected ')' in function type")?;
        let return_type = if self.check(&Token::Arrow) {
            self.advance();
            self.parse_type()?
        } else {
            Type::Unit
        };
        Ok(Type::Fn(param_types, Box::new(return_type)))
    }

    fn parse_named_or_generic_type(&mut self) -> Result<Type> {
        match self.peek() {
            Token::Ident(name) => {
                let mut name = name.clone();
                self.advance();
                while self.check(&Token::PathSep) {
                    self.advance();
                    let sub = self.expect_ident("Expected type name after '::'")?;
                    name = format!("{}::{}", name, sub);
                }
                if self.check(&Token::Less) {
                    if name == "Box" {
                        return self.parse_box_type();
                    }
                    return self.parse_applied_type(name);
                }
                if self.is_type_param(&name) {
                    Ok(Type::Generic(name))
                } else {
                    Ok(Type::Named(name))
                }
            }
            _ => Err(self.error("Expected type")),
        }
    }

    /// 解析 `Box` 后的 `<T>`(调用时 `<` 尚未消费)。`T` 为具名
    /// 结构体、泛型形参或嵌套 `Box<..>`(递归,最内层须为具名结构体或形参);
    /// 词法上 `>>` 为两个 `Greater`,内外层各消费一个。
    fn parse_box_type(&mut self) -> Result<Type> {
        self.advance(); // consume '<'
        let inner = self.parse_type()?;
        match &inner {
            Type::Named(_) | Type::Box(_) | Type::Generic(_) | Type::Applied(..) => {}
            _ => {
                return Err(self.error(format!(
                    "Box<T> requires a named struct type (found '{}')",
                    inner
                )))
            }
        }
        self.expect(&Token::Greater, "Expected '>' in Box<T>")?;
        Ok(Type::Box(Box::new(inner)))
    }

    /// 解析具名类型后的 `<T1, T2, ...>` 参数化类型(如 `Stack<i32>`, `vec<i32>`)。
    fn parse_applied_type(&mut self, name: String) -> Result<Type> {
        self.advance(); // consume '<'
        let args = self.parse_comma_separated(&Token::Greater, true, |p, _| p.parse_type())?;
        self.expect(&Token::Greater, "Expected '>' after type arguments")?;
        if args.is_empty() {
            return Err(self.error(format!(
                "Type '{}' requires at least one type argument",
                name
            )));
        }
        Ok(Type::Applied(name, args))
    }
}
