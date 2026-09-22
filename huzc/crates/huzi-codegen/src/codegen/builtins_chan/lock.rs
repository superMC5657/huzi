//! 通道自旋锁:模内私有 `__huzi_chan_lock`/`__huzi_chan_unlock`,基于
//! LLVM `atomicrmw xchg` 的 acquire/release 语义;获取失败睡 1ms 重试。

use huzi_error::Result;

use super::{CodeGen, CHAN_LOCK};

impl<'ctx> CodeGen<'ctx> {
    /// 自旋锁获取(模内私有函数,只生成一次):
    /// `atomicrmw xchg` 0→1(acquire),失败睡 1ms 重试。
    pub(super) fn lock_chan(&mut self, chan: inkwell::values::PointerValue<'ctx>) -> Result<()> {
        let p = self.chan_field_ptr(chan, CHAN_LOCK, "chan_lock_field")?;
        let (lock_fn, _) = self.ensure_chan_lock_fns()?;
        self.builder
            .build_call(lock_fn, &[p.into()], "chan_lock_call")
            .unwrap();
        Ok(())
    }

    /// 释放锁:`atomicrmw xchg` → 0(release)。
    pub(super) fn unlock_chan(&mut self, chan: inkwell::values::PointerValue<'ctx>) -> Result<()> {
        let p = self.chan_field_ptr(chan, CHAN_LOCK, "chan_lock_field")?;
        let (_, unlock_fn) = self.ensure_chan_lock_fns()?;
        self.builder
            .build_call(unlock_fn, &[p.into()], "chan_unlock_call")
            .unwrap();
        Ok(())
    }

    /// `__huzi_chan_lock(i64*)` / `__huzi_chan_unlock(i64*)`,模内私有。
    fn ensure_chan_lock_fns(
        &mut self,
    ) -> Result<(inkwell::values::FunctionValue<'ctx>, inkwell::values::FunctionValue<'ctx>)> {
        if let (Some(l), Some(u)) = (
            self.module.get_function("__huzi_chan_lock"),
            self.module.get_function("__huzi_chan_unlock"),
        ) {
            return Ok((l, u));
        }
        let i64_ty = self.context.i64_type();
        let ptr_ty = self.context.ptr_type(inkwell::AddressSpace::default());
        let lock_ty = self.context.void_type().fn_type(&[ptr_ty.into()], false);

        let lock_fn = self.module.add_function("__huzi_chan_lock", lock_ty, None);
        let saved = self.builder.get_insert_block();
        let entry = self.context.append_basic_block(lock_fn, "entry");
        self.builder.position_at_end(entry);
        let lock_param = lock_fn.get_first_param().unwrap().into_pointer_value();
        let one = i64_ty.const_int(1, false);
        let spin_bb = self.context.append_basic_block(lock_fn, "spin");
        let sleep_bb = self.context.append_basic_block(lock_fn, "spin_sleep");
        let done_bb = self.context.append_basic_block(lock_fn, "spin_done");
        self.builder.build_unconditional_branch(spin_bb).unwrap();

        self.builder.position_at_end(spin_bb);
        let old = self
            .builder
            .build_atomicrmw(
                inkwell::AtomicRMWBinOp::Xchg,
                lock_param,
                one,
                inkwell::AtomicOrdering::Acquire,
            )
            .unwrap();
        let got_it = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                old,
                i64_ty.const_int(0, false),
                "chan_got_lock",
            )
            .unwrap();
        self.builder
            .build_conditional_branch(got_it, done_bb, sleep_bb)
            .unwrap();

        self.builder.position_at_end(sleep_bb);
        self.emit_chan_sleep()?;
        self.builder.build_unconditional_branch(spin_bb).unwrap();

        self.builder.position_at_end(done_bb);
        self.builder.build_return(None).unwrap();

        let unlock_fn = self.emit_chan_unlock_fn(lock_ty)?;

        if let Some(b) = saved {
            self.builder.position_at_end(b);
        }
        Ok((lock_fn, unlock_fn))
    }

    /// 释放锁函数体:`atomicrmw xchg` → 0(release)后返回。
    fn emit_chan_unlock_fn(
        &mut self,
        lock_ty: inkwell::types::FunctionType<'ctx>,
    ) -> Result<inkwell::values::FunctionValue<'ctx>> {
        let i64_ty = self.context.i64_type();
        let unlock_fn = self.module.add_function("__huzi_chan_unlock", lock_ty, None);
        let entry = self.context.append_basic_block(unlock_fn, "entry");
        self.builder.position_at_end(entry);
        let unlock_param = unlock_fn.get_first_param().unwrap().into_pointer_value();
        self.builder
            .build_atomicrmw(
                inkwell::AtomicRMWBinOp::Xchg,
                unlock_param,
                i64_ty.const_int(0, false),
                inkwell::AtomicOrdering::Release,
            )
            .unwrap();
        self.builder.build_return(None).unwrap();
        Ok(unlock_fn)
    }

    /// 睡 1ms(Windows `Sleep`,POSIX `usleep`;均已在 prelude 声明)。
    /// 按编译器宿主平台选择,与 prelude 的声明条件一致;交叉编译
    /// 场景暂不支持(STATUS 同此口径)。
    pub(super) fn emit_chan_sleep(&mut self) -> Result<()> {
        let one_ms = self.context.i32_type().const_int(1, false);
        if cfg!(windows) {
            let f = self.module.get_function("Sleep").unwrap();
            self.builder
                .build_call(f, &[one_ms.into()], "chan_sleep")
                .unwrap();
        } else {
            let f = self.module.get_function("usleep").unwrap();
            let us = self
                .builder
                .build_int_mul(one_ms, self.context.i32_type().const_int(1000, false), "ms_to_us")
                .unwrap();
            self.builder
                .build_call(f, &[us.into()], "chan_sleep")
                .unwrap();
        }
        Ok(())
    }
}
