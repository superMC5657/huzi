//! 字符串内置函数的循环类 IR 助手:split 的计数/填充、子串匹配、
//! 区间拷贝、vec<str> 组装。入口(`compile_split` 等)保留在
//! `builtins_string.rs`,此处只放超长循环体,保证两文件都不超 500 行。
//!
//! 约定:字符串为 NUL 结尾的字节堆指针,下标与区间一律按字节语义
//! (UTF-8 多字节不做字符语义,与 `s[i]`/`emit_str_bounds_check` 一致)。

use super::CodeGen;
use huzi_ast::*;
use huzi_error::{HuziError, Result};
use inkwell::AddressSpace;
use inkwell::values::{BasicValueEnum, IntValue, PointerValue};

impl<'ctx> CodeGen<'ctx> {
    /// 取字符串实参的指针;非指针报类型错误。
    pub(super) fn str_ptr_arg(&mut self, expr: &Expr, what: &str) -> Result<PointerValue<'ctx>> {
        let v = self.compile_expr(expr)?;
        if v.is_pointer_value() {
            Ok(v.into_pointer_value())
        } else {
            Err(HuziError::new_global(format!("{what} requires string arguments")))
        }
    }

    /// strlen 取字节长度(i32)。
    pub(super) fn str_len_of(
        &mut self,
        ptr: PointerValue<'ctx>,
        name: &str,
    ) -> Result<IntValue<'ctx>> {
        let strlen_fn = self.module.get_function("strlen").unwrap();
        Ok(self
            .builder
            .build_call(strlen_fn, &[ptr.into()], name)
            .unwrap()
            .try_as_basic_value()
            .unwrap_left()
            .into_int_value())
    }

    /// ASCII 空白判定:空格/`\t`/`\n`/`\r`(返回 i1)。
    pub(super) fn is_space_byte(&self, b: IntValue<'ctx>) -> IntValue<'ctx> {
        let i8_type = self.context.i8_type();
        let mut cond = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                b,
                i8_type.const_int(32, false),
                "ws_sp",
            )
            .unwrap();
        for code in [9u64, 10, 13] {
            let eq = self
                .builder
                .build_int_compare(
                    inkwell::IntPredicate::EQ,
                    b,
                    i8_type.const_int(code, false),
                    "ws_eq",
                )
                .unwrap();
            cond = self.builder.build_or(cond, eq, "ws_or").unwrap();
        }
        cond
    }

    /// 组装 vec<str> 值 `{ data, len, cap = len }`(布局与 vec.rs 一致)。
    pub(super) fn assemble_str_vec(
        &mut self,
        data: PointerValue<'ctx>,
        len: IntValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>> {
        let vec_ty = self.vec_struct_type();
        let tmp = self.build_alloca(vec_ty.into(), "sv_tmp")?;
        let data_ptr = self
            .builder
            .build_struct_gep(vec_ty, tmp, 0, "sv_data")
            .unwrap();
        self.builder.build_store(data_ptr, data).unwrap();
        let len_ptr = self
            .builder
            .build_struct_gep(vec_ty, tmp, 1, "sv_len")
            .unwrap();
        self.builder.build_store(len_ptr, len).unwrap();
        let cap_ptr = self
            .builder
            .build_struct_gep(vec_ty, tmp, 2, "sv_cap")
            .unwrap();
        self.builder.build_store(cap_ptr, len).unwrap();
        Ok(self.builder.build_load(vec_ty, tmp, "sv_val").unwrap())
    }

    /// 拷贝 `s[start..end)` 为新的 NUL 结尾堆字符串(区间由调用方保证合法)。
    pub(super) fn str_copy_range(
        &mut self,
        s: PointerValue<'ctx>,
        start: IntValue<'ctx>,
        end: IntValue<'ctx>,
    ) -> Result<PointerValue<'ctx>> {
        let i32_type = self.context.i32_type();
        let i8_type = self.context.i8_type();
        let len = self.builder.build_int_sub(end, start, "range_len").unwrap();
        let size = self
            .builder
            .build_int_add(len, i32_type.const_int(1, false), "range_size")
            .unwrap();
        let malloc_fn = self.module.get_function("malloc").unwrap();
        let buf = self
            .builder
            .build_call(malloc_fn, &[size.into()], "range_buf")
            .unwrap()
            .try_as_basic_value()
            .unwrap_left()
            .into_pointer_value();
        let function = self.current_function()?;
        let loop_bb = self.context.append_basic_block(function, "range_loop");
        let body_bb = self.context.append_basic_block(function, "range_body");
        let done_bb = self.context.append_basic_block(function, "range_done");
        let k = self.build_alloca(i32_type.into(), "range_k")?;
        self.builder.build_store(k, i32_type.const_int(0, false)).unwrap();
        self.builder.build_unconditional_branch(loop_bb).unwrap();
        self.builder.position_at_end(loop_bb);
        let kv = self.builder.build_load(i32_type, k, "range_k").unwrap().into_int_value();
        let cond = self
            .builder
            .build_int_compare(inkwell::IntPredicate::ULT, kv, len, "range_cond")
            .unwrap();
        self.builder.build_conditional_branch(cond, body_bb, done_bb).unwrap();
        self.builder.position_at_end(body_bb);
        let kv2 = self.builder.build_load(i32_type, k, "range_k").unwrap().into_int_value();
        let src_idx = self.builder.build_int_add(start, kv2, "range_src").unwrap();
        let src_ptr = unsafe { self.builder.build_gep(i8_type, s, &[src_idx], "range_sp").unwrap() };
        let byte = self.builder.build_load(i8_type, src_ptr, "range_byte").unwrap();
        let dst_ptr = unsafe { self.builder.build_gep(i8_type, buf, &[kv2], "range_dp").unwrap() };
        self.builder.build_store(dst_ptr, byte).unwrap();
        let next = self
            .builder
            .build_int_add(kv2, i32_type.const_int(1, false), "range_next")
            .unwrap();
        self.builder.build_store(k, next).unwrap();
        self.builder.build_unconditional_branch(loop_bb).unwrap();
        self.builder.position_at_end(done_bb);
        let nul_ptr = unsafe { self.builder.build_gep(i8_type, buf, &[len], "range_nul").unwrap() };
        self.builder.build_store(nul_ptr, i8_type.const_int(0, false)).unwrap();
        Ok(buf)
    }

    /// `s[pos..]` 是否以 `d[0..d_len)` 开头(返回 i1,`d_len >= 1` 由调用方保证)。
    pub(super) fn match_at_str(
        &mut self,
        s: PointerValue<'ctx>,
        pos: IntValue<'ctx>,
        d: PointerValue<'ctx>,
        d_len: IntValue<'ctx>,
    ) -> Result<IntValue<'ctx>> {
        let i32_type = self.context.i32_type();
        let i8_type = self.context.i8_type();
        let function = self.current_function()?;
        let loop_bb = self.context.append_basic_block(function, "match_loop");
        let ok_bb = self.context.append_basic_block(function, "match_ok");
        let check_bb = self.context.append_basic_block(function, "match_check");
        let next_bb = self.context.append_basic_block(function, "match_next");
        let fail_bb = self.context.append_basic_block(function, "match_fail");
        let done_bb = self.context.append_basic_block(function, "match_done");
        let res = self.build_alloca(self.context.bool_type().into(), "match_res")?;
        let j = self.build_alloca(i32_type.into(), "match_j")?;
        self.builder.build_store(j, i32_type.const_int(0, false)).unwrap();
        self.builder.build_unconditional_branch(loop_bb).unwrap();
        self.builder.position_at_end(loop_bb);
        let jv = self.builder.build_load(i32_type, j, "match_j").unwrap().into_int_value();
        let cond = self
            .builder
            .build_int_compare(inkwell::IntPredicate::ULT, jv, d_len, "match_cond")
            .unwrap();
        self.builder.build_conditional_branch(cond, check_bb, ok_bb).unwrap();
        self.builder.position_at_end(ok_bb);
        self.builder.build_store(res, self.context.bool_type().const_int(1, false)).unwrap();
        self.builder.build_unconditional_branch(done_bb).unwrap();
        self.builder.position_at_end(check_bb);
        let jv2 = self.builder.build_load(i32_type, j, "match_j").unwrap().into_int_value();
        let s_idx = self.builder.build_int_add(pos, jv2, "match_si").unwrap();
        let s_ptr = unsafe { self.builder.build_gep(i8_type, s, &[s_idx], "match_sp").unwrap() };
        let a = self.builder.build_load(i8_type, s_ptr, "match_a").unwrap().into_int_value();
        let d_ptr = unsafe { self.builder.build_gep(i8_type, d, &[jv2], "match_dp").unwrap() };
        let b = self.builder.build_load(i8_type, d_ptr, "match_b").unwrap().into_int_value();
        let eq = self.builder.build_int_compare(inkwell::IntPredicate::EQ, a, b, "match_eq").unwrap();
        self.builder.build_conditional_branch(eq, next_bb, fail_bb).unwrap();
        self.builder.position_at_end(next_bb);
        let jv3 = self.builder.build_load(i32_type, j, "match_j").unwrap().into_int_value();
        let inc = self.builder.build_int_add(jv3, i32_type.const_int(1, false), "match_inc").unwrap();
        self.builder.build_store(j, inc).unwrap();
        self.builder.build_unconditional_branch(loop_bb).unwrap();
        self.builder.position_at_end(fail_bb);
        self.builder.build_store(res, self.context.bool_type().const_int(0, false)).unwrap();
        self.builder.build_unconditional_branch(done_bb).unwrap();
        self.builder.position_at_end(done_bb);
        Ok(self.builder.build_load(self.context.bool_type(), res, "match_r").unwrap().into_int_value())
    }

    /// 统计 split 段数(`d_len >= 1`):1 + 分隔符不重叠出现次数。
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
    pub(super) fn emit_split_single(
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
        let data = self
            .builder
            .build_array_malloc(str_ty, one, "split_one_data")
            .map_err(|_| HuziError::new_global("Failed to allocate split storage"))?;
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
    pub(super) fn emit_split_many(
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
        let data = self
            .builder
            .build_array_malloc(str_ty, count, "split_data")
            .map_err(|_| HuziError::new_global("Failed to allocate split storage"))?;
        self.split_fill_segments(data, s, d, s_len, d_len)?;
        let vec_val = self.assemble_str_vec(data, count)?;
        self.builder.build_store(result, vec_val).unwrap();
        self.builder.build_unconditional_branch(done_bb).unwrap();
        Ok(())
    }
}
