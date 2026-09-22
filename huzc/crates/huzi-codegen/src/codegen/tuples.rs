use super::{CodeGen, VarSlot};
use inkwell::values::PointerValue;
use huzi_ast::*;
use huzi_error::{HuziError, Result};

impl<'ctx> CodeGen<'ctx> {
    /// 判定是否为匿名结构体字面量（元组），它们绝不作为具名结构体或枚举体部注册。
    pub(super) fn is_tuple_type(&self, st: inkwell::types::StructType<'ctx>) -> bool {
        self.structs.values().all(|(s, _)| *s != st)
            && self.enums.values().all(|info| info.llvm != Some(st))
    }

    /// `(a, b, c)` — 从已编译的元素值构建字面量结构体，从各元素自身推导元组类型。
    pub(super) fn compile_tuple_literal(
        &mut self,
        elements: &[Expr],
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        let mut values = Vec::with_capacity(elements.len());
        for elem in elements {
            values.push(self.compile_expr(elem)?);
        }

        let field_types: Vec<inkwell::types::BasicTypeEnum<'ctx>> =
            values.iter().map(|v| v.get_type()).collect();
        let tuple_ty = self.context.struct_type(&field_types, false);
        let tmp = self.store_tuple_fields(tuple_ty, &values, "tuple_val")?;
        Ok(self.builder.build_load(tuple_ty, tmp, "tuple_load").unwrap())
    }

    /// `let name = (a, b)` / `let name: (T1, T2) = (a, b)`。若显式指定类型注解，
    /// 则每个元素强制转换为声明的元素类型；否则元组类型从各元素值推导。
    pub(super) fn compile_let_tuple(
        &mut self,
        stmt: &LetStmt,
        elements: &[Expr],
        span: Span,
    ) -> Result<()> {
        let annotated = match &stmt.type_annotation {
            Some(Type::Tuple(elems)) => Some(elems.clone()),
            Some(other) => {
                return Err(HuziError::new_global(format!(
                    "Type mismatch: cannot assign a tuple literal to {}",
                    other
                )))
            }
            None => None,
        };

        let mut values = Vec::with_capacity(elements.len());
        for elem in elements {
            values.push(self.compile_expr(elem)?);
        }

        let field_types: Vec<inkwell::types::BasicTypeEnum<'ctx>> = match &annotated {
            Some(elem_types) => {
                if elem_types.len() != values.len() {
                    return Err(HuziError::new_global(format!(
                        "Tuple type has {} element(s), but the literal has {}",
                        elem_types.len(),
                        values.len()
                    )));
                }
                elem_types
                    .iter()
                    .map(|t| self.type_to_llvm(t))
                    .collect::<Result<Vec<_>>>()?
            }
            None => values.iter().map(|v| v.get_type()).collect(),
        };

        let tuple_ty = self.context.struct_type(&field_types, false);
        let tuple_ptr = self.store_tuple_fields(tuple_ty, &values, &stmt.name)?;

        self.scope_insert(
            stmt.name.clone(),
            VarSlot {
                ptr: tuple_ptr,
                ty: tuple_ty.into(),
                elem: None,
                array_len: None,
                mutable: stmt.mutable,
                box_inner: None,
                map_kind: None,
            },
        );
        self.declare_local(&stmt.name, tuple_ptr, tuple_ty.into(), span);
        Ok(())
    }

    /// 为 `tuple_ty` 分配栈空间，逐一存储各元素值（强制转换为字段类型），并返回指针。
    fn store_tuple_fields(
        &mut self,
        tuple_ty: inkwell::types::StructType<'ctx>,
        values: &[inkwell::values::BasicValueEnum<'ctx>],
        name: &str,
    ) -> Result<PointerValue<'ctx>> {
        let tmp = self.build_alloca(tuple_ty.into(), name)?;
        for (i, val) in values.iter().enumerate() {
            let val = self.coerce_value(tuple_ty.get_field_type_at_index(i as u32).unwrap(), *val)?;
            let field_ptr = self
                .builder
                .build_struct_gep(tuple_ty, tmp, i as u32, "tuple_field_ptr")
                .unwrap();
            self.builder.build_store(field_ptr, val).unwrap();
        }
        Ok(tmp)
    }

    /// 对存储在 `base_ptr` 的元组执行 GEP，获取第 `index` 个元素的指针。
    pub(super) fn gep_tuple_field(
        &self,
        base_ptr: PointerValue<'ctx>,
        tuple_ty: inkwell::types::StructType<'ctx>,
        index: usize,
    ) -> Result<(PointerValue<'ctx>, inkwell::types::BasicTypeEnum<'ctx>)> {
        if index >= tuple_ty.count_fields() as usize {
            return Err(HuziError::new_global(format!(
                "Tuple index {} out of bounds (tuple has {} element(s))",
                index,
                tuple_ty.count_fields()
            )));
        }
        let field_ptr = self
            .builder
            .build_struct_gep(tuple_ty, base_ptr, index as u32, "tuple_elem_ptr")
            .unwrap();
        let field_ty = tuple_ty.get_field_type_at_index(index as u32).unwrap();
        Ok((field_ptr, field_ty))
    }

    /// 将元组按 `(v1, v2, ...)` 格式输出：将值暂存到临时栈槽，随后通过 `format_print_value` 逐一格式化各字段。
    pub(super) fn format_tuple_value(
        &mut self,
        value: inkwell::values::BasicValueEnum<'ctx>,
        format_string: &mut String,
        args: &mut Vec<inkwell::values::BasicMetadataValueEnum<'ctx>>,
    ) -> Result<()> {
        let tuple_ty = match value.get_type() {
            inkwell::types::BasicTypeEnum::StructType(st) if self.is_tuple_type(st) => st,
            _ => return Err(HuziError::new_global("print() does not support this value type")),
        };

        let tmp = self.build_alloca(tuple_ty.into(), "tuple_print")?;
        self.builder.build_store(tmp, value).unwrap();

        format_string.push('(');
        for i in 0..tuple_ty.count_fields() {
            if i > 0 {
                format_string.push_str(", ");
            }
            let field_ptr = self
                .builder
                .build_struct_gep(tuple_ty, tmp, i, "tuple_print_field")
                .unwrap();
            let field_ty = tuple_ty.get_field_type_at_index(i).unwrap();
            let field_val = self
                .builder
                .build_load(field_ty, field_ptr, "tuple_print_val")
                .unwrap();
            self.format_print_value(field_val, format_string, args)?;
        }
        format_string.push(')');
        Ok(())
    }
}
