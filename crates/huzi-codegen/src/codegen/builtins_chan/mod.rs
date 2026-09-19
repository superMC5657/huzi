//! 通道(chan)内置函数:跨线程字符串消息队列。
//!
//! `chan_new(cap) -> i64` / `chan_send(ch, msg) -> bool` / `chan_recv(ch) -> str`。
//! 实现为固定容量环形缓冲:自旋锁(LLVM `atomicrmw xchg`)保护索引,满发/空收
//! 时解锁并睡 1ms 轮询等待。句柄是堆结构体指针的 i64 整数形式,可直接作为
//! `spawn` 实参传给线程;句柄为 null 时 send 返回 false、recv 返回空串。
//! 不提供 chan_free——通道随进程退出回收(与 str/vec 的内存约定一致)。
//!
//! 子模块:`lock`(自旋锁与轮询睡眠)、`mem`(结构体布局访问与消息拷贝)。

mod lock;
mod mem;

use huzi_ast::Expr;
use huzi_error::{HuziError, Result};
use inkwell::values::BasicValueEnum;

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

        // 无效句柄:结果为真实空串(复用全局 NUL 常量,可安全
        // strlen/print),不是 NULL 指针。
        self.builder.position_at_end(invalid_bb);
        let empty = self.empty_str_ptr();
        self.builder.build_store(result_alloca, empty).unwrap();
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
        // head 回绕到 [0, cap):否则 i32 持续递增,约 2^31 条消息后
        // 溢出为负,ring_mod 产生负下标越界读写。
        let head2 = self.ring_mod(head2, cap)?;
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
}
