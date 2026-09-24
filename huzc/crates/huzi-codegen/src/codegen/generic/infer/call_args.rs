use huzi_ast::*;
use huzi_error::{HuziError, Result};
use std::collections::HashMap;

impl super::TypeInferrer {
    /// 在泛型调用点根据形参列表和实参列表推导具体类型实参。
    pub(in super::super) fn infer_call_type_args(
        &self,
        callee_name: &str,
        template: &FnStmt,
        args: &[Expr],
    ) -> Result<Vec<Type>> {
        if args.len() != template.params.len() {
            return Err(HuziError::new_global(format!(
                "泛型函数 '{}' 实参数量不匹配:期望 {} 个,实际 {} 个",
                callee_name,
                template.params.len(),
                args.len()
            )));
        }

        let mut inferred: HashMap<String, Type> = HashMap::new();
        for (param, arg) in template.params.iter().zip(args.iter()) {
            if !matches!(arg, Expr::Closure(_)) {
                if let Some(arg_ty) = self.infer_expr_type(arg) {
                    let _ = self.unify_type(
                        &param.param_type,
                        &arg_ty,
                        &template.type_params,
                        &mut inferred,
                    );
                }
            }
        }
        for (idx, (param, arg)) in template.params.iter().zip(args.iter()).enumerate() {
            let expected_ty = crate::codegen::generic::subst::substitute_type(&param.param_type, &inferred);
            let arg_ty = match arg {
                Expr::Closure(c) => self.infer_closure_type(c, Some(&expected_ty)),
                _ => self.infer_expr_type(arg),
            }.ok_or_else(|| {
                HuziError::new_global(format!(
                    "泛型函数 '{}' 第 {} 个实参类型无法推导:形参为 '{}: {}',请显式指定类型实参,如 `{}<{}>(...)`",
                    callee_name,
                    idx + 1,
                    param.name,
                    param.param_type,
                    callee_name,
                    template.type_params.join(", ")
                ))
            })?;
            self.unify_type(
                &param.param_type,
                &arg_ty,
                &template.type_params,
                &mut inferred,
            )?;
        }

        let mut result = Vec::new();
        for tp in &template.type_params {
            if let Some(ty) = inferred.get(tp) {
                result.push(ty.clone());
            } else {
                return Err(HuziError::new_global(format!(
                    "泛型函数 '{}' 的类型形参 '{}' 无法推导:期望由实参确定,实际没有对应推导来源;请显式指定,如 `{}<{}>(...)`",
                    callee_name, tp, callee_name,
                    template.type_params.join(", ")
                )));
            }
        }
        Ok(result)
    }
}
