//! 数组下标/字面量与 `if` 表达式值(自 `aggregates.rs` 纯搬移,零逻辑变化)。

use super::super::CodeGen;
use huzi_ast::*;
use huzi_error::{HuziError, Result};
use inkwell::types::BasicType;

impl<'ctx> CodeGen<'ctx> {
    fn compile_field_vec_index(
        &mut self,
        expr: &huzi_ast::ArrayIndexExpr,
        fa: &huzi_ast::FieldAccessExpr,
    ) -> Result<Option<inkwell::values::BasicValueEnum<'ctx>>> {
        let Some(field_ty) = self.field_ast_type(&fa.base, &fa.field) else {
            return Ok(None);
        };
        if !matches!(field_ty, Type::Applied(ref n, _) if n == "vec") {
            return Ok(None);
        }
        let elem_llvm_ty = self.elem_and_mark_from_ast(&field_ty)?.0.unwrap();
        let vec_val = self.compile_expr(&expr.array)?;
        if !vec_val.is_struct_value() {
            return Ok(None);
        }
        let sv = vec_val.into_struct_value();
        let data = self.builder.build_extract_value(sv, 0, "vec_data").unwrap().into_pointer_value();
        let len = self.builder.build_extract_value(sv, 1, "vec_len").unwrap().into_int_value();
        let index_val = self.compile_expr(&expr.index)?;
        let index_i32 = self.coerce_index(index_val)?;
        self.emit_vec_bounds_check(len, index_i32)?;
        let elem_ptr = unsafe {
            self.builder.build_gep(elem_llvm_ty, data, &[index_i32], "vec_elem_ptr").unwrap()
        };
        Ok(Some(self.builder.build_load(elem_llvm_ty, elem_ptr, "vec_elem").unwrap()))
    }

    pub(in crate::codegen) fn compile_array_index(
        &mut self,
        expr: &huzi_ast::ArrayIndexExpr,
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        // vec 下标读走动态长度路径。
        if let huzi_ast::Expr::Ident(name) = &*expr.array {
            if let Some(slot) = self.scope_lookup(name) {
                if Self::is_vec_slot(&slot) {
                    return self.compile_vec_index_load(name, &expr.index);
                }
            }
        }
        if let huzi_ast::Expr::FieldAccess(fa) = &*expr.array {
            if let Some(val) = self.compile_field_vec_index(expr, fa)? {
                return Ok(val);
            }
        }
        let array_ptr = self.compile_expr(&expr.array)?;
        let array_ptr_val = if array_ptr.is_pointer_value() {
            array_ptr.into_pointer_value()
        } else {
            return Err(HuziError::new_global("Indexed value is not an array"));
        };

        let elem_type = self.resolve_elem_type(&expr.array, None)?;

        let index_val = self.compile_expr(&expr.index)?;
        let index_i32 = self.coerce_index(index_val)?;

        if self.is_string_index(&expr.array, elem_type)? {
            self.emit_str_bounds_check(array_ptr_val, index_i32)?;
        } else {
            self.emit_bounds_check(&expr.array, index_i32)?;
        }

        // 构建 GEP 以获取元素指针
        let elem_ptr = unsafe {
            self.builder
                .build_gep(elem_type, array_ptr_val, &[index_i32], "elem_ptr")
                .unwrap()
        };

        // 加载元素值
        let loaded = self
            .builder
            .build_load(elem_type, elem_ptr, "load_elem")
            .unwrap();

        Ok(loaded)
    }

    pub(in crate::codegen) fn compile_array_literal(
        &mut self,
        elements: &[Expr],
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        let (ptr, _) = self.compile_array_literal_typed(elements)?;
        Ok(ptr.into())
    }

    /// 数组字面量(含元素类型):编译各元素、栈上分配定长数组并逐一存入,
    /// 返回(数组指针, 元素类型)。供表达式值与 `let` 存槽复用,避免双重编译。
    pub(in crate::codegen) fn compile_array_literal_typed(
        &mut self,
        elements: &[Expr],
    ) -> Result<(inkwell::values::PointerValue<'ctx>, inkwell::types::BasicTypeEnum<'ctx>)> {
        if elements.is_empty() {
            return Err(HuziError::new_global("Empty array literal not supported"));
        }

        // 编译所有元素
        let mut elem_values = Vec::new();
        for elem in elements {
            let val = self.compile_expr(elem)?;
            elem_values.push(val);
        }

        // 从首个元素获取元素类型
        let elem_type = elem_values[0].get_type();

        // 创建数组类型
        let array_type = elem_type.array_type(elements.len() as u32);

        // 为数组分配栈空间
        let array_ptr = self.build_alloca(array_type.into(), "array")?;

        // 逐一存储每个元素
        for (i, val) in elem_values.iter().enumerate() {
            let val = self.coerce_value(elem_type, *val)?;
            let index = self.context.i32_type().const_int(i as u64, false);
            let elem_ptr = unsafe {
                self.builder
                    .build_gep(elem_type, array_ptr, &[index], "elem_ptr")
                    .unwrap()
            };
            self.builder.build_store(elem_ptr, val).unwrap();
        }

        Ok((array_ptr, elem_type))
    }

    pub(in crate::codegen) fn compile_if_expr(
        &mut self,
        expr: &IfExpr,
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        let cond_value = self.compile_expr(&expr.condition)?;
        let cond = self.to_i1(cond_value)?;

        let function = self.current_function()?;
        let then_block = self.context.append_basic_block(function, "ifex_then");
        let else_block = self.context.append_basic_block(function, "ifex_else");
        let merge_block = self.context.append_basic_block(function, "ifex_merge");

        self.builder
            .build_conditional_branch(cond, then_block, else_block)
            .unwrap();

        self.builder.position_at_end(then_block);
        let then_val = self.compile_block_value(&expr.then_branch)?;
        let result_type = then_val.get_type();
        let result_ptr = self.build_alloca(result_type, "ifex_val")?;
        self.builder.build_store(result_ptr, then_val).unwrap();
        self.builder
            .build_unconditional_branch(merge_block)
            .unwrap();

        self.builder.position_at_end(else_block);
        let else_val = self.compile_block_value(&expr.else_branch)?;
        let else_val = self.coerce_value(result_type, else_val)?;
        self.builder.build_store(result_ptr, else_val).unwrap();
        self.builder
            .build_unconditional_branch(merge_block)
            .unwrap();

        self.builder.position_at_end(merge_block);
        let result = self
            .builder
            .build_load(result_type, result_ptr, "ifex_load")
            .unwrap();

        Ok(result)
    }
}
