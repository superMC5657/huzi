//! 变体负载字段比较:联合体插槽加载、命名/结构体/元组字段判等、位宽统一。(自 `enum_eq.rs` 纯搬移,零逻辑变化)。

use super::super::{CodeGen, EnumVariantInfo};
use super::is_primitive_type_name;
use huzi_ast::*;
use huzi_error::{HuziError, Result};
use inkwell::values::{BasicValueEnum, IntValue};

impl<'ctx> CodeGen<'ctx> {
    pub(super) fn build_variant_fields_eq(
        &mut self,
        enum_st: inkwell::types::StructType<'ctx>,
        union_st: inkwell::types::StructType<'ctx>,
        vinfo: &EnumVariantInfo<'ctx>,
        left_ptr: inkwell::values::PointerValue<'ctx>,
        right_ptr: inkwell::values::PointerValue<'ctx>,
    ) -> Result<IntValue<'ctx>> {
        let payload_ty = vinfo.payload.unwrap();
        let slot_l = self.load_union_slot(enum_st, union_st, vinfo, payload_ty, left_ptr)?;
        let slot_r = self.load_union_slot(enum_st, union_st, vinfo, payload_ty, right_ptr)?;

        let mut fields_eq = self.context.bool_type().const_int(1, false);
        if vinfo.ast_payloads.len() == 1 {
            let f = self.build_payload_field_eq(&vinfo.ast_payloads[0], slot_l, slot_r)?;
            fields_eq = self.builder.build_and(fields_eq, f, "enum_fld_eq").unwrap();
        } else {
            for (i, ast_ty) in vinfo.ast_payloads.iter().enumerate() {
                let lv = self
                    .builder
                    .build_extract_value(slot_l.into_struct_value(), i as u32, "enum_fld_l")
                    .unwrap();
                let rv = self
                    .builder
                    .build_extract_value(slot_r.into_struct_value(), i as u32, "enum_fld_r")
                    .unwrap();
                let f = self.build_payload_field_eq(ast_ty, lv, rv)?;
                fields_eq = self.builder.build_and(fields_eq, f, "enum_fld_eq").unwrap();
            }
        }
        Ok(fields_eq)
    }

    /// 从溢出存储的枚举值中加载单个变体的联合体插槽。
    fn load_union_slot(
        &self,
        enum_st: inkwell::types::StructType<'ctx>,
        union_st: inkwell::types::StructType<'ctx>,
        vinfo: &EnumVariantInfo<'ctx>,
        payload_ty: inkwell::types::BasicTypeEnum<'ctx>,
        enum_ptr: inkwell::values::PointerValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>> {
        let union_ptr = self
            .builder
            .build_struct_gep(enum_st, enum_ptr, 1, "enum_eq_union")
            .unwrap();
        let slot_ptr = self
            .builder
            .build_struct_gep(union_st, union_ptr, vinfo.payload_slot.unwrap(), "enum_eq_slot")
            .unwrap();
        Ok(self
            .builder
            .build_load(payload_ty, slot_ptr, "enum_eq_slotv")
            .unwrap())
    }

    /// 比较具有相同声明 AST 类型的两个负载字段值。
    fn build_payload_field_eq(
        &mut self,
        ast_ty: &Type,
        left: BasicValueEnum<'ctx>,
        right: BasicValueEnum<'ctx>,
    ) -> Result<IntValue<'ctx>> {
        // 命名类型解析为结构体、简单枚举或带数据枚举——但
        // 原生类型拼写（`i32`、`str` 等）走下方的通用值类型比较。
        if let Type::Named(name) = ast_ty {
            if !is_primitive_type_name(name) {
                return self.build_named_field_eq(name, left, right);
            }
        }
        if left.is_int_value() && right.is_int_value() {
            let (l, r) = self.unify_int_width(left.into_int_value(), right.into_int_value());
            return Ok(self
                .builder
                .build_int_compare(inkwell::IntPredicate::EQ, l, r, "enum_int_eq")
                .unwrap());
        }
        if left.is_float_value() && right.is_float_value() {
            let (l, r) = self.unify_float_width(left.into_float_value(), right.into_float_value());
            return Ok(self
                .builder
                .build_float_compare(inkwell::FloatPredicate::OEQ, l, r, "enum_flt_eq")
                .unwrap());
        }
        if left.is_pointer_value() && right.is_pointer_value() {
            // `str` 负载使用 strcmp 比较。
            return Ok(self
                .build_string_compare(&BinOp::Eq, &left, &right)?
                .into_int_value());
        }
        if let Type::Tuple(elems) = ast_ty {
            return self.build_tuple_value_eq(elems, left, right);
        }
        Err(HuziError::new_global(format!(
            "Cannot compare payload values of type '{}' with '=='",
            ast_ty
        )))
    }

    /// 比较声明类型为命名用户类型的两个字段。
    fn build_named_field_eq(
        &mut self,
        name: &str,
        left: BasicValueEnum<'ctx>,
        right: BasicValueEnum<'ctx>,
    ) -> Result<IntValue<'ctx>> {
        if let Some((st, _)) = self.structs.get(name).cloned() {
            return self.build_struct_value_eq(st, left, right);
        }
        if let Some(info) = self.enums.get(name).cloned() {
            if info.llvm.is_none() {
                // 简单枚举：其值就是其 i32 tag。
                return Ok(self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::EQ,
                        left.into_int_value(),
                        right.into_int_value(),
                        "enum_tag_eq",
                    )
                    .unwrap());
            }
            return self.build_data_enum_values_eq(left, right, &info);
        }
        Err(HuziError::new_global(format!(
            "Cannot compare payload values of type '{}' with '=='",
            name
        )))
    }

    /// 递归逐字段比较两个结构体值。
    fn build_struct_value_eq(
        &mut self,
        st: inkwell::types::StructType<'ctx>,
        left: BasicValueEnum<'ctx>,
        right: BasicValueEnum<'ctx>,
    ) -> Result<IntValue<'ctx>> {
        let fields: Vec<(inkwell::types::BasicTypeEnum<'ctx>, Type)> = self
            .structs
            .values()
            .find(|(def_st, _)| *def_st == st)
            .map(|(_, infos)| {
                infos
                    .iter()
                    .map(|f| (f.ty, f.ast_ty.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let mut acc = self.context.bool_type().const_int(1, false);
        for (i, (_, ast_ty)) in fields.iter().enumerate() {
            let lv = self
                .builder
                .build_extract_value(left.into_struct_value(), i as u32, "enum_st_l")
                .unwrap();
            let rv = self
                .builder
                .build_extract_value(right.into_struct_value(), i as u32, "enum_st_r")
                .unwrap();
            let f = self.build_payload_field_eq(ast_ty, lv, rv)?;
            acc = self.builder.build_and(acc, f, "enum_st_eq").unwrap();
        }
        Ok(acc)
    }

    /// 逐元素比较两个元组值。
    fn build_tuple_value_eq(
        &mut self,
        elems: &[Type],
        left: BasicValueEnum<'ctx>,
        right: BasicValueEnum<'ctx>,
    ) -> Result<IntValue<'ctx>> {
        let mut acc = self.context.bool_type().const_int(1, false);
        for (i, ast_ty) in elems.iter().enumerate() {
            let lv = self
                .builder
                .build_extract_value(left.into_struct_value(), i as u32, "enum_tp_l")
                .unwrap();
            let rv = self
                .builder
                .build_extract_value(right.into_struct_value(), i as u32, "enum_tp_r")
                .unwrap();
            let f = self.build_payload_field_eq(ast_ty, lv, rv)?;
            acc = self.builder.build_and(acc, f, "enum_tp_eq").unwrap();
        }
        Ok(acc)
    }

    /// 将较窄的整数操作数符号扩展为较宽的位宽。
    fn unify_int_width(
        &self,
        left: IntValue<'ctx>,
        right: IntValue<'ctx>,
    ) -> (IntValue<'ctx>, IntValue<'ctx>) {
        let lw = left.get_type().get_bit_width();
        let rw = right.get_type().get_bit_width();
        if lw < rw {
            (
                self.builder.build_int_s_extend(left, right.get_type(), "enum_widen").unwrap(),
                right,
            )
        } else if rw < lw {
            (
                left,
                self.builder.build_int_s_extend(right, left.get_type(), "enum_widen").unwrap(),
            )
        } else {
            (left, right)
        }
    }

    /// 将较窄的浮点数操作数转换为较宽的浮点类型。
    fn unify_float_width(
        &self,
        left: inkwell::values::FloatValue<'ctx>,
        right: inkwell::values::FloatValue<'ctx>,
    ) -> (
        inkwell::values::FloatValue<'ctx>,
        inkwell::values::FloatValue<'ctx>,
    ) {
        if left.get_type() == right.get_type() {
            (left, right)
        } else if left.get_type() == self.context.f32_type() {
            (
                self.builder.build_float_cast(left, right.get_type(), "enum_fwiden").unwrap(),
                right,
            )
        } else {
            (
                left,
                self.builder.build_float_cast(right, left.get_type(), "enum_fwiden").unwrap(),
            )
        }
    }
}
