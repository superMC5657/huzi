//! 单态化实例化:泛型结构体/枚举/函数模板按类型实参特化并登记产物。
//! (自 `generic/mod.rs` 纯搬移,零逻辑变化。)

use super::Monomorphizer;
use super::mangle_name;
use super::{substitute_block, substitute_type};
use huzi_ast::*;
use huzi_error::Result;
use std::collections::HashMap;

impl Monomorphizer {
    pub(super) fn monomorphize_struct_type(
        &mut self,
        name: &str,
        args: &[Type],
    ) -> Result<String> {
        let template = self.struct_templates.get(name).cloned().ok_or_else(|| {
            self.diag(format!("未知泛型结构体 '{}'", name))
        })?;
        if args.len() != template.type_params.len() {
            return Err(self.diag(format!(
                "泛型结构体 '{}' 类型实参数量不匹配:期望 {} 个,实际 {} 个",
                name,
                template.type_params.len(),
                args.len()
            )));
        }
        for a in args {
            self.validate_type_arg(a)?;
        }
        let mangled = mangle_name(name, args);
        if !self.instantiated_structs.contains_key(&mangled) {
            let mapping: HashMap<String, Type> = template
                .type_params
                .iter()
                .cloned()
                .zip(args.iter().cloned())
                .collect();
            let mut spec = template.clone();
            spec.name = mangled.clone();
            spec.type_params.clear();
            for field in &mut spec.fields {
                field.field_type = substitute_type(&field.field_type, &mapping);
            }
            self.instantiated_structs
                .insert(mangled.clone(), spec.clone());
            for field in &mut spec.fields {
                self.monomorphize_type(&mut field.field_type)?;
            }
            self.instantiated_structs
                .insert(mangled.clone(), spec.clone());
            self.inferrer.struct_defs.insert(mangled.clone(), spec);
            self.inferrer.instantiated_struct_types.insert(
                mangled.clone(),
                (name.to_string(), args.to_vec()),
            );
        }
        Ok(mangled)
    }

    pub(super) fn monomorphize_enum_type(
        &mut self,
        name: &str,
        args: &[Type],
    ) -> Result<String> {
        let template = self.enum_templates.get(name).cloned().ok_or_else(|| {
            self.diag(format!("未知泛型枚举 '{}'", name))
        })?;
        if args.len() != template.type_params.len() {
            return Err(self.diag(format!(
                "泛型枚举 '{}' 类型实参数量不匹配:期望 {} 个,实际 {} 个",
                name,
                template.type_params.len(),
                args.len()
            )));
        }
        for a in args {
            self.validate_type_arg(a)?;
        }
        let mangled = mangle_name(name, args);
        if !self.instantiated_enums.contains_key(&mangled) {
            let mapping: HashMap<String, Type> = template
                .type_params
                .iter()
                .cloned()
                .zip(args.iter().cloned())
                .collect();
            let mut spec = template.clone();
            spec.name = mangled.clone();
            spec.type_params.clear();
            for variant in &mut spec.variants {
                for payload in &mut variant.payloads {
                    *payload = substitute_type(payload, &mapping);
                }
            }
            self.instantiated_enums
                .insert(mangled.clone(), spec.clone());
            for variant in &mut spec.variants {
                for payload in &mut variant.payloads {
                    self.monomorphize_type(payload)?;
                }
            }
            self.instantiated_enums
                .insert(mangled.clone(), spec.clone());
            self.inferrer.known_enums.insert(mangled.clone());
            self.inferrer.instantiated_enum_types.insert(
                mangled.clone(),
                (name.to_string(), args.to_vec()),
            );
        }
        Ok(mangled)
    }

    /// 实例化泛型函数模板:按实参映射替换签名与函数体,登记单态化
    /// 产物并补齐签名,最后对函数体做嵌套单态化遍历(模板内的 `?`、
    /// 限定调用等在此展开)。
    pub(super) fn instantiate_fn(
        &mut self,
        template: &FnStmt,
        span: Span,
        type_args: &[Type],
        mangled: &str,
    ) -> Result<()> {
        let mapping: HashMap<String, Type> = template
            .type_params
            .iter()
            .cloned()
            .zip(type_args.iter().cloned())
            .collect();
        let mut spec = template.clone();
        spec.name = mangled.to_string();
        spec.type_params.clear();
        for p in &mut spec.params {
            p.param_type = substitute_type(&p.param_type, &mapping);
            self.monomorphize_type(&mut p.param_type)?;
        }
        if let Some(ret) = &mut spec.return_type {
            *ret = substitute_type(ret, &mapping);
            self.monomorphize_type(ret)?;
        }
        substitute_block(&mut spec.body, &mapping);
        self.instantiated_fns
            .insert(mangled.to_string(), (spec.clone(), span));
        self.inferrer.fn_signatures.insert(
            mangled.to_string(),
            (
                spec.params.iter().map(|p| p.param_type.clone()).collect(),
                spec.return_type.clone(),
            ),
        );
        let prev_ret = self.current_fn_return.clone();
        self.current_fn_return = spec.return_type.clone();
        self.inferrer.enter_scope();
        for p in &spec.params {
            self.inferrer.insert_var(&p.name, p.param_type.clone());
        }
        self.monomorphize_block(&mut spec.body)?;
        self.inferrer.leave_scope();
        self.current_fn_return = prev_ret;
        self.instantiated_fns.insert(mangled.to_string(), (spec, span));
        Ok(())
    }
}
