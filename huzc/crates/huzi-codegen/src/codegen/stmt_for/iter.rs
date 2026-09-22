//! for-in 遍历的代码生成:长度已知的数组、vec 结构体(变量/字段/调用
//! 结果)与字符串的逐元素循环。`mod::fn(...)` 限定调用与 `Enum::Variant`
//! 同形,按 codegen 调用分派一致的方式识别返回 `vec<T>` 的调用。

use super::CodeGen;
use crate::codegen::qname;
use huzi_ast::*;
use huzi_error::{HuziError, Result};
use inkwell::types::BasicTypeEnum;
use inkwell::values::{IntValue, PointerValue};

impl<'ctx> CodeGen<'ctx> {
    fn try_compile_for_vec(
        &mut self,
        stmt: &ForStmt,
        array: &Expr,
        span: Span,
    ) -> Result<bool> {
        if let Expr::Ident(name) = array {
            if let Some(slot) = self.scope_lookup(name) {
                if Self::is_vec_slot(&slot) {
                    self.compile_for_vec(stmt, name, span)?;
                    return Ok(true);
                }
            }
        }
        if let Expr::FieldAccess(fa) = array {
            if let Some(field_ty) = self.field_ast_type(&fa.base, &fa.field) {
                if matches!(field_ty, Type::Applied(ref n, _) if n == "vec") {
                    let elem_type = self.elem_and_mark_from_ast(&field_ty)?.0.unwrap();
                    let vec_val = self.compile_expr(array)?;
                    if vec_val.is_struct_value() {
                        let sv = vec_val.into_struct_value();
                        let data = self
                            .builder
                            .build_extract_value(sv, 0, "vec_data")
                            .unwrap()
                            .into_pointer_value();
                        let len = self
                            .builder
                            .build_extract_value(sv, 1, "vec_len")
                            .unwrap()
                            .into_int_value();
                        self.compile_for_vec_parts(stmt, data, len, elem_type, span)?;
                        return Ok(true);
                    }
                }
            }
        }
        if self.try_compile_for_call(stmt, array, span)? {
            return Ok(true);
        }
        Ok(false)
    }

    /// `for x in 调用(...)`:直接遍历返回 `vec<T>` 的函数调用结果,
    /// 无需先 `let` 暂存。元素类型取自被调函数的声明返回类型
    /// (`fn_return_ast`);内置 `split` 恒为 `vec<str>`。其余调用返回
    /// false,交回原错误路径。
    fn try_compile_for_call(
        &mut self,
        stmt: &ForStmt,
        array: &Expr,
        span: Span,
    ) -> Result<bool> {
        let callee_name = match array {
            Expr::Call(c) => match &*c.callee {
                Expr::Ident(n) => Some(n.clone()),
                _ => None,
            },
            // `mod::fn(args)` 与 `Enum::Variant(args)` 同形(解析为
            // EnumConstruct),模块前缀拼接与 codegen 调用分派一致。
            Expr::EnumConstruct(ec) => Some(qname::qualified(&ec.enum_name, &ec.variant)),
            _ => None,
        };
        let Some(callee_name) = callee_name else {
            return Ok(false);
        };

        let elem_type = if callee_name == "split" {
            Some(
                self.context
                    .ptr_type(inkwell::AddressSpace::default())
                    .into(),
            )
        } else if let Some(ret) = self.fn_return_ast.get(&callee_name) {
            match ret {
                Type::Applied(n, args) if n == "vec" => {
                    let Some(arg) = args.first() else {
                        return Ok(false);
                    };
                    match self.type_to_llvm(arg) {
                        Ok(ty) => Some(ty),
                        Err(_) => None,
                    }
                }
                _ => None,
            }
        } else {
            None
        };
        let Some(elem_type) = elem_type else {
            return Ok(false);
        };

        let vec_val = self.compile_expr(array)?;
        if !vec_val.is_struct_value() {
            return Ok(false);
        }
        let sv = vec_val.into_struct_value();
        let data = self
            .builder
            .build_extract_value(sv, 0, "vec_call_data")
            .unwrap()
            .into_pointer_value();
        let len = self
            .builder
            .build_extract_value(sv, 1, "vec_call_len")
            .unwrap()
            .into_int_value();
        self.compile_for_vec_parts(stmt, data, len, elem_type, span)?;
        Ok(true)
    }

    fn compile_for_array_iter(
        &mut self,
        stmt: &ForStmt,
        arr_ptr: PointerValue<'ctx>,
        elem_type: BasicTypeEnum<'ctx>,
        idx_alloca: PointerValue<'ctx>,
        var_alloca: PointerValue<'ctx>,
        loop_block: inkwell::basic_block::BasicBlock<'ctx>,
    ) -> Result<()> {
        let i_type = self.context.i32_type();
        let idx = self
            .builder
            .build_load(i_type, idx_alloca, "for_in_i")
            .unwrap()
            .into_int_value();
        let elem_ptr = unsafe {
            self.builder
                .build_gep(elem_type, arr_ptr, &[idx], "for_in_elem_ptr")
                .unwrap()
        };
        let elem = self
            .builder
            .build_load(elem_type, elem_ptr, "for_in_elem")
            .unwrap();
        self.execute_for_in_body(stmt, var_alloca, elem, elem_type)?;
        let idx = self
            .builder
            .build_load(i_type, idx_alloca, "for_in_i")
            .unwrap()
            .into_int_value();
        let next = self
            .builder
            .build_int_add(idx, i_type.const_int(1, false), "for_in_next")
            .unwrap();
        self.builder.build_store(idx_alloca, next).unwrap();
        self.builder
            .build_unconditional_branch(loop_block)
            .unwrap();
        Ok(())
    }

    fn compile_for_array_loop(
        &mut self,
        stmt: &ForStmt,
        arr_ptr: PointerValue<'ctx>,
        len: u32,
        elem_type: BasicTypeEnum<'ctx>,
        span: Span,
    ) -> Result<()> {
        let function = self.current_function()?;
        let i_type = self.context.i32_type();
        let len_val = i_type.const_int(len as u64, false);

        let loop_block = self.context.append_basic_block(function, "for_in_loop");
        let body_block = self.context.append_basic_block(function, "for_in_body");
        let after_block = self.context.append_basic_block(function, "for_in_after");
        self.loop_stack.push((loop_block, after_block));

        let idx_alloca = self.build_alloca(i_type.into(), "for_in_idx")?;
        self.builder
            .build_store(idx_alloca, i_type.const_int(0, false))
            .unwrap();
        let var_alloca = self.build_alloca(elem_type, &stmt.var_name)?;
        self.declare_local(&stmt.var_name, var_alloca, elem_type, span);

        self.builder
            .build_unconditional_branch(loop_block)
            .unwrap();

        self.builder.position_at_end(loop_block);
        let idx = self
            .builder
            .build_load(i_type, idx_alloca, "for_in_i")
            .unwrap()
            .into_int_value();
        let cond = self
            .builder
            .build_int_compare(inkwell::IntPredicate::ULT, idx, len_val, "for_in_cond")
            .unwrap();
        self.builder
            .build_conditional_branch(cond, body_block, after_block)
            .unwrap();

        self.builder.position_at_end(body_block);
        self.compile_for_array_iter(
            stmt,
            arr_ptr,
            elem_type,
            idx_alloca,
            var_alloca,
            loop_block,
        )?;

        self.loop_stack.pop();
        self.builder.position_at_end(after_block);
        Ok(())
    }

    /// `for x in arr`:遍历长度编译期已知的数组,循环变量逐轮绑定
    /// 当前元素的值。支持数值/字符串/结构体/元组元素的数组。
    pub(super) fn compile_for_array(
        &mut self,
        stmt: &ForStmt,
        array: &Expr,
        span: Span,
    ) -> Result<()> {
        if self.try_compile_for_vec(stmt, array, span)? {
            return Ok(());
        }
        let arr_value = self.compile_expr(array)?;
        let arr_ptr = if arr_value.is_pointer_value() {
            arr_value.into_pointer_value()
        } else {
            return Err(HuziError::new_global("for-in requires an array to iterate"));
        };
        let elem_type = self.resolve_elem_type(array, None)?;
        let Some(len) = self.resolve_array_len(array)? else {
            return Err(HuziError::new_global(
                "for-in requires an array with a known length (a variable or struct field)",
            ));
        };
        self.compile_for_array_loop(stmt, arr_ptr, len, elem_type, span)
    }

    /// `for x in v`:循环变量逐轮绑定当前元素值;进入前一次性读取长度。
    fn compile_for_vec(
        &mut self,
        stmt: &ForStmt,
        name: &str,
        span: Span,
    ) -> Result<()> {
        let slot = self.vec_slot_of(name)?;
        let elem_type = slot.elem.unwrap();
        let parts = self.load_vec_parts(&slot)?;
        self.compile_for_vec_parts(stmt, parts.data, parts.len, elem_type, span)
    }

    pub(super) fn compile_for_vec_parts(
        &mut self,
        stmt: &ForStmt,
        data: PointerValue<'ctx>,
        len: IntValue<'ctx>,
        elem_type: BasicTypeEnum<'ctx>,
        span: Span,
    ) -> Result<()> {
        let function = self.current_function()?;
        let i_type = self.context.i32_type();
        let loop_block = self.context.append_basic_block(function, "vec_for_loop");
        let body_block = self.context.append_basic_block(function, "vec_for_body");
        let after_block = self.context.append_basic_block(function, "vec_for_after");
        self.loop_stack.push((loop_block, after_block));

        let idx_alloca = self.build_alloca(i_type.into(), "vec_for_idx")?;
        self.builder
            .build_store(idx_alloca, i_type.const_int(0, false))
            .unwrap();
        let var_alloca = self.build_alloca(elem_type, &stmt.var_name)?;
        self.declare_local(&stmt.var_name, var_alloca, elem_type, span);

        self.builder
            .build_unconditional_branch(loop_block)
            .unwrap();

        self.builder.position_at_end(loop_block);
        let idx = self
            .builder
            .build_load(i_type, idx_alloca, "vec_for_i")
            .unwrap()
            .into_int_value();
        let cond = self
            .builder
            .build_int_compare(inkwell::IntPredicate::ULT, idx, len, "vec_for_cond")
            .unwrap();
        self.builder
            .build_conditional_branch(cond, body_block, after_block)
            .unwrap();

        self.builder.position_at_end(body_block);
        self.compile_for_vec_iter(stmt, data, elem_type, idx_alloca, var_alloca, loop_block)?;

        self.loop_stack.pop();
        self.builder.position_at_end(after_block);
        Ok(())
    }

    fn compile_for_vec_iter(
        &mut self,
        stmt: &ForStmt,
        data: PointerValue<'ctx>,
        elem_type: BasicTypeEnum<'ctx>,
        idx_alloca: PointerValue<'ctx>,
        var_alloca: PointerValue<'ctx>,
        loop_block: inkwell::basic_block::BasicBlock<'ctx>,
    ) -> Result<()> {
        let i_type = self.context.i32_type();
        let idx = self
            .builder
            .build_load(i_type, idx_alloca, "vec_for_i")
            .unwrap()
            .into_int_value();
        let elem_ptr = unsafe {
            self.builder
                .build_gep(elem_type, data, &[idx], "vec_for_elem_ptr")
                .unwrap()
        };
        let elem = self
            .builder
            .build_load(elem_type, elem_ptr, "vec_for_elem")
            .unwrap();
        self.execute_for_in_body(stmt, var_alloca, elem, elem_type)?;
        let idx = self
            .builder
            .build_load(i_type, idx_alloca, "vec_for_i")
            .unwrap()
            .into_int_value();
        let next = self
            .builder
            .build_int_add(idx, i_type.const_int(1, false), "vec_for_next")
            .unwrap();
        self.builder.build_store(idx_alloca, next).unwrap();
        self.builder
            .build_unconditional_branch(loop_block)
            .unwrap();
        Ok(())
    }
}
