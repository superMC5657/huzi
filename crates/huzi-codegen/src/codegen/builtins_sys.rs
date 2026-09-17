//! 系统交互类内置函数:随机数、时间、进程退出与睡眠。
//!
//! 这些函数直接封装 libc/CRT:`rand`/`srand`/`time`/`exit`,以及毫秒级
//! 睡眠(Windows 走 `Sleep`,POSIX 走 `usleep`)。`srand`/`sleep_ms`/
//! `exit` 在 Huzi 层返回整数 0,仅为兼容表达式位置,无实际返回值。

use huzi_ast::Expr;
use huzi_error::{HuziError, Result};
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
    fn i32_builtin_arg(&mut self, arguments: &[Expr], name: &str) -> Result<IntValue<'ctx>> {
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
}
