//! 环境与子进程:`env_get`/`localtime`/`process_run` 及管道读取。(自 `builtins_sys.rs` 纯搬移,零逻辑变化)。

use super::super::CodeGen;
use huzi_ast::Expr;
use huzi_error::{HuziError, Result};
use inkwell::AddressSpace;
use inkwell::basic_block::BasicBlock;
use inkwell::types::StructType;
use inkwell::values::{BasicValueEnum, FunctionValue, IntValue, PointerValue};

impl<'ctx> CodeGen<'ctx> {
    /// `env_get(k)`: 读取环境变量,返回 `(bool, str)`。缺键返回 `(false, "")`。
    pub(in crate::codegen) fn compile_env_get(&mut self, arguments: &[Expr]) -> Result<BasicValueEnum<'ctx>> {
        if arguments.len() != 1 {
            return Err(HuziError::new_global("env_get() requires exactly 1 argument (key)"));
        }
        let key = match self.compile_expr(&arguments[0])? {
            BasicValueEnum::PointerValue(p) => p,
            _ => return Err(HuziError::new_global("env_get() argument must be a string")),
        };

        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let bool_t = self.context.bool_type();
        let getenv_fn = self.module.get_function("getenv").unwrap();

        let raw = self
            .builder
            .build_call(getenv_fn, &[key.into()], "getenv_call")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_pointer_value();

        let is_null = self.builder.build_is_null(raw, "env_is_null").unwrap();
        let found = self.builder.build_not(is_null, "env_found").unwrap();

        let empty_str = self.builder.build_global_string_ptr("", "empty_str").unwrap().as_pointer_value();
        let val_ptr = self.builder.build_select(found, raw, empty_str, "env_str").unwrap().into_pointer_value();

        let tup_ty = self.context.struct_type(&[bool_t.into(), ptr_t.into()], false);
        let tup_alloca = self.build_alloca(tup_ty.into(), "env_get_tup")?;
        let f0 = self.builder.build_struct_gep(tup_ty, tup_alloca, 0, "env_f0").unwrap();
        self.builder.build_store(f0, found).unwrap();
        let f1 = self.builder.build_struct_gep(tup_ty, tup_alloca, 1, "env_f1").unwrap();
        self.builder.build_store(f1, val_ptr).unwrap();

        Ok(self.builder.build_load(tup_ty, tup_alloca, "env_get_res").unwrap())
    }

    /// `localtime(ts: i64) -> str`: 格式化时间戳为 `YYYY-MM-DD hh:mm:ss`。
    pub(in crate::codegen) fn compile_localtime(&mut self, arguments: &[Expr]) -> Result<BasicValueEnum<'ctx>> {
        if arguments.len() != 1 {
            return Err(HuziError::new_global("localtime() requires exactly 1 argument (timestamp)"));
        }
        let ts_val = self.compile_expr(&arguments[0])?;
        let i64_t = self.context.i64_type();
        let ts_i64 = match self.coerce_value(i64_t.into(), ts_val)? {
            BasicValueEnum::IntValue(iv) => iv,
            _ => return Err(HuziError::new_global("localtime() argument must be an i64 timestamp")),
        };

        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let ts_alloca = self.build_alloca(i64_t.into(), "localtime_ts")?;
        self.builder.build_store(ts_alloca, ts_i64).unwrap();

        let malloc_fn = self.module.get_function("malloc").unwrap();
        let i32_t = self.context.i32_type();
        let malloc_size = i32_t.const_int(32, false);
        let buf_size = i64_t.const_int(32, false);
        let buf = self
            .builder
            .build_call(malloc_fn, &[malloc_size.into()], "localtime_buf")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_pointer_value();

        let localtime_fn = self.module.get_function("localtime").unwrap();
        let tm_ptr = self
            .builder
            .build_call(localtime_fn, &[ts_alloca.into()], "localtime_call")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_pointer_value();

        let function = self.current_function()?;
        let format_bb = self.context.append_basic_block(function, "localtm_format");
        let fail_bb = self.context.append_basic_block(function, "localtm_fail");
        let done_bb = self.context.append_basic_block(function, "localtm_done");

        let is_null = self.builder.build_is_null(tm_ptr, "tm_null").unwrap();
        self.builder.build_conditional_branch(is_null, fail_bb, format_bb).unwrap();

        self.builder.position_at_end(format_bb);
        let strftime_fn = self.module.get_function("strftime").unwrap();
        let fmt_str = self.builder.build_global_string_ptr("%Y-%m-%d %H:%M:%S", "tm_fmt").unwrap().as_pointer_value();
        self.builder.build_call(strftime_fn, &[buf.into(), buf_size.into(), fmt_str.into(), tm_ptr.into()], "strftime_call").unwrap();
        self.builder.build_unconditional_branch(done_bb).unwrap();

        self.builder.position_at_end(fail_bb);
        let fallback_str = self.builder.build_global_string_ptr("1970-01-01 00:00:00", "tm_fallback").unwrap().as_pointer_value();
        let strcpy_fn = self.module.get_function("strcpy").unwrap_or_else(|| {
            let fn_t = ptr_t.fn_type(&[ptr_t.into(), ptr_t.into()], false);
            self.module.add_function("strcpy", fn_t, None)
        });
        self.builder.build_call(strcpy_fn, &[buf.into(), fallback_str.into()], "strcpy_call").unwrap();
        self.builder.build_unconditional_branch(done_bb).unwrap();

        self.builder.position_at_end(done_bb);
        Ok(buf.into())
    }

    /// 循环从子进程管道读取输出并动态扩容,返回以 '\0' 结尾的堆上字符串。
    fn read_pipe_to_string(&mut self, pipe: PointerValue<'ctx>) -> Result<PointerValue<'ctx>> {
        let i32_t = self.context.i32_type();
        let i64_t = self.context.i64_type();
        let ptr_t = self.context.ptr_type(AddressSpace::default());

        let malloc_fn = self.module.get_function("malloc").unwrap();
        let realloc_fn = self.module.get_function("realloc").unwrap();
        let fread_fn = self.module.get_function("fread").unwrap();

        let init_cap = i32_t.const_int(1024, false);
        let chunk_size_i64 = i64_t.const_int(1024, false);
        let one_i64 = i64_t.const_int(1, false);

        let buf_alloca = self.build_alloca(ptr_t.into(), "pipe_buf")?;
        let cap_alloca = self.build_alloca(i32_t.into(), "pipe_cap")?;
        let len_alloca = self.build_alloca(i32_t.into(), "pipe_len")?;

        let init_buf = self
            .builder
            .build_call(malloc_fn, &[init_cap.into()], "pipe_init_buf")
            .unwrap()
            .try_as_basic_value()
            .unwrap_left()
            .into_pointer_value();

        self.builder.build_store(buf_alloca, init_buf).unwrap();
        self.builder.build_store(cap_alloca, init_cap).unwrap();
        self.builder.build_store(len_alloca, i32_t.const_int(0, false)).unwrap();

        let (loop_bb, grow_bb, read_bb, check_bb, done_bb) = self.make_pipe_blocks()?;

        self.builder.position_at_end(loop_bb);
        let cur_len = self.builder.build_load(i32_t, len_alloca, "cur_len").unwrap().into_int_value();
        let cur_cap = self.builder.build_load(i32_t, cap_alloca, "cur_cap").unwrap().into_int_value();
        let needed = self.builder.build_int_add(cur_len, i32_t.const_int(1025, false), "needed").unwrap();
        let need_grow = self.builder.build_int_compare(inkwell::IntPredicate::SGT, needed, cur_cap, "need_grow").unwrap();
        self.builder.build_conditional_branch(need_grow, grow_bb, read_bb).unwrap();

        self.builder.position_at_end(grow_bb);
        self.emit_pipe_grow_block(buf_alloca, cap_alloca, cur_cap, realloc_fn, read_bb);

        self.builder.position_at_end(read_bb);
        let n_read_i32 = self.emit_pipe_read_block(
            buf_alloca,
            len_alloca,
            fread_fn,
            one_i64,
            chunk_size_i64,
            pipe,
            check_bb,
            done_bb,
        );

        self.builder.position_at_end(check_bb);
        let next_len = self.builder.build_int_add(cur_len, n_read_i32, "next_len").unwrap();
        self.builder.build_store(len_alloca, next_len).unwrap();
        self.builder.build_unconditional_branch(loop_bb).unwrap();

        self.builder.position_at_end(done_bb);
        Ok(self.finish_pipe_string(buf_alloca, len_alloca))
    }

    /// 创建管道读取循环的 5 个基本块(循环/扩容/读取/累计/收尾),并跳入循环。
    /// (自 `read_pipe_to_string` 提炼,块创建与入循环分支逐行原样搬移,零逻辑变化。)
    fn make_pipe_blocks(
        &mut self,
    ) -> Result<(
        BasicBlock<'ctx>,
        BasicBlock<'ctx>,
        BasicBlock<'ctx>,
        BasicBlock<'ctx>,
        BasicBlock<'ctx>,
    )> {
        let function = self.current_function()?;
        let loop_bb = self.context.append_basic_block(function, "pipe_loop");
        let grow_bb = self.context.append_basic_block(function, "pipe_grow");
        let read_bb = self.context.append_basic_block(function, "pipe_read");
        let check_bb = self.context.append_basic_block(function, "pipe_check");
        let done_bb = self.context.append_basic_block(function, "pipe_done");

        self.builder.build_unconditional_branch(loop_bb).unwrap();
        Ok((loop_bb, grow_bb, read_bb, check_bb, done_bb))
    }

    /// 管道读循环的扩容分支:容量翻倍并 `realloc`,随后跳到读取分支。
    /// (自 `read_pipe_to_string` 提炼,分支体逐行原样搬移,零逻辑变化。调用时 builder 已定位到 `grow_bb` 末尾。)
    fn emit_pipe_grow_block(
        &mut self,
        buf_alloca: PointerValue<'ctx>,
        cap_alloca: PointerValue<'ctx>,
        cur_cap: IntValue<'ctx>,
        realloc_fn: FunctionValue<'ctx>,
        read_bb: BasicBlock<'ctx>,
    ) {
        let i32_t = self.context.i32_type();
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let new_cap = self.builder.build_int_mul(cur_cap, i32_t.const_int(2, false), "new_cap").unwrap();
        let cur_buf = self.builder.build_load(ptr_t, buf_alloca, "cur_buf").unwrap().into_pointer_value();
        let grown_buf = self
            .builder
            .build_call(realloc_fn, &[cur_buf.into(), new_cap.into()], "grown_buf")
            .unwrap()
            .try_as_basic_value()
            .unwrap_left()
            .into_pointer_value();
        self.builder.build_store(buf_alloca, grown_buf).unwrap();
        self.builder.build_store(cap_alloca, new_cap).unwrap();
        self.builder.build_unconditional_branch(read_bb).unwrap();
    }

    /// 管道读循环的读取分支:从当前尾部追加读入最多一个块,返回本次字节数。
    /// (自 `read_pipe_to_string` 提炼,分支体逐行原样搬移,零逻辑变化。调用时 builder 已定位到 `read_bb` 末尾。)
    #[allow(clippy::too_many_arguments)]
    fn emit_pipe_read_block(
        &mut self,
        buf_alloca: PointerValue<'ctx>,
        len_alloca: PointerValue<'ctx>,
        fread_fn: FunctionValue<'ctx>,
        one_i64: IntValue<'ctx>,
        chunk_size_i64: IntValue<'ctx>,
        pipe: PointerValue<'ctx>,
        check_bb: BasicBlock<'ctx>,
        done_bb: BasicBlock<'ctx>,
    ) -> IntValue<'ctx> {
        let i32_t = self.context.i32_type();
        let i8_t = self.context.i8_type();
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let cur_buf = self.builder.build_load(ptr_t, buf_alloca, "cur_buf").unwrap().into_pointer_value();
        let cur_len = self.builder.build_load(i32_t, len_alloca, "cur_len").unwrap().into_int_value();
        let dst_ptr = unsafe {
            self.builder.build_gep(i8_t, cur_buf, &[cur_len], "dst_ptr").unwrap()
        };
        let n_read = self
            .builder
            .build_call(fread_fn, &[dst_ptr.into(), one_i64.into(), chunk_size_i64.into(), pipe.into()], "n_read")
            .unwrap()
            .try_as_basic_value()
            .unwrap_left()
            .into_int_value();
        let n_read_i32 = self.builder.build_int_cast(n_read, i32_t, "n_read_i32").unwrap();
        let has_bytes = self.builder.build_int_compare(inkwell::IntPredicate::SGT, n_read_i32, i32_t.const_int(0, false), "has_bytes").unwrap();
        self.builder.build_conditional_branch(has_bytes, check_bb, done_bb).unwrap();
        n_read_i32
    }

    /// 管道读取收尾:在有效载荷末尾补 NUL 并返回堆字符串指针。
    /// (自 `read_pipe_to_string` 提炼,收尾体逐行原样搬移,零逻辑变化。调用时 builder 已定位到 `done_bb` 末尾。)
    fn finish_pipe_string(
        &mut self,
        buf_alloca: PointerValue<'ctx>,
        len_alloca: PointerValue<'ctx>,
    ) -> PointerValue<'ctx> {
        let i32_t = self.context.i32_type();
        let i8_t = self.context.i8_type();
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let final_buf = self.builder.build_load(ptr_t, buf_alloca, "final_buf").unwrap().into_pointer_value();
        let final_len = self.builder.build_load(i32_t, len_alloca, "final_len").unwrap().into_int_value();
        let null_slot = unsafe {
            self.builder.build_gep(i8_t, final_buf, &[final_len], "null_slot").unwrap()
        };
        self.builder.build_store(null_slot, i8_t.const_int(0, false)).unwrap();
        final_buf
    }

    /// `process_run(cmd: str) -> (i32, str)`: 执行系统命令并捕获其退出码与标准输出。
    pub(in crate::codegen) fn compile_process_run(
        &mut self,
        arguments: &[Expr],
    ) -> Result<BasicValueEnum<'ctx>> {
        if arguments.len() != 1 {
            return Err(HuziError::new_global(
                "process_run() requires exactly 1 argument (command)",
            ));
        }
        let cmd = self.compile_str_arg(&arguments[0], "process_run")?;
        let mode = self.cstr_const("r");

        let popen_name = if cfg!(windows) { "_popen" } else { "popen" };
        let pclose_name = if cfg!(windows) { "_pclose" } else { "pclose" };

        let popen_fn = self.module.get_function(popen_name).unwrap();
        let pclose_fn = self.module.get_function(pclose_name).unwrap();

        let pipe_call = self
            .builder
            .build_call(popen_fn, &[cmd.into(), mode.into()], "pipe_open")
            .unwrap();
        let pipe = pipe_call
            .try_as_basic_value()
            .unwrap_left()
            .into_pointer_value();

        let function = self.current_function()?;
        let ok_bb = self.context.append_basic_block(function, "proc_ok");
        let fail_bb = self.context.append_basic_block(function, "proc_fail");
        let done_bb = self.context.append_basic_block(function, "proc_done");

        let is_null = self.builder.build_is_null(pipe, "pipe_is_null").unwrap();
        self.builder
            .build_conditional_branch(is_null, fail_bb, ok_bb)
            .unwrap();

        let i32_t = self.context.i32_type();
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let tup_ty = self.context.struct_type(&[i32_t.into(), ptr_t.into()], false);
        let res_alloca = self.build_alloca(tup_ty.into(), "proc_res")?;

        self.emit_process_fail_branch(tup_ty, res_alloca, fail_bb, done_bb);

        self.emit_process_ok_result(pipe, pclose_fn, tup_ty, res_alloca, ok_bb, done_bb)?;

        self.builder.position_at_end(done_bb);
        Ok(self.builder.build_load(tup_ty, res_alloca, "proc_tuple").unwrap())
    }

    /// `process_run` 的管道打开失败分支:返回 `(-1, "")` 并跳到收尾块。
    /// (自 `compile_process_run` 提炼,分支体逐行原样搬移,零逻辑变化。调用时不预设 builder 位置,内部先定位到 `fail_bb`。)
    fn emit_process_fail_branch(
        &mut self,
        tup_ty: StructType<'ctx>,
        res_alloca: PointerValue<'ctx>,
        fail_bb: BasicBlock<'ctx>,
        done_bb: BasicBlock<'ctx>,
    ) {
        let i32_t = self.context.i32_type();
        self.builder.position_at_end(fail_bb);
        let empty_str = self
            .builder
            .build_global_string_ptr("", "proc_empty")
            .unwrap()
            .as_pointer_value();
        let minus_one = i32_t.const_int((-1i32) as u64, true);
        let f0_fail = self.builder.build_struct_gep(tup_ty, res_alloca, 0, "fail_f0").unwrap();
        self.builder.build_store(f0_fail, minus_one).unwrap();
        let f1_fail = self.builder.build_struct_gep(tup_ty, res_alloca, 1, "fail_f1").unwrap();
        self.builder.build_store(f1_fail, empty_str).unwrap();
        self.builder.build_unconditional_branch(done_bb).unwrap();
    }

    /// `process_run` 的成功分支:排空管道、关闭并解码退出码、落盘 `(code, output)`。
    /// (自 `compile_process_run` 提炼,分支体逐行原样搬移,零逻辑变化。调用时不预设 builder 位置,内部先定位到 `ok_bb`。)
    fn emit_process_ok_result(
        &mut self,
        pipe: PointerValue<'ctx>,
        pclose_fn: FunctionValue<'ctx>,
        tup_ty: StructType<'ctx>,
        res_alloca: PointerValue<'ctx>,
        ok_bb: BasicBlock<'ctx>,
        done_bb: BasicBlock<'ctx>,
    ) -> Result<()> {
        let i32_t = self.context.i32_type();
        self.builder.position_at_end(ok_bb);
        let out_buf = self.read_pipe_to_string(pipe)?;
        let close_call = self
            .builder
            .build_call(pclose_fn, &[pipe.into()], "pipe_close")
            .unwrap();
        let raw_status = close_call
            .try_as_basic_value()
            .unwrap_left()
            .into_int_value();

        let exit_code = if cfg!(windows) {
            raw_status
        } else {
            let shifted = self
                .builder
                .build_right_shift(raw_status, i32_t.const_int(8, false), false, "shifted")
                .unwrap();
            self.builder
                .build_and(shifted, i32_t.const_int(255, false), "exit_code")
                .unwrap()
        };

        let f0_ok = self.builder.build_struct_gep(tup_ty, res_alloca, 0, "ok_f0").unwrap();
        self.builder.build_store(f0_ok, exit_code).unwrap();
        let f1_ok = self.builder.build_struct_gep(tup_ty, res_alloca, 1, "ok_f1").unwrap();
        self.builder.build_store(f1_ok, out_buf).unwrap();
        self.builder.build_unconditional_branch(done_bb).unwrap();
        Ok(())
    }
}
