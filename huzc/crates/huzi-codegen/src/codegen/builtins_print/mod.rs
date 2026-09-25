mod struct_print;
mod vec_print;

use super::CodeGen;
use huzi_ast::*;
use huzi_error::{HuziError, Result};
use inkwell::values::{BasicMetadataValueEnum, BasicValueEnum, FloatValue, IntValue};

impl<'ctx> CodeGen<'ctx> {
    // ==================== 作用域辅助函数 ====================

    /// 将 Windows 控制台切换至 UTF-8 代码页（65001），以便 `printf` 能正确显示中文及其他非 ASCII 字符。
    /// 字符串字面量以 UTF-8 字节存储；若不切换，控制台会用其默认代码页（如 GBK）解码从而导致乱码。
    /// 在非 Windows 平台为空操作。
    pub(super) fn emit_console_utf8_setup(&mut self) {
        if !cfg!(windows) {
            return;
        }
        let set_cp_fn = self.module.get_function("SetConsoleOutputCP").unwrap();
        let utf8_cp = self.context.i32_type().const_int(65001, false);
        self.builder
            .build_call(set_cp_fn, &[utf8_cp.into()], "console_utf8")
            .unwrap();
    }

    pub(super) fn compile_print(
        &mut self,
        arguments: &[Expr],
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        if arguments.is_empty() {
            let empty_str = unsafe { self.builder.build_global_string("", "empty_str").unwrap() };
            let printf_fn = self.module.get_function("printf").unwrap();
            let call = self
                .builder
                .build_call(
                    printf_fn,
                    &[empty_str.as_pointer_value().into()],
                    "print_empty",
                )
                .unwrap();
            return Ok(call.try_as_basic_value().unwrap_left());
        }

        let mut format_string = String::new();
        let mut args: Vec<inkwell::values::BasicMetadataValueEnum> = Vec::new();

        for arg in arguments.iter() {
            // Box 与 null 直接打印:null 输出 `null`,Box 递归展开字段。
            if self.try_emit_box_arg(arg, &mut format_string, &mut args)? {
                continue;
            }
            // vec 实参走运行期循环打印(先落盘待定的标量片段)。
            if self.try_emit_vec_arg(arg, &mut format_string, &mut args)? {
                continue;
            }
            // 结构体实参走编译期字段展开(同上先落盘)。
            if self.try_emit_struct_arg(arg, &mut format_string, &mut args)? {
                continue;
            }
            let value = self.compile_expr(arg)?;
            // 函数返回的结构体等无 AST 线索的值,按编译后类型兜底。
            if let inkwell::values::BasicValueEnum::StructValue(sv) = value {
                let ty = sv.get_type();
                if !self.is_tuple_type(ty) && self.struct_def_by_type(ty.into()).is_some() {
                    self.flush_print_chunk(&mut format_string, &mut args)?;
                    self.emit_struct_value(value)?;
                    continue;
                }
            }
            self.format_print_value(value, &mut format_string, &mut args)?;
        }

        format_string.push('\n');
        Ok(self.call_printf(&format_string, args))
    }

    /// vec 实参则打印并返回 true;非 vec 返回 false(调用方走常规路径)。
    fn try_emit_vec_arg(
        &mut self,
        arg: &Expr,
        format_string: &mut String,
        args: &mut Vec<inkwell::values::BasicMetadataValueEnum<'ctx>>,
    ) -> Result<bool> {
        if let Expr::Ident(name) = arg {
            if let Some(slot) = self.scope_lookup(name) {
                if Self::is_vec_slot(&slot) {
                    self.flush_print_chunk(format_string, args)?;
                    let (data, len, elem_ty) = self.vec_print_parts(name)?;
                    self.emit_vec_loop(data, len, elem_ty)?;
                    return Ok(true);
                }
            }
            return Ok(false);
        }
        if let Expr::VecEmpty(elem_ast_ty) = arg {
            self.flush_print_chunk(format_string, args)?;
            let elem_ty = self.type_to_llvm(elem_ast_ty)?;
            let vec_val = self.compile_vec_empty_value(elem_ast_ty)?;
            self.emit_vec_value(vec_val, elem_ty)?;
            return Ok(true);
        }
        // `print(split(s, d))` 直接打印临时 vec<str>。
        if let Expr::Call(call) = arg {
            if Self::is_split_ctor(call) {
                self.flush_print_chunk(format_string, args)?;
                let vec_val = self.compile_split(&call.arguments)?;
                let str_ty = self.context.ptr_type(inkwell::AddressSpace::default()).into();
                self.emit_vec_value(vec_val, str_ty)?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// 结构体实参则打印并返回 true;非结构体返回 false。
    fn try_emit_struct_arg(
        &mut self,
        arg: &Expr,
        format_string: &mut String,
        args: &mut Vec<inkwell::values::BasicMetadataValueEnum<'ctx>>,
    ) -> Result<bool> {
        let is_struct = matches!(arg, Expr::StructLiteral(_)) || self.struct_def_of_expr(arg).is_some();
        if !is_struct {
            return Ok(false);
        }
        self.flush_print_chunk(format_string, args)?;
        let (base_ptr, base_ty) = self.compile_addr(arg)?;
        let value = self.builder.build_load(base_ty, base_ptr, "struct_print_val").unwrap();
        self.emit_struct_value(value)?;
        Ok(true)
    }

    /// 将一个待打印的值追加到 printf 格式化字符串和参数列表中，
    /// 并按照 C 可变参数约定进行提升或窄化转换。
    pub(in crate::codegen) fn format_print_value(
        &mut self,
        value: inkwell::values::BasicValueEnum<'ctx>,
        format_string: &mut String,
        args: &mut Vec<inkwell::values::BasicMetadataValueEnum<'ctx>>,
    ) -> Result<()> {
        // 元组按 `(v1, v2, ...)` 打印，每个字段分别进行格式化。
        if let inkwell::values::BasicValueEnum::StructValue(sv) = value {
            if self.is_tuple_type(sv.get_type()) {
                return self.format_tuple_value(sv.into(), format_string, args);
            }
        }

        match value {
            inkwell::values::BasicValueEnum::IntValue(iv)
                if iv.get_type().get_bit_width() == 1 =>
            {
                // 布尔值打印为 true/false。
                let s = self.build_bool_str(iv)?;
                format_string.push_str("%s");
                args.push(s.into());
            }
            inkwell::values::BasicValueEnum::IntValue(iv) => {
                self.format_int_value(iv, format_string, args)?;
            }
            inkwell::values::BasicValueEnum::FloatValue(fv) => {
                self.format_float_value(fv, format_string, args)?;
            }
            inkwell::values::BasicValueEnum::PointerValue(pv) => {
                format_string.push_str("%s");
                args.push(pv.into());
            }
            _ => {
                return Err(HuziError::new_global(
                    "print() does not support this value type",
                ))
            }
        }

        Ok(())
    }

    /// 整型值的格式化:char(i8)按 `%c` 打印并提升为 i32,i64 用 `%lld`
    /// (Windows LLP64 下 `%ld` 会截断),其余整型用 `%d`。
    fn format_int_value(
        &mut self,
        iv: IntValue<'ctx>,
        format_string: &mut String,
        args: &mut Vec<BasicMetadataValueEnum<'ctx>>,
    ) -> Result<()> {
        match iv.get_type().get_bit_width() {
            8 => {
                // char 按字符打印。
                format_string.push_str("%c");
                let c = self
                    .builder
                    .build_int_z_extend(iv, self.context.i32_type(), "char_promote")
                    .unwrap();
                args.push(c.into());
            }
            64 => {
                // Windows 为 LLP64（long=32 位），%ld 会截断 i64；
                // %lld（long long）在 Windows/Linux/macOS 均为 64 位。
                format_string.push_str("%lld");
                args.push(iv.into());
            }
            _ => {
                format_string.push_str("%d");
                args.push(iv.into());
            }
        }
        Ok(())
    }

    /// 浮点值的格式化:f32 用 `%g`,f64 用 `%f`;可变参数将 float 提升为 double。
    fn format_float_value(
        &mut self,
        fv: FloatValue<'ctx>,
        format_string: &mut String,
        args: &mut Vec<BasicMetadataValueEnum<'ctx>>,
    ) -> Result<()> {
        // 可变参数将 float 提升为 double
        let f64_val = if fv.get_type() == self.context.f64_type() {
            fv
        } else {
            self.builder
                .build_float_ext(fv, self.context.f64_type(), "f_promote")
                .unwrap()
        };
        if fv.get_type() == self.context.f32_type() {
            format_string.push_str("%g");
        } else {
            format_string.push_str("%f");
        }
        args.push(f64_val.into());
        Ok(())
    }

    /// 单次 printf 调用(无换行),返回调用值。
    pub(in crate::codegen) fn call_printf(
        &mut self,
        format: &str,
        args: Vec<BasicMetadataValueEnum<'ctx>>,
    ) -> BasicValueEnum<'ctx> {
        let printf_fn = self.module.get_function("printf").unwrap();
        let format_ptr = unsafe { self.builder.build_global_string(format, "print_str").unwrap() };
        let mut call_args: Vec<BasicMetadataValueEnum<'ctx>> =
            vec![format_ptr.as_pointer_value().into()];
        call_args.extend(args);
        self.builder
            .build_call(printf_fn, &call_args, "print_call")
            .unwrap()
            .try_as_basic_value()
            .unwrap_left()
    }

    /// 落盘待定的标量片段(无换行),清空缓冲。
    pub(in crate::codegen) fn flush_print_chunk(
        &mut self,
        format_string: &mut String,
        args: &mut Vec<BasicMetadataValueEnum<'ctx>>,
    ) -> Result<()> {
        if format_string.is_empty() && args.is_empty() {
            return Ok(());
        }
        let taken: Vec<BasicMetadataValueEnum<'ctx>> = std::mem::take(args);
        let format = std::mem::take(format_string);
        self.call_printf(&format, taken);
        Ok(())
    }

    /// 无参数的纯文本 printf(括号/分隔符/`Name {` 等)。
    pub(in crate::codegen) fn emit_printf_text(&mut self, text: &str) -> Result<()> {
        self.call_printf(text, Vec::new());
        Ok(())
    }

    /// 单个标量/元组值的直接打印(复用 format_print_value,不换行)。
    fn emit_scalar_print(&mut self, value: BasicValueEnum<'ctx>) -> Result<()> {
        let mut format_string = String::new();
        let mut args: Vec<BasicMetadataValueEnum<'ctx>> = Vec::new();
        self.format_print_value(value, &mut format_string, &mut args)?;
        self.call_printf(&format_string, args);
        Ok(())
    }

    /// 任意一等值的打印分发:命名结构体递归,其它走标量/元组路径。
    pub(super) fn emit_value_print(&mut self, value: BasicValueEnum<'ctx>) -> Result<()> {
        if let BasicValueEnum::StructValue(sv) = value {
            let ty = sv.get_type();
            if self.struct_def_by_type(ty.into()).is_some() {
                return self.emit_struct_value(value);
            }
            if self.is_vec_layout(ty) {
                return Err(HuziError::new_global(
                    "print() does not support nested vec (vec of vec)",
                ));
            }
        }
        self.emit_scalar_print(value)
    }
}
