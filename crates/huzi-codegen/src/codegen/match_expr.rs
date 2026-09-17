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

        // Address of the scrutinee (rvalues are spilled to a temporary).
        let (scrut_addr, scrut_ty) = self.compile_addr(&expr.scrutinee)?;

        // The enum being matched, named by the first variant pattern.
        let pat_enum_name = Self::match_pattern_enum(&expr.arms);

        // Data-carrying enums keep their tag in field 0 of the struct; simple
        // enums ARE the i32 tag, so the scrutinee value is the tag itself.
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

    /// Compile the arm chain recursively: each variant arm branches on the tag
    /// and falls through to the remaining arms on mismatch.
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

    /// Compile a variant arm that has following arms: mismatch falls through
    /// to the remaining arm chain.
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

        // Matching arm: optionally bind the payload, evaluate the body.
        self.builder.position_at_end(then_bb);
        let then_val = self.compile_match_arm_body(data, info, vinfo, bindings, body)?;

        let result_ty = then_val.get_type();
        let result_ptr = self.build_alloca(result_ty, "match_val")?;
        self.builder.build_store(result_ptr, then_val).unwrap();
        self.builder
            .build_unconditional_branch(merge_bb)
            .unwrap();

        // Remaining arms run when the tag does not match.
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

    /// Compile the final variant arm of an exhaustive match: mismatch is
    /// statically unreachable (exhaustiveness was checked up front).
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

    /// Emit the tag comparison and the then/else/merge blocks for one arm.
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

    /// The enum named by the first variant pattern, if any.
    fn match_pattern_enum(arms: &[MatchArm]) -> Option<&str> {
        arms.iter().find_map(|arm| match &arm.pattern {
            Pattern::Variant { enum_name, .. } => Some(enum_name.as_str()),
            Pattern::Wildcard => None,
        })
    }

    /// Exhaustiveness analysis: a wildcard arm covers everything; otherwise
    /// every variant of the matched enum must appear in the arm list.
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

    /// Every variant arm must name the enum being matched.
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

    /// Bind the payload fields (if the pattern names bindings) and evaluate
    /// the arm body on the matching branch.
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

    /// Enter a scope with the pattern bindings bound to the variant's
    /// payload fields, in declaration order.
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

    /// Bind one name to a single-payload variant's slot.
    fn bind_single_payload(
        &mut self,
        ast_ty: &Type,
        payload_ty: inkwell::types::BasicTypeEnum<'ctx>,
        slot_ptr: PointerValue<'ctx>,
        binding: &str,
    ) -> Result<()> {
        // Arrays decay to pointers; keep the element type for indexing.
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
        let box_inner = self.box_pointee_of_ast(ast_ty)?;
        self.scope_insert(
            binding.to_string(),
            VarSlot {
                ptr: slot_ptr,
                ty: payload_ty,
                elem,
                array_len,
                mutable: false,
                box_inner,
            },
        );
        Ok(())
    }

    /// Bind each name to its field of a multi-payload variant's field struct.
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
            let box_inner = self.box_pointee_of_ast(ast_ty)?;
            self.scope_insert(
                binding.to_string(),
                VarSlot {
                    ptr: field_ptr,
                    ty: field_ty,
                    elem,
                    array_len,
                    mutable: false,
                    box_inner,
                },
            );
        }
        Ok(())
    }
}

/// Find the variant info for `variant` inside `info`.
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
