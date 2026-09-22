//! Data-carrying enum `==` / `!=`: compare the discriminant first, then the
//! payload fields of the matching variant. Field rules: integers/char/bool
//! compare by value, floats by `OEQ`, `str` via `strcmp`, nested structs by
//! recursive field comparison. Only `==` / `!=` are supported; comparing two
//! different enum types is a compile error.

use super::{CodeGen, EnumInfo};
use huzi_ast::*;
use huzi_error::{HuziError, Result};
use inkwell::values::{BasicValueEnum, IntValue};

impl<'ctx> CodeGen<'ctx> {
    /// Entry from the comparison path in `expr_binary`: both operands are
    /// values of the same data-carrying enum. Returns the `i1` result.
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

    /// Look at two already-compiled operand values: if either side is a
    /// data-carrying enum, resolve the comparison (or report a type error).
    /// Returns `None` when neither side is a data enum (normal path).
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

    /// Core equality: spill both values, compare tags, then AND the field
    /// comparisons of whichever variant the tag selects. Each variant's
    /// fields are compared only on its selected branch — other variants'
    /// union slots hold garbage (notably dangling `str` pointers that
    /// `strcmp` must never dereference).
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

    /// `if tag == variant.tag { acc &= fields_eq }` — one guarded block per
    /// payload-carrying variant.
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

    /// Load the `i32` discriminant (field 0) of a spilled enum value.
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

    /// AND of all payload-field comparisons for one variant. Runs only on
    /// the variant's selected branch, so both union slots genuinely carry
    /// this variant's payload.
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

    /// Load one variant's union slot from a spilled enum value.
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

    /// Compare two payload field values of the same declared AST type.
    fn build_payload_field_eq(
        &mut self,
        ast_ty: &Type,
        left: BasicValueEnum<'ctx>,
        right: BasicValueEnum<'ctx>,
    ) -> Result<IntValue<'ctx>> {
        // Named types resolve to structs, simple enums or data enums — but
        // primitive spellings (`i32`, `str`, ...) go through the generic
        // value-kind comparison below.
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
            // `str` payloads compare with strcmp.
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

    /// Compare two fields whose declared type is a named user type.
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
                // Simple enum: the value is its i32 tag.
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

    /// Recursively compare two struct values field by field.
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

    /// Compare two tuple values element by element.
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

    /// Sign-extend the narrower int operand to the wider width.
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

    /// Cast the narrower float operand to the wider float type.
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

/// Primitive type spellings as they appear in `Type::Named` payloads (the
/// parser stores `i32` as `Named("i32")`, not `Type::I32`).
fn is_primitive_type_name(name: &str) -> bool {
    matches!(
        name,
        "i32" | "u32" | "i64" | "u64" | "f32" | "f64" | "bool" | "char" | "str"
    )
}

/// Single-character symbol of a comparison operator, for error messages.
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
