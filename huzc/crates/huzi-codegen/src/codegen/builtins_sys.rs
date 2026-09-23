//! 系统交互类内置函数:随机数、时间、进程退出与睡眠。
//!
//! 这些函数直接封装 libc/CRT:`rand`/`srand`/`time`/`exit`,以及毫秒级
//! 睡眠(Windows 走 `Sleep`,POSIX 走 `usleep`)。`srand`/`sleep_ms`/
//! `exit` 在 Huzi 层返回整数 0,仅为兼容表达式位置,无实际返回值。

use huzi_ast::Expr;
use huzi_error::{HuziError, Result};
use inkwell::AddressSpace;
use inkwell::values::{BasicValueEnum, IntValue, PointerValue};

use super::CodeGen;

impl<'ctx> CodeGen<'ctx> {
    /// `rand() -> i32`:libc 伪随机数,范围 `0..=RAND_MAX`(Windows 为
    /// 32767)。序列由 `srand(seed)` 决定,同一 seed 产生同一序列。
    pub(super) fn compile_rand(&mut self) -> Result<BasicValueEnum<'ctx>> {
        let rand_fn = self.module.get_function("rand").expect("rand in prelude");
        Ok(self
            .builder
            .build_call(rand_fn, &[], "rand_call")
            .unwrap()
            .try_as_basic_value()
            .unwrap_left())
    }

    /// `srand(seed)`:设置伪随机数序列起点。
    pub(super) fn compile_srand(&mut self, arguments: &[Expr]) -> Result<BasicValueEnum<'ctx>> {
        let seed = self.i32_builtin_arg(arguments, "srand")?;
        let srand_fn = self.module.get_function("srand").expect("srand in prelude");
        self.builder
            .build_call(srand_fn, &[seed.into()], "srand_call")
            .unwrap();
        Ok(self.context.i32_type().const_int(0, false).into())
    }

    /// `time() -> i64`:当前 Unix 时间戳(秒)。参数传空指针。
    pub(super) fn compile_time(&mut self) -> Result<BasicValueEnum<'ctx>> {
        let time_fn = self.module.get_function("time").expect("time in prelude");
        let null_timer = self
            .context
            .ptr_type(inkwell::AddressSpace::default())
            .const_null();
        Ok(self
            .builder
            .build_call(time_fn, &[null_timer.into()], "time_call")
            .unwrap()
            .try_as_basic_value()
            .unwrap_left())
    }

    /// `exit(code)`:立即终止进程,`code` 作为退出码。
    /// 之后以 `unreachable` 收尾,后续语句不可达(与 return 同规则)。
    pub(super) fn compile_exit(&mut self, arguments: &[Expr]) -> Result<BasicValueEnum<'ctx>> {
        let code = self.i32_builtin_arg(arguments, "exit")?;
        let exit_fn = self.module.get_function("exit").expect("exit in prelude");
        self.builder
            .build_call(exit_fn, &[code.into()], "exit_call")
            .unwrap();
        self.builder.build_unreachable().unwrap();
        Ok(self.context.i32_type().const_int(0, false).into())
    }

    /// `panic(msg)`:向 stdout 打印 `Runtime error: <msg>`,随后以退出码
    /// 1 终止进程。除零/越界等检查共用同一 `emit_runtime_check` 路径。
    /// 表达式位置返回整数 0(运行时不可达,仅为兼容表达式位置)。
    pub(super) fn compile_panic(&mut self, arguments: &[Expr]) -> Result<BasicValueEnum<'ctx>> {
        if arguments.len() != 1 {
            return Err(HuziError::new_global("panic() requires exactly 1 argument (message)"));
        }
        let msg = match self.compile_expr(&arguments[0])? {
            BasicValueEnum::PointerValue(p) => p,
            _ => return Err(HuziError::new_global("panic() argument must be a string")),
        };
        let never = self.context.bool_type().const_int(0, false);
        self.emit_runtime_check(never, "Runtime error: %s\n\0", &[msg.into()])?;
        Ok(self.context.i32_type().const_int(0, false).into())
    }

    /// `sleep_ms(ms)`:毫秒级睡眠;负值按 0 处理。
    pub(super) fn compile_sleep_ms(&mut self, arguments: &[Expr]) -> Result<BasicValueEnum<'ctx>> {
        let ms = self.i32_builtin_arg(arguments, "sleep_ms")?;
        let zero = self.context.i32_type().const_int(0, false);
        let positive = self
            .builder
            .build_int_compare(inkwell::IntPredicate::SGT, ms, zero, "ms_pos")
            .unwrap();
        let clamped = self
            .builder
            .build_select(positive, ms, zero, "ms_clamped")
            .unwrap()
            .into_int_value();

        // huzc 以宿主平台为目标,编译期选择睡眠实现。
        let (name, arg) = if cfg!(windows) {
            ("Sleep", clamped.into())
        } else {
            // usleep 以微秒为单位。
            let scale = self.context.i32_type().const_int(1000, false);
            let us = self.builder.build_int_mul(clamped, scale, "ms_to_us").unwrap();
            ("usleep", us.into())
        };
        let sleep_fn = self.module.get_function(name).expect("sleep in prelude");
        self.builder
            .build_call(sleep_fn, &[arg], "sleep_call")
            .unwrap();
        Ok(self.context.i32_type().const_int(0, false).into())
    }

    /// 系统类内置函数的单整数参数校验 + 装载。
    pub(super) fn i32_builtin_arg(&mut self, arguments: &[Expr], name: &str) -> Result<IntValue<'ctx>> {
        if arguments.len() != 1 {
            return Err(HuziError::new_global(format!(
                "{}() requires exactly 1 argument",
                name
            )));
        }
        let value = self.compile_expr(&arguments[0])?;
        match self.coerce_value(self.context.i32_type().into(), value)? {
            BasicValueEnum::IntValue(iv) => Ok(iv),
            _ => Err(HuziError::new_global(format!(
                "{}() argument must be an integer",
                name
            ))),
        }
    }

    /// `env_get(k)`: 读取环境变量,返回 `(bool, str)`。缺键返回 `(false, "")`。
    pub(super) fn compile_env_get(&mut self, arguments: &[Expr]) -> Result<BasicValueEnum<'ctx>> {
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
    pub(super) fn compile_localtime(&mut self, arguments: &[Expr]) -> Result<BasicValueEnum<'ctx>> {
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
        let i8_t = self.context.i8_type();
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

        let function = self.current_function()?;
        let loop_bb = self.context.append_basic_block(function, "pipe_loop");
        let grow_bb = self.context.append_basic_block(function, "pipe_grow");
        let read_bb = self.context.append_basic_block(function, "pipe_read");
        let check_bb = self.context.append_basic_block(function, "pipe_check");
        let done_bb = self.context.append_basic_block(function, "pipe_done");

        self.builder.build_unconditional_branch(loop_bb).unwrap();

        self.builder.position_at_end(loop_bb);
        let cur_len = self.builder.build_load(i32_t, len_alloca, "cur_len").unwrap().into_int_value();
        let cur_cap = self.builder.build_load(i32_t, cap_alloca, "cur_cap").unwrap().into_int_value();
        let needed = self.builder.build_int_add(cur_len, i32_t.const_int(1025, false), "needed").unwrap();
        let need_grow = self.builder.build_int_compare(inkwell::IntPredicate::SGT, needed, cur_cap, "need_grow").unwrap();
        self.builder.build_conditional_branch(need_grow, grow_bb, read_bb).unwrap();

        self.builder.position_at_end(grow_bb);
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

        self.builder.position_at_end(read_bb);
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

        self.builder.position_at_end(check_bb);
        let next_len = self.builder.build_int_add(cur_len, n_read_i32, "next_len").unwrap();
        self.builder.build_store(len_alloca, next_len).unwrap();
        self.builder.build_unconditional_branch(loop_bb).unwrap();

        self.builder.position_at_end(done_bb);
        let final_buf = self.builder.build_load(ptr_t, buf_alloca, "final_buf").unwrap().into_pointer_value();
        let final_len = self.builder.build_load(i32_t, len_alloca, "final_len").unwrap().into_int_value();
        let null_slot = unsafe {
            self.builder.build_gep(i8_t, final_buf, &[final_len], "null_slot").unwrap()
        };
        self.builder.build_store(null_slot, i8_t.const_int(0, false)).unwrap();

        Ok(final_buf)
    }

    /// `process_run(cmd: str) -> (i32, str)`: 执行系统命令并捕获其退出码与标准输出。
    pub(super) fn compile_process_run(
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

        self.builder.position_at_end(done_bb);
        Ok(self.builder.build_load(tup_ty, res_alloca, "proc_tuple").unwrap())
    }
}
