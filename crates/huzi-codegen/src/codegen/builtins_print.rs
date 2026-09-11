use super::CodeGen;
use huzi_ast::*;
use huzi_error::{HuziError, Result};

impl<'ctx> CodeGen<'ctx> {
    // ==================== Scope helpers ====================

    /// Switch the Windows console to the UTF-8 code page (65001) so `printf`
    /// shows Chinese and other non-ASCII text correctly. String literals are
    /// stored as UTF-8 bytes; without this the console decodes them with its
    /// default code page (e.g. GBK) and prints mojibake. No-op elsewhere.
    pub(super) fn emit_console_utf8_setup(&mut self) {
        if !cfg!(windows) {
            return;
        }
        let set_cp_fn = self.module.get_function("SetConsoleOutputCP").unwrap();
        let utf8_cp = self.context.i32_type().const_int(65001, false);
        self.builder
            .build_call(set_cp_fn, &[utf8_cp.into()], "console_utf8")
            .unwrap();
    }

    pub(super) fn compile_print(
        &mut self,
        arguments: &[Expr],
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        let printf_fn = self.module.get_function("printf").unwrap();

        if arguments.is_empty() {
            let empty_str = unsafe { self.builder.build_global_string("", "empty_str").unwrap() };
            let call = self
                .builder
                .build_call(
                    printf_fn,
                    &[empty_str.as_pointer_value().into()],
                    "print_empty",
                )
                .unwrap();
            return Ok(call.try_as_basic_value().unwrap_left());
        }

        let mut format_string = String::new();
        let mut args: Vec<inkwell::values::BasicMetadataValueEnum> = Vec::new();

        for arg in arguments.iter() {
            // vec 整体不可打印(会误入元组格式化);请打印 len(v) 或 v[i]。
            if let Expr::Ident(name) = arg {
                if let Some(slot) = self.scope_lookup(name) {
                    if Self::is_vec_slot(&slot) {
                        return Err(HuziError::new_global(
                            "print() does not support vec directly; print len(v) or elements instead",
                        ));
                    }
                }
            }
            let value = self.compile_expr(arg)?;
            self.format_print_value(value, &mut format_string, &mut args)?;
        }

        format_string.push('\n');

        let format_ptr = unsafe {
            self.builder
                .build_global_string(&format_string, "format_str")
                .unwrap()
        };

        let mut call_args = vec![format_ptr.as_pointer_value().into()];
        call_args.extend(args);

        let call = self
            .builder
            .build_call(printf_fn, &call_args, "print_call")
            .unwrap();

        Ok(call.try_as_basic_value().unwrap_left())
    }

    /// Append one printed value to the printf format string and argument
    /// list, promoting/narrowing according to C varargs conventions.
    pub(super) fn format_print_value(
        &mut self,
        value: inkwell::values::BasicValueEnum<'ctx>,
        format_string: &mut String,
        args: &mut Vec<inkwell::values::BasicMetadataValueEnum<'ctx>>,
    ) -> Result<()> {
        // Tuples print as `(v1, v2, ...)` with each field formatted.
        if let inkwell::values::BasicValueEnum::StructValue(sv) = value {
            if self.is_tuple_type(sv.get_type()) {
                return self.format_tuple_value(sv.into(), format_string, args);
            }
        }

        match value {
            inkwell::values::BasicValueEnum::IntValue(iv)
                if iv.get_type().get_bit_width() == 1 =>
            {
                // Booleans print as true/false.
                let s = self.build_bool_str(iv)?;
                format_string.push_str("%s");
                args.push(s.into());
            }
            inkwell::values::BasicValueEnum::IntValue(iv) => {
                match iv.get_type().get_bit_width() {
                    8 => {
                        // Chars are printed as characters.
                        format_string.push_str("%c");
                        let c = self
                            .builder
                            .build_int_z_extend(iv, self.context.i32_type(), "char_promote")
                            .unwrap();
                        args.push(c.into());
                    }
                    64 => {
                        format_string.push_str("%ld");
                        args.push(iv.into());
                    }
                    _ => {
                        format_string.push_str("%d");
                        args.push(iv.into());
                    }
                }
            }
            inkwell::values::BasicValueEnum::FloatValue(fv) => {
                // varargs promote floats to double
                let f64_val = if fv.get_type() == self.context.f64_type() {
                    fv
                } else {
                    self.builder
                        .build_float_ext(fv, self.context.f64_type(), "f_promote")
                        .unwrap()
                };
                if fv.get_type() == self.context.f32_type() {
                    format_string.push_str("%g");
                } else {
                    format_string.push_str("%f");
                }
                args.push(f64_val.into());
            }
            inkwell::values::BasicValueEnum::PointerValue(pv) => {
                format_string.push_str("%s");
                args.push(pv.into());
            }
            _ => {
                return Err(HuziError::new_global(
                    "print() does not support this value type",
                ))
            }
        }

        Ok(())
    }
}
