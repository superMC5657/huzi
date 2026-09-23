use super::super::{CodeGen, EnumInfo, EnumVariantInfo, VarSlot};
use huzi_ast::*;
use huzi_error::{HuziError, Result};
use inkwell::values::PointerValue;

impl<'ctx> CodeGen<'ctx> {
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

    /// 将待匹配项作为变量绑定到局部作用域（用于 `x => ...` 模式）。
    pub(super) fn bind_variable_pattern(
        &mut self,
        name: &str,
        scrut_val: inkwell::values::BasicValueEnum<'ctx>,
        scrut_ty: inkwell::types::BasicTypeEnum<'ctx>,
    ) -> Result<()> {
        let slot_ptr = self.build_alloca(scrut_ty, name)?;
        self.builder.build_store(slot_ptr, scrut_val).unwrap();
        let elem = if scrut_ty == self.context.ptr_type(inkwell::AddressSpace::default()).into() {
            Some(self.context.i8_type().into())
        } else {
            None
        };
        self.scope_insert(
            name.to_string(),
            VarSlot {
                ptr: slot_ptr,
                ty: scrut_ty,
                elem,
                array_len: None,
                mutable: false,
                box_inner: None,
                map_kind: None,
            },
        );
        Ok(())
    }
}

/// 在 `info` 中查找变体 `variant` 的信息。
pub(super) fn find_variant<'ctx, 'a>(
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
