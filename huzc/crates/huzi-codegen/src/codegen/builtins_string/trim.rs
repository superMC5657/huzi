//! `trim`/`contains`:空白切除边界与子串匹配(自 `builtins_string/mod.rs` 纯搬移,零逻辑变化)。

use super::super::CodeGen;
use huzi_ast::*;
use huzi_error::{HuziError, Result};
use inkwell::values::PointerValue;

impl<'ctx> CodeGen<'ctx> {
    /// `trim(s)` — 去除两端 ASCII 空白(空格/`\t`/`\n`/`\r`),返回新堆字符串。
    pub(in crate::codegen) fn compile_trim(
        &mut self,
        arguments: &[Expr],
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        if arguments.len() != 1 {
            return Err(HuziError::new_global("trim() requires exactly 1 argument"));
        }
        let s = self.str_ptr_arg(&arguments[0], "trim()")?;
        let s_len = self.str_len_of(s, "trim_len")?;
        let function = self.current_function()?;
        let lo = self.trim_left_bound(s, s_len, function)?;
        let hi = self.trim_right_bound(s, lo, s_len, function)?;
        Ok(self.str_copy_range(s, lo, hi)?.into())
    }

    /// 左边界:跳过前导空白后的首个下标。
    fn trim_left_bound(
        &mut self,
        s: PointerValue<'ctx>,
        s_len: inkwell::values::IntValue<'ctx>,
        function: inkwell::values::FunctionValue<'ctx>,
    ) -> Result<inkwell::values::IntValue<'ctx>> {
        let i32_type = self.context.i32_type();
        let i8_type = self.context.i8_type();
        let loop_bb = self.context.append_basic_block(function, "trim_lo_loop");
        let check_bb = self.context.append_basic_block(function, "trim_lo_check");
        let next_bb = self.context.append_basic_block(function, "trim_lo_next");
        let done_bb = self.context.append_basic_block(function, "trim_lo_done");
        let lo = self.build_alloca(i32_type.into(), "trim_lo")?;
        self.builder.build_store(lo, i32_type.const_int(0, false)).unwrap();
        self.builder.build_unconditional_branch(loop_bb).unwrap();
        self.builder.position_at_end(loop_bb);
        let lv = self.builder.build_load(i32_type, lo, "trim_lo").unwrap().into_int_value();
        let cond = self.builder.build_int_compare(inkwell::IntPredicate::ULT, lv, s_len, "trim_lo_c").unwrap();
        self.builder.build_conditional_branch(cond, check_bb, done_bb).unwrap();
        self.builder.position_at_end(check_bb);
        let lv2 = self.builder.build_load(i32_type, lo, "trim_lo").unwrap().into_int_value();
        let ptr = unsafe { self.builder.build_gep(i8_type, s, &[lv2], "trim_lo_p").unwrap() };
        let byte = self.builder.build_load(i8_type, ptr, "trim_lo_b").unwrap().into_int_value();
        let ws = self.is_space_byte(byte);
        self.builder.build_conditional_branch(ws, next_bb, done_bb).unwrap();
        self.builder.position_at_end(next_bb);
        let lv3 = self.builder.build_load(i32_type, lo, "trim_lo").unwrap().into_int_value();
        let inc = self.builder.build_int_add(lv3, i32_type.const_int(1, false), "trim_lo_inc").unwrap();
        self.builder.build_store(lo, inc).unwrap();
        self.builder.build_unconditional_branch(loop_bb).unwrap();
        self.builder.position_at_end(done_bb);
        Ok(self.builder.build_load(i32_type, lo, "trim_lo").unwrap().into_int_value())
    }

    /// 右边界:跳过尾部空白,不小于左边界(全空白时为空串)。
    fn trim_right_bound(
        &mut self,
        s: PointerValue<'ctx>,
        lo: inkwell::values::IntValue<'ctx>,
        s_len: inkwell::values::IntValue<'ctx>,
        function: inkwell::values::FunctionValue<'ctx>,
    ) -> Result<inkwell::values::IntValue<'ctx>> {
        let i32_type = self.context.i32_type();
        let i8_type = self.context.i8_type();
        let loop_bb = self.context.append_basic_block(function, "trim_hi_loop");
        let check_bb = self.context.append_basic_block(function, "trim_hi_check");
        let next_bb = self.context.append_basic_block(function, "trim_hi_next");
        let done_bb = self.context.append_basic_block(function, "trim_hi_done");
        let hi = self.build_alloca(i32_type.into(), "trim_hi")?;
        self.builder.build_store(hi, s_len).unwrap();
        self.builder.build_unconditional_branch(loop_bb).unwrap();
        self.builder.position_at_end(loop_bb);
        let hv = self.builder.build_load(i32_type, hi, "trim_hi").unwrap().into_int_value();
        let cond = self.builder.build_int_compare(inkwell::IntPredicate::UGT, hv, lo, "trim_hi_c").unwrap();
        self.builder.build_conditional_branch(cond, check_bb, done_bb).unwrap();
        self.builder.position_at_end(check_bb);
        let hv2 = self.builder.build_load(i32_type, hi, "trim_hi").unwrap().into_int_value();
        let prev = self.builder.build_int_sub(hv2, i32_type.const_int(1, false), "trim_hi_prev").unwrap();
        let ptr = unsafe { self.builder.build_gep(i8_type, s, &[prev], "trim_hi_p").unwrap() };
        let byte = self.builder.build_load(i8_type, ptr, "trim_hi_b").unwrap().into_int_value();
        let ws = self.is_space_byte(byte);
        self.builder.build_conditional_branch(ws, next_bb, done_bb).unwrap();
        self.builder.position_at_end(next_bb);
        let hv3 = self.builder.build_load(i32_type, hi, "trim_hi").unwrap().into_int_value();
        let dec = self.builder.build_int_sub(hv3, i32_type.const_int(1, false), "trim_hi_dec").unwrap();
        self.builder.build_store(hi, dec).unwrap();
        self.builder.build_unconditional_branch(loop_bb).unwrap();
        self.builder.position_at_end(done_bb);
        Ok(self.builder.build_load(i32_type, hi, "trim_hi").unwrap().into_int_value())
    }

    /// `contains(s, sub)` — `sub` 为空或在 `s` 中出现时返回 true(字节语义)。
    pub(in crate::codegen) fn compile_contains(
        &mut self,
        arguments: &[Expr],
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        if arguments.len() != 2 {
            return Err(HuziError::new_global(
                "contains() requires exactly 2 arguments (string, sub)",
            ));
        }
        let s = self.str_ptr_arg(&arguments[0], "contains()")?;
        let sub = self.str_ptr_arg(&arguments[1], "contains()")?;
        let s_len = self.str_len_of(s, "contains_slen")?;
        let sub_len = self.str_len_of(sub, "contains_sublen")?;
        self.emit_contains_loop(s, s_len, sub, sub_len)
    }
}
