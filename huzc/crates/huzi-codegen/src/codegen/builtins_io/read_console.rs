//! 控制台输入:`read_line` / `read_int` / `read_float`。
//! (自 `builtins_io.rs` 纯搬移,零逻辑变化。)

use super::super::CodeGen;
use huzi_error::Result;
use inkwell::values::PointerValue;

impl<'ctx> CodeGen<'ctx> {
    pub(in crate::codegen) fn compile_read_line(&mut self) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        let getchar_fn = self.module.get_function("getchar").unwrap();

        // 分配缓冲区（256 字节）
        let buffer = self.alloc_str_buffer(256)?;

        let i32_type = self.context.i32_type();
        let idx_ptr = self.build_alloca(i32_type.into(), "read_idx")?;
        self.builder
            .build_store(idx_ptr, i32_type.const_int(0, false))
            .unwrap();

        let function = self.current_function()?;
        let loop_block = self.context.append_basic_block(function, "read_loop");
        let store_block = self.context.append_basic_block(function, "read_store");
        let done_block = self.context.append_basic_block(function, "read_done");

        self.builder
            .build_unconditional_branch(loop_block)
            .unwrap();

        // 每次循环读取一个字符，直到遇到 '\n'、EOF 或缓冲区满。
        self.builder.position_at_end(loop_block);
        let c = self
            .builder
            .build_call(getchar_fn, &[], "ch")
            .unwrap()
            .try_as_basic_value()
            .unwrap_left()
            .into_int_value();

        // 记录 EOF 供 is_eof() 查询：输入结束时 getchar 返回 -1。
        let eof_hit = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                c,
                i32_type.const_int(-1i64 as u64, true),
                "eof_hit",
            )
            .unwrap();
        self.mark_eof_flag(eof_hit);

        let idx = self
            .builder
            .build_load(i32_type, idx_ptr, "idx")
            .unwrap()
            .into_int_value();

        let cont = self.read_line_continue(c, idx, i32_type)?;

        self.builder
            .build_conditional_branch(cont, store_block, done_block)
            .unwrap();

        self.read_line_store(buffer, idx_ptr, idx, c, i32_type, store_block, loop_block)?;

        // 写入 NUL 终止符并在 done 块中继续。
        self.builder.position_at_end(done_block);
        let term_ptr = unsafe {
            self.builder
                .build_gep(self.context.i8_type(), buffer, &[idx], "term_ptr")
                .unwrap()
        };
        self.builder
            .build_store(term_ptr, self.context.i8_type().const_int(0, false))
            .unwrap();

        Ok(buffer.into())
    }

    /// 读取循环是否应继续：缓冲区仍有空间、当前字符非换行符且非 EOF。
    fn read_line_continue(
        &mut self,
        c: inkwell::values::IntValue<'ctx>,
        idx: inkwell::values::IntValue<'ctx>,
        i32_type: inkwell::types::IntType<'ctx>,
    ) -> Result<inkwell::values::IntValue<'ctx>> {
        let has_space = self
            .builder
            .build_int_compare(inkwell::IntPredicate::SLT, idx, i32_type.const_int(255, false), "has_space")
            .unwrap();
                let not_nl = self
            .builder
            .build_int_compare(inkwell::IntPredicate::NE, c, i32_type.const_int('\n' as u64, false), "not_nl")
            .unwrap();
        let not_eof = self
            .builder
            .build_int_compare(inkwell::IntPredicate::NE, c, i32_type.const_int(-1i64 as u64, true), "not_eof")
            .unwrap();
        let cont = self.builder.build_and(has_space, not_nl, "cont").unwrap();
        let cont = self.builder.build_and(cont, not_eof, "cont2").unwrap();
        Ok(cont)
    }

    /// 发射 store 块：将字符截断为 i8，写入当前索引处，索引递增并跳回循环头。
    fn read_line_store(
        &mut self,
        buffer: PointerValue<'ctx>,
        idx_ptr: PointerValue<'ctx>,
        idx: inkwell::values::IntValue<'ctx>,
        c: inkwell::values::IntValue<'ctx>,
        i32_type: inkwell::types::IntType<'ctx>,
        store_block: inkwell::basic_block::BasicBlock<'ctx>,
        loop_block: inkwell::basic_block::BasicBlock<'ctx>,
    ) -> Result<()> {
        self.builder.position_at_end(store_block);
        let c8 = self
            .builder
            .build_int_truncate(c, self.context.i8_type(), "ch_i8")
            .unwrap();
        let ch_ptr = unsafe {
            self.builder
                .build_gep(self.context.i8_type(), buffer, &[idx], "ch_ptr")
                .unwrap()
        };
        self.builder.build_store(ch_ptr, c8).unwrap();
        let idx_next = self
            .builder
            .build_int_add(idx, i32_type.const_int(1, false), "idx_next")
            .unwrap();
        self.builder.build_store(idx_ptr, idx_next).unwrap();
        self.builder
            .build_unconditional_branch(loop_block)
            .unwrap();
        Ok(())
    }

    pub(in crate::codegen) fn compile_read_int(&mut self) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        let scanf_fn = self.module.get_function("scanf").unwrap();

        // %d 的格式化字符串
        let format_str = unsafe {
            self.builder
                .build_global_string("%d", "scanf_format_int")
                .unwrap()
        };

        // 为 int 分配空间
        let int_ptr = self.build_alloca(self.context.i32_type().into(), "int_input")?;

        let scanf_ret = self
            .builder
            .build_call(
                scanf_fn,
                &[
                    format_str.as_pointer_value().into(),
                    int_ptr.into(),
                ],
                "scanf_int",
            )
            .unwrap()
            .try_as_basic_value()
            .unwrap_left()
            .into_int_value();

        // 记录 EOF 供 is_eof() 查询：输入结束时 scanf 返回 -1。
        let eof_hit = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                scanf_ret,
                self.context.i32_type().const_int(-1i64 as u64, true),
                "eof_hit",
            )
            .unwrap();
        self.mark_eof_flag(eof_hit);

        let value = self
            .builder
            .build_load(self.context.i32_type(), int_ptr, "int_value")
            .unwrap();

        Ok(value)
    }

    pub(in crate::codegen) fn compile_read_float(&mut self) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        let scanf_fn = self.module.get_function("scanf").unwrap();

        // %lf 的格式化字符串
        let format_str = unsafe {
            self.builder
                .build_global_string("%lf", "scanf_format_float")
                .unwrap()
        };

        // 为 double 分配空间
        let float_ptr = self.build_alloca(self.context.f64_type().into(), "float_input")?;

        let scanf_ret = self
            .builder
            .build_call(
                scanf_fn,
                &[
                    format_str.as_pointer_value().into(),
                    float_ptr.into(),
                ],
                "scanf_float",
            )
            .unwrap()
            .try_as_basic_value()
            .unwrap_left()
            .into_int_value();

        // 记录 EOF 供 is_eof() 查询：输入结束时 scanf 返回 -1。
        let eof_hit = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                scanf_ret,
                self.context.i32_type().const_int(-1i64 as u64, true),
                "eof_hit",
            )
            .unwrap();
        self.mark_eof_flag(eof_hit);

        let value = self
            .builder
            .build_load(self.context.f64_type(), float_ptr, "float_value")
            .unwrap();

        Ok(value)
    }
}
