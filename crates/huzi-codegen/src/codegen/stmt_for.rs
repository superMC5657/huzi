use super::{CodeGen, VarSlot};
use huzi_ast::*;
use huzi_error::{HuziError, Result};


impl<'ctx> CodeGen<'ctx> {
    pub(super) fn compile_for(&mut self, stmt: &ForStmt, span: Span) -> Result<()> {
        match &stmt.source {
            ForSource::Range { .. } => self.compile_for_range(stmt, span),
            ForSource::Array(array) => self.compile_for_array(stmt, array, span),
        }
    }

    /// `for i in start..end`:整数范围循环(与此前行为一致)。
    fn compile_for_range(&mut self, stmt: &ForStmt, span: Span) -> Result<()> {
        let i_type = self.context.i32_type();
        let (start, end) = self.compile_for_bounds(stmt)?;

        let function = self.current_function()?;

        let loop_block = self.context.append_basic_block(function, "for_loop");
        let body_block = self.context.append_basic_block(function, "for_body");
        let after_block = self.context.append_basic_block(function, "for_after");

        self.loop_stack.push((loop_block, after_block));

        // Allocate and initialize the loop variable.
        let i_alloca = self.build_alloca(i_type.into(), &stmt.var_name)?;
        self.builder.build_store(i_alloca, start).unwrap();
        self.declare_local(&stmt.var_name, i_alloca, i_type.into(), span);

        self.builder
            .build_unconditional_branch(loop_block)
            .unwrap();

        self.emit_for_condition(i_type, i_alloca, end, body_block, loop_block, after_block)?;
        self.emit_for_body(stmt, i_type, i_alloca, body_block, loop_block)?;

        self.loop_stack.pop();

        // Continue after the loop.
        self.builder.position_at_end(after_block);

        Ok(())
    }

    /// Compile the range bounds; both must coerce to i32.
    fn compile_for_bounds(
        &mut self,
        stmt: &ForStmt,
    ) -> Result<(inkwell::values::IntValue<'ctx>, inkwell::values::IntValue<'ctx>)> {
        let i_type = self.context.i32_type();

        let ForSource::Range { start, end } = &stmt.source else {
            return Err(HuziError::new_global("internal: not a range for loop"));
        };

        let start = self.compile_expr(start)?;
        let start = match self.coerce_value(i_type.into(), start)? {
            inkwell::values::BasicValueEnum::IntValue(iv) => iv,
            _ => return Err(HuziError::new_global("for loop start must be an integer")),
        };

        let end = self.compile_expr(end)?;
        let end = match self.coerce_value(i_type.into(), end)? {
            inkwell::values::BasicValueEnum::IntValue(iv) => iv,
            _ => return Err(HuziError::new_global("for loop end must be an integer")),
        };

        Ok((start, end))
    }

    /// `for x in arr`:遍历长度编译期已知的数组,循环变量逐轮绑定
    /// 当前元素的值。支持数值/字符串/结构体/元组元素的数组。
    fn compile_for_array(
        &mut self,
        stmt: &ForStmt,
        array: &Expr,
        span: Span,
    ) -> Result<()> {
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

        let function = self.current_function()?;
        let i_type = self.context.i32_type();
        let len_val = i_type.const_int(len as u64, false);

        let loop_block = self.context.append_basic_block(function, "for_in_loop");
        let body_block = self.context.append_basic_block(function, "for_in_body");
        let after_block = self.context.append_basic_block(function, "for_in_after");
        self.loop_stack.push((loop_block, after_block));

        // 隐藏下标计数器 + 每轮重新绑定的元素变量。
        let idx_alloca = self.build_alloca(i_type.into(), "for_in_idx")?;
        self.builder
            .build_store(idx_alloca, i_type.const_int(0, false))
            .unwrap();
        let var_alloca = self.build_alloca(elem_type, &stmt.var_name)?;
        self.declare_local(&stmt.var_name, var_alloca, elem_type, span);

        self.builder
            .build_unconditional_branch(loop_block)
            .unwrap();

        // 循环头:idx < len(无符号比较,负数不会出现)。
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

        // 循环体:装载当前元素存入变量,执行块,递增下标。
        self.builder.position_at_end(body_block);
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
        self.builder.build_store(var_alloca, elem).unwrap();
        self.push_scope();
        self.scope_insert(
            stmt.var_name.clone(),
            VarSlot {
                ptr: var_alloca,
                ty: elem_type,
                elem: None,
                array_len: None,
                mutable: true,
            },
        );
        self.compile_block(&stmt.body)?;
        self.pop_scope();
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

        self.loop_stack.pop();
        self.builder.position_at_end(after_block);
        Ok(())
    }

    /// Emit the loop-header block that re-checks `i < end` every iteration.
    fn emit_for_condition(
        &mut self,
        i_type: inkwell::types::IntType<'ctx>,
        i_alloca: inkwell::values::PointerValue<'ctx>,
        end: inkwell::values::IntValue<'ctx>,
        body_block: inkwell::basic_block::BasicBlock<'ctx>,
        loop_block: inkwell::basic_block::BasicBlock<'ctx>,
        after_block: inkwell::basic_block::BasicBlock<'ctx>,
    ) -> Result<()> {
        self.builder.position_at_end(loop_block);
        let i = self
            .builder
            .build_load(i_type, i_alloca, "i")
            .unwrap()
            .into_int_value();
        let condition = self
            .builder
            .build_int_compare(inkwell::IntPredicate::SLT, i, end, "loop_cond")
            .unwrap();
        self.builder
            .build_conditional_branch(condition, body_block, after_block)
            .unwrap();
        Ok(())
    }

    /// Emit the loop-body block: bind the loop variable in a fresh scope,
    /// run the body, then increment `i` and jump back to the header.
    fn emit_for_body(
        &mut self,
        stmt: &ForStmt,
        i_type: inkwell::types::IntType<'ctx>,
        i_alloca: inkwell::values::PointerValue<'ctx>,
        body_block: inkwell::basic_block::BasicBlock<'ctx>,
        loop_block: inkwell::basic_block::BasicBlock<'ctx>,
    ) -> Result<()> {
        self.builder.position_at_end(body_block);
        self.push_scope();
        self.scope_insert(
            stmt.var_name.clone(),
            VarSlot {
                ptr: i_alloca,
                ty: i_type.into(),
                elem: None,
                array_len: None,
                mutable: true,
            },
        );
        self.compile_block(&stmt.body)?;
        self.pop_scope();

        // Increment the loop variable before jumping back to the condition.
        let i = self
            .builder
            .build_load(i_type, i_alloca, "i")
            .unwrap()
            .into_int_value();
        let i_next = self
            .builder
            .build_int_add(i, i_type.const_int(1, false), "i_next")
            .unwrap();
        self.builder.build_store(i_alloca, i_next).unwrap();
        self.builder
            .build_unconditional_branch(loop_block)
            .unwrap();
        Ok(())
    }
}
