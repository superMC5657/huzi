//! 基础字符串解析内置函数: `parse_int(s)` 与 `parse_float(s)`。
//!
//! - 解析整数：`parse_int(s: str) -> (bool, i32)`
//! - 解析浮点数：`parse_float(s: str) -> (bool, f64)`
//!
//! 非法输入返回 `(false, 0)` 或 `(false, 0.0)`，沿用哨兵语义，不 abort。

use super::CodeGen;
use huzi_ast::Expr;
use huzi_error::Result;
use inkwell::AddressSpace;
use inkwell::values::{BasicValueEnum, IntValue, PointerValue};

impl<'ctx> CodeGen<'ctx> {
    /// 检查解析末尾指针: 略过尾部空白字符后必须到达 '\0'。
    fn check_tail_whitespace_nul(
        &mut self,
        end_val: PointerValue<'ctx>,
    ) -> Result<IntValue<'ctx>> {
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let i32_t = self.context.i32_type();
        let i8_t = self.context.i8_type();
        let function = self.current_function()?;

        let check_loop_bb = self.context.append_basic_block(function, "tail_loop");
        let check_body_bb = self.context.append_basic_block(function, "tail_body");
        let check_done_bb = self.context.append_basic_block(function, "tail_done");

        let cur_ptr_alloca = self.build_alloca(ptr_t.into(), "tail_cur_p")?;
        self.builder.build_store(cur_ptr_alloca, end_val).unwrap();

        self.builder.build_unconditional_branch(check_loop_bb).unwrap();
        self.builder.position_at_end(check_loop_bb);

        let cur_p = self.builder.build_load(ptr_t, cur_ptr_alloca, "cur_p").unwrap().into_pointer_value();
        let c = self.builder.build_load(i8_t, cur_p, "char").unwrap().into_int_value();

        // 尾部空白判定复用 `is_space_byte`(空格/`\t`/`\n`/`\r`),与 trim 等一致。
        let is_ws = self.is_space_byte(c);

        self.builder.build_conditional_branch(is_ws, check_body_bb, check_done_bb).unwrap();

        self.builder.position_at_end(check_body_bb);
        let next_p = unsafe { self.builder.build_gep(i8_t, cur_p, &[i32_t.const_int(1, false)], "next_p").unwrap() };
        self.builder.build_store(cur_ptr_alloca, next_p).unwrap();
        self.builder.build_unconditional_branch(check_loop_bb).unwrap();

        self.builder.position_at_end(check_done_bb);
        let final_p = self.builder.build_load(ptr_t, cur_ptr_alloca, "final_p").unwrap().into_pointer_value();
        let final_c = self.builder.build_load(i8_t, final_p, "final_c").unwrap().into_int_value();
        let is_nul = self.builder.build_int_compare(inkwell::IntPredicate::EQ, final_c, i8_t.const_int(0, false), "is_nul").unwrap();
        Ok(is_nul)
    }

    /// `parse_int(s)`: 将字符串解析为 i32，返回 `(bool, i32)`。
    pub(super) fn compile_parse_int(
        &mut self,
        arguments: &[Expr],
    ) -> Result<BasicValueEnum<'ctx>> {
        self.expect_arg_count("parse_int", arguments, 1)?;
        let str_val = self.compile_str_arg(&arguments[0], "parse_int")?;

        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let i32_t = self.context.i32_type();
        let i64_t = self.context.i64_type();

        let endptr_alloca = self.build_alloca(ptr_t.into(), "parse_int_endptr")?;
        let strtoll_fn = self.module.get_function("strtoll").unwrap();
        let base = i32_t.const_int(10, false);

        let parsed_i64 = self
            .builder
            .build_call(strtoll_fn, &[str_val.into(), endptr_alloca.into(), base.into()], "strtoll_call")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_int_value();

        let end_val = self
            .builder
            .build_load(ptr_t, endptr_alloca, "endptr_val")
            .unwrap()
            .into_pointer_value();

        let advanced = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::NE,
                self.builder.build_ptr_to_int(end_val, i64_t, "end_int").unwrap(),
                self.builder.build_ptr_to_int(str_val, i64_t, "str_int").unwrap(),
                "parse_advanced",
            )
            .unwrap();

        let is_nul = self.check_tail_whitespace_nul(end_val)?;
        let valid = self.builder.build_and(advanced, is_nul, "parse_ok").unwrap();

        let val_i32 = self.builder.build_int_cast(parsed_i64, i32_t, "val_i32").unwrap();
        let selected_val = self.builder.build_select(valid, val_i32, i32_t.const_int(0, false), "ret_val").unwrap().into_int_value();

        self.emit_bool_tuple(valid, selected_val.into(), "parse_int_res")
    }

    /// `parse_float(s)`: 将字符串解析为 f64，返回 `(bool, f64)`。
    pub(super) fn compile_parse_float(
        &mut self,
        arguments: &[Expr],
    ) -> Result<BasicValueEnum<'ctx>> {
        self.expect_arg_count("parse_float", arguments, 1)?;
        let str_val = self.compile_str_arg(&arguments[0], "parse_float")?;

        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let i64_t = self.context.i64_type();
        let f64_t = self.context.f64_type();

        let endptr_alloca = self.build_alloca(ptr_t.into(), "parse_float_endptr")?;
        let strtod_fn = self.module.get_function("strtod").unwrap();

        let parsed_f64 = self
            .builder
            .build_call(strtod_fn, &[str_val.into(), endptr_alloca.into()], "strtod_call")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_float_value();

        let end_val = self
            .builder
            .build_load(ptr_t, endptr_alloca, "endptr_val")
            .unwrap()
            .into_pointer_value();

        let advanced = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::NE,
                self.builder.build_ptr_to_int(end_val, i64_t, "end_int").unwrap(),
                self.builder.build_ptr_to_int(str_val, i64_t, "str_int").unwrap(),
                "parse_advanced",
            )
            .unwrap();

        let is_nul = self.check_tail_whitespace_nul(end_val)?;
        let valid = self.builder.build_and(advanced, is_nul, "parse_ok").unwrap();

        let zero_f64 = f64_t.const_float(0.0);
        let selected_val = self.builder.build_select(valid, parsed_f64, zero_f64, "ret_val").unwrap().into_float_value();

        self.emit_bool_tuple(valid, selected_val.into(), "parse_float_res")
    }
}
