//! 系统交互类内置函数:随机数、时间、进程退出与睡眠。
//!
//! 这些函数直接封装 libc/CRT:`rand`/`srand`/`time`/`exit`,以及毫秒级
//! 睡眠(Windows 走 `Sleep`,POSIX 走 `usleep`)。`srand`/`sleep_ms`/
//! `exit` 在 Huzi 层返回整数 0,仅为兼容表达式位置,无实际返回值。

use huzi_ast::Expr;
use huzi_error::{HuziError, Result};
use inkwell::AddressSpace;
use inkwell::values::{BasicValueEnum, IntValue};

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
}
