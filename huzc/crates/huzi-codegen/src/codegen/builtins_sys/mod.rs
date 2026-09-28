//! 系统交互类内置函数:随机数、时间、进程退出与睡眠。
//!
//! 这些函数直接封装 libc/CRT:`rand`/`srand`/`time`/`exit`,以及毫秒级
//! 睡眠(Windows 走 `Sleep`,POSIX 走 `usleep`)。`srand`/`sleep_ms`/
//! `exit` 在 Huzi 层返回整数 0,仅为兼容表达式位置,无实际返回值。
//!
//! 子模块:`exit_sleep`(退出/断言/睡眠)、`process`(环境/时间格式化/子进程)。
//! (目录化自 `builtins_sys.rs` 纯搬移,零逻辑变化。)

mod exit_sleep;
mod process;

use super::CodeGen;
use huzi_ast::Expr;
use huzi_error::{HuziError, Result};
use inkwell::values::{BasicValueEnum, IntValue};

impl<'ctx> CodeGen<'ctx> {
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

    pub(super) fn i32_builtin_arg(&mut self, arguments: &[Expr], name: &str) -> Result<IntValue<'ctx>> {
        self.expect_arg_count(name, arguments, 1)?;
        let value = self.compile_expr(&arguments[0])?;
        match self.coerce_value(self.context.i32_type().into(), value)? {
            BasicValueEnum::IntValue(iv) => Ok(iv),
            _ => Err(HuziError::new_global(format!(
                "{}() argument must be an integer",
                name
            ))),
        }
    }

    /// 校验内置函数实参个数;不符时报统一格式的错误。
    /// 供各 `compile_*` 入口复用,替代散落的 `if arguments.len() != N`。
    pub(super) fn expect_arg_count(
        &self,
        name: &str,
        arguments: &[Expr],
        expected: usize,
    ) -> Result<()> {
        if arguments.len() != expected {
            let noun = if expected == 1 {
                "1 argument".to_string()
            } else {
                format!("{} arguments", expected)
            };
            return Err(HuziError::new_global(format!(
                "{}() requires exactly {}",
                name, noun
            )));
        }
        Ok(())
    }
}
