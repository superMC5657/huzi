use huzi_ast::*;
use huzi_error::{HuziError, Result};
use std::collections::HashMap;

impl super::TypeInferrer {
    /// 将形参类型与实参类型进行统一匹配，收集类型变量的具体绑定。
    pub(in super::super) fn unify_type(
        &self,
        param_ty: &Type,
        arg_ty: &Type,
        type_params: &[String],
        inferred: &mut HashMap<String, Type>,
    ) -> Result<()> {
        match param_ty {
            Type::Generic(name) | Type::Named(name) if type_params.contains(name) => {
                if let Some(existing) = inferred.get(name) {
                    if existing != arg_ty {
                        return Err(HuziError::new_global(format!(
                            "类型形参 '{}' 推导冲突:期望各实参推导结果一致,实际先后为 '{}' 与 '{}'\n  help: 统一对应实参类型,或显式写出类型实参",
                            name, existing, arg_ty
                        )));
                    }
                } else {
                    inferred.insert(name.clone(), arg_ty.clone());
                }
            }
            Type::Box(inner_p) => {
                if let Type::Box(inner_a) = arg_ty {
                    self.unify_type(inner_p, inner_a, type_params, inferred)?;
                }
            }
            Type::Weak(inner_p) => {
                if let Type::Weak(inner_a) = arg_ty {
                    self.unify_type(inner_p, inner_a, type_params, inferred)?;
                }
            }
            Type::Applied(p_name, p_args) => {
                if let Type::Applied(a_name, a_args) = arg_ty {
                    if p_name == a_name && p_args.len() == a_args.len() {
                        for (pa, aa) in p_args.iter().zip(a_args) {
                            self.unify_type(pa, aa, type_params, inferred)?;
                        }
                    }
                } else if let Type::Named(mangled) = arg_ty {
                    if let Some((base_name, actual_args)) = self
                        .instantiated_struct_types
                        .get(mangled)
                        .or_else(|| self.instantiated_enum_types.get(mangled))
                    {
                        if p_name == base_name && p_args.len() == actual_args.len() {
                            for (pa, aa) in p_args.iter().zip(actual_args) {
                                self.unify_type(pa, aa, type_params, inferred)?;
                            }
                        }
                    }
                }
            }
            Type::Array(p_elem, _) => {
                if let Type::Array(a_elem, _) = arg_ty {
                    self.unify_type(p_elem, a_elem, type_params, inferred)?;
                }
            }
            Type::Tuple(p_elems) => {
                if let Type::Tuple(a_elems) = arg_ty {
                    if p_elems.len() == a_elems.len() {
                        for (pe, ae) in p_elems.iter().zip(a_elems) {
                            self.unify_type(pe, ae, type_params, inferred)?;
                        }
                    }
                }
            }
            Type::Fn(p_params, p_ret) => {
                if let Type::Fn(a_params, a_ret) = arg_ty {
                    if p_params.len() == a_params.len() {
                        for (pp, ap) in p_params.iter().zip(a_params) {
                            self.unify_type(pp, ap, type_params, inferred)?;
                        }
                        self.unify_type(p_ret, a_ret, type_params, inferred)?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// 根据泛型枚举变体构造的实参反推枚举的类型实参。
    pub(in super::super) fn infer_enum_variant_type_args(
        &self,
        enum_name: &str,
        variant_name: &str,
        template: &EnumDef,
        args: &[Expr],
    ) -> Result<Vec<Type>> {
        let variant = template
            .variants
            .iter()
            .find(|v| v.name == variant_name)
            .ok_or_else(|| {
                HuziError::new_global(format!(
                    "枚举 '{}' 不存在变体 '{}'",
                    enum_name, variant_name
                ))
            })?;

        if args.len() != variant.payloads.len() {
            return Err(HuziError::new_global(format!(
                "枚举变体 '{}::{}' 载荷参数数量不匹配:期望 {} 个,实际 {} 个",
                enum_name,
                variant_name,
                variant.payloads.len(),
                args.len()
            )));
        }

        let mut inferred: HashMap<String, Type> = HashMap::new();
        for (idx, (payload_ty, arg)) in variant.payloads.iter().zip(args.iter()).enumerate() {
            let arg_ty = self.infer_expr_type(arg).ok_or_else(|| {
                HuziError::new_global(format!(
                    "枚举变体 '{}::{}' 第 {} 个载荷实参类型无法推导:期望为 '{}',请显式指定类型实参,如 `{}<...>::{}(...)`",
                    enum_name, variant_name, idx + 1, payload_ty, enum_name, variant_name
                ))
            })?;
            self.unify_type(payload_ty, &arg_ty, &template.type_params, &mut inferred)?;
        }

        let mut result = Vec::new();
        for tp in &template.type_params {
            if let Some(ty) = inferred.get(tp) {
                result.push(ty.clone());
            } else {
                return Err(HuziError::new_global(format!(
                    "泛型枚举 '{}' 的类型形参 '{}' 无法根据变体 '{}' 的实参推导:请显式指定类型实参,如 `{}<...>::{}(...)`",
                    enum_name, tp, variant_name, enum_name, variant_name
                )));
            }
        }
        Ok(result)
    }
}
