//! 文件 I/O 内置函数:`read_file(path)` / `write_file(path, content)`。
//!
//! `read_file` 一次性读入整个文件(基于 fseek/ftell 定长,≤2GB),
//! 返回以 `\0` 结尾的堆上字符串;打开失败返回空串。
//! `write_file` 以文本模式整体写入,返回是否成功(true/false)。
//!
//! 子模块:`file_rw`(读写成功/失败分支发射)、`read_console`(控制台输入)。

mod file_rw;
mod read_console;

use huzi_ast::Expr;
use huzi_error::{HuziError, Result};
use inkwell::values::BasicValueEnum;

use super::CodeGen;

impl<'ctx> CodeGen<'ctx> {
    /// `read_file(path: str) -> str`:读入整个文件;失败返回空串。
    pub(super) fn compile_read_file(
        &mut self,
        arguments: &[Expr],
    ) -> Result<BasicValueEnum<'ctx>> {
        self.expect_arg_count("read_file", arguments, 1)?;
        let path = self.compile_str_arg(&arguments[0], "read_file")?;

        let fopen_fn = self.module.get_function("fopen").expect("fopen in prelude");
        let fseek_fn = self.module.get_function("fseek").expect("fseek in prelude");
        let ftell_fn = self.module.get_function("ftell").expect("ftell in prelude");
        let fread_fn = self.module.get_function("fread").expect("fread in prelude");
        let fclose_fn = self.module.get_function("fclose").expect("fclose in prelude");
        let malloc_fn = self.module.get_function("malloc").expect("malloc in prelude");

        let function = self.current_function()?;
        let ok_block = self.context.append_basic_block(function, "rf_ok");
        let fail_block = self.context.append_basic_block(function, "rf_fail");
        let end_block = self.context.append_basic_block(function, "rf_end");

        let mode = self.cstr_const("rb");
        let call = self.builder.build_call(
            fopen_fn,
            &[path.into(), mode.into()],
            "rf_file",
        ).unwrap();
        let file = call.try_as_basic_value().unwrap_left().into_pointer_value();
        let result_ptr = self.build_alloca(self.context.ptr_type(inkwell::AddressSpace::default()).into(), "rf_res")?;

        let is_null = self
            .builder
            .build_is_null(file, "rf_is_null")
            .unwrap();
        self.builder
            .build_conditional_branch(is_null, fail_block, ok_block)
            .unwrap();

        self.emit_read_file_fail(fail_block, result_ptr, end_block);

        self.emit_read_file_ok(ok_block, file, fseek_fn, ftell_fn, fread_fn, fclose_fn, malloc_fn, result_ptr, end_block);

        self.builder.position_at_end(end_block);
        let result = self
            .builder
            .build_load(self.context.ptr_type(inkwell::AddressSpace::default()), result_ptr, "rf_load")
            .unwrap();
        Ok(result)
    }

    /// `write_file(path: str, content: str) -> bool`:整体写入文本文件。
    pub(super) fn compile_write_file(
        &mut self,
        arguments: &[Expr],
    ) -> Result<BasicValueEnum<'ctx>> {
        self.expect_arg_count("write_file", arguments, 2)?;
        let path = self.compile_str_arg(&arguments[0], "write_file")?;
        let content = self.compile_str_arg(&arguments[1], "write_file")?;

        let fopen_fn = self.module.get_function("fopen").expect("fopen in prelude");
        let fwrite_fn = self.module.get_function("fwrite").expect("fwrite in prelude");
        let fclose_fn = self.module.get_function("fclose").expect("fclose in prelude");
        let strlen_fn = self.module.get_function("strlen").expect("strlen in prelude");

        let function = self.current_function()?;
        let ok_block = self.context.append_basic_block(function, "wf_ok");
        let fail_block = self.context.append_basic_block(function, "wf_fail");
        let end_block = self.context.append_basic_block(function, "wf_end");

        let mode = self.cstr_const("wb");
        let call = self.builder.build_call(
            fopen_fn,
            &[path.into(), mode.into()],
            "wf_file",
        ).unwrap();
        let file = call.try_as_basic_value().unwrap_left().into_pointer_value();
        let result_ptr = self.build_alloca(self.context.bool_type().into(), "wf_res")?;

        let is_null = self.builder.build_is_null(file, "wf_is_null").unwrap();
        self.builder
            .build_conditional_branch(is_null, fail_block, ok_block)
            .unwrap();

        self.emit_write_file_fail(fail_block, result_ptr, end_block);

        self.emit_write_file_ok(ok_block, file, content, fwrite_fn, fclose_fn, strlen_fn, result_ptr, end_block);

        self.builder.position_at_end(end_block);
        let result = self
            .builder
            .build_load(self.context.bool_type(), result_ptr, "wf_load")
            .unwrap();
        Ok(result)
    }

    /// 编译一个求值为字符串(i8*)的参数。
    pub(super) fn compile_str_arg(&mut self, expr: &Expr, name: &str) -> Result<inkwell::values::PointerValue<'ctx>> {
        let value = self.compile_expr(expr)?;
        match value {
            BasicValueEnum::PointerValue(p) => Ok(p),
            _ => Err(HuziError::new_global(format!(
                "{}() argument must be a string",
                name
            ))),
        }
    }

    /// 取(或惰性创建)模块级 C 字符串常量。
    pub(super) fn cstr_const(&mut self, s: &str) -> inkwell::values::PointerValue<'ctx> {
        let global = unsafe { self.builder.build_global_string(s, "huzi_cstr").unwrap() };
        global.as_pointer_value()
    }
}
