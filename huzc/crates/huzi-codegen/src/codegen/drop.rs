//! 确定性作用域析构引擎 (RAII Drop)。
//!
//! 支持 `Box<T>` 与 `vec<T>` 出作用域自动释放底层堆内存；
//! 通过对堆缓冲区统一采用 8 字节 RC 头部布局（`user_ptr - 8`），
//! 实现零额外抽象开销、无垃圾回收停顿的即时确定性自动回收。

use super::CodeGen;
use huzi_error::Result;
use inkwell::types::BasicTypeEnum;
use inkwell::values::PointerValue;

/// 自动析构目标种类。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DropKind {
    Box,
    Vec,
}

/// 登记在作用域中的可析构槽位。
#[derive(Clone, Copy)]
pub(crate) struct DroppableSlot<'ctx> {
    pub(crate) ptr: PointerValue<'ctx>,
    pub(crate) ty: BasicTypeEnum<'ctx>,
    pub(crate) kind: DropKind,
}

impl<'ctx> CodeGen<'ctx> {
    /// 向当前最内层作用域注册一个可析构槽位。
    pub(super) fn register_droppable(
        &mut self,
        ptr: PointerValue<'ctx>,
        ty: BasicTypeEnum<'ctx>,
        kind: DropKind,
    ) {
        if let Some(scope) = self.droppable_scopes.last_mut() {
            scope.push(DroppableSlot { ptr, ty, kind });
        }
    }

    /// 对单个槽位执行析构释放，并将其底层数据置空以防二次释放。
    pub(super) fn emit_drop_slot(&mut self, slot: &DroppableSlot<'ctx>) -> Result<()> {
        match slot.kind {
            DropKind::Box => self.emit_drop_box(slot.ptr, slot.ty),
            DropKind::Vec => self.emit_drop_vec(slot.ptr, slot.ty),
        }
    }

    /// 释放 Box 槽位并将其置为 null。
    pub(super) fn emit_drop_box(
        &mut self,
        slot_ptr: PointerValue<'ctx>,
        slot_ty: BasicTypeEnum<'ctx>,
    ) -> Result<()> {
        let cur = self
            .builder
            .build_load(slot_ty, slot_ptr, "drop_box_cur")
            .unwrap()
            .into_pointer_value();
        self.emit_release_box(cur)?;
        let null_ptr = slot_ty.into_pointer_type().const_null();
        self.builder.build_store(slot_ptr, null_ptr).unwrap();
        Ok(())
    }

    /// 释放 vec 槽位底层堆缓冲并将其清空为 `{ null, 0, 0 }`。
    pub(super) fn emit_drop_vec(
        &mut self,
        slot_ptr: PointerValue<'ctx>,
        slot_ty: BasicTypeEnum<'ctx>,
    ) -> Result<()> {
        let vec_val = self
            .builder
            .build_load(slot_ty, slot_ptr, "drop_vec_val")
            .unwrap()
            .into_struct_value();
        let data = self
            .builder
            .build_extract_value(vec_val, 0, "drop_vec_data")
            .unwrap()
            .into_pointer_value();
        self.emit_release_box(data)?;

        let null_ptr = self
            .context
            .ptr_type(inkwell::AddressSpace::default())
            .const_null();
        let zero = self.context.i32_type().const_int(0, false);
        let struct_ty = slot_ty.into_struct_type();
        let zero_struct = struct_ty.const_named_struct(&[null_ptr.into(), zero.into(), zero.into()]);
        self.builder.build_store(slot_ptr, zero_struct).unwrap();
        Ok(())
    }

    /// 析构当前最内层作用域中的所有变量（LIFO 逆序）。
    pub(super) fn emit_drop_current_scope(&mut self) -> Result<()> {
        if let Some(scope) = self.droppable_scopes.last() {
            let slots = scope.clone();
            for slot in slots.iter().rev() {
                self.emit_drop_slot(slot)?;
            }
        }
        Ok(())
    }

    /// 析构当前函数中所有活跃作用域的变量（除 `skip_ptrs` 外，供 return 与 try_op）。
    pub(super) fn emit_drop_all_scopes(
        &mut self,
        skip_ptrs: &[PointerValue<'ctx>],
    ) -> Result<()> {
        let all_scopes = self.droppable_scopes.clone();
        for scope in all_scopes.iter().rev() {
            for slot in scope.iter().rev() {
                if skip_ptrs.iter().any(|&skip| slot.ptr == skip) {
                    continue;
                }
                self.emit_drop_slot(slot)?;
            }
        }
        Ok(())
    }

    /// 重新分配 vec 堆底层缓冲，维护 8 字节 RC 头部。
    pub(super) fn realloc_vec_buffer(
        &mut self,
        data: PointerValue<'ctx>,
        is_empty: inkwell::values::IntValue<'ctx>,
        new_bytes: inkwell::values::IntValue<'ctx>,
    ) -> PointerValue<'ctx> {
        let i32_type = self.context.i32_type();
        let minus_eight = i32_type.const_int((-8i64) as u64, true);
        let eight = i32_type.const_int(8, false);
        let raw_from_data = unsafe {
            self.builder
                .build_gep(self.context.i8_type(), data, &[minus_eight], "vec_raw_old")
                .unwrap()
        };
        let null_raw = self
            .context
            .ptr_type(inkwell::AddressSpace::default())
            .const_null();
        let raw_to_realloc = self
            .builder
            .build_select(is_empty, null_raw, raw_from_data, "vec_raw_to_realloc")
            .unwrap()
            .into_pointer_value();
        let alloc_bytes = self
            .builder
            .build_int_add(new_bytes, eight, "vec_realloc_sz")
            .unwrap();
        let realloc_fn = self.module.get_function("realloc").expect("realloc in prelude");
        let new_raw = self
            .builder
            .build_call(realloc_fn, &[raw_to_realloc.into(), alloc_bytes.into()], "vec_realloc")
            .unwrap()
            .try_as_basic_value()
            .unwrap_left()
            .into_pointer_value();

        let function = self.current_function().unwrap();
        let init_rc_bb = self.context.append_basic_block(function, "vec_init_rc");
        let cont_bb = self.context.append_basic_block(function, "vec_realloc_cont");
        self.builder
            .build_conditional_branch(is_empty, init_rc_bb, cont_bb)
            .unwrap();

        self.builder.position_at_end(init_rc_bb);
        let one_i64 = self.context.i64_type().const_int(1, false);
        self.builder.build_store(new_raw, one_i64).unwrap();
        self.builder.build_unconditional_branch(cont_bb).unwrap();

        self.builder.position_at_end(cont_bb);
        unsafe {
            self.builder
                .build_gep(self.context.i8_type(), new_raw, &[eight], "vec_new_user")
                .unwrap()
        }
    }

    /// 为新 vec 分配堆底层缓冲并初始化 8 字节 RC 头部为 1，返回指向用户数据的首指针。
    pub(super) fn alloc_vec_buffer(
        &mut self,
        elem_type: inkwell::types::BasicTypeEnum<'ctx>,
        capacity: inkwell::values::IntValue<'ctx>,
        name: &str,
    ) -> Result<PointerValue<'ctx>> {
        let i32_type = self.context.i32_type();
        let elem_bytes = self.elem_bytes_i32(elem_type)?;
        let total_bytes = self.builder.build_int_mul(elem_bytes, capacity, "vb_bytes").unwrap();
        let eight = i32_type.const_int(8, false);
        let alloc_bytes = self.builder.build_int_add(total_bytes, eight, "vb_alloc_sz").unwrap();
        let malloc_fn = self.module.get_function("malloc").expect("malloc in prelude");
        let raw_ptr = self
            .builder
            .build_call(malloc_fn, &[alloc_bytes.into()], "vb_raw")
            .unwrap()
            .try_as_basic_value()
            .unwrap_left()
            .into_pointer_value();
        let one_i64 = self.context.i64_type().const_int(1, false);
        self.builder.build_store(raw_ptr, one_i64).unwrap();
        let data = unsafe {
            self.builder
                .build_gep(self.context.i8_type(), raw_ptr, &[eight], name)
                .unwrap()
        };
        Ok(data)
    }
}

