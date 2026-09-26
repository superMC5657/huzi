//! 携带数据的枚举 `==` / `!=`:首先比较判别码,然后比较匹配变体的负载字段。
//! 字段规则:整数/char/bool 按值比较,浮点数通过 `OEQ` 比较,`str` 通过 `strcmp` 比较,
//! 嵌套结构体通过递归字段比较。仅支持 `==` / `!=`;比较两个不同枚举类型属于编译错误。
//!
//! 子模块:`fields`(变体负载字段比较)。(目录化自 `enum_eq.rs` 纯搬移,零逻辑变化。)

mod fields;

use super::{CodeGen, EnumInfo};
use huzi_ast::*;
use huzi_error::{HuziError, Result};
use inkwell::values::{BasicValueEnum, IntValue};

impl<'ctx> CodeGen<'ctx> {
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

    pub(super) fn build_data_enum_values_eq(
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
}

/// 出现于 `Type::Named` 负载中的原生类型拼写（解析器将 `i32` 存储为 `Named("i32")` 而非 `Type::I32`）。
pub(super) fn is_primitive_type_name(name: &str) -> bool {
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
