//! 闭包自由变量的表达式递归收集(体/块遍历保留在父模块)。

use super::CodeGen;
use huzi_ast::*;
use inkwell::types::BasicTypeEnum;
use std::collections::HashSet;

impl<'ctx> CodeGen<'ctx> {
    pub(super) fn collect_free_vars_expr(
        &self,
        expr: &Expr,
        known: &mut HashSet<String>,
        free_vars: &mut Vec<(String, Type, BasicTypeEnum<'ctx>)>,
    ) {
        match expr {
            Expr::Ident(name) => {
                if !known.contains(name) && !free_vars.iter().any(|(n, _, _)| n == name) {
                    if let Some(slot) = self.scope_lookup(name) {
                        let ast_ty = self
                            .local_ast
                            .get(name)
                            .cloned()
                            .unwrap_or(Type::Named("i32".to_string()));
                        free_vars.push((name.clone(), ast_ty, slot.ty));
                    }
                }
            }
            Expr::Binary(b) => {
                self.collect_free_vars_expr(&b.left, known, free_vars);
                self.collect_free_vars_expr(&b.right, known, free_vars);
            }
            Expr::Unary(u) => self.collect_free_vars_expr(&u.operand, known, free_vars),
            Expr::Call(c) => {
                self.collect_free_vars_expr(&c.callee, known, free_vars);
                for arg in &c.arguments {
                    self.collect_free_vars_expr(arg, known, free_vars);
                }
            }
            Expr::MethodCall(m) => {
                self.collect_free_vars_expr(&m.receiver, known, free_vars);
                for arg in &m.arguments {
                    self.collect_free_vars_expr(arg, known, free_vars);
                }
            }
            Expr::ArrayIndex(a) => {
                self.collect_free_vars_expr(&a.array, known, free_vars);
                self.collect_free_vars_expr(&a.index, known, free_vars);
            }
            Expr::Assign(a) => {
                self.collect_free_vars_expr(&a.target, known, free_vars);
                self.collect_free_vars_expr(&a.value, known, free_vars);
            }
            Expr::If(i) => {
                self.collect_free_vars_expr(&i.condition, known, free_vars);
                self.collect_free_vars_block(&i.then_branch, known, free_vars);
                self.collect_free_vars_block(&i.else_branch, known, free_vars);
            }
            Expr::Closure(c) => {
                let mut inner_known = known.clone();
                for p in &c.params {
                    inner_known.insert(p.name.clone());
                }
                self.collect_free_vars_body(&c.body, &mut inner_known, free_vars);
            }
            Expr::ArrayLiteral(elems) | Expr::TupleLiteral(elems) => {
                for e in elems {
                    self.collect_free_vars_expr(e, known, free_vars);
                }
            }
            _ => {}
        }
    }
}
