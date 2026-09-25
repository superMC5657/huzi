//! for 语句脱糖:区间/数组源改写并绑定循环变量类型。

use super::super::TraitDesugarer;
use huzi_ast::*;
use huzi_error::Result;
use std::collections::HashMap;

impl TraitDesugarer {
    pub(super) fn resolve_for_stmt(&self, f: &mut ForStmt, env: &mut HashMap<String, Type>) -> Result<()> {
        match &mut f.source {
            ForSource::Range { start, end } => {
                self.resolve_expr(start, env)?;
                self.resolve_expr(end, env)?;
                env.insert(f.var_name.clone(), Type::I32);
            }
            ForSource::Array(arr) => {
                self.resolve_expr(arr, env)?;
                let elem_ty = match self.infer_expr_type(arr, env) {
                    Some(Type::Array(elem, _)) => *elem,
                    Some(Type::Applied(name, args)) if name == "vec" && !args.is_empty() => {
                        args[0].clone()
                    }
                    _ => Type::I32,
                };
                env.insert(f.var_name.clone(), elem_ty);
            }
        }
        self.resolve_block(&mut f.body, env)
    }
}
