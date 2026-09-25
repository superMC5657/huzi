//! 方法调用脱糖的语句/块遍历入口。
//!
//! 子模块:`stmt`(for 语句)、`expr`(表达式改写分发)、`method`
//! (方法解析与 UFCS 降解)、`infer`(接收者类型推导)。

mod expr;
mod infer;
mod method;
mod stmt;

use super::TraitDesugarer;
use huzi_ast::*;
use huzi_error::Result;
use std::collections::HashMap;

impl TraitDesugarer {
    pub(super) fn resolve_stmt(&self, stmt: &mut Stmt) -> Result<()> {
        let mut local_env = HashMap::new();
        self.resolve_stmt_scoped(stmt, &mut local_env)
    }

    pub(super) fn resolve_stmt_scoped(
        &self,
        stmt: &mut Stmt,
        env: &mut HashMap<String, Type>,
    ) -> Result<()> {
        match stmt {
            Stmt::Let(l) => {
                if let Some(val) = &mut l.value {
                    self.resolve_expr(val, env)?;
                }
                let ty = if let Some(ann) = &l.type_annotation {
                    ann.clone()
                } else if let Some(val) = &l.value {
                    self.infer_expr_type(val, env).unwrap_or(Type::I32)
                } else {
                    Type::I32
                };
                env.insert(l.name.clone(), ty);
            }
            Stmt::Expr(e) => self.resolve_expr(&mut e.expr, env)?,
            Stmt::Return(r) => {
                if let Some(val) = &mut r.value {
                    self.resolve_expr(val, env)?;
                }
            }
            Stmt::Block(b) => self.resolve_block(b, env)?,
            Stmt::If(i) => {
                self.resolve_expr(&mut i.condition, env)?;
                self.resolve_block(&mut i.then_branch, env)?;
                for (cond, blk) in &mut i.elif_branches {
                    self.resolve_expr(cond, env)?;
                    self.resolve_block(blk, env)?;
                }
                if let Some(b) = &mut i.else_branch {
                    self.resolve_block(b, env)?;
                }
            }
            Stmt::For(f) => self.resolve_for_stmt(f, env)?,
            Stmt::While(w) => {
                self.resolve_expr(&mut w.condition, env)?;
                self.resolve_block(&mut w.body, env)?;
            }
            Stmt::Defer(d) => self.resolve_stmt_scoped(&mut d.node, env)?,
            Stmt::Fn(f) => {
                let mut fn_env = env.clone();
                for p in &f.params {
                    fn_env.insert(p.name.clone(), p.param_type.clone());
                }
                self.resolve_block(&mut f.body, &fn_env)?;
            }
            _ => {}
        }
        Ok(())
    }

    pub(super) fn resolve_block(
        &self,
        block: &mut Block,
        env: &HashMap<String, Type>,
    ) -> Result<()> {
        let mut scope_env = env.clone();
        for stmt in &mut block.statements {
            let span = stmt.span;
            self.resolve_stmt_scoped(&mut stmt.node, &mut scope_env)
                .map_err(|e| e.with_position(span.line, span.column))?;
        }
        Ok(())
    }
}

fn is_vec_type(ty: &Type) -> bool {
    match ty {
        Type::Applied(name, _) => name == "vec",
        Type::Named(name) => name == "vec",
        _ => false,
    }
}
