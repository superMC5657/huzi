//! 方法解析与 UFCS 降解:固有/Trait 方法、vec 前缀、全局函数、报错。

use super::is_vec_type;
use super::super::TraitDesugarer;
use huzi_ast::*;
use huzi_error::{HuziError, Result, did_you_mean};
use std::collections::HashMap;

impl TraitDesugarer {
    pub(super) fn desugar_method_call(
        &self,
        mc: &MethodCallExpr,
        env: &HashMap<String, Type>,
    ) -> Result<Expr> {
        let receiver_ty = self.infer_expr_type(&mc.receiver, env);

        // 优先 1: 具名类型的固有方法或 Trait 方法
        let base_type_name = match &receiver_ty {
            Some(Type::Named(type_name)) => Some(type_name.as_str()),
            Some(Type::Applied(type_name, _)) => Some(type_name.as_str()),
            _ => None,
        };
        if let Some(type_name) = base_type_name {
            if let Some(methods) = self.implemented_methods.get(type_name) {
                if methods.contains_key(&mc.method) {
                    let mangled = format!("{}__{}", type_name, mc.method);
                    let mut args = vec![*mc.receiver.clone()];
                    args.extend(mc.arguments.iter().cloned());
                    return Ok(Expr::Call(CallExpr {
                        callee: Box::new(Expr::Ident(mangled)),
                        arguments: args,
                        type_args: Vec::new(),
                    }));
                }
            }
        }

        // 优先 2: 向量专属前缀降解 (vec_{method})
        if receiver_ty.as_ref().is_some_and(is_vec_type) {
            let vec_fn = format!("vec_{}", mc.method);
            if self.fn_return_types.contains_key(&vec_fn) {
                let mut args = vec![*mc.receiver.clone()];
                args.extend(mc.arguments.iter().cloned());
                return Ok(Expr::Call(CallExpr {
                    callee: Box::new(Expr::Ident(vec_fn)),
                    arguments: args,
                    type_args: Vec::new(),
                }));
            }
        }

        // 优先 3: 普通全局函数 / 内置函数 UFCS 降解
        if self.fn_return_types.contains_key(&mc.method) {
            let mut args = vec![*mc.receiver.clone()];
            args.extend(mc.arguments.iter().cloned());
            return Ok(Expr::Call(CallExpr {
                callee: Box::new(Expr::Ident(mc.method.clone())),
                arguments: args,
                type_args: Vec::new(),
            }));
        }

        // 优先 4: 未匹配报错与 did_you_mean 建议
        self.method_resolution_error(mc, &receiver_ty)
    }

    fn method_resolution_error(
        &self,
        mc: &MethodCallExpr,
        receiver_ty: &Option<Type>,
    ) -> Result<Expr> {
        let base_type_name = match receiver_ty {
            Some(Type::Named(type_name)) => Some(type_name.as_str()),
            Some(Type::Applied(type_name, _)) => Some(type_name.as_str()),
            _ => None,
        };
        if let Some(type_name) = base_type_name {
            let available: Vec<&str> = self
                .implemented_methods
                .get(type_name)
                .map(|m| m.keys().map(|k| k.as_str()).collect())
                .unwrap_or_default();
            let hint = did_you_mean(&mc.method, available.iter().copied());
            let mut msg = if available.is_empty() {
                format!(
                    "类型 '{}' 没有方法 '{}':期望已实现的方法,实际该类型尚未实现任何方法,亦未找到同名全局函数",
                    type_name, mc.method
                )
            } else {
                format!(
                    "类型 '{}' 没有方法 '{}':期望为 [{}] 之一,实际未找到",
                    type_name,
                    mc.method,
                    available.join(", ")
                )
            };
            if let Some(h) = hint {
                msg.push_str(&format!("\n  help: {}", h));
            }
            return Err(HuziError::new_global(msg));
        }
        match receiver_ty {
            Some(other_ty) => Err(HuziError::new_global(format!(
                "类型 '{}' 没有方法 '{}',且未找到匹配的 UFCS 同名函数\n  help: 检查方法拼写或定义接受该类型为首参数的同名函数",
                other_ty, mc.method
            ))),
            None => Err(HuziError::new_global(format!(
                "无法确定方法 '{}' 的目标:接收者类型未知且未找到名为 '{}' 的全局函数\n  help: 请为接收者提供类型标注或检查函数名拼写",
                mc.method, mc.method
            ))),
        }
    }
}
