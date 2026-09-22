//! 多线程并发内置函数 (spawn / join)。
//!
//! 提供 `spawn(fn, [arg])` 与 `join(handle) -> i32`。
//! 在 Windows 下走 Win32 API (`CreateThread`/`WaitForSingleObject`/`GetExitCodeThread`),
//! 在 POSIX 下走 `pthread` (`pthread_create`/`pthread_join`)。

use huzi_ast::Expr;
use huzi_error::{HuziError, Result};
use inkwell::AddressSpace;
use inkwell::values::{BasicValueEnum, FunctionValue, IntValue, PointerValue};

use super::CodeGen;

impl<'ctx> CodeGen<'ctx> {
    /// 启动新线程：`spawn(func, [arg]) -> i64`
    pub(super) fn compile_spawn(&mut self, arguments: &[Expr]) -> Result<BasicValueEnum<'ctx>> {
        if arguments.is_empty() || arguments.len() > 2 {
            return Err(HuziError::new_global(
                "spawn() requires 1 or 2 arguments: (fn_name, [arg])",
            ));
        }

        let fn_name = match &arguments[0] {
            Expr::Ident(name) => name,
            _ => return Err(HuziError::new_global("spawn() first argument must be a function name")),
        };

        let lookup_key = self.qualify_name(fn_name);
        let (target_fn, param_types) = self
            .functions
            .get(&lookup_key)
            .cloned()
            .ok_or_else(|| self.unknown_function_error(fn_name))?;

        let thunk = self.ensure_thread_thunk(&lookup_key, target_fn, &param_types);
        let thunk_ptr = thunk.as_global_value().as_pointer_value();

        let arg_ptr = if arguments.len() == 2 {
            self.compile_thread_arg(&arguments[1])?
        } else {
            self.context.ptr_type(AddressSpace::default()).const_null()
        };

        if cfg!(windows) {
            self.spawn_win32_thread(thunk_ptr, arg_ptr)
        } else {
            self.spawn_posix_thread(thunk_ptr, arg_ptr)
        }
    }

    fn compile_thread_arg(&mut self, expr: &Expr) -> Result<PointerValue<'ctx>> {
        let val = self.compile_expr(expr)?;
        match val {
            BasicValueEnum::IntValue(i) => {
                let i64_val = self
                    .builder
                    .build_int_z_extend(i, self.context.i64_type(), "arg_i64")
                    .unwrap();
                Ok(self
                    .builder
                    .build_int_to_ptr(
                        i64_val,
                        self.context.ptr_type(AddressSpace::default()),
                        "arg_ptr",
                    )
                    .unwrap())
            }
            BasicValueEnum::PointerValue(p) => Ok(p),
            _ => Err(HuziError::new_global("spawn() argument must be an integer or pointer")),
        }
    }

    fn spawn_win32_thread(
        &mut self,
        thunk_ptr: PointerValue<'ctx>,
        arg_ptr: PointerValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>> {
        let ct_fn = self.module.get_function("CreateThread").unwrap();
        let null_ptr = self.context.ptr_type(AddressSpace::default()).const_null();
        let zero64 = self.context.i64_type().const_int(0, false);
        let zero32 = self.context.i32_type().const_int(0, false);

        let handle_ptr = self
            .builder
            .build_call(
                ct_fn,
                &[
                    null_ptr.into(),
                    zero64.into(),
                    thunk_ptr.into(),
                    arg_ptr.into(),
                    zero32.into(),
                    null_ptr.into(),
                ],
                "thread_handle",
            )
            .unwrap()
            .try_as_basic_value()
            .unwrap_left()
            .into_pointer_value();

        let handle_i64 = self
            .builder
            .build_ptr_to_int(handle_ptr, self.context.i64_type(), "handle_i64")
            .unwrap();
        Ok(handle_i64.into())
    }

    fn spawn_posix_thread(
        &mut self,
        thunk_ptr: PointerValue<'ctx>,
        arg_ptr: PointerValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>> {
        let pc_fn = self.module.get_function("pthread_create").unwrap();
        let pt_alloca = self.builder.build_alloca(self.context.i64_type(), "pt").unwrap();
        let null_ptr = self.context.ptr_type(AddressSpace::default()).const_null();

        self.builder
            .build_call(
                pc_fn,
                &[pt_alloca.into(), null_ptr.into(), thunk_ptr.into(), arg_ptr.into()],
                "pc_ret",
            )
            .unwrap();

        let handle = self
            .builder
            .build_load(self.context.i64_type(), pt_alloca, "handle_i64")
            .unwrap();
        Ok(handle)
    }

    fn ensure_thread_thunk(
        &mut self,
        lookup_key: &str,
        target_fn: FunctionValue<'ctx>,
        param_types: &[inkwell::types::BasicTypeEnum<'ctx>],
    ) -> FunctionValue<'ctx> {
        let thunk_name = format!("__thunk_{}", lookup_key.replace("::", "_"));
        if let Some(f) = self.module.get_function(&thunk_name) {
            return f;
        }

        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let thunk_ty = if cfg!(windows) {
            self.context.i32_type().fn_type(&[ptr_ty.into()], false)
        } else {
            ptr_ty.fn_type(&[ptr_ty.into()], false)
        };

        let thunk = self.module.add_function(&thunk_name, thunk_ty, None);
        let saved_block = self.builder.get_insert_block();

        let entry = self.context.append_basic_block(thunk, "entry");
        self.builder.position_at_end(entry);

        let param = thunk.get_first_param().unwrap().into_pointer_value();
        let call_res = if param_types.len() == 1 {
            let arg_ty = param_types[0];
            let arg_val = if arg_ty.is_int_type() {
                let p_i64 = self
                    .builder
                    .build_ptr_to_int(param, self.context.i64_type(), "p_i64")
                    .unwrap();
                self.builder
                    .build_int_truncate(p_i64, arg_ty.into_int_type(), "arg_val")
                    .unwrap()
                    .into()
            } else {
                param.into()
            };
            self.builder.build_call(target_fn, &[arg_val], "worker_call").unwrap()
        } else {
            self.builder.build_call(target_fn, &[], "worker_call").unwrap()
        };

        self.emit_thunk_return(call_res);

        if let Some(b) = saved_block {
            self.builder.position_at_end(b);
        }
        thunk
    }

    fn emit_thunk_return(&mut self, call_res: inkwell::values::CallSiteValue<'ctx>) {
        let is_win = cfg!(windows);
        let ptr_ty = self.context.ptr_type(AddressSpace::default());

        if let Some(basic) = call_res.try_as_basic_value().left() {
            if basic.is_int_value() {
                let int_val = basic.into_int_value();
                if is_win {
                    let ret_i32 = self
                        .builder
                        .build_int_truncate(int_val, self.context.i32_type(), "ret_i32")
                        .unwrap();
                    self.builder.build_return(Some(&ret_i32)).unwrap();
                } else {
                    let ret_i64 = self
                        .builder
                        .build_int_z_extend(int_val, self.context.i64_type(), "ret_i64")
                        .unwrap();
                    let ret_ptr = self
                        .builder
                        .build_int_to_ptr(ret_i64, ptr_ty, "ret_ptr")
                        .unwrap();
                    self.builder.build_return(Some(&ret_ptr)).unwrap();
                }
                return;
            }
        }

        if is_win {
            let zero = self.context.i32_type().const_int(0, false);
            self.builder.build_return(Some(&zero)).unwrap();
        } else {
            let null = ptr_ty.const_null();
            self.builder.build_return(Some(&null)).unwrap();
        }
    }

    /// 等待线程结束：`join(handle: i64) -> i32`
    pub(super) fn compile_join(&mut self, arguments: &[Expr]) -> Result<BasicValueEnum<'ctx>> {
        if arguments.len() != 1 {
            return Err(HuziError::new_global("join() requires 1 argument: (handle: i64)"));
        }

        let handle_val = match self.compile_expr(&arguments[0])? {
            BasicValueEnum::IntValue(i) => i,
            _ => return Err(HuziError::new_global("join() handle must be an integer")),
        };

        let handle_i64 = if handle_val.get_type().get_bit_width() < 64 {
            self.builder
                .build_int_z_extend(handle_val, self.context.i64_type(), "h64")
                .unwrap()
        } else {
            handle_val
        };

        if cfg!(windows) {
            self.join_win32_thread(handle_i64)
        } else {
            self.join_posix_thread(handle_i64)
        }
    }

    fn join_win32_thread(&mut self, handle_i64: IntValue<'ctx>) -> Result<BasicValueEnum<'ctx>> {
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let handle_ptr = self
            .builder
            .build_int_to_ptr(handle_i64, ptr_ty, "h_ptr")
            .unwrap();

        let wfso = self.module.get_function("WaitForSingleObject").unwrap();
        let infinite = self.context.i32_type().const_int(0xFFFFFFFF, false);
        self.builder
            .build_call(wfso, &[handle_ptr.into(), infinite.into()], "wfso_call")
            .unwrap();

        let exit_code_alloca = self.builder.build_alloca(self.context.i32_type(), "exit_code").unwrap();
        let gect = self.module.get_function("GetExitCodeThread").unwrap();
        self.builder
            .build_call(gect, &[handle_ptr.into(), exit_code_alloca.into()], "gect_call")
            .unwrap();

        let ch = self.module.get_function("CloseHandle").unwrap();
        self.builder
            .build_call(ch, &[handle_ptr.into()], "ch_call")
            .unwrap();

        let exit_val = self
            .builder
            .build_load(self.context.i32_type(), exit_code_alloca, "exit_val")
            .unwrap();
        Ok(exit_val)
    }

    fn join_posix_thread(&mut self, handle_i64: IntValue<'ctx>) -> Result<BasicValueEnum<'ctx>> {
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let retval_alloca = self.builder.build_alloca(ptr_ty, "retval").unwrap();
        let pj = self.module.get_function("pthread_join").unwrap();
        self.builder
            .build_call(pj, &[handle_i64.into(), retval_alloca.into()], "pj_call")
            .unwrap();

        let res_ptr = self
            .builder
            .build_load(ptr_ty, retval_alloca, "res_ptr")
            .unwrap()
            .into_pointer_value();
        let res_i64 = self
            .builder
            .build_ptr_to_int(res_ptr, self.context.i64_type(), "res_i64")
            .unwrap();
        let res_i32 = self
            .builder
            .build_int_truncate(res_i64, self.context.i32_type(), "res_i32")
            .unwrap();
        Ok(res_i32.into())
    }
}
