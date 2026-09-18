use super::{CodeGen, VarSlot};
use huzi_ast::*;
use huzi_error::{HuziError, Result};
use inkwell::AddressSpace;
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
                if Self::is_map_slot(&slot) {
                    return Err(HuziError::new_global(
                        "len() does not support HashMap; use map_len() (HashMap is str->i32 only)",
                    ));
                }
                if Self::is_vec_slot(&slot) {
                    return self.vec_len_value(name);
                }
                if let Some(len) = slot.array_len {
                    return Ok(self.context.i32_type().const_int(len as u64, false).into());
                }
            }
        }

        // len(s.arr) on a struct array field uses the declared array size;
        // len(s.items) on a vec field returns its dynamic length.
        if let Expr::FieldAccess(fa) = &arguments[0] {
            if let Some((_, fields)) = self.struct_def_of_expr(&fa.base) {
                if let Some(info) = fields.iter().find(|info| info.name == fa.field) {
                    if let Type::Array(_, size) = &info.ast_ty {
                        return Ok(self.context.i32_type().const_int(*size as u64, false).into());
                    }
                    if matches!(&info.ast_ty, Type::Applied(n, _) if n == "vec") {
                        let vec_val = self.compile_expr(&arguments[0])?;
                        if vec_val.is_struct_value() {
                            let len = self
                                .builder
                                .build_extract_value(vec_val.into_struct_value(), 1, "vec_field_len")
                                .unwrap();
                            return Ok(len.into());
                        }
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

impl<'ctx> CodeGen<'ctx> {
    /// AST 层面判断是否为 `split(...)` 构造(供 `let` 分发,不生成指令)。
    pub(super) fn is_split_ctor(call: &CallExpr) -> bool {
        matches!(&*call.callee, Expr::Ident(name) if name == "split")
    }

    /// `let parts = split(s, d)` — vec<str> 存槽,elem 标记 str 指针类型,
    /// 使下标/`len`/`print`/`for-in`/`push` 与 `vec(...)` 完全互通。
    pub(super) fn compile_let_split(
        &mut self,
        stmt: &LetStmt,
        arguments: &[Expr],
        span: Span,
    ) -> Result<()> {
        if stmt.type_annotation.is_some() {
            return Err(HuziError::new_global(
                "split() infers its vec<str> type from the arguments; remove the type annotation",
            ));
        }
        let vec_val = self.compile_split(arguments)?;
        let vec_ty = vec_val.get_type();
        let alloca = self.build_alloca(vec_ty, &stmt.name)?;
        self.builder.build_store(alloca, vec_val).unwrap();
        let str_ty = self.context.ptr_type(AddressSpace::default()).into();
        self.scope_insert(
            stmt.name.clone(),
            VarSlot {
                ptr: alloca,
                ty: vec_ty,
                elem: Some(str_ty),
                array_len: None,
                mutable: stmt.mutable,
                box_inner: None,
            },
        );
        self.declare_local(&stmt.name, alloca, vec_ty, span);
        Ok(())
    }

    /// `split(s, delim)` — 按单字符或多字符分隔符切分,返回 vec<str>;
    /// 空分隔符时整体作为唯一一段(均为堆拷贝,与 concat 同分配模式)。
    pub(super) fn compile_split(
        &mut self,
        arguments: &[Expr],
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        if arguments.len() != 2 {
            return Err(HuziError::new_global(
                "split() requires exactly 2 arguments (string, delimiter)",
            ));
        }
        let s = self.str_ptr_arg(&arguments[0], "split()")?;
        let d = self.str_ptr_arg(&arguments[1], "split()")?;
        let s_len = self.str_len_of(s, "split_slen")?;
        let d_len = self.str_len_of(d, "split_dlen")?;
        let i32_type = self.context.i32_type();
        let str_ty: inkwell::types::BasicTypeEnum<'ctx> =
            self.context.ptr_type(AddressSpace::default()).into();
        let function = self.current_function()?;
        let one_bb = self.context.append_basic_block(function, "split_one");
        let many_bb = self.context.append_basic_block(function, "split_many");
        let done_bb = self.context.append_basic_block(function, "split_done");
        let result = self.build_alloca(self.vec_struct_type().into(), "split_result")?;
        let empty = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                d_len,
                i32_type.const_int(0, false),
                "split_empty",
            )
            .unwrap();
        self.builder.build_conditional_branch(empty, one_bb, many_bb).unwrap();
        self.builder.position_at_end(one_bb);
        self.emit_split_single(s, s_len, str_ty, result, done_bb)?;
        self.builder.position_at_end(many_bb);
        self.emit_split_many(s, d, s_len, d_len, str_ty, result, done_bb)?;
        self.builder.position_at_end(done_bb);
        Ok(self.builder.build_load(self.vec_struct_type(), result, "split_val").unwrap())
    }

    /// 空分隔符分支:整体拷贝为唯一一段。
    /// 正常分支:先计数分配指针数组,再逐段拷贝填充。
    /// (实现见 builtins_string_util.rs 的同名助手,保持本文件在 500 行内。)

    /// `substring(s, start, end)` — 字节区间 `[start, end)` 拷贝;
    /// 越界(含负数)报运行时错误。仍按字节语义,不做字符语义。
    pub(super) fn compile_substring(
        &mut self,
        arguments: &[Expr],
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        if arguments.len() != 3 {
            return Err(HuziError::new_global(
                "substring() requires exactly 3 arguments (string, start, end)",
            ));
        }
        let s = self.str_ptr_arg(&arguments[0], "substring()")?;
        let start_expr = self.compile_expr(&arguments[1])?;
        let start = self.coerce_index(start_expr)?;
        let end_expr = self.compile_expr(&arguments[2])?;
        let end = self.coerce_index(end_expr)?;
        let s_len = self.str_len_of(s, "sub_slen")?;
        // 无符号比较:0 <= start <= end <= len,负数自然落入失败分支。
        let ok_lo = self
            .builder
            .build_int_compare(inkwell::IntPredicate::ULE, start, end, "sub_se")
            .unwrap();
        self.emit_runtime_check(ok_lo, "Runtime error: substring out of bounds\n\0", &[])?;
        let ok_hi = self
            .builder
            .build_int_compare(inkwell::IntPredicate::ULE, end, s_len, "sub_el")
            .unwrap();
        self.emit_runtime_check(ok_hi, "Runtime error: substring out of bounds\n\0", &[])?;
        Ok(self.str_copy_range(s, start, end)?.into())
    }

    /// `trim(s)` — 去除两端 ASCII 空白(空格/`\t`/`\n`/`\r`),返回新堆字符串。
    pub(super) fn compile_trim(
        &mut self,
        arguments: &[Expr],
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        if arguments.len() != 1 {
            return Err(HuziError::new_global("trim() requires exactly 1 argument"));
        }
        let s = self.str_ptr_arg(&arguments[0], "trim()")?;
        let s_len = self.str_len_of(s, "trim_len")?;
        let function = self.current_function()?;
        let lo = self.trim_left_bound(s, s_len, function)?;
        let hi = self.trim_right_bound(s, lo, s_len, function)?;
        Ok(self.str_copy_range(s, lo, hi)?.into())
    }

    /// 左边界:跳过前导空白后的首个下标。
    fn trim_left_bound(
        &mut self,
        s: PointerValue<'ctx>,
        s_len: inkwell::values::IntValue<'ctx>,
        function: inkwell::values::FunctionValue<'ctx>,
    ) -> Result<inkwell::values::IntValue<'ctx>> {
        let i32_type = self.context.i32_type();
        let i8_type = self.context.i8_type();
        let loop_bb = self.context.append_basic_block(function, "trim_lo_loop");
        let check_bb = self.context.append_basic_block(function, "trim_lo_check");
        let next_bb = self.context.append_basic_block(function, "trim_lo_next");
        let done_bb = self.context.append_basic_block(function, "trim_lo_done");
        let lo = self.build_alloca(i32_type.into(), "trim_lo")?;
        self.builder.build_store(lo, i32_type.const_int(0, false)).unwrap();
        self.builder.build_unconditional_branch(loop_bb).unwrap();
        self.builder.position_at_end(loop_bb);
        let lv = self.builder.build_load(i32_type, lo, "trim_lo").unwrap().into_int_value();
        let cond = self.builder.build_int_compare(inkwell::IntPredicate::ULT, lv, s_len, "trim_lo_c").unwrap();
        self.builder.build_conditional_branch(cond, check_bb, done_bb).unwrap();
        self.builder.position_at_end(check_bb);
        let lv2 = self.builder.build_load(i32_type, lo, "trim_lo").unwrap().into_int_value();
        let ptr = unsafe { self.builder.build_gep(i8_type, s, &[lv2], "trim_lo_p").unwrap() };
        let byte = self.builder.build_load(i8_type, ptr, "trim_lo_b").unwrap().into_int_value();
        let ws = self.is_space_byte(byte);
        self.builder.build_conditional_branch(ws, next_bb, done_bb).unwrap();
        self.builder.position_at_end(next_bb);
        let lv3 = self.builder.build_load(i32_type, lo, "trim_lo").unwrap().into_int_value();
        let inc = self.builder.build_int_add(lv3, i32_type.const_int(1, false), "trim_lo_inc").unwrap();
        self.builder.build_store(lo, inc).unwrap();
        self.builder.build_unconditional_branch(loop_bb).unwrap();
        self.builder.position_at_end(done_bb);
        Ok(self.builder.build_load(i32_type, lo, "trim_lo").unwrap().into_int_value())
    }

    /// 右边界:跳过尾部空白,不小于左边界(全空白时为空串)。
    fn trim_right_bound(
        &mut self,
        s: PointerValue<'ctx>,
        lo: inkwell::values::IntValue<'ctx>,
        s_len: inkwell::values::IntValue<'ctx>,
        function: inkwell::values::FunctionValue<'ctx>,
    ) -> Result<inkwell::values::IntValue<'ctx>> {
        let i32_type = self.context.i32_type();
        let i8_type = self.context.i8_type();
        let loop_bb = self.context.append_basic_block(function, "trim_hi_loop");
        let check_bb = self.context.append_basic_block(function, "trim_hi_check");
        let next_bb = self.context.append_basic_block(function, "trim_hi_next");
        let done_bb = self.context.append_basic_block(function, "trim_hi_done");
        let hi = self.build_alloca(i32_type.into(), "trim_hi")?;
        self.builder.build_store(hi, s_len).unwrap();
        self.builder.build_unconditional_branch(loop_bb).unwrap();
        self.builder.position_at_end(loop_bb);
        let hv = self.builder.build_load(i32_type, hi, "trim_hi").unwrap().into_int_value();
        let cond = self.builder.build_int_compare(inkwell::IntPredicate::UGT, hv, lo, "trim_hi_c").unwrap();
        self.builder.build_conditional_branch(cond, check_bb, done_bb).unwrap();
        self.builder.position_at_end(check_bb);
        let hv2 = self.builder.build_load(i32_type, hi, "trim_hi").unwrap().into_int_value();
        let prev = self.builder.build_int_sub(hv2, i32_type.const_int(1, false), "trim_hi_prev").unwrap();
        let ptr = unsafe { self.builder.build_gep(i8_type, s, &[prev], "trim_hi_p").unwrap() };
        let byte = self.builder.build_load(i8_type, ptr, "trim_hi_b").unwrap().into_int_value();
        let ws = self.is_space_byte(byte);
        self.builder.build_conditional_branch(ws, next_bb, done_bb).unwrap();
        self.builder.position_at_end(next_bb);
        let hv3 = self.builder.build_load(i32_type, hi, "trim_hi").unwrap().into_int_value();
        let dec = self.builder.build_int_sub(hv3, i32_type.const_int(1, false), "trim_hi_dec").unwrap();
        self.builder.build_store(hi, dec).unwrap();
        self.builder.build_unconditional_branch(loop_bb).unwrap();
        self.builder.position_at_end(done_bb);
        Ok(self.builder.build_load(i32_type, hi, "trim_hi").unwrap().into_int_value())
    }

    /// `contains(s, sub)` — `sub` 为空或在 `s` 中出现时返回 true(字节语义)。
    pub(super) fn compile_contains(
        &mut self,
        arguments: &[Expr],
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        if arguments.len() != 2 {
            return Err(HuziError::new_global(
                "contains() requires exactly 2 arguments (string, sub)",
            ));
        }
        let s = self.str_ptr_arg(&arguments[0], "contains()")?;
        let sub = self.str_ptr_arg(&arguments[1], "contains()")?;
        let s_len = self.str_len_of(s, "contains_slen")?;
        let sub_len = self.str_len_of(sub, "contains_sublen")?;
        self.emit_contains_loop(s, s_len, sub, sub_len)
    }
}
