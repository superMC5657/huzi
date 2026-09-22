use super::Parser;
use huzi_ast::*;
use huzi_error::Result;
use huzi_lexer::Token;

impl Parser {
    /// 作为表达式使用的 if：`let m = if c { a } else { b }`，其中
    /// `elif` 链折叠为嵌套表达式。
    pub(super) fn parse_if_expression(&mut self) -> Result<Expr> {
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

    /// 将 elif 分支折叠为嵌套 if 表达式作为 else 块。
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
}
