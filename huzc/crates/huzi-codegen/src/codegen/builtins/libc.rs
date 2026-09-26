//! C 运行时函数声明(printf/scanf/malloc/文件/管道等,链接到 libc)。(自 `builtins.rs` 纯搬移,零逻辑变化)。

use super::super::CodeGen;
use inkwell::AddressSpace;

impl<'ctx> CodeGen<'ctx> {
    /// 声明内置函数使用的 C 运行时函数（链接到 libc）。
    pub(super) fn declare_libc_functions(&mut self) {
        self.declare_print_input();
        self.declare_alloc_str();
        self.declare_parse_env();
        self.declare_exit_time();
        self.declare_file_io();
        self.declare_pipe_str();
    }

    /// 输入输出:printf/scanf/getchar。(自 `declare_libc_functions` 提炼,声明逐行原样搬移,零逻辑变化。)
    fn declare_print_input(&mut self) {
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
    }

    /// 堆与字符串:malloc/realloc/free/sprintf/strlen/strcmp。(自 `declare_libc_functions` 提炼,声明逐行原样搬移,零逻辑变化。)
    fn declare_alloc_str(&mut self) {
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
    }

    /// 解析与环境:strtoll/strtod/getenv/localtime/strftime。(自 `declare_libc_functions` 提炼,声明逐行原样搬移,零逻辑变化。)
    fn declare_parse_env(&mut self) {
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
    }

    /// 退出与时间:exit/rand/srand/time/sleep。(自 `declare_libc_functions` 提炼,声明逐行原样搬移,零逻辑变化。)
    fn declare_exit_time(&mut self) {
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
    }

    /// 文件读写:fopen/fclose/fread/fwrite/fseek/ftell。(自 `declare_libc_functions` 提炼,声明逐行原样搬移,零逻辑变化。)
    fn declare_file_io(&mut self) {
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
    }

    /// 管道与字符串:popen/pclose/strcpy/SetConsoleOutputCP。(自 `declare_libc_functions` 提炼,声明逐行原样搬移,零逻辑变化。)
    fn declare_pipe_str(&mut self) {
        // 用于子进程管道的 popen 与 pclose
        let popen_fn_ty = self.context.ptr_type(AddressSpace::default()).fn_type(
            &[
                self.context.ptr_type(AddressSpace::default()).into(),
                self.context.ptr_type(AddressSpace::default()).into(),
            ],
            false,
        );
        let pclose_fn_ty = self.context.i32_type().fn_type(
            &[self.context.ptr_type(AddressSpace::default()).into()],
            false,
        );
        if cfg!(windows) {
            self.module.add_function("_popen", popen_fn_ty, None);
            self.module.add_function("_pclose", pclose_fn_ty, None);
        } else {
            self.module.add_function("popen", popen_fn_ty, None);
            self.module.add_function("pclose", pclose_fn_ty, None);
        }

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
}
