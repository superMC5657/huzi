//! `push(v, x)` 的追加实现:满时容量翻倍(空 vec 首扩 4),返回整数 0。

use super::{CodeGen, VecParts};
use huzi_ast::*;
use huzi_error::Result;
use inkwell::values::BasicValueEnum;

impl<'ctx> CodeGen<'ctx> {
    /// `push(v, x)` — 需 `let mut` 变量;满时容量翻倍(realloc),返回整数 0
    /// (与 srand/sleep_ms 一致,仅为兼容表达式位置)。
    pub(in super::super) fn compile_vec_push(
        &mut self,
        arguments: &[Expr],
    ) -> Result<BasicValueEnum<'ctx>> {
        self.expect_arg_count("push", arguments, 2)?;
        let name = self.first_var_name("push", &arguments[0], "first argument must be a vec variable")?;
        let slot = self.vec_slot_of(&name)?;
        self.ensure_mutable(&arguments[0])?;
        let elem_type = slot.elem.unwrap();
        let value = self.compile_expr(&arguments[1])?;
        let value = self.coerce_value(elem_type, value)?;

        let vec_ty = self.vec_struct_type();
        let parts = self.load_vec_parts(&slot)?;
        let i32_type = self.context.i32_type();

        // 满时翻倍扩容复用 `ensure_vec_capacity`(空 vec 首扩 4);
        // 返回重载后的 parts(data 可能已被 realloc 搬移)。
        let now = self.ensure_vec_capacity(&slot, &parts, elem_type)?;

        // 写入新元素并回写 (data, len+1, cap)。
        let elem_ptr = unsafe {
            self.builder
                .build_gep(elem_type, now.data, &[now.len], "vec_push_ptr")
                .unwrap()
        };
        self.builder.build_store(elem_ptr, value).unwrap();
        let new_len = self
            .builder
            .build_int_add(now.len, i32_type.const_int(1, false), "vec_new_len")
            .unwrap();
        self.store_vec_parts(&slot, vec_ty, &VecParts {
            data: now.data,
            len: new_len,
            cap: now.cap,
        });
        Ok(i32_type.const_int(0, false).into())
    }
}
