use super::bind::find_variant;
use super::super::{CodeGen, EnumInfo};
use huzi_ast::*;
use huzi_error::{HuziError, Result};
use inkwell::types::BasicTypeEnum;

impl<'ctx> CodeGen<'ctx> {
    /// 检查 match 表达式的类型匹配度与分支穷尽性。
    pub(super) fn check_match_exhaustiveness(
        &self,
        arms: &[MatchArm],
        scrut_ty: BasicTypeEnum<'ctx>,
        info: Option<&EnumInfo<'ctx>>,
    ) -> Result<()> {
        self.check_arm_pattern_types(arms, scrut_ty, info)?;

        if let Some(enum_info) = info {
            Self::check_enum_exhaustiveness(arms, enum_info)?;
        } else if scrut_ty == self.context.bool_type().into() {
            Self::check_bool_exhaustiveness(arms)?;
        } else {
            Self::check_scalar_exhaustiveness(arms)?;
        }

        Ok(())
    }

    /// 校验各个分支模式与待匹配项的类型是否兼容。
    fn check_arm_pattern_types(
        &self,
        arms: &[MatchArm],
        scrut_ty: BasicTypeEnum<'ctx>,
        info: Option<&EnumInfo<'ctx>>,
    ) -> Result<()> {
        for arm in arms {
            match &arm.pattern {
                Pattern::Wildcard | Pattern::Variable(_) => {}
                Pattern::Variant { enum_name, variant, .. } => {
                    let enum_info = info.ok_or_else(|| {
                        HuziError::new_global("Cannot match enum variant without an enum scrutinee")
                    })?;
                    if enum_name != &enum_info.name {
                        return Err(HuziError::new_global(format!(
                            "Match arms use '{}' but the scrutinee is '{}'",
                            enum_name, enum_info.name
                        )));
                    }
                    find_variant(enum_info, variant)?;
                }
                Pattern::Literal(lit) => {
                    if info.is_some() {
                        return Err(HuziError::new_global(
                            "Cannot match literal pattern against an enum scrutinee",
                        ));
                    }
                    self.check_literal_type_compat(lit, scrut_ty)?;
                }
            }
        }
        Ok(())
    }

    /// 校验字面量模式与标量待匹配项的类型兼容性。
    fn check_literal_type_compat(&self, lit: &Literal, scrut_ty: BasicTypeEnum<'ctx>) -> Result<()> {
        let is_compat = match lit {
            Literal::Int(_) => scrut_ty == self.context.i32_type().into() || scrut_ty == self.context.i64_type().into(),
            Literal::Float(_) => scrut_ty == self.context.f64_type().into() || scrut_ty == self.context.f32_type().into(),
            Literal::Bool(_) => scrut_ty == self.context.bool_type().into(),
            Literal::Char(_) => scrut_ty == self.context.i8_type().into(),
            Literal::String(_) => scrut_ty == self.context.ptr_type(inkwell::AddressSpace::default()).into(),
        };
        if !is_compat {
            return Err(HuziError::new_global(format!(
                "Literal pattern '{:?}' is not compatible with scrutinee type",
                lit
            )));
        }
        Ok(())
    }

    /// 校验枚举模式穷尽性：必须拥有无守卫的通配符/变量，或无守卫地覆盖所有变体。
    fn check_enum_exhaustiveness(arms: &[MatchArm], info: &EnumInfo<'ctx>) -> Result<()> {
        let has_universal = arms.iter().any(|arm| {
            arm.guard.is_none() && matches!(arm.pattern, Pattern::Wildcard | Pattern::Variable(_))
        });
        if has_universal {
            return Ok(());
        }

        let missing: Vec<&str> = info
            .variants
            .iter()
            .filter(|v| {
                !arms.iter().any(|arm| {
                    if arm.guard.is_some() {
                        return false;
                    }
                    match &arm.pattern {
                        Pattern::Variant { variant, .. } => variant == &v.name,
                        _ => false,
                    }
                })
            })
            .map(|v| v.name.as_str())
            .collect();

        if missing.is_empty() {
            return Ok(());
        }

        Err(HuziError::new_global(format!(
            "Non-exhaustive match for enum '{}': missing variants: {}\n  help: add arms for the missing variants or an unguarded wildcard arm `_`",
            info.name,
            missing.join(", ")
        )))
    }

    /// 校验布尔模式穷尽性：必须拥有无守卫通配符/变量，或同时拥有无守卫的 true 与 false。
    fn check_bool_exhaustiveness(arms: &[MatchArm]) -> Result<()> {
        let has_universal = arms.iter().any(|arm| {
            arm.guard.is_none() && matches!(arm.pattern, Pattern::Wildcard | Pattern::Variable(_))
        });
        if has_universal {
            return Ok(());
        }

        let has_true = arms.iter().any(|arm| {
            arm.guard.is_none() && matches!(&arm.pattern, Pattern::Literal(Literal::Bool(true)))
        });
        let has_false = arms.iter().any(|arm| {
            arm.guard.is_none() && matches!(&arm.pattern, Pattern::Literal(Literal::Bool(false)))
        });

        if has_true && has_false {
            return Ok(());
        }

        Err(HuziError::new_global(
            "Non-exhaustive match for bool\n  help: add arms for both `true` and `false` or an unguarded wildcard arm `_`",
        ))
    }

    /// 校验标量模式穷尽性：必须拥有无守卫的通配符或变量绑定分支。
    fn check_scalar_exhaustiveness(arms: &[MatchArm]) -> Result<()> {
        let has_universal = arms.iter().any(|arm| {
            arm.guard.is_none() && matches!(arm.pattern, Pattern::Wildcard | Pattern::Variable(_))
        });
        if has_universal {
            return Ok(());
        }

        Err(HuziError::new_global(
            "Non-exhaustive match: match on scalar value must have an unguarded wildcard arm `_` or variable pattern",
        ))
    }
}
