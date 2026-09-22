use super::CodeGen;
use huzi_ast::*;
use huzi_error::{HuziError, Result};


impl<'ctx> CodeGen<'ctx> {
    pub(super) fn compile_if(&mut self, stmt: &IfStmt, span: Span) -> Result<()> {
        // Fold the elif chain into nested if/else so each branch is compiled.
        let else_block: Option<Block> = if stmt.elif_branches.is_empty() {
            stmt.else_branch.clone()
        } else {
            let nested = Self::fold_elif(&stmt.elif_branches, stmt.else_branch.as_ref(), span);
            Some(Block {
                statements: vec![Spanned::with_span(Stmt::If(nested), span)],
            })
        };

        self.compile_branch(&stmt.condition, &stmt.then_branch, else_block.as_ref())
    }

    pub(super) fn fold_elif(
        elifs: &[(Expr, Block)],
        else_b: Option<&Block>,
        span: Span,
    ) -> IfStmt {
        let (first, rest) = elifs.split_first().expect("elif list is not empty");
        let inner_else = if rest.is_empty() {
            else_b.cloned()
        } else {
            Some(Block {
                statements: vec![Spanned::with_span(
                    Stmt::If(Self::fold_elif(rest, else_b, span)),
                    span,
                )],
            })
        };
        IfStmt {
            condition: first.0.clone(),
            then_branch: first.1.clone(),
            elif_branches: Vec::new(),
            else_branch: inner_else,
        }
    }

    pub(super) fn compile_branch(
        &mut self,
        condition: &Expr,
        then_b: &Block,
        else_b: Option<&Block>,
    ) -> Result<()> {
        let cond_value = self.compile_expr(condition)?;
        let cond = self.to_i1(cond_value)?;

        let function = self.current_function()?;

        let then_block = self.context.append_basic_block(function, "then");
        let else_block = self.context.append_basic_block(function, "else");
        let merge_block = self.context.append_basic_block(function, "merge");

        self.builder
            .build_conditional_branch(cond, then_block, else_block)
            .unwrap();

        self.builder.position_at_end(then_block);
        self.compile_block(then_b)?;
        let then_open = self.at_open_end();
        if then_open {
            self.builder
                .build_unconditional_branch(merge_block)
                .unwrap();
        }

        self.builder.position_at_end(else_block);
        if let Some(else_branch) = else_b {
            self.compile_block(else_branch)?;
        }
        let else_open = self.at_open_end();
        if else_open {
            self.builder
                .build_unconditional_branch(merge_block)
                .unwrap();
        }

        self.builder.position_at_end(merge_block);

        // If every branch returned, the merge block is unreachable.
        if !then_open && !else_open {
            self.builder.build_unreachable().unwrap();
        }

        Ok(())
    }

    pub(super) fn compile_break(&mut self) -> Result<()> {
        let (_, break_target) = *self
            .loop_stack
            .last()
            .ok_or_else(|| HuziError::new_global("`break` outside of a loop"))?;
        self.builder
            .build_unconditional_branch(break_target)
            .unwrap();
        self.start_dead_block()
    }

    pub(super) fn compile_continue(&mut self) -> Result<()> {
        let (continue_target, _) = *self
            .loop_stack
            .last()
            .ok_or_else(|| HuziError::new_global("`continue` outside of a loop"))?;
        self.builder
            .build_unconditional_branch(continue_target)
            .unwrap();
        self.start_dead_block()
    }

    /// After a break/continue the current block is terminated; move to a fresh
    /// block so following statements still have somewhere to go.
    pub(super) fn start_dead_block(&mut self) -> Result<()> {
        let function = self.current_function()?;
        let dead = self.context.append_basic_block(function, "dead");
        self.builder.position_at_end(dead);
        Ok(())
    }

    pub(super) fn compile_while(&mut self, stmt: &WhileStmt) -> Result<()> {
        let function = self.current_function()?;

        let cond_block = self.context.append_basic_block(function, "while_cond");
        let body_block = self.context.append_basic_block(function, "while_body");
        let after_block = self.context.append_basic_block(function, "while_after");

        self.loop_stack.push((cond_block, after_block));

        self.builder
            .build_unconditional_branch(cond_block)
            .unwrap();

        // Re-evaluate the condition on every iteration.
        self.builder.position_at_end(cond_block);
        let cond_value = self.compile_expr(&stmt.condition)?;
        let condition = self.to_i1(cond_value)?;
        self.builder
            .build_conditional_branch(condition, body_block, after_block)
            .unwrap();

        self.builder.position_at_end(body_block);
        self.compile_block(&stmt.body)?;
        self.builder
            .build_unconditional_branch(cond_block)
            .unwrap();

        self.loop_stack.pop();

        self.builder.position_at_end(after_block);

        Ok(())
    }
}
