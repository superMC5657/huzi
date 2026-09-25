//! 单态化分派:类型遍历(`monomorphize_type`)与调用点泛型推导加实例化触发
//! (`try_monomorphize_call`)。(自 `generic/mod.rs` 纯搬移,零逻辑变化。)

use super::Monomorphizer;
use super::mangle_name;
use super::substitute_type;
use crate::codegen::qname::bare_name;
use huzi_ast::*;
use huzi_error::{Result, did_you_mean};
use std::collections::HashMap;

impl Monomorphizer {
    pub(super) fn monomorphize_type(&mut self, ty: &mut Type) -> Result<()> {
        match ty {
            Type::Applied(name, args) => {
                for a in args.iter_mut() {
                    self.monomorphize_type(a)?;
                }
                // 内置容器不做单态化:`vec<T>` 与 `Map<K,V>` 直接放行。
                if name == "vec" || name == "map" || name == "Map" || name == "HashMap" {
                    return Ok(());
                }
                let mangled = if self.struct_templates.contains_key(name) {
                    self.monomorphize_struct_type(name, args)?
                } else if self.enum_templates.contains_key(name) {
                    self.monomorphize_enum_type(name, args)?
                } else {
                    let candidates = self
                        .struct_templates
                        .keys()
                        .chain(self.enum_templates.keys())
                        .map(|s| s.as_str());
                    let hint = did_you_mean(name, candidates);
                    let mut message = format!(
                        "未知泛型类型 '{}':期望已定义的泛型结构体或枚举,实际未找到",
                        name
                    );
                    if let Some(h) = hint {
                        message.push_str(&format!("\n  help: {}", h));
                    }
                    return Err(self.diag(message));
                };
                *ty = Type::Named(mangled);
            }
            Type::Box(inner) | Type::Weak(inner) => self.monomorphize_type(inner)?,
            Type::Array(elem, _) => self.monomorphize_type(elem)?,
            Type::Tuple(elems) => {
                for elem in elems {
                    self.monomorphize_type(elem)?;
                }
            }
            Type::Fn(params, ret) => {
                for p in params {
                    self.monomorphize_type(p)?;
                }
                self.monomorphize_type(ret)?;
            }
            _ => {}
        }
        Ok(())
    }

    /// 泛型函数模板查找:先按调用名原样查找,再按末段回退——限定调用
    /// (`result::is_ok`)的模板按定义名 `is_ok` 收录。
    fn fn_template_for(&self, callee_name: &str) -> Option<&(FnStmt, Span)> {
        if let Some(t) = self.fn_templates.get(callee_name) {
            return Some(t);
        }
        let bare = bare_name(callee_name);
        self.fn_templates.get(bare)
    }

    /// 对调用点做泛型推导与单态化(实参须已完成表达式级单态化)。
    /// 返回 Some(单态化名) 时调用方把 callee 重写为该裸名并清空
    /// type_args;返回 None 表示非泛型调用,保持原样由 codegen 分派。
    pub(super) fn try_monomorphize_call(
        &mut self,
        callee_name: &str,
        type_args: &mut Vec<Type>,
        arguments: &mut [Expr],
    ) -> Result<Option<String>> {
        // 实参类型推导：未提供显式类型实参时尝试推导
        if type_args.is_empty() {
            if let Some((template, _)) = self.fn_template_for(callee_name) {
                let inferred = self
                    .inferrer
                    .infer_call_type_args(callee_name, template, arguments)
                    .map_err(|e| match self.current_span {
                        Some(span) => e.with_position(span.line, span.column),
                        None => e,
                    })?;
                *type_args = inferred;
            }
        }
        if type_args.is_empty() {
            return Ok(None);
        }
        let (template, span) = self
            .fn_template_for(callee_name)
            .cloned()
            .ok_or_else(|| {
                let hint = did_you_mean(callee_name, self.fn_templates.keys().map(|s| s.as_str()));
                let mut message = format!(
                    "未知泛型函数 '{}':期望已定义的泛型函数,实际未找到",
                    callee_name
                );
                if let Some(h) = hint {
                    message.push_str(&format!("\n  help: {}", h));
                }
                self.diag(message)
            })?;
        if type_args.len() != template.type_params.len() {
            return Err(self.diag(format!(
                "泛型函数 '{}' 类型实参数量不匹配:期望 {} 个,实际 {} 个;请写成 `{}<{}>(...)`",
                callee_name,
                template.type_params.len(),
                type_args.len(),
                callee_name,
                template.type_params.join(", ")
            )));
        }
        for a in type_args.iter() {
            self.validate_type_arg(a)?;
        }
        let mangled = if let Some((prefix, base)) = callee_name.split_once("::") {
            format!("{}::{}", prefix, mangle_name(base, type_args))
        } else {
            mangle_name(callee_name, type_args)
        };
        if !self.instantiated_fns.contains_key(&mangled) {
            self.instantiate_fn(&template, span, type_args, &mangled)?;
        }
        Self::propagate_closure_types(&template, type_args, arguments);
        Ok(Some(mangled))
    }

    fn propagate_closure_types(
        template: &FnStmt,
        type_args: &[Type],
        arguments: &mut [Expr],
    ) {
        let mapping: HashMap<String, Type> = template
            .type_params
            .iter()
            .cloned()
            .zip(type_args.iter().cloned())
            .collect();
        for (param, arg) in template.params.iter().zip(arguments.iter_mut()) {
            if let Expr::Closure(closure) = arg {
                let inst_ty = substitute_type(&param.param_type, &mapping);
                if let Type::Fn(param_tys, ret_ty) = inst_ty {
                    for (cp, pt) in closure.params.iter_mut().zip(param_tys.into_iter()) {
                        if cp.param_type.is_none() {
                            cp.param_type = Some(pt);
                        }
                    }
                    if closure.return_type.is_none() {
                        closure.return_type = Some(*ret_ty);
                    }
                }
            }
        }
    }
}
