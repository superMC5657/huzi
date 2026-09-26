//! split 计数与填充:段数统计、逐段拷贝落盘、空/正常分支发射。(自 `builtins_string_util.rs` 纯搬移,零逻辑变化)。

use super::super::CodeGen;
use huzi_error::Result;
use inkwell::AddressSpace;
use inkwell::values::{IntValue, PointerValue};

impl<'ctx> CodeGen<'ctx> {
    pub(super) fn split_count_segments(
        &mut self,
        s: PointerValue<'ctx>,
        d: PointerValue<'ctx>,
        s_len: IntValue<'ctx>,
        d_len: IntValue<'ctx>,
    ) -> Result<IntValue<'ctx>> {
        let i32_type = self.context.i32_type();
        let end = self.builder.build_int_sub(s_len, d_len, "cnt_end").unwrap();
        let function = self.current_function()?;
        let loop_bb = self.context.append_basic_block(function, "cnt_loop");
        let body_bb = self.context.append_basic_block(function, "cnt_body");
        let hit_bb = self.context.append_basic_block(function, "cnt_hit");
        let miss_bb = self.context.append_basic_block(function, "cnt_miss");
        let done_bb = self.context.append_basic_block(function, "cnt_done");
        let count = self.build_alloca(i32_type.into(), "cnt_n")?;
        let i = self.build_alloca(i32_type.into(), "cnt_i")?;
        self.builder.build_store(count, i32_type.const_int(1, false)).unwrap();
        self.builder.build_store(i, i32_type.const_int(0, false)).unwrap();
        self.builder.build_unconditional_branch(loop_bb).unwrap();
        self.builder.position_at_end(loop_bb);
        let iv = self.builder.build_load(i32_type, i, "cnt_i").unwrap().into_int_value();
        let cond = self.builder.build_int_compare(inkwell::IntPredicate::SLE, iv, end, "cnt_cond").unwrap();
        self.builder.build_conditional_branch(cond, body_bb, done_bb).unwrap();
        self.builder.position_at_end(body_bb);
        let iv2 = self.builder.build_load(i32_type, i, "cnt_i").unwrap().into_int_value();
        let m = self.match_at_str(s, iv2, d, d_len)?;
        self.builder.build_conditional_branch(m, hit_bb, miss_bb).unwrap();
        self.builder.position_at_end(hit_bb);
        self.split_count_hit(count, i, d_len, loop_bb)?;
        self.builder.position_at_end(miss_bb);
        let iv3 = self.builder.build_load(i32_type, i, "cnt_i").unwrap().into_int_value();
        let inc = self.builder.build_int_add(iv3, i32_type.const_int(1, false), "cnt_inc").unwrap();
        self.builder.build_store(i, inc).unwrap();
        self.builder.build_unconditional_branch(loop_bb).unwrap();
        self.builder.position_at_end(done_bb);
        Ok(self.builder.build_load(i32_type, count, "cnt_n").unwrap().into_int_value())
    }

    /// 命中分隔符:段数加一,下标跳过整个分隔符。
    fn split_count_hit(
        &mut self,
        count: PointerValue<'ctx>,
        i: PointerValue<'ctx>,
        d_len: IntValue<'ctx>,
        loop_bb: inkwell::basic_block::BasicBlock<'ctx>,
    ) -> Result<()> {
        let i32_type = self.context.i32_type();
        let n = self.builder.build_load(i32_type, count, "cnt_n").unwrap().into_int_value();
        let inc_n = self.builder.build_int_add(n, i32_type.const_int(1, false), "cnt_add").unwrap();
        self.builder.build_store(count, inc_n).unwrap();
        let iv = self.builder.build_load(i32_type, i, "cnt_i").unwrap().into_int_value();
        let skip = self.builder.build_int_add(iv, d_len, "cnt_skip").unwrap();
        self.builder.build_store(i, skip).unwrap();
        self.builder.build_unconditional_branch(loop_bb).unwrap();
        Ok(())
    }

    /// 第二遍:逐段拷贝并填入 `data[0..count)`。
    pub(super) fn split_fill_segments(
        &mut self,
        data: PointerValue<'ctx>,
        s: PointerValue<'ctx>,
        d: PointerValue<'ctx>,
        s_len: IntValue<'ctx>,
        d_len: IntValue<'ctx>,
    ) -> Result<()> {
        let i32_type = self.context.i32_type();
        let str_ty: inkwell::types::BasicTypeEnum<'ctx> =
            self.context.ptr_type(AddressSpace::default()).into();
        let function = self.current_function()?;
        let loop_bb = self.context.append_basic_block(function, "fill_loop");
        let body_bb = self.context.append_basic_block(function, "fill_body");
        let chk_bb = self.context.append_basic_block(function, "fill_chk");
        let dom_bb = self.context.append_basic_block(function, "fill_dom");
        let hit_bb = self.context.append_basic_block(function, "fill_hit");
        let miss_bb = self.context.append_basic_block(function, "fill_miss");
        let end_bb = self.context.append_basic_block(function, "fill_end");
        let done_bb = self.context.append_basic_block(function, "fill_done");
        let seg_start = self.build_alloca(i32_type.into(), "fill_ss")?;
        let seg_idx = self.build_alloca(i32_type.into(), "fill_si")?;
        let i = self.build_alloca(i32_type.into(), "fill_i")?;
        for slot in [seg_start, seg_idx, i] {
            self.builder.build_store(slot, i32_type.const_int(0, false)).unwrap();
        }
        self.builder.build_unconditional_branch(loop_bb).unwrap();
        self.builder.position_at_end(loop_bb);
        let iv = self.builder.build_load(i32_type, i, "fill_i").unwrap().into_int_value();
        let cond = self.builder.build_int_compare(inkwell::IntPredicate::ULE, iv, s_len, "fill_cond").unwrap();
        self.builder.build_conditional_branch(cond, body_bb, done_bb).unwrap();
        self.builder.position_at_end(body_bb);
        let iv2 = self.builder.build_load(i32_type, i, "fill_i").unwrap().into_int_value();
        let at_end = self.builder.build_int_compare(inkwell::IntPredicate::EQ, iv2, s_len, "fill_eos").unwrap();
        self.builder.build_conditional_branch(at_end, end_bb, chk_bb).unwrap();
        self.builder.position_at_end(chk_bb);
        let iv3 = self.builder.build_load(i32_type, i, "fill_i").unwrap().into_int_value();
        let sum = self.builder.build_int_add(iv3, d_len, "fill_sum").unwrap();
        let in_range = self.builder.build_int_compare(inkwell::IntPredicate::ULE, sum, s_len, "fill_rng").unwrap();
        self.builder.build_conditional_branch(in_range, dom_bb, miss_bb).unwrap();
        self.builder.position_at_end(dom_bb);
        let iv4 = self.builder.build_load(i32_type, i, "fill_i").unwrap().into_int_value();
        let m = self.match_at_str(s, iv4, d, d_len)?;
        self.builder.build_conditional_branch(m, hit_bb, miss_bb).unwrap();
        self.builder.position_at_end(miss_bb);
        let iv5 = self.builder.build_load(i32_type, i, "fill_i").unwrap().into_int_value();
        let inc = self.builder.build_int_add(iv5, i32_type.const_int(1, false), "fill_inc").unwrap();
        self.builder.build_store(i, inc).unwrap();
        self.builder.build_unconditional_branch(loop_bb).unwrap();
        self.builder.position_at_end(hit_bb);
        self.split_emit_segment(data, str_ty, s, seg_start, seg_idx, i, d_len, true, loop_bb)?;
        self.builder.position_at_end(end_bb);
        self.split_emit_segment(data, str_ty, s, seg_start, seg_idx, i, d_len, false, done_bb)?;
        self.builder.position_at_end(done_bb);
        Ok(())
    }

    /// 落盘一段并推进下标:命中时跳过分隔符,收尾时直接结束。
    #[allow(clippy::too_many_arguments)]
    fn split_emit_segment(
        &mut self,
        data: PointerValue<'ctx>,
        str_ty: inkwell::types::BasicTypeEnum<'ctx>,
        s: PointerValue<'ctx>,
        seg_start: PointerValue<'ctx>,
        seg_idx: PointerValue<'ctx>,
        i: PointerValue<'ctx>,
        d_len: IntValue<'ctx>,
        is_hit: bool,
        next_bb: inkwell::basic_block::BasicBlock<'ctx>,
    ) -> Result<()> {
        let i32_type = self.context.i32_type();
        let ss = self.builder.build_load(i32_type, seg_start, "fill_ss").unwrap().into_int_value();
        let iv = self.builder.build_load(i32_type, i, "fill_i").unwrap().into_int_value();
        let seg = self.str_copy_range(s, ss, iv)?;
        let si = self.builder.build_load(i32_type, seg_idx, "fill_si").unwrap().into_int_value();
        let slot = unsafe { self.builder.build_gep(str_ty, data, &[si], "fill_slot").unwrap() };
        self.builder.build_store(slot, seg).unwrap();
        let inc_si = self.builder.build_int_add(si, i32_type.const_int(1, false), "fill_sinc").unwrap();
        self.builder.build_store(seg_idx, inc_si).unwrap();
        if is_hit {
            let skip = self.builder.build_int_add(iv, d_len, "fill_skip").unwrap();
            self.builder.build_store(seg_start, skip).unwrap();
            self.builder.build_store(i, skip).unwrap();
        }
        self.builder.build_unconditional_branch(next_bb).unwrap();
        Ok(())
    }

    /// 空分隔符分支:整体拷贝为唯一一段。
    pub(in crate::codegen) fn emit_split_single(
        &mut self,
        s: PointerValue<'ctx>,
        s_len: IntValue<'ctx>,
        str_ty: inkwell::types::BasicTypeEnum<'ctx>,
        result: PointerValue<'ctx>,
        done_bb: inkwell::basic_block::BasicBlock<'ctx>,
    ) -> Result<()> {
        let i32_type = self.context.i32_type();
        let one = i32_type.const_int(1, false);
        let whole = self.str_copy_range(s, i32_type.const_int(0, false), s_len)?;
        let data = self.alloc_vec_buffer(str_ty, one, "split_one_data")?;
        let slot = unsafe {
            self.builder
                .build_gep(str_ty, data, &[i32_type.const_int(0, false)], "split_one_slot")
                .unwrap()
        };
        self.builder.build_store(slot, whole).unwrap();
        let vec_val = self.assemble_str_vec(data, one)?;
        self.builder.build_store(result, vec_val).unwrap();
        self.builder.build_unconditional_branch(done_bb).unwrap();
        Ok(())
    }

    /// 正常分支:先计数分配指针数组,再逐段拷贝填充。
    pub(in crate::codegen) fn emit_split_many(
        &mut self,
        s: PointerValue<'ctx>,
        d: PointerValue<'ctx>,
        s_len: IntValue<'ctx>,
        d_len: IntValue<'ctx>,
        str_ty: inkwell::types::BasicTypeEnum<'ctx>,
        result: PointerValue<'ctx>,
        done_bb: inkwell::basic_block::BasicBlock<'ctx>,
    ) -> Result<()> {
        let count = self.split_count_segments(s, d, s_len, d_len)?;
        let data = self.alloc_vec_buffer(str_ty, count, "split_data")?;
        self.split_fill_segments(data, s, d, s_len, d_len)?;
        let vec_val = self.assemble_str_vec(data, count)?;
        self.builder.build_store(result, vec_val).unwrap();
        self.builder.build_unconditional_branch(done_bb).unwrap();
        Ok(())
    }
}
