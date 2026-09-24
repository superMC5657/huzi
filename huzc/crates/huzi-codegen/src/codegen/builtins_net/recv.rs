//! `tcp_recv` 接收实现(自 `builtins_net.rs` 纯搬移,零逻辑变化)。

use super::CodeGen;
use huzi_ast::Expr;
use huzi_error::{HuziError, Result};
use inkwell::values::BasicValueEnum;

impl<'ctx> CodeGen<'ctx> {
    /// 接收 TCP 数据：`tcp_recv(sock: i32, max_len: i32) -> str`
    pub(in crate::codegen) fn compile_tcp_recv(
        &mut self,
        arguments: &[Expr],
    ) -> Result<BasicValueEnum<'ctx>> {
        if arguments.len() != 2 {
            return Err(HuziError::new_global(
                "tcp_recv() requires 2 arguments: (sock: i32, max_len: i32)",
            ));
        }
        let sock_val = match self.compile_expr(&arguments[0])? {
            BasicValueEnum::IntValue(i) => i,
            _ => return Err(HuziError::new_global("tcp_recv() sock must be an integer")),
        };
        let max_len = match self.compile_expr(&arguments[1])? {
            BasicValueEnum::IntValue(i) => i,
            _ => return Err(HuziError::new_global("tcp_recv() max_len must be an integer")),
        };

        let one = self.context.i32_type().const_int(1, false);
        let buf_size = self.builder.build_int_add(max_len, one, "buf_size").unwrap();

        let malloc_fn = self.module.get_function("malloc").unwrap();
        let buf = self
            .builder
            .build_call(malloc_fn, &[buf_size.into()], "recv_buf")
            .unwrap()
            .try_as_basic_value()
            .unwrap_left()
            .into_pointer_value();

        let sock_arg = self.sock_to_param(sock_val);
        let recv_fn = self.module.get_function("recv").unwrap();
        let flags = self.context.i32_type().const_int(0, false);
        let len_arg = if cfg!(windows) {
            max_len.into()
        } else {
            self.builder
                .build_int_z_extend(max_len, self.context.i64_type(), "len_i64")
                .unwrap()
                .into()
        };

        let read_val = self
            .builder
            .build_call(recv_fn, &[sock_arg.into(), buf.into(), len_arg, flags.into()], "read_bytes")
            .unwrap()
            .try_as_basic_value()
            .unwrap_left()
            .into_int_value();

        let read_i32 = if !cfg!(windows) {
            self.builder
                .build_int_truncate(read_val, self.context.i32_type(), "read_i32")
                .unwrap()
        } else {
            read_val
        };

        let zero = self.context.i32_type().const_int(0, false);
        let is_pos = self
            .builder
            .build_int_compare(inkwell::IntPredicate::SGT, read_i32, zero, "is_pos")
            .unwrap();
        let term_idx = self
            .builder
            .build_select(is_pos, read_i32, zero, "term_idx")
            .unwrap()
            .into_int_value();

        let term_ptr = unsafe {
            self.builder
                .build_gep(self.context.i8_type(), buf, &[term_idx], "term_ptr")
                .unwrap()
        };
        self.builder
            .build_store(term_ptr, self.context.i8_type().const_int(0, false))
            .unwrap();

        Ok(buf.into())
    }
}
