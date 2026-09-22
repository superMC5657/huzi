//! 携带数据的枚举 `==` / `!=`：首先比较判别码，然后比较匹配变体的负载字段。
//! 字段规则：整数/char/bool 按值比较，浮点数通过 `OEQ` 比较，`str` 通过 `strcmp` 比较，
//! 嵌套结构体通过递归字段比较。仅支持 `==` / `!=`；比较两个不同枚举类型属于编译错误。

use super::{CodeGen, EnumInfo};
use huzi_ast::*;
use huzi_error::{HuziError, Result};
use inkwell::values::{BasicValueEnum, IntValue};

impl<'ctx> CodeGen<'ctx> {
    /// `expr_binary` 中比较路径的入口：两个操作数都是同一携带数据枚举的值。
    /// 返回 `i1` 结果。
    pub(super) fn build_data_enum_compare(
        &mut self,
        op: &BinOp,
        left: &BasicValueEnum<'ctx>,
        right: &BasicValueEnum<'ctx>,
        enum_name: &str,
    ) -> Result<BasicValueEnum<'ctx>> {
        let info = self
            .enums
            .get(enum_name)
            .cloned()
            .ok_or_else(|| HuziError::new_global(format!("Unknown enum: {}", enum_name)))?;
        let eq = self.build_data_enum_values_eq(*left, *right, &info)?;
        if *op == BinOp::Neq {
            Ok(self.builder.build_not(eq, "enum_neq").unwrap().into())
        } else {
            Ok(eq.into())
        }
    }

    /// 检查两个已编译的操作数值：若任意一侧是携带数据的枚举，则处理比较（或报错类型不匹配）。
    /// 当两侧都不是携带数据的枚举时返回 `None`（走常规路径）。
    pub(super) fn try_build_data_enum_compare(
        &mut self,
        op: &BinOp,
        left: &BasicValueEnum<'ctx>,
        right: &BasicValueEnum<'ctx>,
    ) -> Result<Option<BasicValueEnum<'ctx>>> {
        let left_name = self
            .enum_data_by_type(left.get_type())
            .map(|info| info.name.clone());
        let right_name = self
            .enum_data_by_type(right.get_type())
            .map(|info| info.name.clone());
        match (left_name, right_name) {
            (None, None) => Ok(None),
            (Some(l), Some(r)) => {
                if l != r {
                    return Err(HuziError::new_global(format!(
                        "Cannot compare enum '{}' with enum '{}' using '{}'",
                        l,
                        r,
                        bin_op_symbol(op)
                    )));
                }
                if *op != BinOp::Eq && *op != BinOp::Neq {
                    return Err(HuziError::new_global(format!(
                        "Operator '{}' is not supported for enum '{}'; only '==' and '!=' are",
                        bin_op_symbol(op),
                        l
                    )));
                }
                Ok(Some(self.build_data_enum_compare(op, left, right, &l)?))
            }
            (Some(name), None) | (None, Some(name)) => Err(HuziError::new_global(format!(
                "Cannot compare enum '{}' with a non-enum value using '{}'",
                name,
                bin_op_symbol(op)
            ))),
        }
    }

    /// 核心判等：将两个值溢出到内存，比较 tag，然后与 tag 所选变体的字段比较结果进行 AND。
    /// 每个变体的字段仅在其选中的分支上进行比较——其他变体的联合体插槽持有垃圾数据
    /// （特别是悬空 `str` 指针，`strcmp` 绝不可解引用）。
    fn build_data_enum_values_eq(
        &mut self,
        left_val: BasicValueEnum<'ctx>,
        right_val: BasicValueEnum<'ctx>,
        info: &EnumInfo<'ctx>,
    ) -> Result<IntValue<'ctx>> {
        let enum_st = info.llvm.unwrap();
        let union_st = info.payload_union.unwrap();
        let left_ptr = self.build_alloca(left_val.get_type(), "enum_eq_l")?;
        let right_ptr = self.build_alloca(right_val.get_type(), "enum_eq_r")?;
        self.builder.build_store(left_ptr, left_val).unwrap();
        self.builder.build_store(right_ptr, right_val).unwrap();

        let tag_l = self.load_enum_tag(enum_st, left_ptr)?;
        let tag_r = self.load_enum_tag(enum_st, right_ptr)?;
        let tag_eq = self
            .builder
            .build_int_compare(inkwell::IntPredicate::EQ, tag_l, tag_r, "enum_tag_eq")
            .unwrap();
        let acc_ptr = self.build_alloca(self.context.bool_type().into(), "enum_acc")?;
        self.builder.build_store(acc_ptr, tag_eq).unwrap();

        for v in info.variants.clone() {
            if v.ast_payloads.is_empty() {
                continue;
            }
            self.and_variant_fields_eq(enum_st, union_st, v, left_ptr, right_ptr, tag_l, acc_ptr)?;
        }

        Ok(self
            .builder
            .build_load(self.context.bool_type(), acc_ptr, "enum_eq")
            .unwrap()
            .into_int_value())
    }

    /// `if tag == variant.tag { acc &= fields_eq }` — 每个携带负载的变体生成一个受保护的基本块。
    fn and_variant_fields_eq(
        &mut self,
        enum_st: inkwell::types::StructType<'ctx>,
        union_st: inkwell::types::StructType<'ctx>,
        vinfo: super::EnumVariantInfo<'ctx>,
        left_ptr: inkwell::values::PointerValue<'ctx>,
        right_ptr: inkwell::values::PointerValue<'ctx>,
        tag_l: IntValue<'ctx>,
        acc_ptr: inkwell::values::PointerValue<'ctx>,
    ) -> Result<()> {
        let function = self.current_function()?;
        let do_bb = self.context.append_basic_block(function, "enum_cmp_do");
        let next_bb = self.context.append_basic_block(function, "enum_cmp_next");
        let selected = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                tag_l,
                self.context.i32_type().const_int(vinfo.tag as u64, false),
                "enum_sel",
            )
            .unwrap();
        self.builder.build_conditional_branch(selected, do_bb, next_bb).unwrap();

        self.builder.position_at_end(do_bb);
        let fields_eq =
            self.build_variant_fields_eq(enum_st, union_st, &vinfo, left_ptr, right_ptr)?;
        let cur = self
            .builder
            .build_load(self.context.bool_type(), acc_ptr, "enum_acc_cur")
            .unwrap()
            .into_int_value();
        let updated = self.builder.build_and(cur, fields_eq, "enum_acc_upd").unwrap();
        self.builder.build_store(acc_ptr, updated).unwrap();
        self.builder.build_unconditional_branch(next_bb).unwrap();

        self.builder.position_at_end(next_bb);
        Ok(())
    }

    /// 加载溢出存储的枚举值的 `i32` 判别码（字段 0）。
    fn load_enum_tag(
        &self,
        enum_st: inkwell::types::StructType<'ctx>,
        enum_ptr: inkwell::values::PointerValue<'ctx>,
    ) -> Result<IntValue<'ctx>> {
        let tag_ptr = self
            .builder
            .build_struct_gep(enum_st, enum_ptr, 0, "enum_eq_tag_ptr")
            .unwrap();
        Ok(self
            .builder
            .build_load(self.context.i32_type(), tag_ptr, "enum_eq_tag")
            .unwrap()
            .into_int_value())
    }

    /// 对单个变体的所有负载字段比较结果进行 AND。仅在变体选中的分支上运行，
    /// 确保两个联合体插槽确实持有该变体的负载。
    fn build_variant_fields_eq(
        &mut self,
        enum_st: inkwell::types::StructType<'ctx>,
        union_st: inkwell::types::StructType<'ctx>,
        vinfo: &super::EnumVariantInfo<'ctx>,
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
        vinfo: &super::EnumVariantInfo<'ctx>,
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

/// 出现于 `Type::Named` 负载中的原生类型拼写（解析器将 `i32` 存储为 `Named("i32")` 而非 `Type::I32`）。
fn is_primitive_type_name(name: &str) -> bool {
    matches!(
        name,
        "i32" | "u32" | "i64" | "u64" | "f32" | "f64" | "bool" | "char" | "str"
    )
}

/// 比较运算符的字符符号，用于错误提示信息。
fn bin_op_symbol(op: &BinOp) -> &'static str {    match op {
        BinOp::Eq => "==",
        BinOp::Neq => "!=",
        BinOp::Lt => "<",
        BinOp::Le => "<=",
        BinOp::Gt => ">",
        BinOp::Ge => ">=",
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Mod => "%",
        BinOp::And => "&&",
        BinOp::Or => "||",
    }
}
