//! `len()` 内置函数(自 `builtins_string.rs` 纯搬移,零逻辑变化)。

use super::CodeGen;
use huzi_ast::*;
use huzi_error::{HuziError, Result};

impl<'ctx> CodeGen<'ctx> {
    pub(in crate::codegen) fn compile_len(&mut self, arguments: &[Expr]) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        self.expect_arg_count("len", arguments, 1)?;

        // 对数组变量调用 len(arr) 返回跟踪的数组长度；
        // 字符串则使用 strlen。
        if let Expr::Ident(name) = &arguments[0] {
            if let Some(slot) = self.scope_lookup(name) {
                if Self::is_map_slot(&slot) {
                    return Err(HuziError::new_global(
                        "len() does not support HashMap; use map_len()",
                    ));
                }
                if Self::is_vec_slot(&slot) {
                    return self.vec_len_value(name);
                }
                if let Some(len) = slot.array_len {
                    return Ok(self.context.i32_type().const_int(len as u64, false).into());
                }
            }
        }

        // 对结构体数组成员调用 len(s.arr) 使用声明的数组大小；
        // 对 vec 成员调用 len(s.items) 则返回其动态长度。
        if let Expr::FieldAccess(fa) = &arguments[0] {
            if let Some((_, fields)) = self.struct_def_of_expr(&fa.base) {
                if let Some(info) = fields.iter().find(|info| info.name == fa.field) {
                    if let Type::Array(_, size) = &info.ast_ty {
                        return Ok(self.context.i32_type().const_int(*size as u64, false).into());
                    }
                    if matches!(&info.ast_ty, Type::Applied(n, _) if n == "vec") {
                        let vec_val = self.compile_expr(&arguments[0])?;
                        if vec_val.is_struct_value() {
                            let len = self
                                .builder
                                .build_extract_value(vec_val.into_struct_value(), 1, "vec_field_len")
                                .unwrap();
                            return Ok(len);
                        }
                    }
                }
            }
        }

        let arg = self.compile_expr(&arguments[0])?;
        let arg = if arg.is_pointer_value() {
            arg.into_pointer_value()
        } else {
            return Err(HuziError::new_global("len() requires a string or array argument"));
        };

        let strlen_fn = self.module.get_function("strlen").unwrap();
        let len = self
            .builder
            .build_call(strlen_fn, &[arg.into()], "str_len")
            .unwrap()
            .try_as_basic_value()
            .unwrap_left();

        Ok(len)
    }
}
