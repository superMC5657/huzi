//! 文件状态查询配对:`read_file_ok(path)` / `read_file_err(path)`。
//!
//! `read_file` 失败只返回空串,调用方无法区分"空文件"与"不存在";
//! 该配对提供 Option/Result 风格的最小哨兵消除:先用 `read_file_ok`
//! 分支,再用 `read_file` 取内容;`read_file_err` 在失败时返回一条
//! 稳定的诊断文本(成功时为空串),供展示或日志使用。探测成功时会
//! 立即 `fclose`,不持有句柄,不改变 `read_file` 的既有语义。

use huzi_ast::Expr;
use huzi_error::{HuziError, Result};
use inkwell::values::{BasicValueEnum, PointerValue};

use super::CodeGen;

impl<'ctx> CodeGen<'ctx> {
    /// `read_file_ok(path: str) -> bool`:文件可打开则 true,否则 false。
    pub(super) fn compile_read_file_ok(
        &mut self,
        arguments: &[Expr],
    ) -> Result<BasicValueEnum<'ctx>> {
        if arguments.len() != 1 {
            return Err(HuziError::new_global(
                "read_file_ok() requires exactly 1 argument (path)",
            ));
        }
        let path = self.check_str_arg(&arguments[0], "read_file_ok")?;
        let exists = self.emit_file_probe(path, "rfo")?;
        Ok(exists.into())
    }

    /// `read_file_err(path: str) -> str`:成功返回空串,失败返回诊断文本。
    pub(super) fn compile_read_file_err(
        &mut self,
        arguments: &[Expr],
    ) -> Result<BasicValueEnum<'ctx>> {
        if arguments.len() != 1 {
            return Err(HuziError::new_global(
                "read_file_err() requires exactly 1 argument (path)",
            ));
        }
        let path = self.check_str_arg(&arguments[0], "read_file_err")?;
        Ok(self.emit_file_diag(path)?.into())
    }

    /// 编译一个求值为字符串(i8*)的参数。
    fn check_str_arg(&mut self, expr: &Expr, name: &str) -> Result<PointerValue<'ctx>> {
        match self.compile_expr(expr)? {
            BasicValueEnum::PointerValue(p) => Ok(p),
            _ => Err(HuziError::new_global(format!(
                "{}() argument must be a string",
                name
            ))),
        }
    }

    /// 探测文件是否可读:以 `rb` 打开,成功则关闭并返回 true(i1),
    /// 失败返回 false。成功路径的关闭避免句柄泄漏。
    fn emit_file_probe(
        &mut self,
        path: PointerValue<'ctx>,
        tag: &str,
    ) -> Result<inkwell::values::IntValue<'ctx>> {
        let file = self.try_fopen(path);
        let is_null = self.builder.build_is_null(file, "probe_null").unwrap();
        let function = self.current_function()?;
        let ok_bb = self.context.append_basic_block(function, &format!("{tag}_ok"));
        let fail_bb = self.context.append_basic_block(function, &format!("{tag}_fail"));
        let done_bb = self.context.append_basic_block(function, &format!("{tag}_done"));
        let bool_ty = self.context.bool_type();
        let result = self.build_alloca(bool_ty.into(), "probe_res")?;
        self.builder.build_conditional_branch(is_null, fail_bb, ok_bb).unwrap();
        self.builder.position_at_end(fail_bb);
        self.builder.build_store(result, bool_ty.const_int(0, false)).unwrap();
        self.builder.build_unconditional_branch(done_bb).unwrap();
        self.builder.position_at_end(ok_bb);
        self.emit_fclose(file);
        self.builder.build_store(result, bool_ty.const_int(1, false)).unwrap();
        self.builder.build_unconditional_branch(done_bb).unwrap();
        self.builder.position_at_end(done_bb);
        Ok(self.builder.build_load(bool_ty, result, "probe_val").unwrap().into_int_value())
    }

    /// 失败原因诊断:成功返回空串(并关闭句柄),失败返回稳定文本。
    fn emit_file_diag(&mut self, path: PointerValue<'ctx>) -> Result<PointerValue<'ctx>> {
        let file = self.try_fopen(path);
        let is_null = self.builder.build_is_null(file, "diag_null").unwrap();
        let function = self.current_function()?;
        let ok_bb = self.context.append_basic_block(function, "rfe_ok");
        let fail_bb = self.context.append_basic_block(function, "rfe_fail");
        let done_bb = self.context.append_basic_block(function, "rfe_done");
        let ptr_ty = self.context.ptr_type(inkwell::AddressSpace::default());
        let result = self.build_alloca(ptr_ty.into(), "diag_res")?;
        self.builder.build_conditional_branch(is_null, fail_bb, ok_bb).unwrap();
        self.builder.position_at_end(fail_bb);
        let msg = unsafe { self.builder.build_global_string("cannot open file", "rfe_msg").unwrap() };
        self.builder.build_store(result, msg.as_pointer_value()).unwrap();
        self.builder.build_unconditional_branch(done_bb).unwrap();
        self.builder.position_at_end(ok_bb);
        self.emit_fclose(file);
        let empty = unsafe { self.builder.build_global_string("", "rfe_empty").unwrap() };
        self.builder.build_store(result, empty.as_pointer_value()).unwrap();
        self.builder.build_unconditional_branch(done_bb).unwrap();
        self.builder.position_at_end(done_bb);
        Ok(self.builder.build_load(ptr_ty, result, "diag_val").unwrap().into_pointer_value())
    }

    /// 以 `rb` 模式打开文件,返回 FILE* (可能为空指针)。
    fn try_fopen(&mut self, path: PointerValue<'ctx>) -> PointerValue<'ctx> {
        let fopen_fn = self.module.get_function("fopen").expect("fopen in prelude");
        let mode = unsafe { self.builder.build_global_string("rb", "probe_mode").unwrap() };
        self.builder
            .build_call(fopen_fn, &[path.into(), mode.as_pointer_value().into()], "probe_file")
            .unwrap()
            .try_as_basic_value()
            .unwrap_left()
            .into_pointer_value()
    }

    /// 关闭已成功打开的句柄(返回值忽略)。
    fn emit_fclose(&mut self, file: PointerValue<'ctx>) {
        let fclose_fn = self.module.get_function("fclose").expect("fclose in prelude");
        self.builder.build_call(fclose_fn, &[file.into()], "probe_close").unwrap();
    }
}
