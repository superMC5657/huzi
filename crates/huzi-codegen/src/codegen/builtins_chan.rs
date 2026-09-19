//! 通道(chan)内置函数:跨线程字符串消息队列。
//!
//! `chan_new(cap) -> i64` / `chan_send(ch, msg) -> bool` / `chan_recv(ch) -> str`。
//! 实现为固定容量环形缓冲:自旋锁(LLVM `atomicrmw xchg`)保护索引,满发/空收
//! 时解锁并睡 1ms 轮询等待。句柄是堆结构体指针的 i64 整数形式,可直接作为
//! `spawn` 实参传给线程;句柄为 null 时 send 返回 false、recv 返回空串。
//! 不提供 chan_free——通道随进程退出回收(与 str/vec 的内存约定一致)。

use huzi_ast::Expr;
use huzi_error::{HuziError, Result};
use inkwell::values::{BasicValueEnum, IntValue, PointerValue};

use super::CodeGen;

// chan 结构体字节布局(64 位平台):lock@0(i64) cap@8 count@12 head@16
// (pad@20) slots@24(ptr),共 32 字节;malloc 16 字节对齐满足 i64。
const CHAN_LOCK: u64 = 0;
const CHAN_CAP: u64 = 8;
const CHAN_COUNT: u64 = 12;
const CHAN_HEAD: u64 = 16;
const CHAN_SLOTS: u64 = 24;
const CHAN_SIZE: u64 = 32;
const CHAN_MAX_CAP: i64 = 4096;

impl<'ctx> CodeGen<'ctx> {
    /// `chan_new(cap: i32) -> i64`
    pub(super) fn compile_chan_new(&mut self, arguments: &[Expr]) -> Result<BasicValueEnum<'ctx>> {
        if arguments.len() != 1 {
            return Err(HuziError::new_global(
                "chan_new() requires 1 argument: (cap: i32)",
            ));
        }
        let cap_raw = self.i32_builtin_arg(arguments, "chan_new")?;
        let cap = self.clamp_chan_cap(cap_raw)?;

        let malloc_fn = self.module.get_function("malloc").unwrap();
        let chan_ptr = self
            .builder
            .build_call(
                malloc_fn,
                &[self.context.i32_type().const_int(CHAN_SIZE, false).into()],
                "chan_struct",
            )
            .unwrap()
            .try_as_basic_value()
            .unwrap_left()
            .into_pointer_value();

        let zero32 = self.context.i32_type().const_int(0, false);
        self.store_chan_i32(chan_ptr, CHAN_CAP, cap)?;
        self.store_chan_i32(chan_ptr, CHAN_COUNT, zero32)?;
        self.store_chan_i32(chan_ptr, CHAN_HEAD, zero32)?;

        // 槽位数组:cap 个指针槽。
        let slot_bytes = self
            .builder
            .build_int_mul(cap, self.context.i32_type().const_int(8, false), "slot_bytes")
            .unwrap();
        let slots = self
            .builder
            .build_call(malloc_fn, &[slot_bytes.into()], "chan_slots")
            .unwrap()
            .try_as_basic_value()
            .unwrap_left()
            .into_pointer_value();
        self.store_chan_ptr(chan_ptr, CHAN_SLOTS, slots)?;
        self.unlock_chan(chan_ptr)?;

        let handle = self
            .builder
            .build_ptr_to_int(chan_ptr, self.context.i64_type(), "chan_handle")
            .unwrap();
        Ok(handle.into())
    }

    /// `chan_send(ch: i64, msg: str) -> bool`:缓冲满时阻塞直至有空间;
    /// 句柄为 null 返回 false。
    pub(super) fn compile_chan_send(&mut self, arguments: &[Expr]) -> Result<BasicValueEnum<'ctx>> {
        if arguments.len() != 2 {
            return Err(HuziError::new_global(
                "chan_send() requires 2 arguments: (ch: i64, msg: str)",
            ));
        }
        let chan_ptr = self.chan_handle_ptr(&arguments[0])?;
        let msg = self.str_ptr_arg(&arguments[1], "chan_send()")?;

        let strcpy_fn = self.module.get_function("strcpy").unwrap();
        let function = self.current_function()?;
        let i32_ty = self.context.i32_type();
        let result_alloca = self.build_alloca(self.context.bool_type().into(), "chan_send_res")?;

        let invalid_bb = self.context.append_basic_block(function, "chan_send_invalid");
        let loop_bb = self.context.append_basic_block(function, "chan_send_loop");
        let write_bb = self.context.append_basic_block(function, "chan_send_write");
        let full_bb = self.context.append_basic_block(function, "chan_send_full");
        let after_bb = self.context.append_basic_block(function, "chan_send_done");

        let is_null = self.builder.build_is_null(chan_ptr, "ch_null").unwrap();
        self.builder
            .build_conditional_branch(is_null, invalid_bb, loop_bb)
            .unwrap();

        // 无效句柄:结果 false。
        self.builder.position_at_end(invalid_bb);
        self.builder
            .build_store(result_alloca, self.context.bool_type().const_int(0, false))
            .unwrap();
        self.builder.build_unconditional_branch(after_bb).unwrap();

        // 主循环:加锁检查容量。
        self.builder.position_at_end(loop_bb);
        self.lock_chan(chan_ptr)?;
        let count = self.load_chan_i32(chan_ptr, CHAN_COUNT)?;
        let cap = self.load_chan_i32(chan_ptr, CHAN_CAP)?;
        let has_room = self
            .builder
            .build_int_compare(inkwell::IntPredicate::SLT, count, cap, "chan_room")
            .unwrap();
        self.builder
            .build_conditional_branch(has_room, write_bb, full_bb)
            .unwrap();

        // 有空间:消息堆拷贝写入尾槽,count++,结果 true。
        self.builder.position_at_end(write_bb);
        let head = self.load_chan_i32(chan_ptr, CHAN_HEAD)?;
        let idx = self
            .builder
            .build_int_add(head, count, "chan_write_idx")
            .unwrap();
        let idx = self.ring_mod(idx, cap)?;
        let slot = self.slot_ptr(chan_ptr, idx)?;
        let copy = self.copy_str(msg)?;
        self.builder
            .build_call(strcpy_fn, &[copy.into(), msg.into()], "chan_msg_fill")
            .unwrap();
        self.store_slot(slot, copy)?;
        let count2 = self.load_chan_i32(chan_ptr, CHAN_COUNT)?;
        let count3 = self
            .builder
            .build_int_add(count2, i32_ty.const_int(1, false), "chan_count_inc")
            .unwrap();
        self.store_chan_i32(chan_ptr, CHAN_COUNT, count3)?;
        self.unlock_chan(chan_ptr)?;
        self.builder
            .build_store(result_alloca, self.context.bool_type().const_int(1, false))
            .unwrap();
        self.builder.build_unconditional_branch(after_bb).unwrap();

        // 满:解锁睡 1ms 重试。
        self.builder.position_at_end(full_bb);
        self.unlock_chan(chan_ptr)?;
        self.emit_chan_sleep()?;
        self.builder.build_unconditional_branch(loop_bb).unwrap();

        self.builder.position_at_end(after_bb);
        let result = self
            .builder
            .build_load(self.context.bool_type(), result_alloca, "chan_send_val")
            .unwrap();
        Ok(result)
    }

    /// `chan_recv(ch: i64) -> str`:缓冲空时阻塞直至有消息;句柄为 null
    /// 返回空串。消息所有权移交接收方。
    pub(super) fn compile_chan_recv(&mut self, arguments: &[Expr]) -> Result<BasicValueEnum<'ctx>> {
        if arguments.len() != 1 {
            return Err(HuziError::new_global(
                "chan_recv() requires 1 argument: (ch: i64)",
            ));
        }
        let chan_ptr = self.chan_handle_ptr(&arguments[0])?;

        let function = self.current_function()?;
        let ptr_ty = self.context.ptr_type(inkwell::AddressSpace::default());
        let result_alloca = self.build_alloca(ptr_ty.into(), "chan_recv_res")?;

        let invalid_bb = self.context.append_basic_block(function, "chan_recv_invalid");
        let loop_bb = self.context.append_basic_block(function, "chan_recv_loop");
        let read_bb = self.context.append_basic_block(function, "chan_recv_read");
        let empty_bb = self.context.append_basic_block(function, "chan_recv_empty");
        let after_bb = self.context.append_basic_block(function, "chan_recv_done");

        let is_null = self.builder.build_is_null(chan_ptr, "ch_null").unwrap();
        self.builder
            .build_conditional_branch(is_null, invalid_bb, loop_bb)
            .unwrap();

        // 无效句柄:结果空串。
        self.builder.position_at_end(invalid_bb);
        self.builder.build_store(result_alloca, ptr_ty.const_null()).unwrap();
        self.builder.build_unconditional_branch(after_bb).unwrap();

        // 主循环:加锁检查是否有消息。
        self.builder.position_at_end(loop_bb);
        self.lock_chan(chan_ptr)?;
        let count = self.load_chan_i32(chan_ptr, CHAN_COUNT)?;
        let has_msg = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::SGT,
                count,
                self.context.i32_type().const_int(0, false),
                "chan_has_msg",
            )
            .unwrap();
        self.builder
            .build_conditional_branch(has_msg, read_bb, empty_bb)
            .unwrap();

        // 有消息:取头槽,head 前移,count--,结果 = 消息。
        self.builder.position_at_end(read_bb);
        let head = self.load_chan_i32(chan_ptr, CHAN_HEAD)?;
        let cap = self.load_chan_i32(chan_ptr, CHAN_CAP)?;
        let idx = self.ring_mod(head, cap)?;
        let slot = self.slot_ptr(chan_ptr, idx)?;
        let msg = self.load_slot(slot)?;
        let head2 = self
            .builder
            .build_int_add(head, self.context.i32_type().const_int(1, false), "chan_head_inc")
            .unwrap();
        self.store_chan_i32(chan_ptr, CHAN_HEAD, head2)?;
        let count2 = self.load_chan_i32(chan_ptr, CHAN_COUNT)?;
        let count3 = self
            .builder
            .build_int_sub(count2, self.context.i32_type().const_int(1, false), "chan_count_dec")
            .unwrap();
        self.store_chan_i32(chan_ptr, CHAN_COUNT, count3)?;
        self.unlock_chan(chan_ptr)?;
        self.builder.build_store(result_alloca, msg).unwrap();
        self.builder.build_unconditional_branch(after_bb).unwrap();

        // 空:解锁睡 1ms 重试。
        self.builder.position_at_end(empty_bb);
        self.unlock_chan(chan_ptr)?;
        self.emit_chan_sleep()?;
        self.builder.build_unconditional_branch(loop_bb).unwrap();

        self.builder.position_at_end(after_bb);
        let result = self.builder.build_load(ptr_ty, result_alloca, "chan_recv_val").unwrap();
        Ok(result)
    }

    // ---- 内部助手 ----

    /// 句柄实参 → i64 零扩展 → 结构体指针(可能是 null,由调用方分支)。
    fn chan_handle_ptr(&mut self, expr: &Expr) -> Result<PointerValue<'ctx>> {
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

    fn chan_field_ptr(
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

    fn load_chan_i32(&self, chan: PointerValue<'ctx>, offset: u64) -> Result<IntValue<'ctx>> {
        let p = self.chan_field_ptr(chan, offset, "chan_field")?;
        let v = self
            .builder
            .build_load(self.context.i32_type(), p, "chan_i32")
            .unwrap();
        Ok(v.into_int_value())
    }

    fn store_chan_i32(
        &self,
        chan: PointerValue<'ctx>,
        offset: u64,
        val: IntValue<'ctx>,
    ) -> Result<()> {
        let p = self.chan_field_ptr(chan, offset, "chan_field")?;
        self.builder.build_store(p, val).unwrap();
        Ok(())
    }

    fn load_chan_ptr(&self, chan: PointerValue<'ctx>, offset: u64) -> Result<PointerValue<'ctx>> {
        let p = self.chan_field_ptr(chan, offset, "chan_field")?;
        let v = self
            .builder
            .build_load(self.context.ptr_type(inkwell::AddressSpace::default()), p, "chan_ptr")
            .unwrap();
        Ok(v.into_pointer_value())
    }

    fn store_chan_ptr(
        &self,
        chan: PointerValue<'ctx>,
        offset: u64,
        val: PointerValue<'ctx>,
    ) -> Result<()> {
        let p = self.chan_field_ptr(chan, offset, "chan_field")?;
        self.builder.build_store(p, val).unwrap();
        Ok(())
    }

    /// 自旋锁获取(模内私有函数,只生成一次):
    /// `atomicrmw xchg` 0→1(acquire),失败睡 1ms 重试。
    fn lock_chan(&mut self, chan: PointerValue<'ctx>) -> Result<()> {
        let p = self.chan_field_ptr(chan, CHAN_LOCK, "chan_lock_field")?;
        let (lock_fn, _) = self.ensure_chan_lock_fns()?;
        self.builder
            .build_call(lock_fn, &[p.into()], "chan_lock_call")
            .unwrap();
        Ok(())
    }

    /// 释放锁:`atomicrmw xchg` → 0(release)。
    fn unlock_chan(&mut self, chan: PointerValue<'ctx>) -> Result<()> {
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

        if let Some(b) = saved {
            self.builder.position_at_end(b);
        }
        Ok((lock_fn, unlock_fn))
    }

    /// 睡 1ms(Windows `Sleep`,POSIX `usleep`;均已在 prelude 声明)。
    fn emit_chan_sleep(&mut self) -> Result<()> {
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

    /// 容量收敛到 1..=CHAN_MAX_CAP。
    fn clamp_chan_cap(&mut self, cap: IntValue<'ctx>) -> Result<IntValue<'ctx>> {
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
    fn ring_mod(&self, idx: IntValue<'ctx>, cap: IntValue<'ctx>) -> Result<IntValue<'ctx>> {
        Ok(self
            .builder
            .build_int_signed_rem(idx, cap, "chan_ring")
            .unwrap())
    }

    /// 槽位地址:slots + idx(指针槽宽 8 字节,用 i64 GEP 缩放)。
    fn slot_ptr(
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

    fn store_slot(&self, slot: PointerValue<'ctx>, msg: PointerValue<'ctx>) -> Result<()> {
        self.builder.build_store(slot, msg).unwrap();
        Ok(())
    }

    fn load_slot(&self, slot: PointerValue<'ctx>) -> Result<PointerValue<'ctx>> {
        let v = self
            .builder
            .build_load(self.context.ptr_type(inkwell::AddressSpace::default()), slot, "chan_msg")
            .unwrap();
        Ok(v.into_pointer_value())
    }

    /// 消息堆拷贝:malloc(len+1),由调用方 strcpy 填充。
    fn copy_str(&mut self, msg: PointerValue<'ctx>) -> Result<PointerValue<'ctx>> {
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
