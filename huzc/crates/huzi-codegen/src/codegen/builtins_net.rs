//! 网络通信内置函数 (TCP 同步客户端与服务端)。
//!
//! 提供 `tcp_connect`、`tcp_send`、`tcp_recv`、`tcp_close`、
//! `tcp_listen`、`tcp_accept`。
//! 在 Windows 下走 Winsock (ws2_32.lib),在 POSIX 下走标准 socket API。

use huzi_ast::Expr;
use huzi_error::{HuziError, Result};
use inkwell::AddressSpace;
use inkwell::values::{BasicValueEnum, IntValue};

use super::CodeGen;

impl<'ctx> CodeGen<'ctx> {
    fn ensure_wsa_startup(&mut self) {
        if cfg!(windows) {
            let wsa_fn = self.module.get_function("WSAStartup").expect("WSAStartup");
            let wsa_data = self
                .builder
                .build_alloca(self.context.i8_type().array_type(512), "wsa_data")
                .unwrap();
            let ver = self.context.i32_type().const_int(0x0202, false);
            self.builder
                .build_call(wsa_fn, &[ver.into(), wsa_data.into()], "wsa_init")
                .unwrap();
        }
    }

    fn sockaddr_in_type(&self) -> inkwell::types::StructType<'ctx> {
        self.context.struct_type(
            &[
                self.context.i16_type().into(), // sin_family
                self.context.i16_type().into(), // sin_port
                self.context.i32_type().into(), // sin_addr
                self.context.i64_type().into(), // sin_zero
            ],
            false,
        )
    }

    fn build_htons(&mut self, port: IntValue<'ctx>) -> IntValue<'ctx> {
        let p16 = self
            .builder
            .build_int_truncate(port, self.context.i16_type(), "p16")
            .unwrap();
        let c8 = self.context.i16_type().const_int(8, false);
        let cff = self.context.i16_type().const_int(0x00FF, false);
        let low = self.builder.build_and(p16, cff, "low").unwrap();
        let low_sh = self.builder.build_left_shift(low, c8, "low_sh").unwrap();
        let high_sh = self
            .builder
            .build_right_shift(p16, c8, false, "high_sh")
            .unwrap();
        self.builder.build_or(low_sh, high_sh, "port_be").unwrap()
    }

    /// `tcp_connect(host: str, port: i32) -> i32`
    pub(super) fn compile_tcp_connect(
        &mut self,
        arguments: &[Expr],
    ) -> Result<BasicValueEnum<'ctx>> {
        if arguments.len() != 2 {
            return Err(HuziError::new_global(
                "tcp_connect() requires 2 arguments: (host: str, port: i32)",
            ));
        }
        self.ensure_wsa_startup();

        let host_val = match self.compile_expr(&arguments[0])? {
            BasicValueEnum::PointerValue(p) => p,
            _ => return Err(HuziError::new_global("tcp_connect() host must be a string")),
        };
        let port_val = match self.compile_expr(&arguments[1])? {
            BasicValueEnum::IntValue(i) => i,
            _ => return Err(HuziError::new_global("tcp_connect() port must be an integer")),
        };

        let ip_addr = self.resolve_ip(host_val);
        let port_be = self.build_htons(port_val);

        let (sock_i32, is_valid) = self.create_tcp_socket();

        let sin_ty = self.sockaddr_in_type();
        let sin_alloca = self.builder.build_alloca(sin_ty, "sin").unwrap();
        self.fill_sockaddr(sin_ty, sin_alloca, port_be, ip_addr);

        let conn_fn = self.module.get_function("connect").unwrap();
        let sin_len = self.context.i32_type().const_int(16, false);
        let sock_arg = self.sock_to_param(sock_i32);
        let conn_ret = self
            .builder
            .build_call(conn_fn, &[sock_arg.into(), sin_alloca.into(), sin_len.into()], "conn_ret")
            .unwrap()
            .try_as_basic_value()
            .unwrap_left()
            .into_int_value();

        let zero = self.context.i32_type().const_int(0, false);
        let conn_ok = self
            .builder
            .build_int_compare(inkwell::IntPredicate::EQ, conn_ret, zero, "conn_ok")
            .unwrap();
        let both_ok = self.builder.build_and(is_valid, conn_ok, "both_ok").unwrap();

        let neg_one = self.context.i32_type().const_int(-1i32 as u64, true);
        let final_res = self
            .builder
            .build_select(both_ok, sock_i32, neg_one, "connect_res")
            .unwrap();
        Ok(final_res)
    }

    fn resolve_ip(&mut self, host: inkwell::values::PointerValue<'ctx>) -> IntValue<'ctx> {
        let strcmp_fn = self.module.get_function("strcmp").unwrap();
        let local_str = match self.module.get_global("huzi_str_localhost") {
            Some(g) => g.as_pointer_value(),
            None => unsafe {
                self.builder
                    .build_global_string("localhost", "huzi_str_localhost")
                    .unwrap()
            }
            .as_pointer_value(),
        };
        let cmp = self
            .builder
            .build_call(strcmp_fn, &[host.into(), local_str.into()], "is_local")
            .unwrap()
            .try_as_basic_value()
            .unwrap_left()
            .into_int_value();
        let is_local = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                cmp,
                self.context.i32_type().const_int(0, false),
                "is_local_b",
            )
            .unwrap();

        let inet_fn = self.module.get_function("inet_addr").unwrap();
        let parsed = self
            .builder
            .build_call(inet_fn, &[host.into()], "parsed_ip")
            .unwrap()
            .try_as_basic_value()
            .unwrap_left()
            .into_int_value();
        let loopback = self.context.i32_type().const_int(0x0100007F, false);
        self.builder
            .build_select(is_local, loopback, parsed, "ip_addr")
            .unwrap()
            .into_int_value()
    }

    fn create_tcp_socket(&mut self) -> (IntValue<'ctx>, IntValue<'ctx>) {
        let sock_fn = self.module.get_function("socket").unwrap();
        let af = self.context.i32_type().const_int(2, false);
        let st = self.context.i32_type().const_int(1, false);
        let proto = self.context.i32_type().const_int(0, false);
        let sock_raw = self
            .builder
            .build_call(sock_fn, &[af.into(), st.into(), proto.into()], "sock")
            .unwrap()
            .try_as_basic_value()
            .unwrap_left();

        if cfg!(windows) {
            let sock_i64 = sock_raw.into_int_value();
            let sock_trunc = self
                .builder
                .build_int_truncate(sock_i64, self.context.i32_type(), "sock_i32")
                .unwrap();
            let invalid = self.context.i64_type().const_int(u64::MAX, false);
            let valid = self
                .builder
                .build_int_compare(inkwell::IntPredicate::NE, sock_i64, invalid, "sock_valid")
                .unwrap();
            (sock_trunc, valid)
        } else {
            let sock_val = sock_raw.into_int_value();
            let zero = self.context.i32_type().const_int(0, false);
            let valid = self
                .builder
                .build_int_compare(inkwell::IntPredicate::SGE, sock_val, zero, "sock_valid")
                .unwrap();
            (sock_val, valid)
        }
    }

    fn fill_sockaddr(
        &mut self,
        sin_ty: inkwell::types::StructType<'ctx>,
        sin_alloca: inkwell::values::PointerValue<'ctx>,
        port_be: IntValue<'ctx>,
        ip_addr: IntValue<'ctx>,
    ) {
        let f0 = self.builder.build_struct_gep(sin_ty, sin_alloca, 0, "sin_family").unwrap();
        self.builder.build_store(f0, self.context.i16_type().const_int(2, false)).unwrap();
        let f1 = self.builder.build_struct_gep(sin_ty, sin_alloca, 1, "sin_port").unwrap();
        self.builder.build_store(f1, port_be).unwrap();
        let f2 = self.builder.build_struct_gep(sin_ty, sin_alloca, 2, "sin_addr").unwrap();
        self.builder.build_store(f2, ip_addr).unwrap();
        let f3 = self.builder.build_struct_gep(sin_ty, sin_alloca, 3, "sin_zero").unwrap();
        self.builder.build_store(f3, self.context.i64_type().const_int(0, false)).unwrap();
    }

    fn sock_to_param(&mut self, sock: IntValue<'ctx>) -> BasicValueEnum<'ctx> {
        if cfg!(windows) {
            self.builder
                .build_int_s_extend(sock, self.context.i64_type(), "sock_i64")
                .unwrap()
                .into()
        } else {
            sock.into()
        }
    }

    /// `tcp_send(sock: i32, data: str) -> i32`
    pub(super) fn compile_tcp_send(
        &mut self,
        arguments: &[Expr],
    ) -> Result<BasicValueEnum<'ctx>> {
        if arguments.len() != 2 {
            return Err(HuziError::new_global(
                "tcp_send() requires 2 arguments: (sock: i32, data: str)",
            ));
        }
        let sock_val = match self.compile_expr(&arguments[0])? {
            BasicValueEnum::IntValue(i) => i,
            _ => return Err(HuziError::new_global("tcp_send() sock must be an integer")),
        };
        let data_val = match self.compile_expr(&arguments[1])? {
            BasicValueEnum::PointerValue(p) => p,
            _ => return Err(HuziError::new_global("tcp_send() data must be a string")),
        };

        let strlen_fn = self.module.get_function("strlen").unwrap();
        let len = self
            .builder
            .build_call(strlen_fn, &[data_val.into()], "data_len")
            .unwrap()
            .try_as_basic_value()
            .unwrap_left()
            .into_int_value();

        let sock_arg = self.sock_to_param(sock_val);
        let send_fn = self.module.get_function("send").unwrap();
        let flags = self.context.i32_type().const_int(0, false);
        let len_arg = if cfg!(windows) {
            len.into()
        } else {
            self.builder
                .build_int_z_extend(len, self.context.i64_type(), "len_i64")
                .unwrap()
                .into()
        };

        let sent = self
            .builder
            .build_call(send_fn, &[sock_arg.into(), data_val.into(), len_arg, flags.into()], "sent")
            .unwrap()
            .try_as_basic_value()
            .unwrap_left();

        if !cfg!(windows) {
            let sent_i64 = sent.into_int_value();
            let sent_trunc = self
                .builder
                .build_int_truncate(sent_i64, self.context.i32_type(), "sent_i32")
                .unwrap();
            Ok(sent_trunc.into())
        } else {
            Ok(sent)
        }
    }

    /// `tcp_recv(sock: i32, max_len: i32) -> str`
    pub(super) fn compile_tcp_recv(
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

    /// `tcp_close(sock: i32) -> i32`
    pub(super) fn compile_tcp_close(
        &mut self,
        arguments: &[Expr],
    ) -> Result<BasicValueEnum<'ctx>> {
        let sock_val = self.i32_builtin_arg(arguments, "tcp_close")?;
        self.emit_close_sock(sock_val);
        Ok(self.context.i32_type().const_int(0, false).into())
    }

    fn emit_close_sock(&mut self, sock_val: IntValue<'ctx>) {
        if cfg!(windows) {
            let close_fn = self.module.get_function("closesocket").unwrap();
            let sock_i64 = self
                .builder
                .build_int_s_extend(sock_val, self.context.i64_type(), "sock_i64")
                .unwrap();
            self.builder
                .build_call(close_fn, &[sock_i64.into()], "close_call")
                .unwrap();
        } else {
            let close_fn = self.module.get_function("close").unwrap();
            self.builder
                .build_call(close_fn, &[sock_val.into()], "close_call")
                .unwrap();
        }
    }

    /// `tcp_listen(port: i32) -> i32`
    pub(super) fn compile_tcp_listen(
        &mut self,
        arguments: &[Expr],
    ) -> Result<BasicValueEnum<'ctx>> {
        let port_val = self.i32_builtin_arg(arguments, "tcp_listen")?;
        self.ensure_wsa_startup();

        let port_be = self.build_htons(port_val);
        let (sock_i32, is_valid) = self.create_tcp_socket();

        let sin_ty = self.sockaddr_in_type();
        let sin_alloca = self.builder.build_alloca(sin_ty, "sin").unwrap();
        let zero_ip = self.context.i32_type().const_int(0, false);
        self.fill_sockaddr(sin_ty, sin_alloca, port_be, zero_ip);

        let sock_arg = self.sock_to_param(sock_i32);
        let bind_fn = self.module.get_function("bind").unwrap();
        let sin_len = self.context.i32_type().const_int(16, false);
        let bind_ret = self
            .builder
            .build_call(bind_fn, &[sock_arg.into(), sin_alloca.into(), sin_len.into()], "bind_ret")
            .unwrap()
            .try_as_basic_value()
            .unwrap_left()
            .into_int_value();

        let listen_fn = self.module.get_function("listen").unwrap();
        let backlog = self.context.i32_type().const_int(5, false);
        let listen_ret = self
            .builder
            .build_call(listen_fn, &[sock_arg.into(), backlog.into()], "listen_ret")
            .unwrap()
            .try_as_basic_value()
            .unwrap_left()
            .into_int_value();

        let zero = self.context.i32_type().const_int(0, false);
        let bind_ok = self
            .builder
            .build_int_compare(inkwell::IntPredicate::EQ, bind_ret, zero, "bind_ok")
            .unwrap();
        let listen_ok = self
            .builder
            .build_int_compare(inkwell::IntPredicate::EQ, listen_ret, zero, "listen_ok")
            .unwrap();
        let ok = self
            .builder
            .build_and(is_valid, self.builder.build_and(bind_ok, listen_ok, "bl_ok").unwrap(), "all_ok")
            .unwrap();

        let neg_one = self.context.i32_type().const_int(-1i32 as u64, true);
        let final_res = self
            .builder
            .build_select(ok, sock_i32, neg_one, "tcp_listen_res")
            .unwrap();
        Ok(final_res)
    }

    /// `tcp_accept(listener: i32) -> i32`
    pub(super) fn compile_tcp_accept(
        &mut self,
        arguments: &[Expr],
    ) -> Result<BasicValueEnum<'ctx>> {
        let sock_val = self.i32_builtin_arg(arguments, "tcp_accept")?;
        let sock_arg = self.sock_to_param(sock_val);

        let accept_fn = self.module.get_function("accept").unwrap();
        let null_ptr = self.context.ptr_type(AddressSpace::default()).const_null();
        let client_raw = self
            .builder
            .build_call(accept_fn, &[sock_arg.into(), null_ptr.into(), null_ptr.into()], "client")
            .unwrap()
            .try_as_basic_value()
            .unwrap_left();

        if cfg!(windows) {
            let client_i64 = client_raw.into_int_value();
            let client_trunc = self
                .builder
                .build_int_truncate(client_i64, self.context.i32_type(), "client_i32")
                .unwrap();
            Ok(client_trunc.into())
        } else {
            Ok(client_raw)
        }
    }
}
