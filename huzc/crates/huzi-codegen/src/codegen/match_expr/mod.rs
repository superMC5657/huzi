mod arm;
mod bind;
mod exhaustiveness;

use super::CodeGen;
use huzi_ast::*;
use huzi_error::{HuziError, Result};
use inkwell::values::BasicValueEnum;

impl<'ctx> CodeGen<'ctx> {
    /// 编译 match 表达式，支持代数枚举解构、标量字面量模式、变量绑定与匹配守卫。
    pub(super) fn compile_match_expr(
        &mut self,
        expr: &MatchExpr,
    ) -> Result<BasicValueEnum<'ctx>> {
        if expr.arms.is_empty() {
            return Err(HuziError::new_global("match must have at least one arm"));
        }

        let (scrut_addr, scrut_ty) = self.compile_addr(&expr.scrutinee)?;
        let pat_enum_name = Self::match_pattern_enum(&expr.arms);

        // 1. 携带数据的代数枚举（判别码 tag 保存在结构体字段 0）
        if let Some(info) = self.enum_data_by_type(scrut_ty) {
            let st = info.llvm.unwrap();
            let info = info.clone();
            self.check_match_exhaustiveness(&expr.arms, scrut_ty, Some(&info))?;
            let tag_ptr = self
                .builder
                .build_struct_gep(st, scrut_addr, 0, "match_tag_ptr")
                .unwrap();
            let tag = self
                .builder
                .build_load(self.context.i32_type(), tag_ptr, "match_tag")
                .unwrap();
            return self.compile_match_arms(
                scrut_addr,
                tag,
                scrut_ty,
                Some((st, scrut_addr)),
                Some(&info),
                &expr.arms,
            );
        }

        // 2. 简单无 payload 枚举（直接以 i32 判别码存储）
        if scrut_ty == self.context.i32_type().into() && pat_enum_name.is_some() {
            let name = pat_enum_name.unwrap();
            let info = self.enums.get(name).cloned().ok_or_else(|| {
                HuziError::new_global(format!("Unknown enum: {}", name))
            })?;
            if info.llvm.is_none() {
                self.check_match_exhaustiveness(&expr.arms, scrut_ty, Some(&info))?;
                let tag = self
                    .builder
                    .build_load(self.context.i32_type(), scrut_addr, "match_tag")
                    .unwrap();
                return self.compile_match_arms(
                    scrut_addr,
                    tag,
                    scrut_ty,
                    None,
                    Some(&info),
                    &expr.arms,
                );
            }
        }

        // 3. 标量匹配（整数/浮点/布尔/字符/字符串等）及任意类型变量绑定
        self.check_match_exhaustiveness(&expr.arms, scrut_ty, None)?;
        let scrut_val = self
            .builder
            .build_load(scrut_ty, scrut_addr, "scrut_val")
            .unwrap();
        self.compile_match_arms(
            scrut_addr,
            scrut_val,
            scrut_ty,
            None,
            None,
            &expr.arms,
        )
    }

    /// 首个变体模式所指定的枚举名称（若存在）。
    fn match_pattern_enum(arms: &[MatchArm]) -> Option<&str> {
        arms.iter().find_map(|arm| match &arm.pattern {
            Pattern::Variant { enum_name, .. } => Some(enum_name.as_str()),
            _ => None,
        })
    }
}
