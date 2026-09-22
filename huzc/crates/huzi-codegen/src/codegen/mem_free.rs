//! 手动内存释放:`free_str(s)` / `free_vec(v)` / `free_box(b)`。
//!
//! 约定：str/vec 不做 GC/RC，手动 free 或进程退出时由 OS 统一回收；Box 走引用计数
//! （RC），`free_box` 即 release，计数归零才真正释放（见 runtime.rs）。不调用 free
//! 也能正常运行。
//! free 后槽位置空安全态:str 指向空串(全局 `""`),vec 置 `{ null, 0, 0 }`,
//! Box 置 `null`。二次 free 为 no-op:str 空串跳过,vec 对 `free(null)`
//! (libc 语义即 no-op),Box 判空跳过。vec free 后可继续 `push` 复用
//! (`realloc(null, n)` 即 `malloc`)。
//!
//! 注意:str 字面量是全局常量而非堆分配,仅对堆字符串
//! (`concat`/`substring`/`trim`/`to_string`/`read_line`/`read_file` 等返回)
//! 调用 `free_str`;空串(含已 free)调用为安全的 no-op。`free_vec` 为浅释放,
//! 只释放 vec 自身的 `data` 数组,不释放元素内部的堆内存;`free_box` 只释放
//! Box 自身的 pointee 槽,不递归释放其字段中的堆内存。

use super::{CodeGen, VarSlot};
use huzi_ast::*;
use huzi_error::{HuziError, Result};
use inkwell::AddressSpace;
use inkwell::values::{BasicValueEnum, PointerValue};

impl<'ctx> CodeGen<'ctx> {
    /// 取变量名(首参必须为变量),并校验可变性(回写槽位需 `let mut`)。
    fn free_var_slot(&self, fname: &str, first: &Expr) -> Result<(String, VarSlot<'ctx>)> {
        let name = match first {
            Expr::Ident(name) => name.clone(),
            _ => {
                return Err(HuziError::new_global(format!(
                    "{}() argument must be a variable",
                    fname
                )))
            }
        };
        let slot = self
            .scope_lookup(&name)
            .ok_or_else(|| self.unknown_variable_error(&name))?;
        self.ensure_mutable(first)?;
        Ok((name, slot))
    }

    /// 空串全局指针(首次创建,后续复用同一全局,供 free 后安全态与判空;
    /// chan recv 的 null 句柄返回值亦复用)。
    pub(super) fn empty_str_ptr(&mut self) -> PointerValue<'ctx> {
        if let Some(g) = self.module.get_global("huzi_empty_str") {
            return g.as_pointer_value();
        }
        unsafe {
            self.builder
                .build_global_string("", "huzi_empty_str")
                .unwrap()
        }
        .as_pointer_value()
    }

    /// `free_str(s)` — 堆字符串 free 后指向空串;空串/已 free 为 no-op。
    /// 槽校验:指针类型 + 非 Box + 非定长数组 + 非 vec 结构体。
    pub(super) fn compile_free_str(
        &mut self,
        arguments: &[Expr],
    ) -> Result<BasicValueEnum<'ctx>> {
        if arguments.len() != 1 {
            return Err(HuziError::new_global("free_str() requires exactly 1 argument"));
        }
        let (_, slot) = self.free_var_slot("free_str", &arguments[0])?;
        if !slot.ty.is_pointer_type() || slot.box_inner.is_some() || slot.array_len.is_some() {
            return Err(HuziError::new_global(
                "free_str() requires a str variable (heap string from concat/substring/trim/to_string/read_line/read_file)",
            ));
        }
        if Self::is_vec_slot(&slot) {
            return Err(HuziError::new_global("free_str() requires a str variable, not a vec"));
        }
        let cur = self
            .builder
            .build_load(slot.ty, slot.ptr, "free_str_cur")
            .unwrap()
            .into_pointer_value();
        let not_null = self.ptr_is_not_null(cur);
        let slen = self.str_len_of(cur, "free_str_len")?;
        let nonzero = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::NE,
                slen,
                self.context.i32_type().const_int(0, false),
                "free_str_nz",
            )
            .unwrap();
        let should_free = self.builder.build_and(not_null, nonzero, "free_str_go").unwrap();
        let function = self.current_function()?;
        let do_bb = self.context.append_basic_block(function, "free_str_do");
        let done_bb = self.context.append_basic_block(function, "free_str_done");
        self.builder.build_conditional_branch(should_free, do_bb, done_bb).unwrap();
        self.builder.position_at_end(do_bb);
        let free_fn = self.module.get_function("free").expect("free in prelude");
        self.builder.build_call(free_fn, &[cur.into()], "free_str_call").unwrap();
        self.builder.build_unconditional_branch(done_bb).unwrap();
        self.builder.position_at_end(done_bb);
        let empty = self.empty_str_ptr();
        self.builder.build_store(slot.ptr, empty).unwrap();
        Ok(self.context.i32_type().const_int(0, false).into())
    }

    /// `free_vec(v)` — 释放 `data` 后置 `{ null, 0, 0 }`;二次 free 因
    /// `free(null)` 为 no-op;后续 `push` 经 `realloc(null)` 复用。
    pub(super) fn compile_free_vec(
        &mut self,
        arguments: &[Expr],
    ) -> Result<BasicValueEnum<'ctx>> {
        if arguments.len() != 1 {
            return Err(HuziError::new_global("free_vec() requires exactly 1 argument"));
        }
        let (_, slot) = self.free_var_slot("free_vec", &arguments[0])?;
        if !Self::is_vec_slot(&slot) {
            return Err(HuziError::new_global(
                "free_vec() requires a vec variable (use vec(...) to create one)",
            ));
        }
        let parts = self.load_vec_parts(&slot)?;
        let free_fn = self.module.get_function("free").expect("free in prelude");
        self.builder.build_call(free_fn, &[parts.data.into()], "free_vec_call").unwrap();
        let vec_ty = self.vec_struct_type();
        let null_data = self.context.ptr_type(AddressSpace::default()).const_null();
        let zero = self.context.i32_type().const_int(0, false);
        self.store_vec_parts(&slot, vec_ty, &super::vec::VecParts {
            data: null_data,
            len: zero,
            cap: zero,
        });
        Ok(self.context.i32_type().const_int(0, false).into())
    }

    /// `free_box(b)` — 非空时 release 后置 `null`;空/已 free 为 no-op。
    pub(super) fn compile_free_box(
        &mut self,
        arguments: &[Expr],
    ) -> Result<BasicValueEnum<'ctx>> {
        if arguments.len() != 1 {
            return Err(HuziError::new_global("free_box() requires exactly 1 argument"));
        }
        let (_, slot) = self.free_var_slot("free_box", &arguments[0])?;
        if !Self::is_box_slot(&slot) {
            return Err(HuziError::new_global(
                "free_box() requires a Box<T> variable (use box(...) to create one)",
            ));
        }
        let cur = self
            .builder
            .build_load(slot.ty, slot.ptr, "free_box_cur")
            .unwrap()
            .into_pointer_value();
        self.emit_release_box(cur)?;
        let null = self.context.ptr_type(AddressSpace::default()).const_null();
        self.builder.build_store(slot.ptr, null).unwrap();
        Ok(self.context.i32_type().const_int(0, false).into())
    }
}
