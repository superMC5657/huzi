use super::CodeGen;
use huzi_ast::*;
use huzi_error::{HuziError, Result};
use inkwell::values::PointerValue;

impl<'ctx> CodeGen<'ctx> {
    pub(super) fn compile_len(&mut self, arguments: &[Expr]) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        if arguments.len() != 1 {
            return Err(HuziError::new_global("len() requires exactly 1 argument"));
        }

        // len(arr) on an array variable returns the tracked array length;
        // strings use strlen.
        if let Expr::Ident(name) = &arguments[0] {
            if let Some(slot) = self.scope_lookup(name) {
                if Self::is_vec_slot(&slot) {
                    return self.vec_len_value(name);
                }
                if let Some(len) = slot.array_len {
                    return Ok(self.context.i32_type().const_int(len as u64, false).into());
                }
            }
        }

        // len(s.arr) on a struct array field uses the declared array size.
        if let Expr::FieldAccess(fa) = &arguments[0] {
            if let Some((_, fields)) = self.struct_def_of_expr(&fa.base) {
                if let Some(info) = fields.iter().find(|info| info.name == fa.field) {
                    if let Type::Array(_, size) = &info.ast_ty {
                        return Ok(self.context.i32_type().const_int(*size as u64, false).into());
                    }
                }
            }
        }

        let arg = self.compile_expr(&arguments[0])?;
        let arg = if arg.is_pointer_value() {
            arg.into_pointer_value()
        } else {
            return Err(HuziError::new_global("len() requires a string or array argument"));
        };

        let strlen_fn = self.module.get_function("strlen").unwrap();
        let len = self
            .builder
            .build_call(strlen_fn, &[arg.into()], "str_len")
            .unwrap()
            .try_as_basic_value()
            .unwrap_left();

        Ok(len)
    }

    pub(super) fn compile_concat(&mut self, arguments: &[Expr]) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        if arguments.len() < 2 {
            return Err(HuziError::new_global("concat() requires at least 2 arguments"));
        }

        let malloc_fn = self.module.get_function("malloc").unwrap();
        let strcpy_fn = self.module.get_function("strcpy").unwrap();

        let (arg_ptrs, arg_lens) = self.concat_string_args(arguments)?;

        // Allocate len(args...) + 1 for the null terminator.
        let i32_type = self.context.i32_type();
        let mut total_len = i32_type.const_int(0, false);
        for len in &arg_lens {
            total_len = self
                .builder
                .build_int_add(total_len, *len, "total_len")
                .unwrap();
        }
        let alloc_size = self
            .builder
            .build_int_add(total_len, i32_type.const_int(1, false), "alloc_size")
            .unwrap();
        let buffer = self
            .builder
            .build_call(malloc_fn, &[alloc_size.into()], "concat_buffer")
            .unwrap()
            .try_as_basic_value()
            .unwrap_left()
            .into_pointer_value();

        self.concat_copy_into(buffer, strcpy_fn, &arg_ptrs, &arg_lens)?;

        Ok(buffer.into())
    }

    /// Evaluate the arguments, which must all be strings; return their
    /// pointers and strlen lengths.
    fn concat_string_args(
        &mut self,
        arguments: &[Expr],
    ) -> Result<(Vec<PointerValue<'ctx>>, Vec<inkwell::values::IntValue<'ctx>>)> {
        let strlen_fn = self.module.get_function("strlen").unwrap();

        let mut arg_ptrs = Vec::with_capacity(arguments.len());
        let mut arg_lens = Vec::with_capacity(arguments.len());
        for a in arguments {
            let v = self.compile_expr(a)?;
            let ptr = if v.is_pointer_value() {
                v.into_pointer_value()
            } else {
                return Err(HuziError::new_global("concat() requires string arguments"));
            };
            let len = self
                .builder
                .build_call(strlen_fn, &[ptr.into()], "len")
                .unwrap()
                .try_as_basic_value()
                .unwrap_left()
                .into_int_value();
            arg_ptrs.push(ptr);
            arg_lens.push(len);
        }
        Ok((arg_ptrs, arg_lens))
    }

    /// Copy the first string into `buffer`, then append each remaining one.
    fn concat_copy_into(
        &mut self,
        buffer: PointerValue<'ctx>,
        strcpy_fn: inkwell::values::FunctionValue<'ctx>,
        arg_ptrs: &[PointerValue<'ctx>],
        arg_lens: &[inkwell::values::IntValue<'ctx>],
    ) -> Result<()> {
        self.builder
            .build_call(strcpy_fn, &[buffer.into(), arg_ptrs[0].into()], "copy")
            .unwrap();
        let mut offset = arg_lens[0];
        for (ptr, len) in arg_ptrs.iter().zip(arg_lens.iter()).skip(1) {
            let dest = unsafe {
                self.builder
                    .build_gep(self.context.i8_type(), buffer, &[offset], "concat_dest")
                    .unwrap()
            };
            self.builder
                .build_call(strcpy_fn, &[dest.into(), (*ptr).into()], "copy")
                .unwrap();
            offset = self
                .builder
                .build_int_add(offset, *len, "offset")
                .unwrap();
        }
        Ok(())
    }

    pub(super) fn compile_to_string(&mut self, arguments: &[Expr]) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        if arguments.len() != 1 {
            return Err(HuziError::new_global("to_string() requires exactly 1 argument"));
        }

        let sprintf_fn = self.module.get_function("sprintf").unwrap();

        let arg = self.compile_expr(&arguments[0])?;

        // Booleans take a short fixed-size buffer with no format string.
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

        let (format_ptr, value) = self.to_string_format(arg)?;

        // Allocate buffer (large enough for any double formatting)
        let buffer = self.alloc_str_buffer(320)?;

        // Call sprintf
        self.builder
            .build_call(
                sprintf_fn,
                &[buffer.into(), format_ptr.into(), value.into()],
                "sprintf",
            )
            .unwrap();

        Ok(buffer.into())
    }

    /// Pick the printf-style format string for `arg` and promote the value
    /// to match C varargs conventions (chars to i32, floats to double).
    fn to_string_format(
        &mut self,
        arg: inkwell::values::BasicValueEnum<'ctx>,
    ) -> Result<(PointerValue<'ctx>, inkwell::values::BasicValueEnum<'ctx>)> {
        match arg {
            inkwell::values::BasicValueEnum::IntValue(iv) => {
                if iv.get_type().get_bit_width() == 64 {
                    let fmt = unsafe { self.builder.build_global_string("%ld", "fmt_i64").unwrap() };
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
                // Promote to double for printf-style varargs.
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
                return Err(HuziError::new_global(
                    "to_string() requires a numeric argument",
                ))
            }
        }
    }
}
