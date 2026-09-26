//! `to_string` 数值格式化(自 `builtins_string/mod.rs` 纯搬移,零逻辑变化)。

use super::super::CodeGen;
use huzi_ast::*;
use huzi_error::{HuziError, Result};
use inkwell::values::PointerValue;

impl<'ctx> CodeGen<'ctx> {
    pub(in crate::codegen) fn compile_to_string(&mut self, arguments: &[Expr]) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        if arguments.len() != 1 {
            return Err(HuziError::new_global("to_string() requires exactly 1 argument"));
        }

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
    fn pick_printf_format(
        &mut self,
        arg: inkwell::values::BasicValueEnum<'ctx>,
    ) -> Result<(PointerValue<'ctx>, inkwell::values::BasicValueEnum<'ctx>)> {
        match arg {
            inkwell::values::BasicValueEnum::IntValue(iv) => {
                if iv.get_type().get_bit_width() == 64 {
                    // %lld 跨平台均为 64 位；Windows LLP64 下 %ld 仅 32 位会截断 i64。
                    let fmt = unsafe { self.builder.build_global_string("%lld", "fmt_i64").unwrap() };
                    Ok((fmt.as_pointer_value(), inkwell::values::BasicValueEnum::IntValue(iv)))
                } else if iv.get_type().get_bit_width() == 8 {
                    let fmt = unsafe { self.builder.build_global_string("%c", "fmt_c").unwrap() };
                    let promoted = self
                        .builder
                        .build_int_z_extend(iv, self.context.i32_type(), "char_promote")
                        .unwrap();
                    Ok((fmt.as_pointer_value(), inkwell::values::BasicValueEnum::IntValue(promoted)))
                } else {
                    let fmt = unsafe { self.builder.build_global_string("%d", "fmt_i32").unwrap() };
                    Ok((fmt.as_pointer_value(), inkwell::values::BasicValueEnum::IntValue(iv)))
                }
            }
            inkwell::values::BasicValueEnum::FloatValue(fv) => {
                // 为 printf 风格的可变参数提升为 double。
                let f64_val = if fv.get_type() == self.context.f64_type() {
                    fv
                } else {
                    self.builder
                        .build_float_ext(fv, self.context.f64_type(), "f_promote")
                        .unwrap()
                };
                if fv.get_type() == self.context.f32_type() {
                    let fmt = unsafe { self.builder.build_global_string("%g", "fmt_f32").unwrap() };
                    Ok((fmt.as_pointer_value(), inkwell::values::BasicValueEnum::FloatValue(f64_val)))
                } else {
                    let fmt = unsafe { self.builder.build_global_string("%f", "fmt_f64").unwrap() };
                    Ok((fmt.as_pointer_value(), inkwell::values::BasicValueEnum::FloatValue(f64_val)))
                }
            }
            _ => {
                Err(HuziError::new_global(
                    "to_string() requires a numeric argument",
                ))
            }
        }
    }
}
