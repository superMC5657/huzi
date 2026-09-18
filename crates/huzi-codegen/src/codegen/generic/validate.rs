//! 泛型模板与类型实参合法性校验。

use super::Monomorphizer;
use huzi_ast::*;
use huzi_error::{HuziError, Result, did_you_mean};

impl Monomorphizer {
    pub(super) fn collect_templates(&mut self, program: &Program) -> Result<()> {
        for s in &program.statements {
            match &s.node {
                Stmt::Struct(d) => {
                    self.known_types.insert(d.name.clone());
                    self.inferrer
                        .struct_defs
                        .insert(d.name.clone(), d.clone());
                    if !d.type_params.is_empty() {
                        self.validate_struct_template(d)?;
                        self.struct_templates.insert(d.name.clone(), d.clone());
                    }
                }
                Stmt::Enum(d) => {
                    self.known_types.insert(d.name.clone());
                    self.inferrer.known_enums.insert(d.name.clone());
                }
                Stmt::Fn(f) => {
                    if !f.type_params.is_empty() {
                        self.validate_fn_template(f)?;
                        self.fn_templates
                            .insert(f.name.clone(), (f.clone(), s.span));
                    } else {
                        self.inferrer.fn_signatures.insert(
                            f.name.clone(),
                            (
                                f.params.iter().map(|p| p.param_type.clone()).collect(),
                                f.return_type.clone(),
                            ),
                        );
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub(super) fn validate_struct_template(&self, d: &StructDef) -> Result<()> {
        for field in &d.fields {
            self.validate_type_params(&field.field_type, &d.type_params, &d.name)?;
        }
        Ok(())
    }

    pub(super) fn validate_fn_template(&self, f: &FnStmt) -> Result<()> {
        for p in &f.params {
            self.validate_type_params(&p.param_type, &f.type_params, &f.name)?;
        }
        if let Some(ret) = &f.return_type {
            self.validate_type_params(ret, &f.type_params, &f.name)?;
        }
        Ok(())
    }

    pub(super) fn validate_type_params(
        &self,
        ty: &Type,
        in_scope: &[String],
        def_name: &str,
    ) -> Result<()> {
        match ty {
            Type::Generic(n) | Type::Named(n) => {
                if !in_scope.contains(n) && !self.known_types.contains(n) {
                    let candidates = in_scope
                        .iter()
                        .map(|s| s.as_str())
                        .chain(self.known_types.iter().map(|s| s.as_str()));
                    let hint = did_you_mean(n, candidates);
                    let msg = match hint {
                        Some(h) => format!(
                            "Undefined type variable '{}' in '{}', did you mean '{}'?",
                            n, def_name, h
                        ),
                        None => format!("Undefined type variable '{}' in '{}'", n, def_name),
                    };
                    return Err(HuziError::new_global(msg));
                }
            }
            Type::Box(inner) => self.validate_type_params(inner, in_scope, def_name)?,
            Type::Applied(_, args) => {
                for arg in args {
                    self.validate_type_params(arg, in_scope, def_name)?;
                }
            }
            Type::Array(elem, _) => self.validate_type_params(elem, in_scope, def_name)?,
            Type::Tuple(elems) => {
                for elem in elems {
                    self.validate_type_params(elem, in_scope, def_name)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub(super) fn validate_type_arg(&self, ty: &Type) -> Result<()> {
        match ty {
            Type::Named(n) => {
                if !self.known_types.contains(n) && !self.instantiated_structs.contains_key(n) {
                    let hint = did_you_mean(n, self.known_types.iter().map(|s| s.as_str()));
                    let msg = match hint {
                        Some(h) => format!("Unknown type '{}', did you mean '{}'?", n, h),
                        None => format!("Unknown type '{}'", n),
                    };
                    return Err(HuziError::new_global(msg));
                }
            }
            Type::Box(inner) => self.validate_type_arg(inner)?,
            Type::Applied(_, args) => {
                for a in args {
                    self.validate_type_arg(a)?;
                }
            }
            Type::Array(elem, _) => self.validate_type_arg(elem)?,
            Type::Tuple(elems) => {
                for elem in elems {
                    self.validate_type_arg(elem)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}
