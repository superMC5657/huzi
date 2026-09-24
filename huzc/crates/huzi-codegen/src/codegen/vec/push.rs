//! `push(v, x)` 的追加实现:满时容量翻倍(空 vec 首扩 4),返回整数 0。

use super::{CodeGen, VecParts};
use huzi_ast::*;
use huzi_error::{HuziError, Result};
use inkwell::values::BasicValueEnum;

impl<'ctx> CodeGen<'ctx> {
    /// `push(v, x)` — 需 `let mut` 变量;满时容量翻倍(realloc),返回整数 0
    /// (与 srand/sleep_ms 一致,仅为兼容表达式位置)。
    pub(in super::super) fn compile_vec_push(
        &mut self,
        arguments: &[Expr],
    ) -> Result<BasicValueEnum<'ctx>> {
        if arguments.len() != 2 {
            return Err(HuziError::new_global("push() requires exactly 2 arguments (vec, value)"));
        }
        let name = match &arguments[0] {
            Expr::Ident(name) => name.clone(),
            _ => {
                return Err(HuziError::new_global(
                    "push() first argument must be a vec variable",
                ))
            }
        };
        let slot = self.vec_slot_of(&name)?;
        self.ensure_mutable(&arguments[0])?;
        let elem_type = slot.elem.unwrap();
        let value = self.compile_expr(&arguments[1])?;
        let value = self.coerce_value(elem_type, value)?;

        let vec_ty = self.vec_struct_type();
        let parts = self.load_vec_parts(&slot)?;
        let i32_type = self.context.i32_type();

        // len == cap 时翻倍扩容;grow 块内直接回写新 (data, cap),
        // append 块重载合并(内存即合并点,无需 phi)。
        let function = self.current_function()?;
        let grow_block = self.context.append_basic_block(function, "vec_grow");
        let append_block = self.context.append_basic_block(function, "vec_append");
        let full = self
            .builder
            .build_int_compare(inkwell::IntPredicate::EQ, parts.len, parts.cap, "vec_full")
            .unwrap();
        self.builder
            .build_conditional_branch(full, grow_block, append_block)
            .unwrap();

        self.builder.position_at_end(grow_block);
        // 空 vec 首 push 时 cap 为 0,按初始容量 4 分配;否则翻倍。
        let doubled = self
            .builder
            .build_int_mul(parts.cap, i32_type.const_int(2, false), "vec_doubled")
            .unwrap();
        let is_empty = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                parts.cap,
                i32_type.const_int(0, false),
                "vec_is_empty",
            )
            .unwrap();
        let new_cap = self
            .builder
            .build_select(is_empty, i32_type.const_int(4, false), doubled, "vec_new_cap")
            .unwrap()
            .into_int_value();
        let elem_bytes = self.elem_bytes_i32(elem_type)?;
        let new_bytes = self
            .builder
            .build_int_mul(new_cap, elem_bytes, "vec_new_bytes")
            .unwrap();
        let new_data = self.realloc_vec_buffer(parts.data, is_empty, new_bytes);
        self.store_vec_parts(&slot, vec_ty, &VecParts {
            data: new_data,
            len: parts.len,
            cap: new_cap,
        });
        self.builder
            .build_unconditional_branch(append_block)
            .unwrap();

        // 写入新元素并回写 (data, len+1, cap)。
        self.builder.position_at_end(append_block);
        let now = self.load_vec_parts(&slot)?;
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
