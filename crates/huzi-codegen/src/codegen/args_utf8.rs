//! Windows-only UTF-8 argv fixup: rebuild `huzi_argc`/`huzi_argv` from the
//! Unicode command line.
//!
//! On Windows the CRT feeds `main(argc, argv)` with ANSI-encoded bytes (the
//! active console code page, e.g. GBK), so non-ASCII arguments arrive garbled
//! when the program treats them as UTF-8. The IR emitted here runs at program
//! startup, right after the ANSI args are stored, and replaces the globals
//! with UTF-8 strings converted from `GetCommandLineW` via
//! `CommandLineToArgvW` + `WideCharToMultiByte(CP_UTF8)`. Everything in this
//! file is only emitted when huzc itself is compiled for Windows; the Unix
//! path keeps the original argv untouched.
//!
//! Converted buffers are `malloc`'d once and live for the process lifetime,
//! so `arg(i)` keeps its zero-copy "pointer into argv storage" semantics and
//! out-of-range/negative indexes still yield the shared empty string. If the
//! Unicode fetch fails, the ANSI values are kept as-is (graceful fallback);
//! a per-argument conversion failure yields the empty string.

use inkwell::AddressSpace;
use inkwell::IntPredicate;
use inkwell::values::PointerValue;

use super::CodeGen;

/// UTF-8 code page for `WideCharToMultiByte` (matches the console setup).
const CP_UTF8: u64 = 65001;

impl<'ctx> CodeGen<'ctx> {
    /// Declare the argv-conversion imports (kernel32 + shell32 + libc malloc).
    /// Called from `prelude` only when compiled for Windows.
    pub(super) fn declare_windows_argv_imports(&mut self) {
        let i32_type = self.context.i32_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());

        // LPWSTR GetCommandLineW(void);
        let cmdline_ty = ptr_type.fn_type(&[], false);
        self.module.add_function("GetCommandLineW", cmdline_ty, None);

        // LPWSTR *CommandLineToArgvW(LPCWSTR, int *);
        let to_argv_ty = ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);
        self.module
            .add_function("CommandLineToArgvW", to_argv_ty, None);

        // int WideCharToMultiByte(UINT, DWORD, LPCWCH, int, LPSTR, int, ...);
        let convert_ty = i32_type.fn_type(
            &[
                i32_type.into(), // CodePage
                i32_type.into(), // dwFlags
                ptr_type.into(), // lpWideCharStr
                i32_type.into(), // cchWideChar (-1 = NUL-terminated)
                ptr_type.into(), // lpMultiByteStr
                i32_type.into(), // cbMultiByte
                ptr_type.into(), // lpDefaultChar
                ptr_type.into(), // lpUsedDefaultChar
            ],
            false,
        );
        self.module
            .add_function("WideCharToMultiByte", convert_ty, None);

        // HLOCAL LocalFree(HLOCAL);
        let free_ty = ptr_type.fn_type(&[ptr_type.into()], false);
        self.module.add_function("LocalFree", free_ty, None);
    }

    /// Replace `huzi_argc`/`huzi_argv` with UTF-8 strings from the Unicode
    /// command line. Emitted right after the ANSI store in `store_main_args`.
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

        // LPWSTR *wargv = CommandLineToArgvW(GetCommandLineW(), &n);
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

        // NULL from CommandLineToArgvW: keep the ANSI args as-is.
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

    /// NUL-terminate the converted array, publish it to `huzi_argv`, free the
    /// wide-char list, and rejoin the caller's flow at `skip_block`. Emitted
    /// at the loop-exit block left behind by `fill_argv_array`.
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

    /// `malloc((n + 1) * 8)`: pointer slots for n UTF-8 args plus a NUL end.
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

    /// Convert each `wargv[i]` to UTF-8, publish the array, and NUL-terminate
    /// it. Leaves the builder at the loop-exit block.
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

    /// Convert `wargv[i]` to UTF-8 and store it into `new_argv[i]`. Returns
    /// the next index. The conversion's merge block becomes the builder's
    /// position, so the trailing stores land in the loop body.
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
        // Conversion blocks splice in here; the builder lands on their merge.
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

    /// Convert one NUL-terminated UTF-16 string to a malloc'd UTF-8 buffer.
    /// Conversion failure yields the shared empty string. Leaves the builder
    /// positioned at the merge block; the returned pointer is valid there.
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

        // int need = WideCharToMultiByte(CP_UTF8, 0, wstr, -1, NULL, 0, ...);
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

    /// Emit one `WideCharToMultiByte(CP_UTF8, 0, wstr, -1, out, out_len, ...)`
    /// call and return its i32 result (required size, or bytes written).
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

    /// Address of an arg-support global (`huzi_argc` / `huzi_argv`).
    fn windows_arg_global(&self, name: &str) -> PointerValue<'ctx> {
        self.module
            .get_global(name)
            .expect("arg support globals are declared in prelude")
            .as_pointer_value()
    }

    /// Store a pointer value into an arg-support global.
    fn windows_arg_global_store(&mut self, name: &str, value: PointerValue<'ctx>) {
        let global = self.windows_arg_global(name);
        self.builder.build_store(global, value).unwrap();
    }
}
