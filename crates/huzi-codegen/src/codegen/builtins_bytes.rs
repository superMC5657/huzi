//! 字节构造内置函数:`str_from_bytes(vec<i32>) -> str`。
//!
//! 把字节值向量打包为 C 字符串(供自举标准库做 UTF-8 编码等底层
//! 字符串合成)。每个 i32 按低 8 位截断(`& 255`)后写入——负值
//! 即补码形式的无符号字节,可正常还原;不是按 255 钳位。

use huzi_ast::Expr;
use huzi_error::{HuziError, Result};
use inkwell::values::BasicValueEnum;

use super::CodeGen;

impl<'ctx> CodeGen<'ctx> {
    /// `str_from_bytes(bytes: vec<i32>) -> str`
    pub(super) fn compile_str_from_bytes(
        &mut self,
        arguments: &[Expr],
    ) -> Result<BasicValueEnum<'ctx>> {
        if arguments.len() != 1 {
            return Err(HuziError::new_global(
                "str_from_bytes() requires 1 argument: (bytes: vec<i32>)",
            ));
        }
        let vec_val = self.compile_expr(&arguments[0])?;
        if !vec_val.is_struct_value() {
            return Err(HuziError::new_global(
                "str_from_bytes() requires a vec<i32> argument",
            ));
        }
        let sv = vec_val.into_struct_value();
        let data = self
            .builder
            .build_extract_value(sv, 0, "sfb_data")
            .unwrap()
            .into_pointer_value();
        let len = self
            .builder
            .build_extract_value(sv, 1, "sfb_len")
            .unwrap()
            .into_int_value();

        let malloc_fn = self.module.get_function("malloc").unwrap();
        let i32_ty = self.context.i32_type();
        let one = i32_ty.const_int(1, false);
        let size = self.builder.build_int_add(len, one, "sfb_size").unwrap();
        let buf = self
            .builder
            .build_call(malloc_fn, &[size.into()], "sfb_buf")
            .unwrap()
            .try_as_basic_value()
            .unwrap_left()
            .into_pointer_value();

        let function = self.current_function()?;
        let loop_bb = self.context.append_basic_block(function, "sfb_loop");
        let body_bb = self.context.append_basic_block(function, "sfb_body");
        let done_bb = self.context.append_basic_block(function, "sfb_done");

        let idx_alloca = self.build_alloca(i32_ty.into(), "sfb_idx")?;
        self.builder
            .build_store(idx_alloca, i32_ty.const_int(0, false))
            .unwrap();
        self.builder.build_unconditional_branch(loop_bb).unwrap();

        self.builder.position_at_end(loop_bb);
        let idx = self
            .builder
            .build_load(i32_ty, idx_alloca, "sfb_i")
            .unwrap()
            .into_int_value();
        let more = self
            .builder
            .build_int_compare(inkwell::IntPredicate::SLT, idx, len, "sfb_more")
            .unwrap();
        self.builder
            .build_conditional_branch(more, body_bb, done_bb)
            .unwrap();

        self.builder.position_at_end(body_bb);
        let elem_ptr = unsafe {
            self.builder
                .build_gep(i32_ty, data, &[idx], "sfb_elem")
                .unwrap()
        };
        let byte32 = self
            .builder
            .build_load(i32_ty, elem_ptr, "sfb_byte32")
            .unwrap()
            .into_int_value();
        // 负值(超 127 的无符号字节在 i32 里以补码出现)先按位截到 8 位。
        let byte8 = self
            .builder
            .build_and(
                byte32,
                i32_ty.const_int(255, false),
                "sfb_byte_masked",
            )
            .unwrap();
        let byte_trunc = self
            .builder
            .build_int_truncate(byte8, self.context.i8_type(), "sfb_byte")
            .unwrap();
        let byte_ptr = unsafe {
            self.builder
                .build_gep(self.context.i8_type(), buf, &[idx], "sfb_out")
                .unwrap()
        };
        self.builder.build_store(byte_ptr, byte_trunc).unwrap();
        let next = self
            .builder
            .build_int_add(idx, one, "sfb_next")
            .unwrap();
        self.builder.build_store(idx_alloca, next).unwrap();
        self.builder.build_unconditional_branch(loop_bb).unwrap();

        self.builder.position_at_end(done_bb);
        let null_ptr = unsafe {
            self.builder
                .build_gep(self.context.i8_type(), buf, &[len], "sfb_term")
                .unwrap()
        };
        self.builder
            .build_store(null_ptr, self.context.i8_type().const_int(0, false))
            .unwrap();
        Ok(buf.into())
    }
}
