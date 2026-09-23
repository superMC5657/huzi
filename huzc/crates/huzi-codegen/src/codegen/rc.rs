//! 引用计数 (RC) 与 Swift 风格自动置零弱引用 (Weak Reference) 运行时。
//!
//! 对象堆块采用 16 字节头部布局：
//! [-16 .. -8] : i64 weak_count   (弱引用计数)
//! [-8  ..  0] : i64 strong_count (强引用计数)
//! [ 0  .. size]: 用户实际数据 (User Payload)

use super::CodeGen;
use huzi_error::{HuziError, Result};
use inkwell::values::{BasicValueEnum, PointerValue};

impl<'ctx> CodeGen<'ctx> {
    /// 对一个 Box 指针增加强引用计数(+1)。若指针为空则为 no-op。
    pub(super) fn emit_retain_box(&mut self, user_ptr: PointerValue<'ctx>) -> Result<()> {
        let function = self.current_function()?;
        let not_null = self.ptr_is_not_null(user_ptr);
        let retain_bb = self.context.append_basic_block(function, "retain_do");
        let cont_bb = self.context.append_basic_block(function, "retain_cont");
        self.builder
            .build_conditional_branch(not_null, retain_bb, cont_bb)
            .unwrap();

        self.builder.position_at_end(retain_bb);
        let minus_eight = self.context.i32_type().const_int((-8i64) as u64, true);
        let strong_ptr = unsafe {
            self.builder
                .build_gep(self.context.i8_type(), user_ptr, &[minus_eight], "rc_strong")
                .unwrap()
        };
        let i64_ty = self.context.i64_type();
        let cur = self
            .builder
            .build_load(i64_ty, strong_ptr, "rc_cur")
            .unwrap()
            .into_int_value();
        let one = i64_ty.const_int(1, false);
        let next = self.builder.build_int_add(cur, one, "rc_inc").unwrap();
        self.builder.build_store(strong_ptr, next).unwrap();
        self.builder.build_unconditional_branch(cont_bb).unwrap();

        self.builder.position_at_end(cont_bb);
        Ok(())
    }

    /// 对一个 Box 指针减少引用计数(-1)。若计数归零(<=0)则释放强引用集合的弱计数。
    /// 若指针为空则为 no-op。
    pub(super) fn emit_release_box(&mut self, user_ptr: PointerValue<'ctx>) -> Result<()> {
        let function = self.current_function()?;
        let not_null = self.ptr_is_not_null(user_ptr);
        let release_bb = self.context.append_basic_block(function, "release_do");
        let cont_bb = self.context.append_basic_block(function, "release_cont");
        self.builder
            .build_conditional_branch(not_null, release_bb, cont_bb)
            .unwrap();

        self.builder.position_at_end(release_bb);
        let minus_eight = self.context.i32_type().const_int((-8i64) as u64, true);
        let strong_ptr = unsafe {
            self.builder
                .build_gep(self.context.i8_type(), user_ptr, &[minus_eight], "rc_strong")
                .unwrap()
        };
        let i64_ty = self.context.i64_type();
        let cur = self
            .builder
            .build_load(i64_ty, strong_ptr, "rc_cur")
            .unwrap()
            .into_int_value();
        let one = i64_ty.const_int(1, false);
        let next = self.builder.build_int_sub(cur, one, "rc_dec").unwrap();
        self.builder.build_store(strong_ptr, next).unwrap();

        let zero = i64_ty.const_int(0, false);
        let should_free_weak = self
            .builder
            .build_int_compare(inkwell::IntPredicate::SLE, next, zero, "rc_zero")
            .unwrap();
        let deinit_bb = self.context.append_basic_block(function, "release_deinit");
        self.builder
            .build_conditional_branch(should_free_weak, deinit_bb, cont_bb)
            .unwrap();

        self.builder.position_at_end(deinit_bb);
        // 强引用清零，释放强引用集合持有的 1 个隐式弱计数
        self.emit_release_weak(user_ptr)?;
        self.builder.build_unconditional_branch(cont_bb).unwrap();

        self.builder.position_at_end(cont_bb);
        Ok(())
    }

    /// 对一个 weak Box 指针增加弱引用计数(+1)。若指针为空则为 no-op。
    pub(super) fn emit_retain_weak(&mut self, user_ptr: PointerValue<'ctx>) -> Result<()> {
        let function = self.current_function()?;
        let not_null = self.ptr_is_not_null(user_ptr);
        let retain_bb = self.context.append_basic_block(function, "retain_weak_do");
        let cont_bb = self.context.append_basic_block(function, "retain_weak_cont");
        self.builder
            .build_conditional_branch(not_null, retain_bb, cont_bb)
            .unwrap();

        self.builder.position_at_end(retain_bb);
        let minus_sixteen = self.context.i32_type().const_int((-16i64) as u64, true);
        let weak_ptr = unsafe {
            self.builder
                .build_gep(self.context.i8_type(), user_ptr, &[minus_sixteen], "weak_raw")
                .unwrap()
        };
        let i64_ty = self.context.i64_type();
        let cur = self
            .builder
            .build_load(i64_ty, weak_ptr, "weak_cur")
            .unwrap()
            .into_int_value();
        let one = i64_ty.const_int(1, false);
        let next = self.builder.build_int_add(cur, one, "weak_inc").unwrap();
        self.builder.build_store(weak_ptr, next).unwrap();
        self.builder.build_unconditional_branch(cont_bb).unwrap();

        self.builder.position_at_end(cont_bb);
        Ok(())
    }

    /// 对一个 weak Box 指针减少弱引用计数(-1)。
    /// 当 weak_count <= 0 且 strong_count <= 0 时，调用 free 彻底释放底层的 16 字节外壳。
    /// 若指针为空则为 no-op。
    pub(super) fn emit_release_weak(&mut self, user_ptr: PointerValue<'ctx>) -> Result<()> {
        let function = self.current_function()?;
        let not_null = self.ptr_is_not_null(user_ptr);
        let release_bb = self.context.append_basic_block(function, "rel_weak_do");
        let cont_bb = self.context.append_basic_block(function, "rel_weak_cont");
        self.builder
            .build_conditional_branch(not_null, release_bb, cont_bb)
            .unwrap();

        self.builder.position_at_end(release_bb);
        let minus_sixteen = self.context.i32_type().const_int((-16i64) as u64, true);
        let raw_ptr = unsafe {
            self.builder
                .build_gep(self.context.i8_type(), user_ptr, &[minus_sixteen], "weak_raw")
                .unwrap()
        };
        let i64_ty = self.context.i64_type();
        let cur = self
            .builder
            .build_load(i64_ty, raw_ptr, "weak_cur")
            .unwrap()
            .into_int_value();
        let one = i64_ty.const_int(1, false);
        let next = self.builder.build_int_sub(cur, one, "weak_dec").unwrap();
        self.builder.build_store(raw_ptr, next).unwrap();

        let zero = i64_ty.const_int(0, false);
        let weak_zero = self
            .builder
            .build_int_compare(inkwell::IntPredicate::SLE, next, zero, "weak_zero")
            .unwrap();

        let minus_eight = self.context.i32_type().const_int((-8i64) as u64, true);
        let strong_ptr = unsafe {
            self.builder
                .build_gep(self.context.i8_type(), user_ptr, &[minus_eight], "rc_strong")
                .unwrap()
        };
        let strong_cur = self
            .builder
            .build_load(i64_ty, strong_ptr, "strong_cur")
            .unwrap()
            .into_int_value();
        let strong_zero = self
            .builder
            .build_int_compare(inkwell::IntPredicate::SLE, strong_cur, zero, "strong_zero")
            .unwrap();

        let can_free = self.builder.build_and(weak_zero, strong_zero, "can_free").unwrap();
        let free_bb = self.context.append_basic_block(function, "free_mem");
        self.builder
            .build_conditional_branch(can_free, free_bb, cont_bb)
            .unwrap();

        self.builder.position_at_end(free_bb);
        let free_fn = self.module.get_function("free").expect("free in prelude");
        self.builder
            .build_call(free_fn, &[raw_ptr.into()], "rc_free_call")
            .unwrap();
        self.builder.build_unconditional_branch(cont_bb).unwrap();

        self.builder.position_at_end(cont_bb);
        Ok(())
    }

    /// 读取弱引用指针，执行自动置零检查：
    /// 若指针为 null，返回 null；
    /// 若 strong_count > 0，返回原指针；
    /// 若 strong_count <= 0，返回 null。
    pub(super) fn emit_load_weak(
        &mut self,
        weak_ptr: PointerValue<'ctx>,
    ) -> Result<PointerValue<'ctx>> {
        let function = self.current_function()?;
        let not_null = self.ptr_is_not_null(weak_ptr);
        let check_bb = self.context.append_basic_block(function, "weak_check");
        let dead_bb = self.context.append_basic_block(function, "weak_dead");
        let cont_bb = self.context.append_basic_block(function, "weak_load_cont");

        self.builder
            .build_conditional_branch(not_null, check_bb, dead_bb)
            .unwrap();

        self.builder.position_at_end(check_bb);
        let minus_eight = self.context.i32_type().const_int((-8i64) as u64, true);
        let strong_ptr = unsafe {
            self.builder
                .build_gep(self.context.i8_type(), weak_ptr, &[minus_eight], "weak_strong_ptr")
                .unwrap()
        };
        let strong_cur = self
            .builder
            .build_load(self.context.i64_type(), strong_ptr, "weak_strong_cur")
            .unwrap()
            .into_int_value();
        let zero = self.context.i64_type().const_int(0, false);
        let is_alive = self
            .builder
            .build_int_compare(inkwell::IntPredicate::SGT, strong_cur, zero, "weak_is_alive")
            .unwrap();
        let alive_bb = self.context.append_basic_block(function, "weak_alive");
        self.builder
            .build_conditional_branch(is_alive, alive_bb, dead_bb)
            .unwrap();

        self.builder.position_at_end(alive_bb);
        self.builder.build_unconditional_branch(cont_bb).unwrap();

        self.builder.position_at_end(dead_bb);
        let null_ptr = weak_ptr.get_type().const_null();
        self.builder.build_unconditional_branch(cont_bb).unwrap();

        self.builder.position_at_end(cont_bb);
        let phi = self.builder.build_phi(weak_ptr.get_type(), "weak_val").unwrap();
        phi.add_incoming(&[(&weak_ptr, alive_bb), (&null_ptr, dead_bb)]);
        Ok(phi.as_basic_value().into_pointer_value())
    }

    /// 查询 Box / weak 的头部计数值：-8 为 strong_count，-16 为 weak_count。
    fn compile_rc_query(
        &mut self,
        fname: &str,
        offset: i64,
        arguments: &[huzi_ast::Expr],
    ) -> Result<BasicValueEnum<'ctx>> {
        if arguments.len() != 1 {
            return Err(HuziError::new_global(format!(
                "{}() requires exactly 1 argument",
                fname
            )));
        }
        let arg_expr = &arguments[0];
        if !self.is_box_expr(arg_expr) && !Self::is_null_expr(arg_expr) {
            return Err(HuziError::new_global(format!(
                "{}() requires a Box<T> argument",
                fname
            )));
        }
        let val = self.compile_expr(arg_expr)?;
        let ptr_val = match val {
            BasicValueEnum::PointerValue(pv) => pv,
            _ => {
                return Err(HuziError::new_global(format!(
                    "{}() requires a Box<T> pointer",
                    fname
                )));
            }
        };

        let not_null = self.ptr_is_not_null(ptr_val);
        let function = self.current_function()?;
        let not_null_bb = self.context.append_basic_block(function, "rc_nn");
        let null_bb = self.context.append_basic_block(function, "rc_nil");
        let cont_bb = self.context.append_basic_block(function, "rc_cont");

        self.builder
            .build_conditional_branch(not_null, not_null_bb, null_bb)
            .unwrap();

        self.builder.position_at_end(null_bb);
        let zero = self.context.i32_type().const_int(0, false);
        self.builder.build_unconditional_branch(cont_bb).unwrap();

        self.builder.position_at_end(not_null_bb);
        let offset_val = self.context.i32_type().const_int(offset as u64, true);
        let raw_ptr = unsafe {
            self.builder
                .build_gep(self.context.i8_type(), ptr_val, &[offset_val], "rc_raw")
                .unwrap()
        };
        let rc_i64 = self
            .builder
            .build_load(self.context.i64_type(), raw_ptr, "rc_val")
            .unwrap()
            .into_int_value();
        let rc_i32 = self
            .builder
            .build_int_truncate(rc_i64, self.context.i32_type(), "rc_i32")
            .unwrap();
        self.builder.build_unconditional_branch(cont_bb).unwrap();

        self.builder.position_at_end(cont_bb);
        let phi = self.builder.build_phi(self.context.i32_type(), "rc_res").unwrap();
        phi.add_incoming(&[(&zero, null_bb), (&rc_i32, not_null_bb)]);
        Ok(phi.as_basic_value())
    }

    /// `ref_count(b)` — 查询 Box 的当前引用计数。
    /// 若 b 为 null 返回 0,否则返回其头部计数值 (i32)。
    pub(super) fn compile_ref_count(
        &mut self,
        arguments: &[huzi_ast::Expr],
    ) -> Result<BasicValueEnum<'ctx>> {
        self.compile_rc_query("ref_count", -8, arguments)
    }

    /// `weak_count(b)` — 查询 Box 的当前弱引用计数。
    /// 若 b 为 null 返回 0,否则返回其弱计数值 (i32)。
    pub(super) fn compile_weak_count(
        &mut self,
        arguments: &[huzi_ast::Expr],
    ) -> Result<BasicValueEnum<'ctx>> {
        self.compile_rc_query("weak_count", -16, arguments)
    }

    /// 统一退出点:释放当前函数中除 `skip_ptrs` 外的所有可析构变量（Box/vec）。
    pub(super) fn emit_release_active_boxes(
        &mut self,
        skip_ptrs: &[PointerValue<'ctx>],
    ) -> Result<()> {
        self.emit_drop_all_scopes(skip_ptrs)
    }
}
