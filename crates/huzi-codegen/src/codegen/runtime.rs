//! 运行时错误检查的 IR 生成:整数除零/取模零、数组越界。
//!
//! 检查失败时通过 printf 向 stdout 输出一行错误信息,随后调用 libc
//! `exit(1)` 立即终止进程(fail 块以 `unreachable` 收尾,verify 通过)。

use huzi_error::Result;

use super::CodeGen;
use inkwell::values::{BasicMetadataValueEnum, IntValue, PointerValue};

impl<'ctx> CodeGen<'ctx> {
    /// 生成"条件成立则继续、否则报错退出"的运行时检查。
    ///
    /// `cond` 为真表示检查通过;失败时按 printf 语义打印 `fmt` 与
    /// `printf_args`,并以退出码 1 结束。返回后 builder 定位在继续块。
    pub(super) fn emit_runtime_check(
        &mut self,
        cond: IntValue<'ctx>,
        fmt: &str,
        printf_args: &[BasicMetadataValueEnum<'ctx>],
    ) -> Result<()> {
        let function = self.current_function()?;
        let fail_block = self.context.append_basic_block(function, "rt_fail");
        let cont_block = self.context.append_basic_block(function, "rt_ok");

        self.builder
            .build_conditional_branch(cond, cont_block, fail_block)
            .unwrap();

        self.builder.position_at_end(fail_block);
        let printf_fn = self.module.get_function("printf").expect("printf in prelude");
        let exit_fn = self.module.get_function("exit").expect("exit in prelude");
        let fmt_val = unsafe { self.builder.build_global_string(fmt, "rt_msg").unwrap() };
        let mut call_args: Vec<BasicMetadataValueEnum> =
            vec![fmt_val.as_pointer_value().into()];
        call_args.extend_from_slice(printf_args);
        self.builder
            .build_call(printf_fn, &call_args, "rt_report")
            .unwrap();
        let code = self.context.i32_type().const_int(1, false);
        self.builder
            .build_call(exit_fn, &[code.into()], "rt_exit")
            .unwrap();
        self.builder.build_unreachable().unwrap();

        self.builder.position_at_end(cont_block);
        Ok(())
    }

    /// 整数除法/取模的除零检查:`divisor == 0` 时报错退出。
    /// 浮点除法遵循 IEEE 语义(±inf/NaN),不做检查。
    pub(super) fn emit_div_zero_check(
        &mut self,
        divisor: IntValue<'ctx>,
        is_mod: bool,
    ) -> Result<()> {
        let zero = divisor.get_type().const_int(0, false);
        let cond = self
            .builder
            .build_int_compare(inkwell::IntPredicate::NE, divisor, zero, "div_nz")
            .unwrap();
        let fmt = if is_mod {
            "Runtime error: modulo by zero\n\0"
        } else {
            "Runtime error: division by zero\n\0"
        };
        self.emit_runtime_check(cond, fmt, &[])
    }

    /// 数组下标越界检查。长度来自编译期已知的数组定义(let 数组变量、
    /// 结构体字段);拿不到长度(如下标链中间结果)时静默跳过,
    /// 与既有行为保持一致。
    pub(super) fn emit_bounds_check(
        &mut self,
        array_expr: &huzi_ast::Expr,
        index_i32: IntValue<'ctx>,
    ) -> Result<()> {
        let Some(len) = self.resolve_array_len(array_expr)? else {
            return Ok(());
        };
        let len_val = self
            .context
            .i32_type()
            .const_int(len as u64, false);
        // 无符号比较:负下标的无符号值远大于 len,自然落入失败分支。
        let cond = self
            .builder
            .build_int_compare(inkwell::IntPredicate::ULT, index_i32, len_val, "idx_ok")
            .unwrap();
        let fmt = format!("Runtime error: array index out of bounds (length {})\n\0", len);
        self.emit_runtime_check(cond, &fmt, &[index_i32.into()])
    }

    /// 字符串下标越界检查:strlen 取字节长度,无符号比较,负下标自然
    /// 落入失败分支(与 vec 一致)。UTF-8 多字节按字节语义,不做字符语义。
    pub(super) fn emit_str_bounds_check(
        &mut self,
        str_ptr: PointerValue<'ctx>,
        index_i32: IntValue<'ctx>,
    ) -> Result<()> {
        let strlen_fn = self.module.get_function("strlen").expect("strlen in prelude");
        let len = self
            .builder
            .build_call(strlen_fn, &[str_ptr.into()], "str_len")
            .unwrap()
            .try_as_basic_value()
            .unwrap_left()
            .into_int_value();
        let cond = self
            .builder
            .build_int_compare(inkwell::IntPredicate::ULT, index_i32, len, "str_idx_ok")
            .unwrap();
        self.emit_runtime_check(cond, "Runtime error: string index out of bounds\n\0", &[])
    }

    /// 是否为字符串下标:元素为 i8(char)且无编译期数组长度。
    /// vec 由调用方先行分流,此处遇 vec 直接返回 false;定长 char
    /// 数组有 array_len,不误判为字符串。
    pub(super) fn is_string_index(
        &self,
        array_expr: &huzi_ast::Expr,
        elem_type: inkwell::types::BasicTypeEnum<'ctx>,
    ) -> Result<bool> {
        if let huzi_ast::Expr::Ident(name) = array_expr {
            if let Some(slot) = self.scope_lookup(name) {
                if Self::is_vec_slot(&slot) {
                    return Ok(false);
                }
            }
        }
        let is_i8 = matches!(
            elem_type,
            inkwell::types::BasicTypeEnum::IntType(t) if t.get_bit_width() == 8
        );
        if !is_i8 {
            return Ok(false);
        }
        Ok(self.resolve_array_len(array_expr)?.is_none())
    }

    /// 求数组表达式的编译期长度:let 数组变量取槽内记录,结构体字段
    /// 取 AST 类型标注,数组字面量取元素个数;其余情况返回 None。
    pub(super) fn resolve_array_len(
        &self,
        array_expr: &huzi_ast::Expr,
    ) -> Result<Option<u32>> {
        if let huzi_ast::Expr::ArrayLiteral(elements) = array_expr {
            if !elements.is_empty() {
                return Ok(Some(elements.len() as u32));
            }
        }
        if let huzi_ast::Expr::Ident(name) = array_expr {
            if let Some(slot) = self.scope_lookup(name) {
                return Ok(slot.array_len);
            }
        }
        if let huzi_ast::Expr::FieldAccess(fa) = array_expr {
            if let Some((_, fields)) = self.struct_def_of_expr(&fa.base) {
                if let Some(info) = fields.iter().find(|f| f.name == fa.field) {
                    if let huzi_ast::Type::Array(_, n) = &info.ast_ty {
                        return Ok(Some(*n as u32));
                    }
                }
            }
        }
        Ok(None)
    }
}
