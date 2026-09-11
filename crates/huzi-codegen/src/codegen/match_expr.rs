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
        let pat_enum_name = expr.arms.iter().find_map(|arm| match &arm.pattern {
            Pattern::Variant { enum_name, .. } => Some(enum_name.as_str()),
            Pattern::Wildcard => None,
        });

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
                variant, binding, ..
            } => {
                let info = info.ok_or_else(|| {
                    HuziError::new_global("Cannot match variants without a known enum type")
                })?;
                let vinfo = find_variant(info, variant)?;

                let expected = self
                    .context
                    .i32_type()
                    .const_int(vinfo.tag as u64, false);
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

                // Matching arm: optionally bind the payload, evaluate the body.
                self.builder.position_at_end(then_bb);
                let then_val =
                    self.compile_match_arm_body(data, info, vinfo, binding, &arm.body)?;

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
        }
    }

    /// Bind the payload (if the pattern has a binding) and evaluate the arm
    /// body on the matching branch.
    fn compile_match_arm_body(
        &mut self,
        data: Option<(
            inkwell::types::StructType<'ctx>,
            PointerValue<'ctx>,
        )>,
        info: &EnumInfo<'ctx>,
        vinfo: &EnumVariantInfo<'ctx>,
        binding: &Option<String>,
        body: &Block,
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        if let Some(bname) = binding {
            self.bind_match_payload(data, info, vinfo, bname)?;
        }
        let value = self.compile_block_value(body)?;
        if binding.is_some() {
            self.pop_scope();
        }
        Ok(value)
    }

    /// Enter a scope with the pattern binding bound to the variant's payload.
    pub(super) fn bind_match_payload(
        &mut self,
        data: Option<(inkwell::types::StructType<'ctx>, PointerValue<'ctx>)>,
        info: &EnumInfo<'ctx>,
        vinfo: &EnumVariantInfo<'ctx>,
        binding: &str,
    ) -> Result<()> {
        let (enum_st, scrut_addr) = data.ok_or_else(|| {
            HuziError::new_global(format!(
                "Variant '{}::{}' has no payload to bind",
                info.name, vinfo.name
            ))
        })?;
        let payload_ty = vinfo.payload.ok_or_else(|| {
            HuziError::new_global(format!(
                "Variant '{}::{}' has no payload to bind",
                info.name, vinfo.name
            ))
        })?;

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

        // Arrays decay to pointers; keep the element type for indexing.
        let elem = match &vinfo.ast_payload {
            Some(Type::Array(elem_ty, _)) => Some(self.type_to_llvm(elem_ty)?),
            Some(Type::Str) => Some(self.context.i8_type().into()),
            _ => None,
        };
        let array_len = match &vinfo.ast_payload {
            Some(Type::Array(_, size)) => Some(*size as u32),
            _ => None,
        };

        self.push_scope();
        self.scope_insert(
            binding.to_string(),
            VarSlot {
                ptr: slot_ptr,
                ty: payload_ty,
                elem,
                array_len,
                mutable: false,
            },
        );
        Ok(())
    }
}

/// Find the variant info for `variant` inside `info`.
fn find_variant<'ctx, 'a>(
    info: &'a EnumInfo<'ctx>,
    variant: &str,
) -> Result<&'a EnumVariantInfo<'ctx>> {
    info.variants.iter().find(|v| v.name == variant).ok_or_else(|| {
        HuziError::new_global(format!("Enum '{}' has no variant '{}'", info.name, variant))
    })
}
