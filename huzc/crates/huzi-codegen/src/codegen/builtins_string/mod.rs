//! 字符串拼接:`concat` 求值与拷贝。(其余串函数见子模块;自原 `mod.rs` 纯搬移,零逻辑变化。)
//!
//! 子模块:`len`(`len` 内置)、`to_str`(`to_string` 格式化)、
//! `split_ops`(`split`/`substring`)、`trim`(`trim`/`contains`)。

mod len;
mod split_ops;
mod to_str;
mod trim;

use super::CodeGen;
use huzi_ast::*;
use huzi_error::{HuziError, Result};
use inkwell::values::PointerValue;

impl<'ctx> CodeGen<'ctx> {
    pub(super) fn compile_concat(&mut self, arguments: &[Expr]) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        if arguments.len() < 2 {
            return Err(HuziError::new_global("concat() requires at least 2 arguments"));
        }

        let malloc_fn = self.module.get_function("malloc").unwrap();
        let strcpy_fn = self.module.get_function("strcpy").unwrap();

        let (arg_ptrs, arg_lens) = self.concat_string_args(arguments)?;

        // 为 null 终止符分配 len(args...) + 1。
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

    /// 求值所有参数（必须均为字符串）；返回它们的指针与 strlen 长度。
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

    /// 将第一个字符串复制到 `buffer` 中，随后追加其余各字符串。
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
}
