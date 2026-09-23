use super::bind::find_variant;
use super::super::{CodeGen, EnumInfo};
use huzi_ast::*;
use huzi_error::{HuziError, Result};
use inkwell::types::{BasicTypeEnum, StructType};
use inkwell::values::{BasicValueEnum, PointerValue};

impl<'ctx> CodeGen<'ctx> {
    /// 递归编译分支链：针对每个分支判定模式与守卫，落入分支体或继续回退后续分支。
    pub(super) fn compile_match_arms(
        &mut self,
        scrut_addr: PointerValue<'ctx>,
        scrut_val: BasicValueEnum<'ctx>,
        scrut_ty: BasicTypeEnum<'ctx>,
        enum_data: Option<(StructType<'ctx>, PointerValue<'ctx>)>,
        enum_info: Option<&EnumInfo<'ctx>>,
        arms: &[MatchArm],
    ) -> Result<BasicValueEnum<'ctx>> {
        let (arm, rest) = match arms.split_first() {
            Some(pair) => pair,
            None => {
                let function = self.current_function()?;
                let unreachable_bb = self.context.append_basic_block(function, "match_unreachable");
                self.builder.build_unconditional_branch(unreachable_bb).unwrap();
                self.builder.position_at_end(unreachable_bb);
                self.builder.build_unreachable().unwrap();
                return Ok(self.context.i32_type().const_zero().into());
            }
        };

        match &arm.pattern {
            Pattern::Wildcard | Pattern::Variable(_) => self.compile_unconditional_arm(
                scrut_addr, scrut_val, scrut_ty, enum_data, enum_info, arm, rest,
            ),
            Pattern::Variant { .. } | Pattern::Literal(_) => self.compile_conditional_arm(
                scrut_addr, scrut_val, scrut_ty, enum_data, enum_info, arm, rest,
            ),
        }
    }

    /// 编译无模式条件的通配符或变量绑定分支（可能有守卫）。
    fn compile_unconditional_arm(
        &mut self,
        scrut_addr: PointerValue<'ctx>,
        scrut_val: BasicValueEnum<'ctx>,
        scrut_ty: BasicTypeEnum<'ctx>,
        enum_data: Option<(StructType<'ctx>, PointerValue<'ctx>)>,
        enum_info: Option<&EnumInfo<'ctx>>,
        arm: &MatchArm,
        rest: &[MatchArm],
    ) -> Result<BasicValueEnum<'ctx>> {
        self.push_scope();
        if let Pattern::Variable(name) = &arm.pattern {
            let actual_val = if enum_data.is_some() {
                self.builder.build_load(scrut_ty, scrut_addr, "scrut_enum_load").unwrap()
            } else {
                scrut_val
            };
            self.bind_variable_pattern(name, actual_val, scrut_ty)?;
        }

        if arm.guard.is_none() {
            let val = self.compile_block_value(&arm.body)?;
            self.pop_scope();
            return Ok(val);
        }

        let guard = arm.guard.as_ref().unwrap();
        let guard_val = self.compile_expr(guard)?;
        if guard_val.get_type() != self.context.bool_type().into() {
            return Err(HuziError::new_global("Match guard must evaluate to a bool"));
        }

        let function = self.current_function()?;
        let body_bb = self.context.append_basic_block(function, "arm_body");
        let next_bb = self.context.append_basic_block(function, "arm_next");
        let merge_bb = self.context.append_basic_block(function, "arm_merge");
        self.builder
            .build_conditional_branch(guard_val.into_int_value(), body_bb, next_bb)
            .unwrap();

        self.builder.position_at_end(body_bb);
        let then_val = self.compile_block_value(&arm.body)?;
        self.pop_scope();

        let result_ty = then_val.get_type();
        let result_ptr = self.build_alloca(result_ty, "match_val")?;
        if self.builder.get_insert_block().map_or(false, |bb| bb.get_terminator().is_none()) {
            self.builder.build_store(result_ptr, then_val).unwrap();
            self.builder.build_unconditional_branch(merge_bb).unwrap();
        }

        self.builder.position_at_end(next_bb);
        let else_val = self.compile_match_arms(scrut_addr, scrut_val, scrut_ty, enum_data, enum_info, rest)?;
        let else_val = self.coerce_value(result_ty, else_val)?;
        if self.builder.get_insert_block().map_or(false, |bb| bb.get_terminator().is_none()) {
            self.builder.build_store(result_ptr, else_val).unwrap();
            self.builder.build_unconditional_branch(merge_bb).unwrap();
        }

        self.builder.position_at_end(merge_bb);
        let result = self.builder.build_load(result_ty, result_ptr, "match_load").unwrap();
        Ok(result)
    }

    /// 编译需比对条件（枚举判别码或字面量）的模式分支。
    fn compile_conditional_arm(
        &mut self,
        scrut_addr: PointerValue<'ctx>,
        scrut_val: BasicValueEnum<'ctx>,
        scrut_ty: BasicTypeEnum<'ctx>,
        enum_data: Option<(StructType<'ctx>, PointerValue<'ctx>)>,
        enum_info: Option<&EnumInfo<'ctx>>,
        arm: &MatchArm,
        rest: &[MatchArm],
    ) -> Result<BasicValueEnum<'ctx>> {
        let cond = match &arm.pattern {
            Pattern::Variant { variant, .. } => {
                let vinfo = find_variant(enum_info.unwrap(), variant)?;
                self.emit_tag_compare(scrut_val.into_int_value(), vinfo.tag)?
            }
            Pattern::Literal(lit) => self.emit_literal_compare(scrut_val, scrut_ty, lit)?,
            _ => unreachable!(),
        };

        let function = self.current_function()?;
        let matched_bb = self.context.append_basic_block(function, "arm_matched");
        let next_bb = self.context.append_basic_block(function, "arm_next");
        let merge_bb = self.context.append_basic_block(function, "arm_merge");
        self.builder.build_conditional_branch(cond, matched_bb, next_bb).unwrap();

        self.builder.position_at_end(matched_bb);
        self.push_scope();
        if let Pattern::Variant { variant, bindings, .. } = &arm.pattern {
            if !bindings.is_empty() {
                let vinfo = find_variant(enum_info.unwrap(), variant)?;
                self.bind_match_payload(enum_data, enum_info.unwrap(), vinfo, bindings)?;
            }
        }

        let body_bb = if let Some(guard) = &arm.guard {
            let guard_val = self.compile_expr(guard)?;
            if guard_val.get_type() != self.context.bool_type().into() {
                return Err(HuziError::new_global("Match guard must evaluate to a bool"));
            }
            let pass_bb = self.context.append_basic_block(function, "guard_pass");
            self.builder.build_conditional_branch(guard_val.into_int_value(), pass_bb, next_bb).unwrap();
            pass_bb
        } else {
            matched_bb
        };

        self.builder.position_at_end(body_bb);
        let then_val = self.compile_block_value(&arm.body)?;
        self.pop_scope();

        let result_ty = then_val.get_type();
        let result_ptr = self.build_alloca(result_ty, "match_val")?;
        if self.builder.get_insert_block().map_or(false, |bb| bb.get_terminator().is_none()) {
            self.builder.build_store(result_ptr, then_val).unwrap();
            self.builder.build_unconditional_branch(merge_bb).unwrap();
        }

        self.builder.position_at_end(next_bb);
        if rest.is_empty() {
            self.builder.build_unreachable().unwrap();
        } else {
            let else_val = self.compile_match_arms(scrut_addr, scrut_val, scrut_ty, enum_data, enum_info, rest)?;
            let else_val = self.coerce_value(result_ty, else_val)?;
            if self.builder.get_insert_block().map_or(false, |bb| bb.get_terminator().is_none()) {
                self.builder.build_store(result_ptr, else_val).unwrap();
                self.builder.build_unconditional_branch(merge_bb).unwrap();
            }
        }

        self.builder.position_at_end(merge_bb);
        let result = self.builder.build_load(result_ty, result_ptr, "match_load").unwrap();
        Ok(result)
    }

    /// 发射字面量比对（整数、布尔、字符、浮点或字符串）。
    fn emit_literal_compare(
        &mut self,
        scrut_val: BasicValueEnum<'ctx>,
        scrut_ty: BasicTypeEnum<'ctx>,
        lit: &Literal,
    ) -> Result<inkwell::values::IntValue<'ctx>> {
        match lit {
            Literal::Int(n) => {
                let lit_int = if scrut_ty == self.context.i64_type().into() {
                    self.context.i64_type().const_int(*n as u64, true)
                } else {
                    self.context.i32_type().const_int(*n as u64, true)
                };
                let cond = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::EQ,
                        scrut_val.into_int_value(),
                        lit_int,
                        "int_match",
                    )
                    .unwrap();
                Ok(cond)
            }
            Literal::Bool(b) => {
                let lit_bool = self.context.bool_type().const_int(if *b { 1 } else { 0 }, false);
                let cond = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::EQ,
                        scrut_val.into_int_value(),
                        lit_bool,
                        "bool_match",
                    )
                    .unwrap();
                Ok(cond)
            }
            Literal::Char(c) => {
                let lit_char = self.context.i8_type().const_int(*c as u64, false);
                let cond = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::EQ,
                        scrut_val.into_int_value(),
                        lit_char,
                        "char_match",
                    )
                    .unwrap();
                Ok(cond)
            }
            Literal::Float(f) => {
                let lit_float = self.context.f64_type().const_float(*f);
                let cond = self
                    .builder
                    .build_float_compare(
                        inkwell::FloatPredicate::OEQ,
                        scrut_val.into_float_value(),
                        lit_float,
                        "float_match",
                    )
                    .unwrap();
                Ok(cond)
            }
            Literal::String(s) => {
                let lit_ptr = self.builder.build_global_string_ptr(s, "lit_str").unwrap();
                let cmp_val = self.build_string_compare(
                    &BinOp::Eq,
                    &scrut_val,
                    &lit_ptr.as_pointer_value().into(),
                )?;
                Ok(cmp_val.into_int_value())
            }
        }
    }

    /// 发射枚举判别码比较。
    fn emit_tag_compare(
        &self,
        tag: inkwell::values::IntValue<'ctx>,
        expected_tag: u32,
    ) -> Result<inkwell::values::IntValue<'ctx>> {
        let expected = self
            .context
            .i32_type()
            .const_int(expected_tag as u64, false);
        let cond = self
            .builder
            .build_int_compare(inkwell::IntPredicate::EQ, tag, expected, "tag_match")
            .unwrap();
        Ok(cond)
    }
}
