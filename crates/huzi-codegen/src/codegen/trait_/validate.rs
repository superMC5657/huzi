use super::super::ModuleCode;
use super::TraitDesugarer;
use huzi_ast::*;
use huzi_error::{HuziError, Result};
use std::collections::{HashMap, HashSet};

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
                        return Err(HuziError::new_global(format!(
                            "Duplicate trait definition: {}",
                            t.name
                        )));
                    }
                    let mut mnames = HashSet::new();
                    for m in &t.methods {
                        if !mnames.insert(&m.name) {
                            return Err(HuziError::new_global(format!(
                                "Duplicate method '{}' in trait '{}'",
                                m.name, t.name
                            )));
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
                self.validate_impl(i)?;
            }
        }
        for m in modules {
            if let Some(prog) = &m.program {
                for s in &prog.statements {
                    if let Stmt::Impl(i) = &s.node {
                        self.validate_impl(i)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn validate_impl(&mut self, i: &ImplBlock) -> Result<()> {
        let trait_def = self
            .traits
            .get(&i.trait_name)
            .ok_or_else(|| HuziError::new_global(format!("Unknown trait '{}'", i.trait_name)))?
            .clone();

        if !self.known_types.contains(&i.target_type) {
            return Err(HuziError::new_global(format!(
                "Unknown type '{}' in implementation of trait '{}'",
                i.target_type, i.trait_name
            )));
        }

        // 检查缺漏方法
        for tm in &trait_def.methods {
            if !i.methods.iter().any(|m| m.name == tm.name) {
                return Err(HuziError::new_global(format!(
                    "Missing method '{}' in implementation of trait '{}' for '{}'",
                    tm.name, trait_def.name, i.target_type
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
                    HuziError::new_global(format!(
                        "Method '{}' is not a member of trait '{}'",
                        m.name, trait_def.name
                    ))
                })?;

            self.validate_method_signature(m, tm, &trait_def.name, &i.target_type)?;
            self.record_impl_method(&i.target_type, &i.trait_name, m)?;
        }

        Ok(())
    }

    fn validate_method_signature(
        &self,
        m: &FnStmt,
        tm: &TraitMethodDef,
        trait_name: &str,
        target_type: &str,
    ) -> Result<()> {
        let expected_params = tm.params.len() + if tm.has_self { 1 } else { 0 };
        if m.params.len() != expected_params {
            return Err(HuziError::new_global(format!(
                "Method '{}' expects {} parameter(s), got {}",
                m.name,
                expected_params,
                m.params.len()
            )));
        }

        if tm.has_self {
            if m.params[0].name != "self" {
                return Err(HuziError::new_global(format!(
                    "First parameter of method '{}' must be 'self'",
                    m.name
                )));
            }
            let expected_self_type = Type::Named(target_type.to_string());
            if m.params[0].param_type != expected_self_type {
                return Err(HuziError::new_global(format!(
                    "Parameter 'self' of method '{}' type does not match target type '{}' (expected '{}', got '{}')",
                    m.name, target_type, expected_self_type, m.params[0].param_type
                )));
            }
            for (idx, tp) in tm.params.iter().enumerate() {
                let mp = &m.params[idx + 1];
                if mp.param_type != tp.param_type {
                    return Err(HuziError::new_global(format!(
                        "Parameter '{}' of method '{}' type does not match trait '{}' (expected '{}', got '{}')",
                        mp.name, m.name, trait_name, tp.param_type, mp.param_type
                    )));
                }
            }
        } else {
            for (idx, tp) in tm.params.iter().enumerate() {
                let mp = &m.params[idx];
                if mp.param_type != tp.param_type {
                    return Err(HuziError::new_global(format!(
                        "Parameter '{}' of method '{}' type does not match trait '{}' (expected '{}', got '{}')",
                        mp.name, m.name, trait_name, tp.param_type, mp.param_type
                    )));
                }
            }
        }

        if m.return_type != tm.return_type {
            return Err(HuziError::new_global(format!(
                "Method '{}' return type does not match trait '{}'",
                m.name, trait_name
            )));
        }
        Ok(())
    }

    fn record_impl_method(
        &mut self,
        target_type: &str,
        trait_name: &str,
        m: &FnStmt,
    ) -> Result<()> {
        let type_methods = self
            .implemented_methods
            .entry(target_type.to_string())
            .or_default();
        if let Some(existing_trait) = type_methods.get(&m.name) {
            return Err(HuziError::new_global(format!(
                "Conflict: method '{}' already implemented for type '{}' by trait '{}'",
                m.name, target_type, existing_trait
            )));
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
