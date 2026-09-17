mod expr;
mod pattern;
mod stmt;
#[cfg(test)]
mod tests;

use huzi_ast::*;
use huzi_error::HuziError;
use huzi_error::Result;
use huzi_lexer::SpannedToken;
use huzi_lexer::Token;

pub struct Parser {
    tokens: Vec<SpannedToken>,
    pos: usize,
}

/// 单次 `parse_recoverable` 最多收集的错误数(防级联误报刷屏)。
const MAX_RECOVERY_ERRORS: usize = 32;

impl Parser {
    pub fn new(tokens: Vec<SpannedToken>) -> Self {
        Self { tokens, pos: 0 }
    }

    /// 语句级错误恢复入口:单条语句失败则记错并同步到下一
    /// 同步点,一次返回全部成功语句与全部错误(供 LSP 实时诊断)。
    /// 成功语句的区间仍由 `parse_statement` 经 `with_range` 保留;
    /// 失败语句不入 `Program`。
    pub fn parse_recoverable(&mut self) -> (Program, Vec<HuziError>) {
        let mut statements = Vec::new();
        let mut errors = Vec::new();
        while !self.is_at_end() {
            match self.parse_statement() {
                Ok(stmt) => statements.push(stmt),
                Err(err) => {
                    errors.push(err);
                    if errors.len() >= MAX_RECOVERY_ERRORS {
                        break;
                    }
                    self.synchronize();
                }
            }
        }
        (Program { statements }, errors)
    }

    /// 首错即停的兼容入口:语义与旧 `parse` 一致(返回首错),
    /// 供 huzc 主流程 `unwrap_or_else(die)` 使用。
    pub fn parse(&mut self) -> Result<Program> {
        let (program, errors) = self.parse_recoverable();
        if let Some(first) = errors.into_iter().next() {
            return Err(first);
        }
        Ok(program)
    }

    /// 错误同步:跳到下一个 `;`/`}`/EOF 或下一行行首。
    /// 至少前进一步,避免失败语句原地空转。
    fn synchronize(&mut self) {
        let start = self.pos;
        let fail_line = self.tokens.get(start).map(|t| t.line);
        while !self.is_at_end() {
            if self.check(&Token::Semi) {
                self.advance();
                break;
            }
            if self.check(&Token::RBrace) {
                break;
            }
            let line = self.tokens.get(self.pos).map(|t| t.line);
            if line != fail_line {
                break;
            }
            self.advance();
        }
        if self.pos == start && !self.is_at_end() {
            self.advance();
        }
    }

    /// 解析一条语句并记录其起止区间(供调试行号/断点使用)。
    /// 结束位置取解析成功后上一已消费 token 的列 +1(lexer 无 token
    /// 宽度信息,无法给出精确末列);无历史时用起始 +1 兜底,保证
    /// `end >= start`。
    fn parse_statement(&mut self) -> Result<Spanned<Stmt>> {
        let line = self.current_line();
        let column = self.current_col();
        let node = self.parse_statement_kind()?;
        let (end_line, end_column) = self.stmt_end(line, column);
        Ok(Spanned::with_range(node, line, column, end_line, end_column))
    }

    /// 上一条已消费 token 的结束位置(行,列 +1)。
    fn stmt_end(&self, start_line: usize, start_column: usize) -> (usize, usize) {
        let fallback = (start_line, start_column.saturating_add(1));
        let Some(prev) = self.pos.checked_sub(1).and_then(|i| self.tokens.get(i)) else {
            return fallback;
        };
        if prev.line == usize::MAX {
            return fallback;
        }
        let end = (prev.line, prev.column.saturating_add(1));
        if end < (start_line, start_column) {
            fallback
        } else {
            end
        }
    }

    fn parse_statement_kind(&mut self) -> Result<Stmt> {
        if self.check_keyword(&[Token::Let]) {
            self.parse_let_statement()
        } else if self.check_keyword(&[Token::Struct]) {
            self.parse_struct_statement()
        } else if self.check_keyword(&[Token::Enum]) {
            self.parse_enum_statement()
        } else if self.check_keyword(&[Token::Fn]) {
            self.parse_fn_statement()
        } else if self.check_keyword(&[Token::Import]) {
            self.parse_import_statement()
        } else if self.check_keyword(&[Token::Return]) {
            self.parse_return_statement()
        } else if self.check_keyword(&[Token::Break]) {
            self.advance();
            Ok(Stmt::Break)
        } else if self.check_keyword(&[Token::Continue]) {
            self.advance();
            Ok(Stmt::Continue)
        } else if self.check_keyword(&[Token::If]) {
            self.parse_if_statement()
        } else if self.check_keyword(&[Token::For]) {
            self.parse_for_statement()
        } else if self.check_keyword(&[Token::While]) {
            self.parse_while_statement()
        } else if self.check(&Token::LBrace) {
            Ok(Stmt::Block(self.parse_block()?))
        } else {
            let expr = self.parse_expression()?;
            Ok(Stmt::Expr(ExprStmt { expr }))
        }
    }

    fn parse_type(&mut self) -> Result<Type> {
        // Check for array type: [T; N]
        if self.check(&Token::LBracket) {
            self.advance(); // consume '['
            let elem_type = self.parse_type()?;
            self.expect(&Token::Semi, "Expected ';' in array type")?;
            let size = self.expect_integer("Expected array size")? as usize;
            self.expect(&Token::RBracket, "Expected ']' in array type")?;
            return Ok(Type::Array(Box::new(elem_type), size));
        }

        // Tuple type: () is unit, (T1, T2, ...) is a tuple.
        if self.check(&Token::LParen) {
            self.advance(); // consume '('
            if self.check(&Token::RParen) {
                self.advance();
                return Ok(Type::Unit);
            }
            let mut elems = vec![self.parse_type()?];
            while self.check(&Token::Comma) {
                self.advance();
                elems.push(self.parse_type()?);
            }
            self.expect(&Token::RParen, "Expected ')' in tuple type")?;
            return Ok(Type::Tuple(elems));
        }

        let ty = match self.peek() {
            Token::Ident(name) => {
                let name = name.clone();
                self.advance();
                // `Box<T>` — 全语言唯一的尖括号泛型;其它名字后跟 `<`
                // 是非法的(调用点负责报更友好的比较/泛型错误)。
                if self.check(&Token::Less) {
                    if name != "Box" {
                        return Err(HuziError::new(
                            format!(
                                "Only Box<T> supports generic parameters (found '{}<')",
                                name
                            ),
                            self.current_line(),
                            self.current_col(),
                        ));
                    }
                    return self.parse_box_type();
                }
                Type::Named(name)
            }
            _ => {
                return Err(HuziError::new(
                    "Expected type",
                    self.current_line(),
                    self.current_col(),
                ))
            }
        };
        Ok(ty)
    }

    /// 解析 `Box` 后的 `<T>`(调用时 `<` 尚未消费)。`T` 须为具名
    /// 结构体,嵌套 `Box<Box<..>>` 暂不支持。
    fn parse_box_type(&mut self) -> Result<Type> {
        self.advance(); // consume '<'
        let inner = self.parse_type()?;
        match &inner {
            Type::Named(_) => {}
            Type::Box(_) => {
                return Err(HuziError::new(
                    "Nested Box<Box<..>> is not supported yet; use a struct field instead",
                    self.current_line(),
                    self.current_col(),
                ))
            }
            _ => {
                return Err(HuziError::new(
                    format!("Box<T> requires a named struct type (found '{}')", inner),
                    self.current_line(),
                    self.current_col(),
                ))
            }
        }
        self.expect(&Token::Greater, "Expected '>' in Box<T>")?;
        Ok(Type::Box(Box::new(inner)))
    }

    fn is_expr_start(&self) -> bool {
        matches!(
            self.peek(),
            Token::True
                | Token::False
                | Token::Int(_)
                | Token::Float(_)
                | Token::String(_)
                | Token::Char(_)
                | Token::Ident(_)
                | Token::LParen
                | Token::LBracket
                | Token::If
                | Token::Match
                | Token::Bang
                | Token::Minus
        )
    }

    /// 把单个表达式包成单语句块(if 表达式 / match 手臂的语法糖),
    /// 语句位置取当前 token,即该表达式的起始位置。
    fn expr_block(&self, expr: Expr, line: usize, column: usize) -> Block {
        Block {
            statements: vec![Spanned::new(Stmt::Expr(ExprStmt { expr }), line, column)],
        }
    }

    fn check(&self, token: &Token) -> bool {
        if self.is_at_end() {
            return false;
        }
        let peek_token = self.peek();
        match (peek_token, token) {
            (Token::Int(_), Token::Int(_)) => true,
            (Token::Float(_), Token::Float(_)) => true,
            (Token::String(_), Token::String(_)) => true,
            (Token::Char(_), Token::Char(_)) => true,
            (Token::Ident(_), Token::Ident(_)) => true,
            _ => std::mem::discriminant(peek_token) == std::mem::discriminant(token),
        }
    }

    fn check_keyword(&self, tokens: &[Token]) -> bool {
        tokens.iter().any(|t| self.check(t))
    }

    fn advance(&mut self) {
        if !self.is_at_end() {
            self.pos += 1;
        }
    }

    fn peek(&self) -> &Token {
        &self.tokens[self.pos].token
    }

    fn peek_at(&self, offset: usize) -> Option<&Token> {
        self.tokens.get(self.pos + offset).map(|t| &t.token)
    }

    fn current_line(&self) -> usize {
        self.tokens
            .get(self.pos)
            .map(|t| t.line)
            .unwrap_or(usize::MAX)
    }

    fn current_col(&self) -> usize {
        self.tokens
            .get(self.pos)
            .map(|t| t.column)
            .unwrap_or(usize::MAX)
    }

    fn expect(&mut self, token: &Token, msg: &str) -> Result<()> {
        if self.check(token) {
            self.advance();
            Ok(())
        } else {
            Err(HuziError::new(msg, self.current_line(), self.current_col()))
        }
    }

    fn expect_ident(&mut self, msg: &str) -> Result<String> {
        let token = self.tokens.get(self.pos).map(|t| t.token.clone());
        if let Some(Token::Ident(name)) = token {
            self.advance();
            Ok(name)
        } else {
            Err(HuziError::new(msg, self.current_line(), self.current_col()))
        }
    }

    fn expect_integer(&mut self, msg: &str) -> Result<i64> {
        let token = self.tokens.get(self.pos).map(|t| t.token.clone());
        if let Some(Token::Int(n)) = token {
            self.advance();
            Ok(n)
        } else {
            Err(HuziError::new(msg, self.current_line(), self.current_col()))
        }
    }

    fn is_at_end(&self) -> bool {
        self.pos >= self.tokens.len() || self.peek() == &Token::Eof
    }
}
