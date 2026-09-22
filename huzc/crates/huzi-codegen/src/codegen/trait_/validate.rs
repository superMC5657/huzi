use super::super::ModuleCode;
use super::TraitDesugarer;
use huzi_ast::*;
use huzi_error::{HuziError, Result, did_you_mean};
use std::collections::{HashMap, HashSet};

/// 返回类型展示:无标注时说明默认含义,保证期望/实际对比可读。
fn fmt_return(ty: &Option<Type>) -> String {
    ty.as_ref()
        .map(|t| t.to_string())
        .unwrap_or_else(|| "无标注(默认 i32)".to_string())
}

impl TraitDesugarer {
    pub(super) fn collect_types_and_traits(
        &mut self,
        program: &Program,
        modules: &[ModuleCode],
    ) -> Result<()> {
        self.collect_from_stmts(&program.statements)?;
        for m in modules {
            if let Some(prog) = &m.program {
                self.collect_from_stmts(&prog.statements)?;
            }
        }
        Ok(())
    }

    fn collect_from_stmts(&mut self, stmts: &[Spanned<Stmt>]) -> Result<()> {
        for s in stmts {
            match &s.node {
                Stmt::Struct(d) => {
                    self.known_types.insert(d.name.clone());
                    let mut fields = HashMap::new();
                    for f in &d.fields {
                        fields.insert(f.name.clone(), f.field_type.clone());
                    }
                    self.struct_fields.insert(d.name.clone(), fields);
                }
                Stmt::Enum(d) => {
                    self.known_types.insert(d.name.clone());
                }
                Stmt::Trait(t) => {
                    if self.traits.contains_key(&t.name) {
                        return Err(HuziError::new(
                            format!(
                                "重复定义 trait '{}':期望每个 trait 只定义一次,实际已存在同名定义",
                                t.name
                            ),
                            s.span.line,
                            s.span.column,
                        ));
                    }
                    let mut mnames = HashSet::new();
                    for m in &t.methods {
                        if !mnames.insert(&m.name) {
                            return Err(HuziError::new(
                                format!(
                                    "trait '{}' 中重复定义方法 '{}':期望方法名唯一,实际出现多次",
                                    t.name, m.name
                                ),
                                s.span.line,
                                s.span.column,
                            ));
                        }
                    }
                    self.traits.insert(t.name.clone(), t.clone());
                }
                Stmt::Fn(f) => {
                    self.fn_return_types
                        .insert(f.name.clone(), f.return_type.clone());
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub(super) fn validate_and_collect_impls(
        &mut self,
        program: &Program,
        modules: &[ModuleCode],
    ) -> Result<()> {
        for s in &program.statements {
            if let Stmt::Impl(i) = &s.node {
                self.validate_impl(i, s.span)?;
            }
        }
        for m in modules {
            if let Some(prog) = &m.program {
                for s in &prog.statements {
                    if let Stmt::Impl(i) = &s.node {
                        self.validate_impl(i, s.span)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn validate_impl(&mut self, i: &ImplBlock, span: Span) -> Result<()> {
        let at = |message: String| HuziError::new(message, span.line, span.column);
        let trait_def = self
            .traits
            .get(&i.trait_name)
            .cloned()
            .ok_or_else(|| {
                let hint = did_you_mean(
                    &i.trait_name,
                    self.traits.keys().map(|k| k.as_str()),
                );
                let mut message = format!(
                    "未知 trait '{}':期望已定义的 trait,实际未找到",
                    i.trait_name
                );
                if let Some(h) = hint {
                    message.push_str(&format!(";帮助:{}", h));
                }
                at(message)
            })?;

        if !self.known_types.contains(&i.target_type) {
            let hint = did_you_mean(
                &i.target_type,
                self.known_types.iter().map(|k| k.as_str()),
            );
            let mut message = format!(
                "impl '{}' for '{}' 的目标类型 '{}' 未知:期望已定义的结构体/枚举,实际未找到",
                i.trait_name, i.target_type, i.target_type
            );
            if let Some(h) = hint {
                message.push_str(&format!(";帮助:{}", h));
            }
            return Err(at(message));
        }

        // 检查缺漏方法
        for tm in &trait_def.methods {
            if !i.methods.iter().any(|m| m.name == tm.name) {
                let all: Vec<&str> = trait_def
                    .methods
                    .iter()
                    .map(|m| m.name.as_str())
                    .collect();
                return Err(at(format!(
                    "类型 '{}' 实现 trait '{}' 缺少方法 '{}':期望实现全部 {} 个方法 [{}],实际缺失;请补上 `fn {}(...)` 实现",
                    i.target_type,
                    trait_def.name,
                    tm.name,
                    all.len(),
                    all.join(", "),
                    tm.name
                )));
            }
        }

        // 检查多余方法、签名与冲突
        for m in &i.methods {
            let tm = trait_def
                .methods
                .iter()
                .find(|tm| tm.name == m.name)
                .ok_or_else(|| {
                    let members: Vec<&str> = trait_def
                        .methods
                        .iter()
                        .map(|tm| tm.name.as_str())
                        .collect();
                    let hint =
                        did_you_mean(&m.name, members.iter().copied());
                    let mut message = format!(
                        "方法 '{}' 不是 trait '{}' 的成员:期望为 [{}] 之一,实际多出",
                        m.name,
                        trait_def.name,
                        members.join(", ")
                    );
                    if let Some(h) = hint {
                        message.push_str(&format!(";帮助:{}", h));
                    }
                    at(message)
                })?;

            self.validate_method_signature(m, tm, &trait_def.name, &i.target_type, span)?;
            self.record_impl_method(&i.target_type, &i.trait_name, m, span)?;
        }

        Ok(())
    }

    fn validate_method_signature(
        &self,
        m: &FnStmt,
        tm: &TraitMethodDef,
        trait_name: &str,
        target_type: &str,
        span: Span,
    ) -> Result<()> {
        let at = |message: String| HuziError::new(message, span.line, span.column);
        let expected_params = tm.params.len() + if tm.has_self { 1 } else { 0 };
        if m.params.len() != expected_params {
            return Err(at(format!(
                "方法 '{}' 参数数量不匹配:trait '{}' 期望 {} 个(含 self),实际 {} 个",
                m.name, trait_name, expected_params, m.params.len()
            )));
        }

        if tm.has_self {
            if m.params[0].name != "self" {
                return Err(at(format!(
                    "方法 '{}' 首个参数必须为 'self':期望 'self',实际为 '{}'",
                    m.name, m.params[0].name
                )));
            }
            let expected_self_type = Type::Named(target_type.to_string());
            if m.params[0].param_type != expected_self_type {
                return Err(at(format!(
                    "方法 '{}' 的 'self' 类型与实现目标不一致:期望 '{}',实际 '{}'",
                    m.name, expected_self_type, m.params[0].param_type
                )));
            }
            for (idx, tp) in tm.params.iter().enumerate() {
                let mp = &m.params[idx + 1];
                if mp.param_type != tp.param_type {
                    return Err(at(format!(
                        "方法 '{}' 参数 '{}' 类型与 trait '{}' 不一致:期望 '{}',实际 '{}'",
                        m.name, mp.name, trait_name, tp.param_type, mp.param_type
                    )));
                }
            }
        } else {
            for (idx, tp) in tm.params.iter().enumerate() {
                let mp = &m.params[idx];
                if mp.param_type != tp.param_type {
                    return Err(at(format!(
                        "方法 '{}' 参数 '{}' 类型与 trait '{}' 不一致:期望 '{}',实际 '{}'",
                        m.name, mp.name, trait_name, tp.param_type, mp.param_type
                    )));
                }
            }
        }

        if m.return_type != tm.return_type {
            return Err(at(format!(
                "方法 '{}' 返回类型与 trait '{}' 不一致:期望 '{}',实际 '{}'",
                m.name,
                trait_name,
                fmt_return(&tm.return_type),
                fmt_return(&m.return_type)
            )));
        }
        Ok(())
    }

    fn record_impl_method(
        &mut self,
        target_type: &str,
        trait_name: &str,
        m: &FnStmt,
        span: Span,
    ) -> Result<()> {
        let type_methods = self
            .implemented_methods
            .entry(target_type.to_string())
            .or_default();
        if let Some(existing_trait) = type_methods.get(&m.name) {
            return Err(HuziError::new(
                format!(
                    "类型 '{}' 的方法 '{}' 冲突:已由 trait '{}' 实现,当前 trait '{}' 再次实现;期望每个方法只由一个 trait 提供,实际出现多次;帮助:改名其中一个方法,或通过 impl 归属区分调用",
                    target_type, m.name, existing_trait, trait_name
                ),
                span.line,
                span.column,
            ));
        }
        type_methods.insert(m.name.clone(), trait_name.to_string());

        self.method_return_types
            .entry(target_type.to_string())
            .or_default()
            .insert(m.name.clone(), m.return_type.clone());

        let mangled = format!("{}__{}", target_type, m.name);
        self.fn_return_types.insert(mangled, m.return_type.clone());
        Ok(())
    }
}
