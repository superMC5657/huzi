//! 仅限 Windows 的 UTF-8 argv 修复：从 Unicode 命令行重建 `huzi_argc`/`huzi_argv`。
//!
//! 在 Windows 上，CRT 传递给 `main(argc, argv)` 的是 ANSI 编码字节（当前控制台代码页，例如 GBK），
//! 因此当程序将其视为 UTF-8 时，非 ASCII 参数会出现乱码。此处发射的 IR 在程序启动时运行（紧跟在
//! 存储 ANSI 参数之后），通过 `GetCommandLineW`、`CommandLineToArgvW` 与
//! `WideCharToMultiByte(CP_UTF8)` 将命令行转换为 UTF-8 字符串并替换全局变量。
//! 本文件中的所有内容仅在 huzc 自身为 Windows 编译时才会发射；Unix 路径保持原有的 argv 不动。
//!
//! 转换后的缓冲区仅 `malloc` 一次并在进程生命周期内常驻，因此 `arg(i)` 保持其“指向 argv 存储的指针”
//! 这一零拷贝语义，越界或负索引仍返回共享的空字符串。如果获取 Unicode 失败，则保留 ANSI 原值（优雅降级）；
//! 单个参数转换失败则返回空字符串。

use inkwell::AddressSpace;
use inkwell::IntPredicate;
use inkwell::values::PointerValue;

use super::CodeGen;

/// 用于 `WideCharToMultiByte` 的 UTF-8 代码页（与控制台设置匹配）。
const CP_UTF8: u64 = 65001;

impl<'ctx> CodeGen<'ctx> {
    /// 声明 argv 转换所需的导入函数（kernel32 + shell32 + libc malloc）。
    /// 仅在为 Windows 编译时由 `prelude` 调用。
    pub(super) fn declare_windows_argv_imports(&mut self) {
        let i32_type = self.context.i32_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());

        // 获取命令行宽字符：LPWSTR GetCommandLineW(void);
        let cmdline_ty = ptr_type.fn_type(&[], false);
        self.module.add_function("GetCommandLineW", cmdline_ty, None);

        // 解析宽字符命令行：LPWSTR *CommandLineToArgvW(LPCWSTR, int *);
        let to_argv_ty = ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);
        self.module
            .add_function("CommandLineToArgvW", to_argv_ty, None);

        // 宽字符转多字节：int WideCharToMultiByte(UINT, DWORD, LPCWCH, int, LPSTR, int, ...);
        let convert_ty = i32_type.fn_type(
            &[
                i32_type.into(), // CodePage
                i32_type.into(), // dwFlags
                ptr_type.into(), // lpWideCharStr
                i32_type.into(), // cchWideChar（-1 表示以 NUL 结尾）
                ptr_type.into(), // lpMultiByteStr
                i32_type.into(), // cbMultiByte
                ptr_type.into(), // lpDefaultChar
                ptr_type.into(), // lpUsedDefaultChar
            ],
            false,
        );
        self.module
            .add_function("WideCharToMultiByte", convert_ty, None);

        // 释放局部内存：HLOCAL LocalFree(HLOCAL);
        let free_ty = ptr_type.fn_type(&[ptr_type.into()], false);
        self.module.add_function("LocalFree", free_ty, None);
    }

    /// 用来自 Unicode 命令行转换的 UTF-8 字符串替换 `huzi_argc`/`huzi_argv`。
    /// 在 `store_main_args` 中紧跟 ANSI 存储之后发射。
    pub(super) fn refresh_windows_argv_utf8(&mut self) {
        let i32_type = self.context.i32_type();
        let function = self
            .builder
            .get_insert_block()
            .and_then(|b| b.get_parent())
            .expect("argv fixup needs a current function");

        let cmdline_fn = self
            .module
            .get_function("GetCommandLineW")
            .expect("GetCommandLineW in prelude");
        let to_argv_fn = self
            .module
            .get_function("CommandLineToArgvW")
            .expect("CommandLineToArgvW in prelude");
        let free_fn = self
            .module
            .get_function("LocalFree")
            .expect("LocalFree in prelude");
        let malloc_fn = self.module.get_function("malloc").expect("malloc in prelude");

        // 解析宽字符参数：LPWSTR *wargv = CommandLineToArgvW(GetCommandLineW(), &n);
        let cmdline = self
            .builder
            .build_call(cmdline_fn, &[], "w_cmdline")
            .unwrap()
            .try_as_basic_value()
            .unwrap_left()
            .into_pointer_value();
        let n_slot = self.builder.build_alloca(i32_type, "w_argc").unwrap();
        let wargv = self
            .builder
            .build_call(
                to_argv_fn,
                &[cmdline.into(), n_slot.into()],
                "w_argv",
            )
            .unwrap()
            .try_as_basic_value()
            .unwrap_left()
            .into_pointer_value();

        // CommandLineToArgvW 返回 NULL 时：保留原样 ANSI 参数。
        let ok_block = self.context.append_basic_block(function, "utf8_argv_ok");
        let skip_block = self
            .context
            .append_basic_block(function, "utf8_argv_skip");
        let is_null = self.builder.build_is_null(wargv, "w_argv_null").unwrap();
        self.builder
            .build_conditional_branch(is_null, skip_block, ok_block)
            .unwrap();

        self.builder.position_at_end(ok_block);
        let n = self
            .builder
            .build_load(i32_type, n_slot, "w_argc_val")
            .unwrap()
            .into_int_value();
        let argc_global = self.windows_arg_global("huzi_argc");
        self.builder.build_store(argc_global, n).unwrap();
        let new_argv = self.alloc_argv_array(malloc_fn, n);
        self.fill_argv_array(wargv, new_argv, n);
        self.publish_argv_array(new_argv, wargv, free_fn, skip_block);

        self.builder.position_at_end(skip_block);
    }

    /// 以 NUL 结尾终止转换后的数组，将其发布到 `huzi_argv`，释放宽字符列表，
    /// 并在 `skip_block` 处重新汇入调用方流程。在 `fill_argv_array` 留下的循环退出基本块处发射。
    fn publish_argv_array(
        &mut self,
        new_argv: PointerValue<'ctx>,
        wargv: PointerValue<'ctx>,
        free_fn: inkwell::values::FunctionValue<'ctx>,
        skip_block: inkwell::basic_block::BasicBlock<'ctx>,
    ) {
        self.windows_arg_global_store("huzi_argv", new_argv);
        self.builder
            .build_call(free_fn, &[wargv.into()], "w_argv_free")
            .unwrap();
        self.builder
            .build_unconditional_branch(skip_block)
            .unwrap();
    }

    /// `malloc((n + 1) * 8)`：为 n 个 UTF-8 参数外加一个 NUL 结尾分配指针插槽。
    fn alloc_argv_array(
        &mut self,
        malloc_fn: inkwell::values::FunctionValue<'ctx>,
        n: inkwell::values::IntValue<'ctx>,
    ) -> PointerValue<'ctx> {
        let i32_type = self.context.i32_type();
        let slots = self
            .builder
            .build_int_add(n, i32_type.const_int(1, false), "argv_slots")
            .unwrap();
        let bytes = self
            .builder
            .build_int_mul(slots, i32_type.const_int(8, false), "argv_bytes")
            .unwrap();
        self.builder
            .build_call(malloc_fn, &[bytes.into()], "utf8_argv")
            .unwrap()
            .try_as_basic_value()
            .unwrap_left()
            .into_pointer_value()
    }

    /// 将每个 `wargv[i]` 转换为 UTF-8，发布该数组并以 NUL 结尾。
    /// 退出时构建器停留在循环退出块。
    fn fill_argv_array(
        &mut self,
        wargv: PointerValue<'ctx>,
        new_argv: PointerValue<'ctx>,
        n: inkwell::values::IntValue<'ctx>,
    ) {
        let i32_type = self.context.i32_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let function = self
            .builder
            .get_insert_block()
            .and_then(|b| b.get_parent())
            .expect("argv loop needs a current function");

        let i_slot = self.builder.build_alloca(i32_type, "utf8_i").unwrap();
        self.builder
            .build_store(i_slot, i32_type.const_zero())
            .unwrap();
        let cond_block = self.context.append_basic_block(function, "utf8_cond");
        let body_block = self.context.append_basic_block(function, "utf8_body");
        let end_block = self.context.append_basic_block(function, "utf8_end");
        self.builder
            .build_unconditional_branch(cond_block)
            .unwrap();

        self.builder.position_at_end(cond_block);
        let i = self
            .builder
            .build_load(i32_type, i_slot, "utf8_i_val")
            .unwrap()
            .into_int_value();
        let more = self
            .builder
            .build_int_compare(IntPredicate::SLT, i, n, "utf8_more")
            .unwrap();
        self.builder
            .build_conditional_branch(more, body_block, end_block)
            .unwrap();

        self.builder.position_at_end(body_block);
        let next = self.copy_one_arg(wargv, new_argv, i);
        self.builder.build_store(i_slot, next).unwrap();
        self.builder
            .build_unconditional_branch(cond_block)
            .unwrap();

        self.builder.position_at_end(end_block);
        let end_slot = unsafe {
            self.builder
                .build_gep(ptr_type, new_argv, &[n], "u8_end")
                .unwrap()
        };
        self.builder
            .build_store(end_slot, ptr_type.const_null())
            .unwrap();
    }

    /// 将 `wargv[i]` 转换为 UTF-8 并存入 `new_argv[i]`。返回下一个索引。
    /// 转换的合并块成为构建器的当前位置，因此后续存储会落在循环体内。
    fn copy_one_arg(
        &mut self,
        wargv: PointerValue<'ctx>,
        new_argv: PointerValue<'ctx>,
        i: inkwell::values::IntValue<'ctx>,
    ) -> inkwell::values::IntValue<'ctx> {
        let i32_type = self.context.i32_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let wslot = unsafe {
            self.builder
                .build_gep(ptr_type, wargv, &[i], "w_slot")
                .unwrap()
        };
        let wstr = self
            .builder
            .build_load(ptr_type, wslot, "w_str")
            .unwrap()
            .into_pointer_value();
        // 转换基本块在此插入；构建器停留在其 merge 块。
        let u8str = self.convert_wstr_to_utf8(wstr);
        let dslot = unsafe {
            self.builder
                .build_gep(ptr_type, new_argv, &[i], "u8_slot")
                .unwrap()
        };
        self.builder.build_store(dslot, u8str).unwrap();
        self.builder
            .build_int_add(i, i32_type.const_int(1, false), "utf8_next")
            .unwrap()
    }

    /// 将单个以 NUL 结尾的 UTF-16 字符串转换为 malloc 分配的 UTF-8 缓冲区。
    /// 转换失败则返回共享空字符串。构建器停留在 merge 块；返回的指针在该处有效。
    fn convert_wstr_to_utf8(&mut self, wstr: PointerValue<'ctx>) -> PointerValue<'ctx> {
        let i32_type = self.context.i32_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let function = self
            .builder
            .get_insert_block()
            .and_then(|b| b.get_parent())
            .expect("argv convert needs a current function");
        let convert_fn = self
            .module
            .get_function("WideCharToMultiByte")
            .expect("WideCharToMultiByte in prelude");
        let malloc_fn = self.module.get_function("malloc").expect("malloc in prelude");

        // 查询所需缓冲区大小：int need = WideCharToMultiByte(CP_UTF8, 0, wstr, -1, NULL, 0, ...);
        let null_out = ptr_type.const_null();
        let need = self.emit_wctmb_call(convert_fn, wstr, null_out, i32_type.const_zero(), "u8_need");

        let conv_block = self.context.append_basic_block(function, "u8_conv");
        let fail_block = self.context.append_basic_block(function, "u8_fail");
        let merge_block = self.context.append_basic_block(function, "u8_merge");
        let good = self
            .builder
            .build_int_compare(
                IntPredicate::SGT,
                need,
                i32_type.const_zero(),
                "u8_good",
            )
            .unwrap();
        self.builder
            .build_conditional_branch(good, conv_block, fail_block)
            .unwrap();

        self.builder.position_at_end(conv_block);
        let buf = self
            .builder
            .build_call(malloc_fn, &[need.into()], "u8_buf")
            .unwrap()
            .try_as_basic_value()
            .unwrap_left()
            .into_pointer_value();
        self.emit_wctmb_call(convert_fn, wstr, buf, need, "u8_write");
        self.builder
            .build_unconditional_branch(merge_block)
            .unwrap();

        self.builder.position_at_end(fail_block);
        let empty = self
            .module
            .get_global("huzi_empty_str")
            .expect("arg support globals are declared in prelude")
            .as_pointer_value();
        self.builder
            .build_unconditional_branch(merge_block)
            .unwrap();

        self.builder.position_at_end(merge_block);
        let phi = self
            .builder
            .build_phi(ptr_type, "u8_str")
            .unwrap();
        phi.add_incoming(&[(&buf, conv_block), (&empty, fail_block)]);
        phi.as_basic_value().into_pointer_value()
    }

    /// 发射一次 `WideCharToMultiByte(CP_UTF8, 0, wstr, -1, out, out_len, ...)` 调用，
    /// 并返回其 i32 结果（所需大小或写入的字节数）。
    fn emit_wctmb_call(
        &mut self,
        convert_fn: inkwell::values::FunctionValue<'ctx>,
        wstr: PointerValue<'ctx>,
        out: PointerValue<'ctx>,
        out_len: inkwell::values::IntValue<'ctx>,
        name: &str,
    ) -> inkwell::values::IntValue<'ctx> {
        let i32_type = self.context.i32_type();
        let null_ptr = self.context.ptr_type(AddressSpace::default()).const_null();
        self.builder
            .build_call(
                convert_fn,
                &[
                    i32_type.const_int(CP_UTF8, false).into(),
                    i32_type.const_zero().into(),
                    wstr.into(),
                    i32_type.const_int(u32::MAX as u64, false).into(),
                    out.into(),
                    out_len.into(),
                    null_ptr.into(),
                    null_ptr.into(),
                ],
                name,
            )
            .unwrap()
            .try_as_basic_value()
            .unwrap_left()
            .into_int_value()
    }

    /// 获取参数支持全局变量的地址（`huzi_argc` / `huzi_argv`）。
    fn windows_arg_global(&self, name: &str) -> PointerValue<'ctx> {
        self.module
            .get_global(name)
            .expect("arg support globals are declared in prelude")
            .as_pointer_value()
    }

    /// 向参数支持全局变量中存储一个指针值。
    fn windows_arg_global_store(&mut self, name: &str, value: PointerValue<'ctx>) {
        let global = self.windows_arg_global(name);
        self.builder.build_store(global, value).unwrap();
    }
}
