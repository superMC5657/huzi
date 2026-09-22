use super::{CodeGen, EnumInfo, EnumVariantInfo, VarSlot};
use inkwell::values::PointerValue;
use huzi_ast::*;
use huzi_error::{HuziError, Result};


impl<'ctx> CodeGen<'ctx> {
    pub(super) fn compile_match_expr(
        &mut self,
        expr: &MatchExpr,
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        if expr.arms.is_empty() {
            return Err(HuziError::new_global("match must have at least one arm"));
        }

        // 待匹配项的地址（右值会溢出暂存到临时变量）。
        let (scrut_addr, scrut_ty) = self.compile_addr(&expr.scrutinee)?;

        // 正在匹配的枚举，由首个变体模式命名。
        let pat_enum_name = Self::match_pattern_enum(&expr.arms);

        // 携带数据的枚举将其判别码 tag 保存在结构体的字段 0；简单枚举
        // 本身就是 i32 判别码，因此待匹配项的值即为判别码本身。
        if let Some(info) = self.enum_data_by_type(scrut_ty) {
            if let Some(pat) = pat_enum_name {
                if pat != info.name {
                    return Err(HuziError::new_global(format!(
                        "Match arms use '{}' but the scrutinee is '{}'",
                        pat, info.name
                    )));
                }
            }
            let st = info.llvm.unwrap();
            let info = info.clone();
            Self::check_match_exhaustiveness(&expr.arms, Some(&info))?;
            let tag_ptr = self
                .builder
                .build_struct_gep(st, scrut_addr, 0, "match_tag_ptr")
                .unwrap();
            let tag = self
                .builder
                .build_load(self.context.i32_type(), tag_ptr, "match_tag")
                .unwrap()
                .into_int_value();
            return self.compile_match_arms(tag, Some((st, scrut_addr)), Some(&info), &expr.arms);
        }

        if scrut_ty == self.context.i32_type().into() {
            let info = match pat_enum_name {
                Some(name) => {
                    let info = self.enums.get(name).cloned().ok_or_else(|| {
                        HuziError::new_global(format!("Unknown enum: {}", name))
                    })?;
                    if info.llvm.is_some() {
                        return Err(HuziError::new_global(format!(
                            "Cannot match '{}' against a plain i32 scrutinee; it carries data",
                            name
                        )));
                    }
                    Some(info)
                }
                None => None,
            };
            let tag = self
                .builder
                .build_load(self.context.i32_type(), scrut_addr, "match_tag")
                .unwrap()
                .into_int_value();
            Self::check_match_exhaustiveness(&expr.arms, info.as_ref())?;
            return self.compile_match_arms(tag, None, info.as_ref(), &expr.arms);
        }

        Err(HuziError::new_global(
            "match scrutinee must be an enum value",
        ))
    }

    /// 递归编译分支链：每个变体分支根据判别码进行分支，
    /// 不匹配时向下落入剩余分支。
    pub(super) fn compile_match_arms(
        &mut self,
        tag: inkwell::values::IntValue<'ctx>,
        data: Option<(
            inkwell::types::StructType<'ctx>,
            PointerValue<'ctx>,
        )>,
        info: Option<&EnumInfo<'ctx>>,
        arms: &[MatchArm],
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        let (arm, rest) = arms.split_first().ok_or_else(|| {
            HuziError::new_global("match must have a wildcard arm `_`")
        })?;

        match &arm.pattern {
            Pattern::Wildcard => self.compile_block_value(&arm.body),
            Pattern::Variant {
                variant, bindings, ..
            } => {
                let info = info.ok_or_else(|| {
                    HuziError::new_global("Cannot match variants without a known enum type")
                })?;
                let vinfo = find_variant(info, variant)?;
                if rest.is_empty() {
                    return self.compile_last_variant_arm(tag, data, info, vinfo, bindings, &arm.body);
                }
                self.compile_variant_arm(tag, data, info, vinfo, bindings, &arm.body, rest)
            }
        }
    }

    /// 编译后随其他分支的变体分支：不匹配时落入剩余分支链。
    fn compile_variant_arm(
        &mut self,
        tag: inkwell::values::IntValue<'ctx>,
        data: Option<(
            inkwell::types::StructType<'ctx>,
            PointerValue<'ctx>,
        )>,
        info: &EnumInfo<'ctx>,
        vinfo: &EnumVariantInfo<'ctx>,
        bindings: &[String],
        body: &Block,
        rest: &[MatchArm],
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        let (then_bb, else_bb, merge_bb) = self.emit_match_branch(tag, vinfo.tag)?;

        // 匹配分支：可选绑定负载，求值分支体。
        self.builder.position_at_end(then_bb);
        let then_val = self.compile_match_arm_body(data, info, vinfo, bindings, body)?;

        let result_ty = then_val.get_type();
        let result_ptr = self.build_alloca(result_ty, "match_val")?;
        self.builder.build_store(result_ptr, then_val).unwrap();
        self.builder
            .build_unconditional_branch(merge_bb)
            .unwrap();

        // 判别码不匹配时运行剩余分支。
        self.builder.position_at_end(else_bb);
        let else_val = self.compile_match_arms(tag, data, Some(info), rest)?;
        let else_val = self.coerce_value(result_ty, else_val)?;
        self.builder.build_store(result_ptr, else_val).unwrap();
        self.builder
            .build_unconditional_branch(merge_bb)
            .unwrap();

        self.builder.position_at_end(merge_bb);
        let result = self
            .builder
            .build_load(result_ty, result_ptr, "match_load")
            .unwrap();
        Ok(result)
    }

    /// 编译穷尽匹配的最后一个变体分支：不匹配在静态上不可达
    /// （前面已完成穷尽性检查）。
    fn compile_last_variant_arm(
        &mut self,
        tag: inkwell::values::IntValue<'ctx>,
        data: Option<(
            inkwell::types::StructType<'ctx>,
            PointerValue<'ctx>,
        )>,
        info: &EnumInfo<'ctx>,
        vinfo: &EnumVariantInfo<'ctx>,
        bindings: &[String],
        body: &Block,
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        let (then_bb, else_bb, merge_bb) = self.emit_match_branch(tag, vinfo.tag)?;
        self.builder.position_at_end(then_bb);
        let then_val = self.compile_match_arm_body(data, info, vinfo, bindings, body)?;

        let result_ty = then_val.get_type();
        let result_ptr = self.build_alloca(result_ty, "match_val")?;
        self.builder.build_store(result_ptr, then_val).unwrap();
        self.builder
            .build_unconditional_branch(merge_bb)
            .unwrap();

        self.builder.position_at_end(else_bb);
        self.builder.build_unreachable().unwrap();

        self.builder.position_at_end(merge_bb);
        let result = self
            .builder
            .build_load(result_ty, result_ptr, "match_load")
            .unwrap();
        Ok(result)
    }

    /// 发射单个分支的判别码比较以及 then/else/merge 基本块。
    fn emit_match_branch(
        &self,
        tag: inkwell::values::IntValue<'ctx>,
        expected_tag: u32,
    ) -> Result<(
        inkwell::basic_block::BasicBlock<'ctx>,
        inkwell::basic_block::BasicBlock<'ctx>,
        inkwell::basic_block::BasicBlock<'ctx>,
    )> {
        let expected = self
            .context
            .i32_type()
            .const_int(expected_tag as u64, false);
        let cond = self
            .builder
            .build_int_compare(inkwell::IntPredicate::EQ, tag, expected, "match_cond")
            .unwrap();

        let function = self.current_function()?;
        let then_bb = self.context.append_basic_block(function, "match_arm");
        let else_bb = self.context.append_basic_block(function, "match_next");
        let merge_bb = self.context.append_basic_block(function, "match_merge");
        self.builder
            .build_conditional_branch(cond, then_bb, else_bb)
            .unwrap();
        Ok((then_bb, else_bb, merge_bb))
    }

    /// 首个变体模式所指定的枚举名称（若存在）。
    fn match_pattern_enum(arms: &[MatchArm]) -> Option<&str> {
        arms.iter().find_map(|arm| match &arm.pattern {
            Pattern::Variant { enum_name, .. } => Some(enum_name.as_str()),
            Pattern::Wildcard => None,
        })
    }

    /// 穷尽性分析：通配符分支覆盖所有情况；否则被匹配枚举的
    /// 每个变体都必须在分支列表中出现。
    fn check_match_exhaustiveness(
        arms: &[MatchArm],
        info: Option<&EnumInfo<'ctx>>,
    ) -> Result<()> {
        if arms.iter().any(|arm| matches!(arm.pattern, Pattern::Wildcard)) {
            return Self::check_arm_enum_names(arms, info);
        }
        let info = info.ok_or_else(|| {
            HuziError::new_global("match must have a wildcard arm `_`")
        })?;
        Self::check_arm_enum_names(arms, Some(info))?;
        for arm in arms {
            if let Pattern::Variant { variant, .. } = &arm.pattern {
                find_variant(info, variant)?;
            }
        }
        let missing: Vec<&str> = info
            .variants
            .iter()
            .filter(|v| !arms.iter().any(|arm| match &arm.pattern {
                Pattern::Variant { variant, .. } => variant == &v.name,
                Pattern::Wildcard => false,
            }))
            .map(|v| v.name.as_str())
            .collect();
        if missing.is_empty() {
            return Ok(());
        }
        Err(HuziError::new_global(format!(
            "Non-exhaustive match for enum '{}': missing variants: {}\n  help: add arms for the missing variants or a wildcard arm `_`",
            info.name,
            missing.join(", ")
        )))
    }

    /// 每个变体分支必须指明正在匹配的枚举。
    fn check_arm_enum_names(arms: &[MatchArm], info: Option<&EnumInfo<'ctx>>) -> Result<()> {
        let Some(info) = info else {
            return Ok(());
        };
        for arm in arms {
            if let Pattern::Variant { enum_name, .. } = &arm.pattern {
                if enum_name != &info.name {
                    return Err(HuziError::new_global(format!(
                        "Match arms use '{}' but the scrutinee is '{}'",
                        enum_name, info.name
                    )));
                }
            }
        }
        Ok(())
    }

    /// 在匹配成功的分支上绑定负载字段（若模式指定了绑定变量）并求值分支体。
    fn compile_match_arm_body(
        &mut self,
        data: Option<(
            inkwell::types::StructType<'ctx>,
            PointerValue<'ctx>,
        )>,
        info: &EnumInfo<'ctx>,
        vinfo: &EnumVariantInfo<'ctx>,
        bindings: &[String],
        body: &Block,
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        if !bindings.is_empty() {
            self.bind_match_payload(data, info, vinfo, bindings)?;
        }
        let value = self.compile_block_value(body)?;
        if !bindings.is_empty() {
            self.pop_scope();
        }
        Ok(value)
    }

    /// 进入新作用域，按声明顺序将模式绑定变量绑定到变体的负载字段。
    pub(super) fn bind_match_payload(
        &mut self,
        data: Option<(inkwell::types::StructType<'ctx>, PointerValue<'ctx>)>,
        info: &EnumInfo<'ctx>,
        vinfo: &EnumVariantInfo<'ctx>,
        bindings: &[String],
    ) -> Result<()> {
        let (enum_st, scrut_addr) = data.ok_or_else(|| {
            HuziError::new_global(format!(
                "Variant '{}::{}' has no payload to bind",
                info.name, vinfo.name
            ))
        })?;
        if vinfo.ast_payloads.is_empty() {
            return Err(HuziError::new_global(format!(
                "Variant '{}::{}' has no payload to bind",
                info.name, vinfo.name
            )));
        }
        if bindings.len() != vinfo.ast_payloads.len() {
            return Err(HuziError::new_global(format!(
                "Variant '{}::{}' has {} payload field(s), but the pattern binds {}",
                info.name,
                vinfo.name,
                vinfo.ast_payloads.len(),
                bindings.len()
            )));
        }

        let union_st = info.payload_union.unwrap();
        let union_ptr = self
            .builder
            .build_struct_gep(enum_st, scrut_addr, 1, "bind_payload_ptr")
            .unwrap();
        let slot_ptr = self
            .builder
            .build_struct_gep(
                union_st,
                union_ptr,
                vinfo.payload_slot.unwrap(),
                "bind_slot_ptr",
            )
            .unwrap();

        self.push_scope();
        if vinfo.ast_payloads.len() == 1 {
            self.bind_single_payload(&vinfo.ast_payloads[0], vinfo.payload.unwrap(), slot_ptr, &bindings[0])?;
        } else {
            self.bind_multi_payload_fields(vinfo.payload.unwrap(), slot_ptr, &vinfo.ast_payloads, bindings)?;
        }
        Ok(())
    }

    /// 将单个名称绑定到单负载变体的插槽。
    fn bind_single_payload(
        &mut self,
        ast_ty: &Type,
        payload_ty: inkwell::types::BasicTypeEnum<'ctx>,
        slot_ptr: PointerValue<'ctx>,
        binding: &str,
    ) -> Result<()> {
        // 数组退化为指针；保留元素类型以便索引。
        let elem = match ast_ty {
            Type::Array(elem_ty, _) => Some(self.type_to_llvm(elem_ty)?),
            Type::Str => Some(self.context.i8_type().into()),
            _ => None,
        };
        let array_len = match ast_ty {
            Type::Array(_, size) => Some(*size as u32),
            _ => None,
        };
        // Box payload 绑定同样记录 pointee,供手臂内的字段解引用。
        let box_inner = self.box_nest_of_ast(ast_ty)?;
        self.scope_insert(
            binding.to_string(),
            VarSlot {
                ptr: slot_ptr,
                ty: payload_ty,
                elem,
                array_len,
                mutable: false,
                box_inner,
                map_kind: None,
            },
        );
        Ok(())
    }

    /// 将各个名称绑定到多负载变体字段结构体中对应的字段。
    fn bind_multi_payload_fields(
        &mut self,
        payload_ty: inkwell::types::BasicTypeEnum<'ctx>,
        slot_ptr: PointerValue<'ctx>,
        ast_payloads: &[Type],
        bindings: &[String],
    ) -> Result<()> {
        let field_st = payload_ty.into_struct_type();
        for (i, (ast_ty, binding)) in ast_payloads.iter().zip(bindings.iter()).enumerate() {
            let field_ty = field_st.get_field_type_at_index(i as u32).unwrap();
            let field_ptr = self
                .builder
                .build_struct_gep(field_st, slot_ptr, i as u32, "bind_field_ptr")
                .unwrap();
            let elem = match ast_ty {
                Type::Array(elem_ty, _) => Some(self.type_to_llvm(elem_ty)?),
                Type::Str => Some(self.context.i8_type().into()),
                _ => None,
            };
            let array_len = match ast_ty {
                Type::Array(_, size) => Some(*size as u32),
                _ => None,
            };
            let box_inner = self.box_nest_of_ast(ast_ty)?;
            self.scope_insert(
                binding.to_string(),
                VarSlot {
                    ptr: field_ptr,
                    ty: field_ty,
                    elem,
                    array_len,
                    mutable: false,
                    box_inner,
                    map_kind: None,
                },
            );
        }
        Ok(())
    }
}

/// 在 `info` 中查找变体 `variant` 的信息。
fn find_variant<'ctx, 'a>(
    info: &'a EnumInfo<'ctx>,
    variant: &str,
) -> Result<&'a EnumVariantInfo<'ctx>> {
    info.variants.iter().find(|v| v.name == variant).ok_or_else(|| {
        let mut message = format!("Enum '{}' has no variant '{}'", info.name, variant);
        if let Some(hint) =
            huzi_error::did_you_mean(variant, info.variants.iter().map(|v| v.name.as_str()))
        {
            message.push_str(&format!("\n  help: {}", hint));
        }
        HuziError::new_global(message)
    })
}
