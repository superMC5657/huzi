//! 表达式改写分发:方法调用/`format`/f-string/枚举构造/match。

use super::super::TraitDesugarer;
use huzi_ast::*;
use huzi_error::{HuziError, Result, did_you_mean};
use std::collections::HashMap;

impl TraitDesugarer {
    pub(in super::super) fn resolve_expr(&self, expr: &mut Expr, env: &HashMap<String, Type>) -> Result<()> {
        match expr {
            Expr::MethodCall(mc) => {
                self.resolve_expr(&mut mc.receiver, env)?;
                for arg in &mut mc.arguments {
                    self.resolve_expr(arg, env)?;
                }
                *expr = self.desugar_method_call(mc, env)?;
            }
            Expr::Call(c) => {
                if let Expr::Ident(name) = &*c.callee {
                    if name == "format" {
                        *expr = self.desugar_format_call(c, env)?;
                        return Ok(());
                    }
                }
                self.resolve_expr(&mut c.callee, env)?;
                for a in &mut c.arguments {
                    self.resolve_expr(a, env)?;
                }
            }
            Expr::Binary(b) => {
                self.resolve_expr(&mut b.left, env)?;
                self.resolve_expr(&mut b.right, env)?;
            }
            Expr::Unary(u) => self.resolve_expr(&mut u.operand, env)?,
            Expr::Assign(a) => {
                self.resolve_expr(&mut a.target, env)?;
                self.resolve_expr(&mut a.value, env)?;
            }
            Expr::ArrayIndex(a) => {
                self.resolve_expr(&mut a.array, env)?;
                self.resolve_expr(&mut a.index, env)?;
            }
            Expr::ArrayLiteral(elems) | Expr::TupleLiteral(elems) => {
                for e in elems {
                    self.resolve_expr(e, env)?;
                }
            }
            Expr::BoxAlloc(inner) => self.resolve_expr(inner, env)?,
            Expr::If(i) => {
                self.resolve_expr(&mut i.condition, env)?;
                self.resolve_block(&mut i.then_branch, env)?;
                self.resolve_block(&mut i.else_branch, env)?;
            }
            Expr::FieldAccess(f) => self.resolve_expr(&mut f.base, env)?,
            Expr::StructLiteral(s) => {
                for (_, val) in &mut s.fields {
                    self.resolve_expr(val, env)?;
                }
            }
            Expr::EnumConstruct(e) => {
                if let Some(rewritten) = self.resolve_enum_construct(e, env)? {
                    *expr = rewritten;
                }
            }
            Expr::Match(m) => self.resolve_match_expr(m, env)?,
            Expr::FString(fs) => {
                for a in &mut fs.args {
                    self.resolve_expr(a, env)?;
                }
                *expr = self.desugar_fstring(fs, env)?;
                return Ok(());
            }
            _ => {}
        }
        Ok(())
    }

    fn resolve_enum_construct(
        &self,
        e: &mut EnumConstructExpr,
        env: &HashMap<String, Type>,
    ) -> Result<Option<Expr>> {
        for a in &mut e.args {
            self.resolve_expr(a, env)?;
        }
        let mangled = format!("{}__{}", e.enum_name, e.variant);
        if self.fn_return_types.contains_key(&mangled) {
            return Ok(Some(Expr::Call(CallExpr {
                callee: Box::new(Expr::Ident(mangled)),
                arguments: e.args.clone(),
                type_args: e.type_args.clone(),
            })));
        }
        if self.struct_fields.contains_key(&e.enum_name) {
            let available: Vec<&str> = self
                .implemented_methods
                .get(&e.enum_name)
                .map(|m| m.keys().map(|k| k.as_str()).collect())
                .unwrap_or_default();
            let hint = did_you_mean(&e.variant, available.iter().copied());
            let mut msg = format!(
                "类型 '{}' 没有关联静态方法 '{}'",
                e.enum_name, e.variant
            );
            if let Some(h) = hint {
                msg.push_str(&format!("\n  help: {}", h));
            }
            return Err(HuziError::new_global(msg));
        }
        Ok(None)
    }

    fn resolve_match_expr(&self, m: &mut MatchExpr, env: &HashMap<String, Type>) -> Result<()> {
        self.resolve_expr(&mut m.scrutinee, env)?;
        let scrut_ty = self.infer_expr_type(&m.scrutinee, env);
        for arm in &mut m.arms {
            let mut arm_env = env.clone();
            if let Pattern::Variable(name) = &arm.pattern {
                if let Some(t) = &scrut_ty {
                    arm_env.insert(name.clone(), t.clone());
                }
            }
            if let Some(guard) = &mut arm.guard {
                self.resolve_expr(guard, &mut arm_env)?;
            }
            self.resolve_block(&mut arm.body, &mut arm_env)?;
        }
        Ok(())
    }
}
