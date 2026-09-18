use super::CodeGen;
use huzi_error::Result;
use inkwell::AddressSpace;
use inkwell::values::PointerValue;

impl<'ctx> CodeGen<'ctx> {
    pub(super) fn prelude(&mut self) -> Result<()> {
        self.declare_libc_functions();
        self.declare_libm_functions();
        self.declare_net_functions();
        self.declare_thread_functions();
        self.declare_arg_support();
        if cfg!(windows) {
            self.declare_windows_argv_imports();
        }
        Ok(())
    }

    /// Declare the C runtime functions used by builtins (link to libc).
    fn declare_libc_functions(&mut self) {
        // printf for print function
        let print_fn = self.context.i32_type().fn_type(
            &[self
                .context
                .ptr_type(AddressSpace::default())
                .into()],
            true,
        );
        self.module.add_function("printf", print_fn, None);

        // scanf for input functions
        let scanf_fn = self.context.i32_type().fn_type(
            &[self
                .context
                .ptr_type(AddressSpace::default())
                .into()],
            true,
        );
        self.module.add_function("scanf", scanf_fn, None);

        // getchar for read_line
        let getchar_fn = self.context.i32_type().fn_type(&[], false);
        self.module.add_function("getchar", getchar_fn, None);

        // malloc for string allocation (returns i8*)
        let malloc_fn = self.context.ptr_type(inkwell::AddressSpace::default()).fn_type(
            &[self.context.i32_type().into()],
            false,
        );
        self.module.add_function("malloc", malloc_fn, None);

        // realloc for vec growth (ptr, new byte size) -> ptr
        let realloc_fn = self.context.ptr_type(inkwell::AddressSpace::default()).fn_type(
            &[
                self.context.ptr_type(AddressSpace::default()).into(),
                self.context.i32_type().into(),
            ],
            false,
        );
        self.module.add_function("realloc", realloc_fn, None);

        // free for manual release (free_str/free_vec/free_box)
        let free_fn = self.context.void_type().fn_type(
            &[self.context.ptr_type(AddressSpace::default()).into()],
            false,
        );
        self.module.add_function("free", free_fn, None);

        // sprintf for to_string
        let sprintf_fn = self.context.i32_type().fn_type(
            &[
                self.context.ptr_type(AddressSpace::default()).into(),
                self.context.ptr_type(AddressSpace::default()).into(),
            ],
            true,
        );
        self.module.add_function("sprintf", sprintf_fn, None);

        // strlen for string length
        let strlen_fn = self.context.i32_type().fn_type(
            &[self
                .context
                .ptr_type(AddressSpace::default())
                .into()],
            false,
        );
        self.module.add_function("strlen", strlen_fn, None);

        // strcmp for string comparison (==/!=/< etc. on str operands)
        let strcmp_fn = self.context.i32_type().fn_type(
            &[
                self.context.ptr_type(AddressSpace::default()).into(),
                self.context.ptr_type(AddressSpace::default()).into(),
            ],
            false,
        );
        self.module.add_function("strcmp", strcmp_fn, None);

        // strtoll for parse_int
        let strtoll_fn = self.context.i64_type().fn_type(
            &[
                self.context.ptr_type(AddressSpace::default()).into(),
                self.context.ptr_type(AddressSpace::default()).into(),
                self.context.i32_type().into(),
            ],
            false,
        );
        self.module.add_function("strtoll", strtoll_fn, None);

        // strtod for parse_float
        let strtod_fn = self.context.f64_type().fn_type(
            &[
                self.context.ptr_type(AddressSpace::default()).into(),
                self.context.ptr_type(AddressSpace::default()).into(),
            ],
            false,
        );
        self.module.add_function("strtod", strtod_fn, None);

        // getenv for env_get
        let getenv_fn = self.context.ptr_type(AddressSpace::default()).fn_type(
            &[self.context.ptr_type(AddressSpace::default()).into()],
            false,
        );
        self.module.add_function("getenv", getenv_fn, None);

        // exit for runtime error aborts (division by zero, out-of-bounds, ...)
        let exit_fn = self.context.void_type().fn_type(
            &[self.context.i32_type().into()],
            false,
        );
        self.module.add_function("exit", exit_fn, None);

        // rand/srand for pseudo-random numbers
        let rand_fn = self.context.i32_type().fn_type(&[], false);
        self.module.add_function("rand", rand_fn, None);
        let srand_fn = self
            .context
            .void_type()
            .fn_type(&[self.context.i32_type().into()], false);
        self.module.add_function("srand", srand_fn, None);

        // time for Unix timestamps (seconds); called with a null timer ptr
        let time_fn = self.context.i64_type().fn_type(
            &[self
                .context
                .ptr_type(AddressSpace::default())
                .into()],
            false,
        );
        self.module.add_function("time", time_fn, None);

        // Millisecond sleep: Sleep(DWORD ms) on Windows, usleep(usec) on POSIX.
        if cfg!(windows) {
            let sleep_fn = self
                .context
                .void_type()
                .fn_type(&[self.context.i32_type().into()], false);
            self.module.add_function("Sleep", sleep_fn, None);
        } else {
            let usleep_fn = self
                .context
                .void_type()
                .fn_type(&[self.context.i32_type().into()], false);
            self.module.add_function("usleep", usleep_fn, None);
        }

        // stdio for read_file/write_file (size_t is 64-bit on x86_64)
        let fopen_fn = self.context.ptr_type(AddressSpace::default()).fn_type(
            &[
                self.context.ptr_type(AddressSpace::default()).into(),
                self.context.ptr_type(AddressSpace::default()).into(),
            ],
            false,
        );
        self.module.add_function("fopen", fopen_fn, None);
        let fclose_fn = self.context.i32_type().fn_type(
            &[self.context.ptr_type(AddressSpace::default()).into()],
            false,
        );
        self.module.add_function("fclose", fclose_fn, None);
        let fread_fn = self.context.i64_type().fn_type(
            &[
                self.context.ptr_type(AddressSpace::default()).into(),
                self.context.i64_type().into(),
                self.context.i64_type().into(),
                self.context.ptr_type(AddressSpace::default()).into(),
            ],
            false,
        );
        self.module.add_function("fread", fread_fn, None);
        let fwrite_fn = self.context.i64_type().fn_type(
            &[
                self.context.ptr_type(AddressSpace::default()).into(),
                self.context.i64_type().into(),
                self.context.i64_type().into(),
                self.context.ptr_type(AddressSpace::default()).into(),
            ],
            false,
        );
        self.module.add_function("fwrite", fwrite_fn, None);
        let fseek_fn = self.context.i32_type().fn_type(
            &[
                self.context.ptr_type(AddressSpace::default()).into(),
                self.context.i64_type().into(),
                self.context.i32_type().into(),
            ],
            false,
        );
        self.module.add_function("fseek", fseek_fn, None);
        let ftell_fn = self.context.i32_type().fn_type(
            &[self.context.ptr_type(AddressSpace::default()).into()],
            false,
        );
        self.module.add_function("ftell", ftell_fn, None);

        // strcpy for string copy
        let strcpy_fn = self.context.i32_type().fn_type(
            &[
                self.context.ptr_type(AddressSpace::default()).into(),
                self.context.ptr_type(AddressSpace::default()).into(),
            ],
            false,
        );
        self.module.add_function("strcpy", strcpy_fn, None);

        // SetConsoleOutputCP (kernel32) for UTF-8 console output on Windows
        if cfg!(windows) {
            let set_cp_fn =
                self.context.i32_type().fn_type(&[self.context.i32_type().into()], false);
            self.module.add_function("SetConsoleOutputCP", set_cp_fn, None);
        }
    }

    /// Declare the math functions (link to libm).
    fn declare_libm_functions(&mut self) {
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

    /// Build a global "true"/"false" string selected by the given i1 condition.
    pub(super) fn build_bool_str(
        &mut self,
        cond: inkwell::values::IntValue<'ctx>,
    ) -> Result<PointerValue<'ctx>> {
        let true_ptr = match self.module.get_global("huzi_str_true") {
            Some(g) => g.as_pointer_value(),
            None => unsafe { self.builder.build_global_string("true", "huzi_str_true").unwrap() }
                .as_pointer_value(),
        };
        let false_ptr = match self.module.get_global("huzi_str_false") {
            Some(g) => g.as_pointer_value(),
            None => unsafe {
                self.builder
                    .build_global_string("false", "huzi_str_false")
                    .unwrap()
            }
            .as_pointer_value(),
        };

        let selected = self
            .builder
            .build_select(cond, true_ptr, false_ptr, "bool_str")
            .unwrap();

        Ok(selected.into_pointer_value())
    }

    /// Malloc a string buffer of the given size.
    pub(super) fn alloc_str_buffer(&mut self, size: u64) -> Result<PointerValue<'ctx>> {
        let malloc_fn = self.module.get_function("malloc").unwrap();
        let buffer_size = self.context.i32_type().const_int(size, false);
        let buffer = self
            .builder
            .build_call(malloc_fn, &[buffer_size.into()], "str_buffer")
            .unwrap()
            .try_as_basic_value()
            .unwrap_left()
            .into_pointer_value();
        Ok(buffer)
    }

    /// Declare network functions (link to ws2_32 on Windows, libc on POSIX).
    fn declare_net_functions(&mut self) {
        let i32_ty = self.context.i32_type();
        let i64_ty = self.context.i64_type();
        let ptr_ty = self.context.ptr_type(AddressSpace::default());

        if cfg!(windows) {
            let wsa_fn = i32_ty.fn_type(&[i32_ty.into(), ptr_ty.into()], false);
            self.module.add_function("WSAStartup", wsa_fn, None);

            let sock_fn = i64_ty.fn_type(&[i32_ty.into(), i32_ty.into(), i32_ty.into()], false);
            self.module.add_function("socket", sock_fn, None);

            let close_fn = i32_ty.fn_type(&[i64_ty.into()], false);
            self.module.add_function("closesocket", close_fn, None);

            let bind_fn = i32_ty.fn_type(&[i64_ty.into(), ptr_ty.into(), i32_ty.into()], false);
            self.module.add_function("bind", bind_fn, None);

            let listen_fn = i32_ty.fn_type(&[i64_ty.into(), i32_ty.into()], false);
            self.module.add_function("listen", listen_fn, None);

            let accept_fn = i64_ty.fn_type(&[i64_ty.into(), ptr_ty.into(), ptr_ty.into()], false);
            self.module.add_function("accept", accept_fn, None);

            let conn_fn = i32_ty.fn_type(&[i64_ty.into(), ptr_ty.into(), i32_ty.into()], false);
            self.module.add_function("connect", conn_fn, None);

            let send_fn = i32_ty.fn_type(&[i64_ty.into(), ptr_ty.into(), i32_ty.into(), i32_ty.into()], false);
            self.module.add_function("send", send_fn, None);

            let recv_fn = i32_ty.fn_type(&[i64_ty.into(), ptr_ty.into(), i32_ty.into(), i32_ty.into()], false);
            self.module.add_function("recv", recv_fn, None);
        } else {
            let sock_fn = i32_ty.fn_type(&[i32_ty.into(), i32_ty.into(), i32_ty.into()], false);
            self.module.add_function("socket", sock_fn, None);

            let close_fn = i32_ty.fn_type(&[i32_ty.into()], false);
            self.module.add_function("close", close_fn, None);

            let bind_fn = i32_ty.fn_type(&[i32_ty.into(), ptr_ty.into(), i32_ty.into()], false);
            self.module.add_function("bind", bind_fn, None);

            let listen_fn = i32_ty.fn_type(&[i32_ty.into(), i32_ty.into()], false);
            self.module.add_function("listen", listen_fn, None);

            let accept_fn = i32_ty.fn_type(&[i32_ty.into(), ptr_ty.into(), ptr_ty.into()], false);
            self.module.add_function("accept", accept_fn, None);

            let conn_fn = i32_ty.fn_type(&[i32_ty.into(), ptr_ty.into(), i32_ty.into()], false);
            self.module.add_function("connect", conn_fn, None);

            let send_fn = i64_ty.fn_type(&[i32_ty.into(), ptr_ty.into(), i64_ty.into(), i32_ty.into()], false);
            self.module.add_function("send", send_fn, None);

            let recv_fn = i64_ty.fn_type(&[i32_ty.into(), ptr_ty.into(), i64_ty.into(), i32_ty.into()], false);
            self.module.add_function("recv", recv_fn, None);
        }

        let inet_fn = i32_ty.fn_type(&[ptr_ty.into()], false);
        self.module.add_function("inet_addr", inet_fn, None);
    }

    /// Declare threading functions (link to kernel32 on Windows, lpthread on POSIX).
    fn declare_thread_functions(&mut self) {
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
