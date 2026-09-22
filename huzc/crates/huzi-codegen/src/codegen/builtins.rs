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

    /// 声明内置函数使用的 C 运行时函数（链接到 libc）。
    fn declare_libc_functions(&mut self) {
        // 用于 print 函数的 printf
        let print_fn = self.context.i32_type().fn_type(
            &[self
                .context
                .ptr_type(AddressSpace::default())
                .into()],
            true,
        );
        self.module.add_function("printf", print_fn, None);

        // 用于输入函数的 scanf
        let scanf_fn = self.context.i32_type().fn_type(
            &[self
                .context
                .ptr_type(AddressSpace::default())
                .into()],
            true,
        );
        self.module.add_function("scanf", scanf_fn, None);

        // 用于 read_line 的 getchar
        let getchar_fn = self.context.i32_type().fn_type(&[], false);
        self.module.add_function("getchar", getchar_fn, None);

        // 用于字符串分配的 malloc（返回 i8*）
        let malloc_fn = self.context.ptr_type(inkwell::AddressSpace::default()).fn_type(
            &[self.context.i32_type().into()],
            false,
        );
        self.module.add_function("malloc", malloc_fn, None);

        // 用于 vec 扩容的 realloc（ptr, new byte size）-> ptr
        let realloc_fn = self.context.ptr_type(inkwell::AddressSpace::default()).fn_type(
            &[
                self.context.ptr_type(AddressSpace::default()).into(),
                self.context.i32_type().into(),
            ],
            false,
        );
        self.module.add_function("realloc", realloc_fn, None);

        // 用于手动释放的 free（free_str/free_vec/free_box）
        let free_fn = self.context.void_type().fn_type(
            &[self.context.ptr_type(AddressSpace::default()).into()],
            false,
        );
        self.module.add_function("free", free_fn, None);

        // 用于 to_string 的 sprintf
        let sprintf_fn = self.context.i32_type().fn_type(
            &[
                self.context.ptr_type(AddressSpace::default()).into(),
                self.context.ptr_type(AddressSpace::default()).into(),
            ],
            true,
        );
        self.module.add_function("sprintf", sprintf_fn, None);

        // 用于获取字符串长度的 strlen
        let strlen_fn = self.context.i32_type().fn_type(
            &[self
                .context
                .ptr_type(AddressSpace::default())
                .into()],
            false,
        );
        self.module.add_function("strlen", strlen_fn, None);

        // 用于字符串比较的 strcmp（str 操作数上的 ==/!=/< 等）
        let strcmp_fn = self.context.i32_type().fn_type(
            &[
                self.context.ptr_type(AddressSpace::default()).into(),
                self.context.ptr_type(AddressSpace::default()).into(),
            ],
            false,
        );
        self.module.add_function("strcmp", strcmp_fn, None);

        // 用于 parse_int 的 strtoll
        let strtoll_fn = self.context.i64_type().fn_type(
            &[
                self.context.ptr_type(AddressSpace::default()).into(),
                self.context.ptr_type(AddressSpace::default()).into(),
                self.context.i32_type().into(),
            ],
            false,
        );
        self.module.add_function("strtoll", strtoll_fn, None);

        // 用于 parse_float 的 strtod
        let strtod_fn = self.context.f64_type().fn_type(
            &[
                self.context.ptr_type(AddressSpace::default()).into(),
                self.context.ptr_type(AddressSpace::default()).into(),
            ],
            false,
        );
        self.module.add_function("strtod", strtod_fn, None);

        // 用于 env_get 的 getenv
        let getenv_fn = self.context.ptr_type(AddressSpace::default()).fn_type(
            &[self.context.ptr_type(AddressSpace::default()).into()],
            false,
        );
        self.module.add_function("getenv", getenv_fn, None);

        // 用于时间戳格式化的 localtime 和 strftime
        let localtime_fn = self.context.ptr_type(AddressSpace::default()).fn_type(
            &[self.context.ptr_type(AddressSpace::default()).into()],
            false,
        );
        self.module.add_function("localtime", localtime_fn, None);

        let strftime_fn = self.context.i64_type().fn_type(
            &[
                self.context.ptr_type(AddressSpace::default()).into(),
                self.context.i64_type().into(),
                self.context.ptr_type(AddressSpace::default()).into(),
                self.context.ptr_type(AddressSpace::default()).into(),
            ],
            false,
        );
        self.module.add_function("strftime", strftime_fn, None);

        // 用于运行时错误中止的 exit（除以零、越界等）
        let exit_fn = self.context.void_type().fn_type(
            &[self.context.i32_type().into()],
            false,
        );
        self.module.add_function("exit", exit_fn, None);

        // 用于伪随机数的 rand/srand
        let rand_fn = self.context.i32_type().fn_type(&[], false);
        self.module.add_function("rand", rand_fn, None);
        let srand_fn = self
            .context
            .void_type()
            .fn_type(&[self.context.i32_type().into()], false);
        self.module.add_function("srand", srand_fn, None);

        // 用于 Unix 时间戳（秒）的 time；传入空指针调用
        let time_fn = self.context.i64_type().fn_type(
            &[self
                .context
                .ptr_type(AddressSpace::default())
                .into()],
            false,
        );
        self.module.add_function("time", time_fn, None);

        // 毫秒级睡眠：Windows 下为 Sleep(DWORD ms)，POSIX 下为 usleep(usec)。
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

        // 用于 read_file/write_file 的 stdio 函数（x86_64 上 size_t 为 64 位）
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

        // 用于字符串拷贝的 strcpy
        let strcpy_fn = self.context.i32_type().fn_type(
            &[
                self.context.ptr_type(AddressSpace::default()).into(),
                self.context.ptr_type(AddressSpace::default()).into(),
            ],
            false,
        );
        self.module.add_function("strcpy", strcpy_fn, None);

        // 用于 Windows 上 UTF-8 控制台输出的 SetConsoleOutputCP (kernel32)
        if cfg!(windows) {
            let set_cp_fn =
                self.context.i32_type().fn_type(&[self.context.i32_type().into()], false);
            self.module.add_function("SetConsoleOutputCP", set_cp_fn, None);
        }
    }

    /// 声明数学函数（链接到 libm）。
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

    /// 根据给定的 i1 条件选择并构建全局 "true"/"false" 字符串。
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

    /// 通过 malloc 分配给定大小的字符串缓冲区。
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

    /// 声明网络函数（Windows 链接到 ws2_32，POSIX 链接到 libc）。
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

    /// 声明线程函数（Windows 链接到 kernel32，POSIX 链接到 lpthread）。
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
