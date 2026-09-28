//! 数学/网络/线程函数声明(链接到 libm/ws2_32/libc/pthread)。(自 `builtins.rs` 纯搬移,零逻辑变化)。

use super::super::CodeGen;
use inkwell::AddressSpace;

impl<'ctx> CodeGen<'ctx> {
    /// 声明数学函数（链接到 libm）。
    pub(super) fn declare_libm_functions(&mut self) {
        let sqrt_fn = self.context.f64_type().fn_type(&[self.context.f64_type().into()], false);
        self.module.add_function("sqrt", sqrt_fn, None);

        let pow_fn = self.context.f64_type().fn_type(
            &[
                self.context.f64_type().into(),
                self.context.f64_type().into(),
            ],
            false,
        );
        self.module.add_function("pow", pow_fn, None);

        let sin_fn = self.context.f64_type().fn_type(&[self.context.f64_type().into()], false);
        self.module.add_function("sin", sin_fn, None);

        let cos_fn = self.context.f64_type().fn_type(&[self.context.f64_type().into()], false);
        self.module.add_function("cos", cos_fn, None);

        let fabs_fn = self.context.f64_type().fn_type(&[self.context.f64_type().into()], false);
        self.module.add_function("fabs", fabs_fn, None);

        for name in ["tan", "floor", "ceil", "round"] {
            let f = self.context.f64_type().fn_type(&[self.context.f64_type().into()], false);
            self.module.add_function(name, f, None);
        }
    }

    /// 声明网络函数（Windows 链接到 ws2_32，POSIX 链接到 libc）。
    /// 套接字句柄类型按平台取 `sock_ty`(Windows 为 i64,POSIX 为 i32),
    /// 句柄首参的六个声明复用该类型;仅 `send`/`recv` 的长度与返回值另有差异。
    pub(super) fn declare_net_functions(&mut self) {
        let i32_ty = self.context.i32_type();
        let i64_ty = self.context.i64_type();
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let sock_ty: inkwell::types::BasicMetadataTypeEnum<'ctx> =
            if cfg!(windows) { i64_ty.into() } else { i32_ty.into() };
        let close_name = if cfg!(windows) { "closesocket" } else { "close" };

        if cfg!(windows) {
            let wsa_fn = i32_ty.fn_type(&[i32_ty.into(), ptr_ty.into()], false);
            self.module.add_function("WSAStartup", wsa_fn, None);
        }

        let sock_fn = if cfg!(windows) {
            i64_ty.fn_type(&[i32_ty.into(), i32_ty.into(), i32_ty.into()], false)
        } else {
            i32_ty.fn_type(&[i32_ty.into(), i32_ty.into(), i32_ty.into()], false)
        };
        self.module.add_function("socket", sock_fn, None);

        let close_fn = i32_ty.fn_type(&[sock_ty], false);
        self.module.add_function(close_name, close_fn, None);

        let bind_fn = i32_ty.fn_type(&[sock_ty, ptr_ty.into(), i32_ty.into()], false);
        self.module.add_function("bind", bind_fn, None);

        let listen_fn = i32_ty.fn_type(&[sock_ty, i32_ty.into()], false);
        self.module.add_function("listen", listen_fn, None);

        let accept_fn = if cfg!(windows) {
            i64_ty.fn_type(&[sock_ty, ptr_ty.into(), ptr_ty.into()], false)
        } else {
            i32_ty.fn_type(&[sock_ty, ptr_ty.into(), ptr_ty.into()], false)
        };
        self.module.add_function("accept", accept_fn, None);

        let conn_fn = i32_ty.fn_type(&[sock_ty, ptr_ty.into(), i32_ty.into()], false);
        self.module.add_function("connect", conn_fn, None);

        if cfg!(windows) {
            let send_fn = i32_ty.fn_type(&[i64_ty.into(), ptr_ty.into(), i32_ty.into(), i32_ty.into()], false);
            self.module.add_function("send", send_fn, None);

            let recv_fn = i32_ty.fn_type(&[i64_ty.into(), ptr_ty.into(), i32_ty.into(), i32_ty.into()], false);
            self.module.add_function("recv", recv_fn, None);
        } else {
            let send_fn = i64_ty.fn_type(&[i32_ty.into(), ptr_ty.into(), i64_ty.into(), i32_ty.into()], false);
            self.module.add_function("send", send_fn, None);

            let recv_fn = i64_ty.fn_type(&[i32_ty.into(), ptr_ty.into(), i64_ty.into(), i32_ty.into()], false);
            self.module.add_function("recv", recv_fn, None);
        }

        let inet_fn = i32_ty.fn_type(&[ptr_ty.into()], false);
        self.module.add_function("inet_addr", inet_fn, None);
    }

    /// 声明线程函数（Windows 链接到 kernel32，POSIX 链接到 lpthread）。
    pub(super) fn declare_thread_functions(&mut self) {
        let i32_ty = self.context.i32_type();
        let i64_ty = self.context.i64_type();
        let ptr_ty = self.context.ptr_type(AddressSpace::default());

        if cfg!(windows) {
            let ct_fn = ptr_ty.fn_type(
                &[
                    ptr_ty.into(),
                    i64_ty.into(),
                    ptr_ty.into(),
                    ptr_ty.into(),
                    i32_ty.into(),
                    ptr_ty.into(),
                ],
                false,
            );
            self.module.add_function("CreateThread", ct_fn, None);

            let wfso_fn = i32_ty.fn_type(&[ptr_ty.into(), i32_ty.into()], false);
            self.module.add_function("WaitForSingleObject", wfso_fn, None);

            let gect_fn = i32_ty.fn_type(&[ptr_ty.into(), ptr_ty.into()], false);
            self.module.add_function("GetExitCodeThread", gect_fn, None);

            let ch_fn = i32_ty.fn_type(&[ptr_ty.into()], false);
            self.module.add_function("CloseHandle", ch_fn, None);
        } else {
            let pc_fn = i32_ty.fn_type(
                &[ptr_ty.into(), ptr_ty.into(), ptr_ty.into(), ptr_ty.into()],
                false,
            );
            self.module.add_function("pthread_create", pc_fn, None);

            let pj_fn = i32_ty.fn_type(&[i64_ty.into(), ptr_ty.into()], false);
            self.module.add_function("pthread_join", pj_fn, None);
        }
    }
}
