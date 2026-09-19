//! 通道结构体布局访问:字段 GEP/load/store、句柄转换、环形下标、
//! 槽位寻址与消息堆拷贝。

use huzi_ast::Expr;
use huzi_error::{HuziError, Result};
use inkwell::values::{BasicValueEnum, IntValue, PointerValue};

use super::{CodeGen, CHAN_MAX_CAP, CHAN_SLOTS};

impl<'ctx> CodeGen<'ctx> {
    // ---- 内部助手 ----

    /// 句柄实参 → i64 零扩展 → 结构体指针(可能是 null,由调用方分支)。
    pub(super) fn chan_handle_ptr(&mut self, expr: &Expr) -> Result<PointerValue<'ctx>> {
        let val = self.compile_expr(expr)?;
        let iv = match val {
            BasicValueEnum::IntValue(i) => {
                if i.get_type().get_bit_width() < 64 {
                    self.builder
                        .build_int_z_extend(i, self.context.i64_type(), "ch64")
                        .unwrap()
                } else {
                    i
                }
            }
            _ => return Err(HuziError::new_global("chan handle must be an integer (i64)")),
        };
        let ptr_ty = self.context.ptr_type(inkwell::AddressSpace::default());
        Ok(self.builder.build_int_to_ptr(iv, ptr_ty, "ch_ptr").unwrap())
    }

    fn i64_const(&self, v: u64) -> IntValue<'ctx> {
        self.context.i64_type().const_int(v, false)
    }

    pub(super) fn chan_field_ptr(
        &self,
        chan: PointerValue<'ctx>,
        offset: u64,
        name: &str,
    ) -> Result<PointerValue<'ctx>> {
        let off = self.i64_const(offset);
        let p = unsafe {
            self.builder
                .build_gep(self.context.i8_type(), chan, &[off.into()], name)
                .unwrap()
        };
        Ok(p)
    }

    pub(super) fn load_chan_i32(&self, chan: PointerValue<'ctx>, offset: u64) -> Result<IntValue<'ctx>> {
        let p = self.chan_field_ptr(chan, offset, "chan_field")?;
        let v = self
            .builder
            .build_load(self.context.i32_type(), p, "chan_i32")
            .unwrap();
        Ok(v.into_int_value())
    }

    pub(super) fn store_chan_i32(
        &self,
        chan: PointerValue<'ctx>,
        offset: u64,
        val: IntValue<'ctx>,
    ) -> Result<()> {
        let p = self.chan_field_ptr(chan, offset, "chan_field")?;
        self.builder.build_store(p, val).unwrap();
        Ok(())
    }

    pub(super) fn load_chan_ptr(&self, chan: PointerValue<'ctx>, offset: u64) -> Result<PointerValue<'ctx>> {
        let p = self.chan_field_ptr(chan, offset, "chan_field")?;
        let v = self
            .builder
            .build_load(self.context.ptr_type(inkwell::AddressSpace::default()), p, "chan_ptr")
            .unwrap();
        Ok(v.into_pointer_value())
    }

    pub(super) fn store_chan_ptr(
        &self,
        chan: PointerValue<'ctx>,
        offset: u64,
        val: PointerValue<'ctx>,
    ) -> Result<()> {
        let p = self.chan_field_ptr(chan, offset, "chan_field")?;
        self.builder.build_store(p, val).unwrap();
        Ok(())
    }

    /// 容量收敛到 1..=CHAN_MAX_CAP。
    pub(super) fn clamp_chan_cap(&mut self, cap: IntValue<'ctx>) -> Result<IntValue<'ctx>> {
        let i32_ty = self.context.i32_type();
        let one = i32_ty.const_int(1, false);
        let max = i32_ty.const_int(CHAN_MAX_CAP as u64, false);
        let ge_one = self
            .builder
            .build_int_compare(inkwell::IntPredicate::SGE, cap, one, "chan_cap_ge1")
            .unwrap();
        let lo = self
            .builder
            .build_select(ge_one, cap, one, "chan_cap_lo")
            .unwrap()
            .into_int_value();
        let le_max = self
            .builder
            .build_int_compare(inkwell::IntPredicate::SLE, lo, max, "chan_cap_lemax")
            .unwrap();
        let capped = self
            .builder
            .build_select(le_max, lo, max, "chan_cap")
            .unwrap();
        match capped {
            BasicValueEnum::IntValue(iv) => Ok(iv),
            _ => Err(HuziError::new_global("chan_new: cap select produced non-int")),
        }
    }

    /// 环形下标:idx % cap(索引非负)。
    pub(super) fn ring_mod(&self, idx: IntValue<'ctx>, cap: IntValue<'ctx>) -> Result<IntValue<'ctx>> {
        Ok(self
            .builder
            .build_int_signed_rem(idx, cap, "chan_ring")
            .unwrap())
    }

    /// 槽位地址:slots + idx(指针槽宽 8 字节,用 i64 GEP 缩放)。
    pub(super) fn slot_ptr(
        &self,
        chan: PointerValue<'ctx>,
        idx: IntValue<'ctx>,
    ) -> Result<PointerValue<'ctx>> {
        let slots = self.load_chan_ptr(chan, CHAN_SLOTS)?;
        let p = unsafe {
            self.builder
                .build_gep(self.context.i64_type(), slots, &[idx.into()], "chan_slot")
                .unwrap()
        };
        Ok(p)
    }

    pub(super) fn store_slot(&self, slot: PointerValue<'ctx>, msg: PointerValue<'ctx>) -> Result<()> {
        self.builder.build_store(slot, msg).unwrap();
        Ok(())
    }

    pub(super) fn load_slot(&self, slot: PointerValue<'ctx>) -> Result<PointerValue<'ctx>> {
        let v = self
            .builder
            .build_load(self.context.ptr_type(inkwell::AddressSpace::default()), slot, "chan_msg")
            .unwrap();
        Ok(v.into_pointer_value())
    }

    /// 消息堆拷贝:malloc(len+1),由调用方 strcpy 填充。
    pub(super) fn copy_str(&mut self, msg: PointerValue<'ctx>) -> Result<PointerValue<'ctx>> {
        let strlen_fn = self.module.get_function("strlen").unwrap();
        let n = self
            .builder
            .build_call(strlen_fn, &[msg.into()], "chan_msg_len")
            .unwrap()
            .try_as_basic_value()
            .unwrap_left()
            .into_int_value();
        let one = self.context.i32_type().const_int(1, false);
        let size = self
            .builder
            .build_int_add(n, one, "chan_msg_size")
            .unwrap();
        let malloc_fn = self.module.get_function("malloc").unwrap();
        let buf = self
            .builder
            .build_call(malloc_fn, &[size.into()], "chan_msg_buf")
            .unwrap()
            .try_as_basic_value()
            .unwrap_left()
            .into_pointer_value();
        Ok(buf)
    }
}
