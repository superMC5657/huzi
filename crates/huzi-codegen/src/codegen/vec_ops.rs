//! Vec 增删操作:`pop(v)` / `remove(v, idx)` / `insert(v, idx, val)` / `clear(v)`。
//!
//! 与 `vec.rs` 同属 vec 模块:构造/push/下标/len 留在 `vec.rs`,本文件只放
//! 增删四个内置函数。内存布局不变 `{ data: ptr, len: i32, cap: i32 }`;
//! 扩容复用 push 的 realloc 翻倍逻辑(空 vec 首扩 4);下标检查复用
//! `emit_vec_bounds_check`(无符号比较,负下标自然落入失败分支);元素搬移
//! 用下标循环逐个 load/store(元素类型任意,无需 memcpy)。
//!
//! 返回语义:只有 `pop` 返回弹出值;`remove`/`insert`/`clear` 与 push 一致
//! 返回整数 0(仅为兼容表达式位置),不发明其它语义。

use super::vec::VecParts;
use super::{CodeGen, VarSlot};
use huzi_ast::*;
use huzi_error::{HuziError, Result};
use inkwell::values::{BasicValueEnum, IntValue, PointerValue};

impl<'ctx> CodeGen<'ctx> {
    /// 取增删操作的首个 vec 变量槽(非 vec 或不可变时报友好错误)。
    fn vec_op_slot(&mut self, fname: &str, first: &Expr) -> Result<VarSlot<'ctx>> {
        let name = match first {
            Expr::Ident(name) => name.clone(),
            _ => {
                return Err(HuziError::new_global(format!(
                    "{}() first argument must be a vec variable",
                    fname
                )))
            }
        };
        let slot = self.vec_slot_of(&name)?;
        self.ensure_mutable(first)?;
        Ok(slot)
    }

    /// 满时翻倍扩容(空 vec 首扩 4),返回重载后的 parts(调用方必须用它,
    /// data 可能已被 realloc 搬移)。与 `compile_vec_push` 的 grow 逻辑一致。
    fn ensure_vec_capacity(
        &mut self,
        slot: &VarSlot<'ctx>,
        parts: &VecParts<'ctx>,
        elem_type: inkwell::types::BasicTypeEnum<'ctx>,
    ) -> Result<VecParts<'ctx>> {
        let vec_ty = self.vec_struct_type();
        let i32_type = self.context.i32_type();
        let function = self.current_function()?;
        let grow_block = self.context.append_basic_block(function, "vec_grow");
        let done_block = self.context.append_basic_block(function, "vec_cap_done");
        let full = self
            .builder
            .build_int_compare(inkwell::IntPredicate::EQ, parts.len, parts.cap, "vec_full")
            .unwrap();
        self.builder
            .build_conditional_branch(full, grow_block, done_block)
            .unwrap();

        self.builder.position_at_end(grow_block);
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
        let realloc_fn = self.module.get_function("realloc").expect("realloc in prelude");
        let new_data = self
            .builder
            .build_call(realloc_fn, &[parts.data.into(), new_bytes.into()], "vec_realloc")
            .unwrap()
            .try_as_basic_value()
            .unwrap_left()
            .into_pointer_value();
        self.store_vec_parts(&slot, vec_ty, &VecParts {
            data: new_data,
            len: parts.len,
            cap: new_cap,
        });
        self.builder.build_unconditional_branch(done_block).unwrap();

        self.builder.position_at_end(done_block);
        Ok(self.load_vec_parts(slot)?)
    }

    /// 左移:`data[i-1] = data[i]`,i in `from..len`(`from == len` 时零次)。
    fn emit_vec_shift_left(
        &mut self,
        data: PointerValue<'ctx>,
        elem_type: inkwell::types::BasicTypeEnum<'ctx>,
        from: IntValue<'ctx>,
        len: IntValue<'ctx>,
    ) -> Result<()> {
        let i32_type = self.context.i32_type();
        let function = self.current_function()?;
        let cond_block = self.context.append_basic_block(function, "vec_shift_cond");
        let body_block = self.context.append_basic_block(function, "vec_shift_body");
        let end_block = self.context.append_basic_block(function, "vec_shift_end");
        let cursor = self.build_alloca(i32_type.into(), "vec_shift_i")?;
        self.builder.build_store(cursor, from).unwrap();
        self.builder.build_unconditional_branch(cond_block).unwrap();

        self.builder.position_at_end(cond_block);
        let cur = self.builder.build_load(i32_type, cursor, "vec_shift_cur").unwrap().into_int_value();
        let go = self
            .builder
            .build_int_compare(inkwell::IntPredicate::ULT, cur, len, "vec_shift_go")
            .unwrap();
        self.builder.build_conditional_branch(go, body_block, end_block).unwrap();

        self.builder.position_at_end(body_block);
        let src = unsafe { self.builder.build_gep(elem_type, data, &[cur], "vec_shift_src").unwrap() };
        let val = self.builder.build_load(elem_type, src, "vec_shift_val").unwrap();
        let prev = self.builder.build_int_sub(cur, i32_type.const_int(1, false), "vec_shift_prev").unwrap();
        let dst = unsafe { self.builder.build_gep(elem_type, data, &[prev], "vec_shift_dst").unwrap() };
        self.builder.build_store(dst, val).unwrap();
        let next = self.builder.build_int_add(cur, i32_type.const_int(1, false), "vec_shift_next").unwrap();
        self.builder.build_store(cursor, next).unwrap();
        self.builder.build_unconditional_branch(cond_block).unwrap();

        self.builder.position_at_end(end_block);
        Ok(())
    }

    /// 右移:`data[i+1] = data[i]`,i 从 `len-1` 递减到 `from`;
    /// 入口先检查 `from < len`,零次时直接跳过(尾部 insert 无需搬移)。
    fn emit_vec_shift_right(
        &mut self,
        data: PointerValue<'ctx>,
        elem_type: inkwell::types::BasicTypeEnum<'ctx>,
        from: IntValue<'ctx>,
        len: IntValue<'ctx>,
    ) -> Result<()> {
        let i32_type = self.context.i32_type();
        let one = i32_type.const_int(1, false);
        let function = self.current_function()?;
        let init_block = self.context.append_basic_block(function, "vec_rshift_init");
        let cond_block = self.context.append_basic_block(function, "vec_rshift_cond");
        let body_block = self.context.append_basic_block(function, "vec_rshift_body");
        let end_block = self.context.append_basic_block(function, "vec_rshift_end");
        let need = self
            .builder
            .build_int_compare(inkwell::IntPredicate::ULT, from, len, "vec_rshift_need")
            .unwrap();
        self.builder.build_conditional_branch(need, init_block, end_block).unwrap();

        self.builder.position_at_end(init_block);
        let cursor = self.build_alloca(i32_type.into(), "vec_rshift_i")?;
        let last = self.builder.build_int_sub(len, one, "vec_rshift_last").unwrap();
        self.builder.build_store(cursor, last).unwrap();
        self.builder.build_unconditional_branch(cond_block).unwrap();

        self.builder.position_at_end(cond_block);
        let cur = self.builder.build_load(i32_type, cursor, "vec_rshift_cur").unwrap().into_int_value();
        let go = self
            .builder
            .build_int_compare(inkwell::IntPredicate::SGE, cur, from, "vec_rshift_go")
            .unwrap();
        self.builder.build_conditional_branch(go, body_block, end_block).unwrap();

        self.builder.position_at_end(body_block);
        let src = unsafe { self.builder.build_gep(elem_type, data, &[cur], "vec_rshift_src").unwrap() };
        let val = self.builder.build_load(elem_type, src, "vec_rshift_val").unwrap();
        let next = self.builder.build_int_add(cur, one, "vec_rshift_next").unwrap();
        let dst = unsafe { self.builder.build_gep(elem_type, data, &[next], "vec_rshift_dst").unwrap() };
        self.builder.build_store(dst, val).unwrap();
        let prev = self.builder.build_int_sub(cur, one, "vec_rshift_prev").unwrap();
        self.builder.build_store(cursor, prev).unwrap();
        self.builder.build_unconditional_branch(cond_block).unwrap();

        self.builder.position_at_end(end_block);
        Ok(())
    }

    /// `pop(v)` — 弹出末尾元素并返回;空 vec 报运行时错误。
    pub(super) fn compile_vec_pop(
        &mut self,
        arguments: &[Expr],
    ) -> Result<BasicValueEnum<'ctx>> {
        if arguments.len() != 1 {
            return Err(HuziError::new_global("pop() requires exactly 1 argument (vec)"));
        }
        let slot = self.vec_op_slot("pop", &arguments[0])?;
        let elem_type = slot.elem.unwrap();
        let parts = self.load_vec_parts(&slot)?;
        let i32_type = self.context.i32_type();
        let non_empty = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::NE,
                parts.len,
                i32_type.const_int(0, false),
                "vec_pop_nz",
            )
            .unwrap();
        self.emit_runtime_check(non_empty, "Runtime error: vec pop from empty vec\n\0", &[])?;
        let last = self
            .builder
            .build_int_sub(parts.len, i32_type.const_int(1, false), "vec_pop_last")
            .unwrap();
        let elem_ptr = unsafe {
            self.builder.build_gep(elem_type, parts.data, &[last], "vec_pop_ptr").unwrap()
        };
        let value = self.builder.build_load(elem_type, elem_ptr, "vec_pop_val").unwrap();
        let vec_ty = self.vec_struct_type();
        self.store_vec_parts(&slot, vec_ty, &VecParts {
            data: parts.data,
            len: last,
            cap: parts.cap,
        });
        Ok(value)
    }

    /// `remove(v, idx)` — 删下标处元素并左移填补;越界报运行时错误。
    /// 返回整数 0(与 push 一致,仅为兼容表达式位置)。
    pub(super) fn compile_vec_remove(
        &mut self,
        arguments: &[Expr],
    ) -> Result<BasicValueEnum<'ctx>> {
        if arguments.len() != 2 {
            return Err(HuziError::new_global("remove() requires exactly 2 arguments (vec, index)"));
        }
        let slot = self.vec_op_slot("remove", &arguments[0])?;
        let elem_type = slot.elem.unwrap();
        let parts = self.load_vec_parts(&slot)?;
        let i32_type = self.context.i32_type();
        let index_val = self.compile_expr(&arguments[1])?;
        let index_i32 = self.coerce_index(index_val)?;
        self.emit_vec_bounds_check(parts.len, index_i32)?;
        let from = self
            .builder
            .build_int_add(index_i32, i32_type.const_int(1, false), "vec_remove_from")
            .unwrap();
        self.emit_vec_shift_left(parts.data, elem_type, from, parts.len)?;
        let new_len = self
            .builder
            .build_int_sub(parts.len, i32_type.const_int(1, false), "vec_remove_len")
            .unwrap();
        let vec_ty = self.vec_struct_type();
        self.store_vec_parts(&slot, vec_ty, &VecParts {
            data: parts.data,
            len: new_len,
            cap: parts.cap,
        });
        Ok(i32_type.const_int(0, false).into())
    }

    /// `insert(v, idx, val)` — `idx` 允许 `0..=len`(尾插等价 push);
    /// 超出报运行时错误;满时先翻倍扩容再右移腾位。
    /// 返回整数 0(与 push 一致,仅为兼容表达式位置)。
    pub(super) fn compile_vec_insert(
        &mut self,
        arguments: &[Expr],
    ) -> Result<BasicValueEnum<'ctx>> {
        if arguments.len() != 3 {
            return Err(HuziError::new_global(
                "insert() requires exactly 3 arguments (vec, index, value)",
            ));
        }
        let slot = self.vec_op_slot("insert", &arguments[0])?;
        let elem_type = slot.elem.unwrap();
        let index_val = self.compile_expr(&arguments[1])?;
        let index_i32 = self.coerce_index(index_val)?;
        let value = self.compile_expr(&arguments[2])?;
        let value = self.coerce_value(elem_type, value)?;
        let parts = self.load_vec_parts(&slot)?;
        let in_range = self
            .builder
            .build_int_compare(inkwell::IntPredicate::ULE, index_i32, parts.len, "vec_insert_ok")
            .unwrap();
        self.emit_runtime_check(in_range, "Runtime error: vec index out of bounds\n\0", &[])?;
        let grown = self.ensure_vec_capacity(&slot, &parts, elem_type)?;
        self.emit_vec_shift_right(grown.data, elem_type, index_i32, grown.len)?;
        let dst = unsafe {
            self.builder.build_gep(elem_type, grown.data, &[index_i32], "vec_insert_ptr").unwrap()
        };
        self.builder.build_store(dst, value).unwrap();
        let i32_type = self.context.i32_type();
        let new_len = self
            .builder
            .build_int_add(grown.len, i32_type.const_int(1, false), "vec_insert_len")
            .unwrap();
        let vec_ty = self.vec_struct_type();
        self.store_vec_parts(&slot, vec_ty, &VecParts {
            data: grown.data,
            len: new_len,
            cap: grown.cap,
        });
        Ok(i32_type.const_int(0, false).into())
    }

    /// `clear(v)` — 长度置 0(data/cap 保留,后续 push 复用已有容量)。
    /// 返回整数 0(与 push 一致,仅为兼容表达式位置)。
    pub(super) fn compile_vec_clear(
        &mut self,
        arguments: &[Expr],
    ) -> Result<BasicValueEnum<'ctx>> {
        if arguments.len() != 1 {
            return Err(HuziError::new_global("clear() requires exactly 1 argument (vec)"));
        }
        let slot = self.vec_op_slot("clear", &arguments[0])?;
        let parts = self.load_vec_parts(&slot)?;
        let vec_ty = self.vec_struct_type();
        let i32_type = self.context.i32_type();
        self.store_vec_parts(&slot, vec_ty, &VecParts {
            data: parts.data,
            len: i32_type.const_int(0, false),
            cap: parts.cap,
        });
        Ok(i32_type.const_int(0, false).into())
    }

    /// 从原始结构体指针装载 (data, len, cap) 三件套。
    pub(super) fn load_vec_parts_from_ptr(
        &mut self,
        ptr: PointerValue<'ctx>,
        ty: inkwell::types::BasicTypeEnum<'ctx>,
    ) -> Result<VecParts<'ctx>> {
        let vec_val = self
            .builder
            .build_load(ty, ptr, "vec_load")
            .unwrap()
            .into_struct_value();
        let data = self
            .builder
            .build_extract_value(vec_val, 0, "vec_data")
            .unwrap()
            .into_pointer_value();
        let len = self
            .builder
            .build_extract_value(vec_val, 1, "vec_len")
            .unwrap()
            .into_int_value();
        let cap = self
            .builder
            .build_extract_value(vec_val, 2, "vec_cap")
            .unwrap()
            .into_int_value();
        Ok(VecParts { data, len, cap })
    }
}
