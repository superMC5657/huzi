//! 退出/断言/睡眠:`exit`/`panic`/`sleep_ms`。(自 `builtins_sys.rs` 纯搬移,零逻辑变化)。

use super::super::CodeGen;
use huzi_ast::Expr;
use huzi_error::Result;
use inkwell::values::BasicValueEnum;

impl<'ctx> CodeGen<'ctx> {
    pub(in crate::codegen) fn compile_exit(&mut self, arguments: &[Expr]) -> Result<BasicValueEnum<'ctx>> {
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
    pub(in crate::codegen) fn compile_panic(&mut self, arguments: &[Expr]) -> Result<BasicValueEnum<'ctx>> {
        self.expect_arg_count("panic", arguments, 1)?;
        let msg = self.compile_str_arg(&arguments[0], "panic")?;
        let never = self.context.bool_type().const_int(0, false);
        self.emit_runtime_check(never, "Runtime error: %s\n\0", &[msg.into()])?;
        Ok(self.context.i32_type().const_int(0, false).into())
    }

    /// `sleep_ms(ms)`:毫秒级睡眠;负值按 0 处理。
    pub(in crate::codegen) fn compile_sleep_ms(&mut self, arguments: &[Expr]) -> Result<BasicValueEnum<'ctx>> {
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
}
