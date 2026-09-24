use huzi_error::{HuziError, Result};

impl<'ctx> super::CodeGen<'ctx> {
    /// 当属于无损/预期的数值强制转换时，将值转换为目标类型；否则报告类型不匹配错误。
    pub(in super::super) fn coerce_value(
        &self,
        target: inkwell::types::BasicTypeEnum<'ctx>,
        value: inkwell::values::BasicValueEnum<'ctx>,
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        if value.get_type() == target {
            return Ok(value);
        }

        match (target, value) {
            (inkwell::types::BasicTypeEnum::IntType(it), inkwell::values::BasicValueEnum::IntValue(iv)) => {
                let tw = it.get_bit_width();
                let sw = iv.get_type().get_bit_width();
                if tw > sw {
                    Ok(self
                        .builder
                        .build_int_s_extend(iv, it, "coerce")
                        .unwrap()
                        .into())
                } else if tw < sw {
                    Ok(self
                        .builder
                        .build_int_truncate(iv, it, "coerce")
                        .unwrap()
                        .into())
                } else {
                    Ok(iv.into())
                }
            }
            (inkwell::types::BasicTypeEnum::FloatType(ft), inkwell::values::BasicValueEnum::IntValue(iv)) => {
                Ok(self
                    .builder
                    .build_signed_int_to_float(iv, ft, "coerce")
                    .unwrap()
                    .into())
            }
            (inkwell::types::BasicTypeEnum::IntType(it), inkwell::values::BasicValueEnum::FloatValue(fv)) => {
                Ok(self
                    .builder
                    .build_float_to_signed_int(fv, it, "coerce")
                    .unwrap()
                    .into())
            }
            (inkwell::types::BasicTypeEnum::FloatType(ft), inkwell::values::BasicValueEnum::FloatValue(fv)) => {
                if fv.get_type() == ft {
                    Ok(fv.into())
                } else {
                    Ok(self.builder.build_float_cast(fv, ft, "coerce").unwrap().into())
                }
            }
            (inkwell::types::BasicTypeEnum::IntType(it), inkwell::values::BasicValueEnum::PointerValue(pv))
                if it.get_bit_width() == 64 =>
            {
                Ok(self.builder.build_ptr_to_int(pv, it, "coerce").unwrap().into())
            }
            _ => Err(HuziError::new_global(format!(
                "Type mismatch: expected {}, got {}",
                target,
                value.get_type()
            ))),
        }
    }
}
