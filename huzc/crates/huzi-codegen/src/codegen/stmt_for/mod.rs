//! `for` 语句编译:range 循环与 for-in 遍历的分派与公共助手。
//! 数组/vec/字符串遍历的具体生成在子模块 `iter` 中。

mod iter;

use super::{CodeGen, VarSlot};
use huzi_ast::*;
use huzi_error::{HuziError, Result};
use inkwell::values::PointerValue;


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

        // 分配并初始化循环变量。
        let i_alloca = self.build_alloca(i_type.into(), &stmt.var_name)?;
        self.builder.build_store(i_alloca, start).unwrap();
        self.declare_local(&stmt.var_name, i_alloca, i_type.into(), span);

        self.builder
            .build_unconditional_branch(loop_block)
            .unwrap();

        self.emit_for_condition(i_type, i_alloca, end, body_block, loop_block, after_block)?;
        self.emit_for_body(stmt, i_type, i_alloca, body_block, loop_block)?;

        self.loop_stack.pop();

        // 循环结束后继续执行后续指令。
        self.builder.position_at_end(after_block);

        Ok(())
    }

    /// 编译范围边界；两者均必须能强制转换为 i32。
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

    /// for-in 单轮绑定:写入循环变量,入新作用域执行循环体后退出。
    pub(super) fn execute_for_in_body(
        &mut self,
        stmt: &ForStmt,
        var_alloca: PointerValue<'ctx>,
        elem: inkwell::values::BasicValueEnum<'ctx>,
        elem_type: inkwell::types::BasicTypeEnum<'ctx>,
    ) -> Result<()> {
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
                box_inner: None,
                map_kind: None,
            },
        );
        self.compile_block(&stmt.body)?;
        self.pop_scope();
        Ok(())
    }

    /// 生成循环头 BasicBlock，在每次迭代时重新检查 `i < end`。
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

    /// 生成循环体 BasicBlock：在全新作用域中绑定循环变量，
    /// 执行循环体，随后递增 `i` 并跳回循环头。
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
                box_inner: None,
                map_kind: None,
            },
        );
        self.compile_block(&stmt.body)?;
        self.pop_scope();

        // 在跳回循环条件判断之前递增循环变量。
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
