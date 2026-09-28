//! `to_string` 数值格式化(自 `builtins_string/mod.rs` 纯搬移,零逻辑变化)。

use super::super::CodeGen;
use huzi_ast::*;
use huzi_error::{HuziError, Result};
use inkwell::values::PointerValue;

impl<'ctx> CodeGen<'ctx> {
    pub(in crate::codegen) fn compile_to_string(&mut self, arguments: &[Expr]) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        self.expect_arg_count("to_string", arguments, 1)?;

        let sprintf_fn = self.module.get_function("sprintf").unwrap();

        let arg = self.compile_expr(&arguments[0])?;

        // 若参数本身已是字符串指针，直接返回。
        if arg.is_pointer_value() {
            return Ok(arg);
        }

        // 布尔值使用较短的固定大小缓冲区，无需格式化字符串。
        if let inkwell::values::BasicValueEnum::IntValue(iv) = arg {
            if iv.get_type().get_bit_width() == 1 {
                let s = self.build_bool_str(iv)?;
                let buffer = self.alloc_str_buffer(8)?;
                self.builder
                    .build_call(sprintf_fn, &[buffer.into(), s.into()], "sprintf")
                    .unwrap();
                return Ok(buffer.into());
            }
        }

        let (format_ptr, value) = self.pick_printf_format(arg)?;

        // 分配缓冲区（足够容纳任何双精度浮点数的格式化输出）
        let buffer = self.alloc_str_buffer(320)?;

        // 调用 sprintf
        self.builder
            .build_call(
                sprintf_fn,
                &[buffer.into(), format_ptr.into(), value.into()],
                "sprintf",
            )
            .unwrap();

        Ok(buffer.into())
    }

    /// 为 `arg` 选择 printf 风格的格式化字符串，并提升其值以匹配 C 可变参数规范（char 转 i32，float 转 double）。
    /// 格式与提升复用 `print` 的 `int_printf_parts`/`float_printf_parts`。
    fn pick_printf_format(
        &mut self,
        arg: inkwell::values::BasicValueEnum<'ctx>,
    ) -> Result<(PointerValue<'ctx>, inkwell::values::BasicValueEnum<'ctx>)> {
        let (spec, value) = match arg {
            inkwell::values::BasicValueEnum::IntValue(iv) => self.int_printf_parts(iv),
            inkwell::values::BasicValueEnum::FloatValue(fv) => self.float_printf_parts(fv),
            _ => {
                return Err(HuziError::new_global(
                    "to_string() requires a numeric argument",
                ))
            }
        };
        // %lld 跨平台均为 64 位；Windows LLP64 下 %ld 仅 32 位会截断 i64。
        let tag = match spec {
            "%lld" => "fmt_i64",
            "%c" => "fmt_c",
            "%d" => "fmt_i32",
            "%g" => "fmt_f32",
            _ => "fmt_f64",
        };
        let fmt = unsafe { self.builder.build_global_string(spec, tag).unwrap() };
        Ok((fmt.as_pointer_value(), value))
    }
}
