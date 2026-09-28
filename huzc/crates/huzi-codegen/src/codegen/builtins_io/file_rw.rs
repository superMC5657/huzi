//! 文件读写分支发射:`read_file` / `write_file` 的成功/失败分支与公共调用辅助。
//! (新建子函数自 `builtins_io.rs` 提炼,分支体逐行原样搬移,零逻辑变化。)

use super::super::CodeGen;
use inkwell::values::{FunctionValue, PointerValue};

impl<'ctx> CodeGen<'ctx> {
    /// `read_file` 失败分支:打开失败时返回空串并跳到收尾块。
    pub(super) fn emit_read_file_fail(
        &mut self,
        fail_block: inkwell::basic_block::BasicBlock<'ctx>,
        result_ptr: PointerValue<'ctx>,
        end_block: inkwell::basic_block::BasicBlock<'ctx>,
    ) {
        // 打开失败:返回共享空串(复用 `empty_str_ptr`)。
        self.builder.position_at_end(fail_block);
        let empty = self.empty_str_ptr();
        self.builder
            .build_store(result_ptr, empty)
            .unwrap();
        self.builder
            .build_unconditional_branch(end_block)
            .unwrap();
    }

    /// `read_file` 成功分支:定位文件尾取长度,回到开头整块读入并补 `\0`,
    /// 关闭文件后把堆串存入结果槽并跳到收尾块。
    pub(super) fn emit_read_file_ok(
        &mut self,
        ok_block: inkwell::basic_block::BasicBlock<'ctx>,
        file: PointerValue<'ctx>,
        fseek_fn: FunctionValue<'ctx>,
        ftell_fn: FunctionValue<'ctx>,
        fread_fn: FunctionValue<'ctx>,
        fclose_fn: FunctionValue<'ctx>,
        malloc_fn: FunctionValue<'ctx>,
        result_ptr: PointerValue<'ctx>,
        end_block: inkwell::basic_block::BasicBlock<'ctx>,
    ) {
        // 打开成功:定位文件尾取长度,回到开头整块读入,补 \0。
        self.builder.position_at_end(ok_block);
        let zero32 = self.context.i32_type().const_int(0, false);
        let seek_end = self.context.i32_type().const_int(2, false);
        self.emit_void_call(fseek_fn, &[
            file.into(),
            self.context.i64_type().const_int(0, false).into(),
            seek_end.into(),
        ], "rf_seek_end");
        let size = self
            .builder
            .build_call(ftell_fn, &[file.into()], "rf_size")
            .unwrap()
            .try_as_basic_value()
            .unwrap_left()
            .into_int_value();
        self.emit_void_call(fseek_fn, &[
            file.into(),
            self.context.i64_type().const_int(0, false).into(),
            zero32.into(),
        ], "rf_seek_set");

        let size_plus_one = self
            .builder
            .build_int_add(size, self.context.i32_type().const_int(1, false), "rf_buf_len")
            .unwrap();
        let buf = self
            .builder
            .build_call(malloc_fn, &[size_plus_one.into()], "rf_buf")
            .unwrap()
            .try_as_basic_value()
            .unwrap_left()
            .into_pointer_value();
        let size64 = self
            .builder
            .build_int_s_extend(size, self.context.i64_type(), "rf_size64")
            .unwrap();
        let one64 = self.context.i64_type().const_int(1, false);
        let n = self
            .builder
            .build_call(fread_fn, &[buf.into(), one64.into(), size64.into(), file.into()], "rf_n")
            .unwrap()
            .try_as_basic_value()
            .unwrap_left()
            .into_int_value();
        let term = unsafe {
            self.builder
                .build_gep(self.context.i8_type(), buf, &[n], "rf_term")
                .unwrap()
        };
        self.builder
            .build_store(term, self.context.i8_type().const_int(0, false))
            .unwrap();
        self.emit_void_call(fclose_fn, &[file.into()], "rf_close");
        self.builder
            .build_store(result_ptr, buf)
            .unwrap();
        self.builder
            .build_unconditional_branch(end_block)
            .unwrap();
    }

    /// `write_file` 失败分支:打开失败时返回 false 并跳到收尾块。
    pub(super) fn emit_write_file_fail(
        &mut self,
        fail_block: inkwell::basic_block::BasicBlock<'ctx>,
        result_ptr: PointerValue<'ctx>,
        end_block: inkwell::basic_block::BasicBlock<'ctx>,
    ) {
        // 打开失败:返回 false。
        self.builder.position_at_end(fail_block);
        self.builder
            .build_store(result_ptr, self.context.bool_type().const_int(0, false))
            .unwrap();
        self.builder
            .build_unconditional_branch(end_block)
            .unwrap();
    }

    /// `write_file` 成功分支:全量写入内容,按写入字节数判定成败后存入结果槽
    /// 并跳到收尾块。
    pub(super) fn emit_write_file_ok(
        &mut self,
        ok_block: inkwell::basic_block::BasicBlock<'ctx>,
        file: PointerValue<'ctx>,
        content: PointerValue<'ctx>,
        fwrite_fn: FunctionValue<'ctx>,
        fclose_fn: FunctionValue<'ctx>,
        strlen_fn: FunctionValue<'ctx>,
        result_ptr: PointerValue<'ctx>,
        end_block: inkwell::basic_block::BasicBlock<'ctx>,
    ) {
        // 打开成功:全量写入,按写入字节数判定成败。
        self.builder.position_at_end(ok_block);
        let one64 = self.context.i64_type().const_int(1, false);
        let len = self
            .builder
            .build_call(strlen_fn, &[content.into()], "wf_len")
            .unwrap()
            .try_as_basic_value()
            .unwrap_left()
            .into_int_value();
        let len64 = self
            .builder
            .build_int_z_extend(len, self.context.i64_type(), "wf_len64")
            .unwrap();
        let written = self
            .builder
            .build_call(fwrite_fn, &[content.into(), one64.into(), len64.into(), file.into()], "wf_n")
            .unwrap()
            .try_as_basic_value()
            .unwrap_left()
            .into_int_value();
        let all_written = self
            .builder
            .build_int_compare(inkwell::IntPredicate::EQ, written, len64, "wf_all")
            .unwrap();
        self.emit_void_call(fclose_fn, &[file.into()], "wf_close");
        self.builder
            .build_store(result_ptr, all_written)
            .unwrap();
        self.builder
            .build_unconditional_branch(end_block)
            .unwrap();
    }

    /// 生成一个不使用返回值的调用(void 函数)。
    fn emit_void_call(
        &mut self,
        function: inkwell::values::FunctionValue<'ctx>,
        args: &[inkwell::values::BasicMetadataValueEnum<'ctx>],
        name: &str,
    ) {
        self.builder.build_call(function, args, name).unwrap();
    }
}
