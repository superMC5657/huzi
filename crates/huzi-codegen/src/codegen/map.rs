//! HashMap 最小可用特化:`str -> i32`(开放寻址线性探测)。
//!
//! 表示:变量槽复用 vec 三元组 `{ ptr, len, cap }`(ptr 指向条目数组,
//! len 为已占条目数,cap 为桶数);条目为 `{ hash:i32, state:i32,
//! key_ptr:ptr, key_len:i32, val:i32 }`,state 0 空/1 已占/2 墓碑。
//! 哈希为自实现 FNV-1a(免 libc 依赖);键相等借用 `strcmp`(见
//! `expr_binary::build_string_compare`);缺键 `map_get` 返回
//! `(false, 0)` 不 abort;扩容翻倍并重哈希,墓碑清零。
//!
//! 边界(诚实声明):仅 `str->i32` 特化,无泛型/方法/impl/trait;
//! 键须为 `str`,值须为 `i32`;map 仅局部变量形态(无字段/参数类型
//! 语法),不可 `print`/不可函数传参,返回值仅 `map_get` 的元组。

use super::{CodeGen, VarSlot};
use huzi_ast::*;
use huzi_error::{HuziError, Result};
use inkwell::values::{BasicValueEnum, IntValue, PointerValue};

/// map 槽标记:结构体类型 + elem 为空 + array_len 为该哨兵。
/// 与 vec(需 elem)、元组/结构体(array_len 为空)均不重叠。
pub(super) const MAP_MARK: u32 = u32::MAX;

impl<'ctx> CodeGen<'ctx> {
    /// 是否为 `map_new(...)` 构造调用(供 `let` 分发)。
    pub(super) fn is_map_ctor(call: &CallExpr) -> bool {
        matches!(&*call.callee, Expr::Ident(name) if name == "map_new")
    }

    /// 变量槽是否为 map(三元组结构体 + 哨兵标记)。
    pub(super) fn is_map_slot(slot: &VarSlot<'_>) -> bool {
        slot.ty.is_struct_type() && slot.elem.is_none() && slot.array_len == Some(MAP_MARK)
    }

    /// 按名查找 map 槽;非 map 报边界清晰的错误。
    pub(super) fn map_slot_of(&self, name: &str) -> Result<VarSlot<'ctx>> {
        let slot = self
            .scope_lookup(name)
            .ok_or_else(|| self.unknown_variable_error(name))?;
        if !Self::is_map_slot(&slot) {
            return Err(HuziError::new_global(format!(
                "'{}' is not a HashMap (use `let mut {} = map_new()`; HashMap is specialized to str->i32 only, no generics)",
                name, name
            )));
        }
        Ok(slot)
    }

    /// 条目类型 `{ hash, state, key_ptr, key_len, val }`。
    pub(super) fn map_entry_type(&self) -> inkwell::types::StructType<'ctx> {
        self.context.struct_type(
            &[
                self.context.i32_type().into(),
                self.context.i32_type().into(),
                self.context.ptr_type(inkwell::AddressSpace::default()).into(),
                self.context.i32_type().into(),
                self.context.i32_type().into(),
            ],
            false,
        )
    }

    /// `let mut m = map_new()` — 零长存槽,标记为 map。
    pub(super) fn compile_let_map(
        &mut self,
        stmt: &LetStmt,
        arguments: &[Expr],
        span: Span,
    ) -> Result<()> {
        if !arguments.is_empty() {
            return Err(HuziError::new_global("map_new() takes no arguments"));
        }
        if stmt.type_annotation.is_some() {
            return Err(HuziError::new_global(
                "map_new() needs no type annotation; HashMap is always str->i32 (no generics)",
            ));
        }
        let vec_ty = self.vec_struct_type();
        let tmp = self.build_alloca(vec_ty.into(), "map_tmp")?;
        let null = self.context.ptr_type(inkwell::AddressSpace::default()).const_null();
        let zero = self.context.i32_type().const_int(0, false);
        let inits: [BasicValueEnum<'ctx>; 3] = [null.into(), zero.into(), zero.into()];
        for (i, v) in inits.iter().enumerate() {
            let p = self.builder.build_struct_gep(vec_ty, tmp, i as u32, "map_tmp_f").unwrap();
            self.builder.build_store(p, *v).unwrap();
        }
        let val = self.builder.build_load(vec_ty, tmp, "map_val").unwrap();
        let alloca = self.build_alloca(val.get_type(), &stmt.name)?;
        self.builder.build_store(alloca, val).unwrap();
        self.scope_insert(
            stmt.name.clone(),
            VarSlot {
                ptr: alloca,
                ty: val.get_type(),
                elem: None,
                array_len: Some(MAP_MARK),
                mutable: stmt.mutable,
                box_inner: None,
            },
        );
        self.declare_local(&stmt.name, alloca, val.get_type(), span);
        Ok(())
    }

    /// 表达式位置的 `map_new()`(如 `let` 之外,理论上少用)。
    pub(super) fn compile_map_new(&mut self, arguments: &[Expr]) -> Result<BasicValueEnum<'ctx>> {
        if !arguments.is_empty() {
            return Err(HuziError::new_global("map_new() takes no arguments"));
        }
        let vec_ty = self.vec_struct_type();
        let tmp = self.build_alloca(vec_ty.into(), "map_tmp")?;
        let null = self.context.ptr_type(inkwell::AddressSpace::default()).const_null();
        let zero = self.context.i32_type().const_int(0, false);
        let inits: [BasicValueEnum<'ctx>; 3] = [null.into(), zero.into(), zero.into()];
        for (i, v) in inits.iter().enumerate() {
            let p = self.builder.build_struct_gep(vec_ty, tmp, i as u32, "map_tmp_f").unwrap();
            self.builder.build_store(p, *v).unwrap();
        }
        Ok(self.builder.build_load(vec_ty, tmp, "map_val").unwrap())
    }

    /// 取 map 首参槽(表达式须为 map 变量名)。
    pub(super) fn map_first_slot(&self, fname: &str, first: &Expr) -> Result<VarSlot<'ctx>> {
        match first {
            Expr::Ident(name) => self.map_slot_of(name),
            _ => Err(HuziError::new_global(format!(
                "{}() first argument must be a HashMap variable (str->i32 only)",
                fname
            ))),
        }
    }

    /// 解析 map 实参的 (data, len, cap) 三件套，支持变量与结构体字段。
    pub(super) fn resolve_map_parts(
        &mut self,
        fname: &str,
        first: &Expr,
    ) -> Result<super::vec::VecParts<'ctx>> {
        if let Expr::Ident(name) = first {
            let slot = self.map_slot_of(name)?;
            return self.load_vec_parts(&slot);
        }
        if let Expr::FieldAccess(fa) = first {
            if let Some(ty) = self.field_ast_type(&fa.base, &fa.field) {
                if matches!(ty, Type::Named(ref n) | Type::Applied(ref n, _) if n == "Map" || n == "HashMap" || n == "map") {
                    let map_val = self.compile_expr(first)?;
                    if map_val.is_struct_value() {
                        let sv = map_val.into_struct_value();
                        let data = self
                            .builder
                            .build_extract_value(sv, 0, "map_data")
                            .unwrap()
                            .into_pointer_value();
                        let len = self
                            .builder
                            .build_extract_value(sv, 1, "map_len")
                            .unwrap()
                            .into_int_value();
                        let cap = self
                            .builder
                            .build_extract_value(sv, 2, "map_cap")
                            .unwrap()
                            .into_int_value();
                        return Ok(super::vec::VecParts { data, len, cap });
                    }
                }
            }
        }
        Err(HuziError::new_global(format!(
            "{}() first argument must be a HashMap variable or field (str->i32 only)",
            fname
        )))
    }

    /// 键实参须为 `str`(指针),否则报特化边界错误。
    pub(super) fn map_key_ptr(&mut self, expr: &Expr, fname: &str) -> Result<PointerValue<'ctx>> {
        let v = self.compile_expr(expr)?;
        match v {
            BasicValueEnum::PointerValue(p) => Ok(p),
            _ => Err(HuziError::new_global(format!(
                "{}() key must be str (HashMap is specialized to str->i32 only)",
                fname
            ))),
        }
    }

    /// 值实参须为 `i32`,否则报特化边界错误。
    pub(super) fn map_val_i32(&mut self, expr: &Expr) -> Result<IntValue<'ctx>> {
        let v = self.compile_expr(expr)?;
        match v {
            BasicValueEnum::IntValue(iv) if iv.get_type().get_bit_width() == 32 => Ok(iv),
            _ => Err(HuziError::new_global(
                "map_put() value must be i32 (HashMap is specialized to str->i32 only, no generics)",
            )),
        }
    }

    /// FNV-1a 32 自实现:按字节循环 `(h ^ b) * prime`。
    pub(super) fn map_hash(&mut self, key: PointerValue<'ctx>, klen: IntValue<'ctx>) -> Result<IntValue<'ctx>> {
        let i32_t = self.context.i32_type();
        let i8_t = self.context.i8_type();
        let f = self.current_function()?;
        let loop_bb = self.context.append_basic_block(f, "fnv_loop");
        let body_bb = self.context.append_basic_block(f, "fnv_body");
        let done_bb = self.context.append_basic_block(f, "fnv_done");
        let h = self.build_alloca(i32_t.into(), "fnv_h")?;
        let i = self.build_alloca(i32_t.into(), "fnv_i")?;
        self.builder.build_store(h, i32_t.const_int(2166136261, false)).unwrap();
        self.builder.build_store(i, i32_t.const_int(0, false)).unwrap();
        self.builder.build_unconditional_branch(loop_bb).unwrap();
        self.builder.position_at_end(loop_bb);
        let iv = self.builder.build_load(i32_t, i, "fnv_iv").unwrap().into_int_value();
        let go = self.builder.build_int_compare(inkwell::IntPredicate::ULT, iv, klen, "fnv_go").unwrap();
        self.builder.build_conditional_branch(go, body_bb, done_bb).unwrap();
        self.builder.position_at_end(body_bb);
        self.map_hash_step(h, i, key, i8_t, i32_t, loop_bb)?;
        self.builder.position_at_end(done_bb);
        Ok(self.builder.build_load(i32_t, h, "fnv_r").unwrap().into_int_value())
    }

    /// FNV 单步:取字节、xor、乘 prime,下标加一。
    fn map_hash_step(
        &mut self,
        h: PointerValue<'ctx>,
        i: PointerValue<'ctx>,
        key: PointerValue<'ctx>,
        i8_t: inkwell::types::IntType<'ctx>,
        i32_t: inkwell::types::IntType<'ctx>,
        next: inkwell::basic_block::BasicBlock<'ctx>,
    ) -> Result<()> {
        let hv = self.builder.build_load(i32_t, h, "fnv_hv").unwrap().into_int_value();
        let iv = self.builder.build_load(i32_t, i, "fnv_iv").unwrap().into_int_value();
        let bp = unsafe { self.builder.build_gep(i8_t, key, &[iv], "fnv_bp").unwrap() };
        let b = self.builder.build_load(i8_t, bp, "fnv_b").unwrap().into_int_value();
        let bz = self.builder.build_int_z_extend(b, i32_t, "fnv_bz").unwrap();
        let x = self.builder.build_xor(hv, bz, "fnv_x").unwrap();
        let nh = self.builder.build_int_mul(x, i32_t.const_int(16777619, false), "fnv_m").unwrap();
        self.builder.build_store(h, nh).unwrap();
        let ni = self.builder.build_int_add(iv, i32_t.const_int(1, false), "fnv_ni").unwrap();
        self.builder.build_store(i, ni).unwrap();
        self.builder.build_unconditional_branch(next).unwrap();
        Ok(())
    }

    /// 键相等:借用 `strcmp`(与 `==` 同语义,字节比较)。
    fn map_key_matches(
        &mut self,
        stored: PointerValue<'ctx>,
        key: PointerValue<'ctx>,
    ) -> IntValue<'ctx> {
        let sc = self.module.get_function("strcmp").expect("strcmp in prelude");
        let c = self.builder.build_call(sc, &[stored.into(), key.into()], "map_cmp").unwrap()
            .try_as_basic_value().unwrap_left().into_int_value();
        self.builder.build_int_compare(inkwell::IntPredicate::EQ, c,
            self.context.i32_type().const_int(0, false), "map_eq").unwrap()
    }

    /// 读条目 state 字段。
    pub(super) fn map_entry_state(
        &self,
        entry_ty: inkwell::types::StructType<'ctx>,
        data: PointerValue<'ctx>,
        ety: inkwell::types::BasicTypeEnum<'ctx>,
        idx: IntValue<'ctx>,
    ) -> IntValue<'ctx> {
        let ep = unsafe { self.builder.build_gep(ety, data, &[idx], "map_ep").unwrap() };
        let sp = self.builder.build_struct_gep(entry_ty, ep, 1, "map_sp").unwrap();
        self.builder.build_load(self.context.i32_type(), sp, "map_st").unwrap().into_int_value()
    }

    /// 查找键下标,命中返回下标否则 -1(空桶截断,墓碑跳过)。
    pub(super) fn map_find_index(
        &mut self,
        data: PointerValue<'ctx>,
        cap: IntValue<'ctx>,
        hash: IntValue<'ctx>,
        key: PointerValue<'ctx>,
    ) -> Result<IntValue<'ctx>> {
        let i32_t = self.context.i32_type();
        let ety = self.map_entry_type();
        let e = ety.into();
        let f = self.current_function()?;
        let loop_bb = self.context.append_basic_block(f, "find_loop");
        let body_bb = self.context.append_basic_block(f, "find_body");
        let chk_bb = self.context.append_basic_block(f, "find_chk");
        let next_bb = self.context.append_basic_block(f, "find_next");
        let done_bb = self.context.append_basic_block(f, "find_done");
        let idx = self.build_alloca(i32_t.into(), "find_idx")?;
        let cnt = self.build_alloca(i32_t.into(), "find_cnt")?;
        let out = self.build_alloca(i32_t.into(), "find_out")?;
        let start = self.builder.build_int_unsigned_rem(hash, cap, "find_start").unwrap();
        self.builder.build_store(idx, start).unwrap();
        self.builder.build_store(cnt, i32_t.const_int(0, false)).unwrap();
        self.builder.build_store(out, i32_t.const_int((-1i32) as u64, false)).unwrap();
        self.builder.build_unconditional_branch(loop_bb).unwrap();
        self.builder.position_at_end(loop_bb);
        let cv = self.builder.build_load(i32_t, cnt, "find_cv").unwrap().into_int_value();
        let go = self.builder.build_int_compare(inkwell::IntPredicate::ULT, cv, cap, "find_go").unwrap();
        self.builder.build_conditional_branch(go, body_bb, done_bb).unwrap();
        self.builder.position_at_end(body_bb);
        let iv = self.builder.build_load(i32_t, idx, "find_iv").unwrap().into_int_value();
        let st = self.map_entry_state(ety, data, e, iv);
        let empty = self.builder.build_int_compare(inkwell::IntPredicate::EQ, st,
            i32_t.const_int(0, false), "find_empty").unwrap();
        self.builder.build_conditional_branch(empty, done_bb, chk_bb).unwrap();
        self.builder.position_at_end(chk_bb);
        self.map_find_check(data, e, ety, idx, hash, key, out, next_bb, done_bb)?;
        self.builder.position_at_end(next_bb);
        self.map_probe_advance(idx, cnt, cap, i32_t, loop_bb)?;
        self.builder.position_at_end(done_bb);
        Ok(self.builder.build_load(i32_t, out, "find_r").unwrap().into_int_value())
    }

    /// 查找命中判定:state==1 且 hash 相等且 strcmp 相等。
    #[allow(clippy::too_many_arguments)]
    fn map_find_check(
        &mut self,
        data: PointerValue<'ctx>,
        e: inkwell::types::BasicTypeEnum<'ctx>,
        ety: inkwell::types::StructType<'ctx>,
        idx: PointerValue<'ctx>,
        hash: IntValue<'ctx>,
        key: PointerValue<'ctx>,
        out: PointerValue<'ctx>,
        next: inkwell::basic_block::BasicBlock<'ctx>,
        done: inkwell::basic_block::BasicBlock<'ctx>,
    ) -> Result<()> {
        let i32_t = self.context.i32_type();
        let iv = self.builder.build_load(i32_t, idx, "find_iv").unwrap().into_int_value();
        let ep = unsafe { self.builder.build_gep(e, data, &[iv], "find_ep").unwrap() };
        let hp = self.builder.build_struct_gep(ety, ep, 0, "find_hp").unwrap();
        let hv = self.builder.build_load(i32_t, hp, "find_hv").unwrap().into_int_value();
        let kp = self.builder.build_struct_gep(ety, ep, 2, "find_kp").unwrap();
        let ptr_t = self.context.ptr_type(inkwell::AddressSpace::default());
        let sv = self.builder.build_load(ptr_t, kp, "find_sv").unwrap().into_pointer_value();
        let st = self.map_entry_state(ety, data, e, iv);
        let occ = self.builder.build_int_compare(inkwell::IntPredicate::EQ, st,
            i32_t.const_int(1, false), "find_occ").unwrap();
        let heq = self.builder.build_int_compare(inkwell::IntPredicate::EQ, hv, hash, "find_heq").unwrap();
        let keq = self.map_key_matches(sv, key);
        let a = self.builder.build_and(occ, heq, "find_a").unwrap();
        let hit = self.builder.build_and(a, keq, "find_hit").unwrap();
        let f = self.current_function()?;
        let hit_bb = self.context.append_basic_block(f, "find_hit");
        self.builder.build_conditional_branch(hit, hit_bb, next).unwrap();
        self.builder.position_at_end(hit_bb);
        self.builder.build_store(out, iv).unwrap();
        self.builder.build_unconditional_branch(done).unwrap();
        Ok(())
    }

    /// 探测推进:idx=(idx+1)%cap, cnt+1。
    fn map_probe_advance(
        &mut self,
        idx: PointerValue<'ctx>,
        cnt: PointerValue<'ctx>,
        cap: IntValue<'ctx>,
        i32_t: inkwell::types::IntType<'ctx>,
        next: inkwell::basic_block::BasicBlock<'ctx>,
    ) -> Result<()> {
        let iv = self.builder.build_load(i32_t, idx, "find_iv").unwrap().into_int_value();
        let ni = self.builder.build_int_add(iv, i32_t.const_int(1, false), "find_ni").unwrap();
        let wi = self.builder.build_int_unsigned_rem(ni, cap, "find_wi").unwrap();
        self.builder.build_store(idx, wi).unwrap();
        let cv = self.builder.build_load(i32_t, cnt, "find_cv").unwrap().into_int_value();
        let nc = self.builder.build_int_add(cv, i32_t.const_int(1, false), "find_nc").unwrap();
        self.builder.build_store(cnt, nc).unwrap();
        self.builder.build_unconditional_branch(next).unwrap();
        Ok(())
    }

    /// 首个非已占桶下标(空或墓碑,调用方已确认键不存在)。
    pub(super) fn map_free_index(
        &mut self,
        data: PointerValue<'ctx>,
        cap: IntValue<'ctx>,
        hash: IntValue<'ctx>,
    ) -> Result<IntValue<'ctx>> {
        let i32_t = self.context.i32_type();
        let ety = self.map_entry_type();
        let e = ety.into();
        let f = self.current_function()?;
        let loop_bb = self.context.append_basic_block(f, "free_loop");
        let body_bb = self.context.append_basic_block(f, "free_body");
        let done_bb = self.context.append_basic_block(f, "free_done");
        let idx = self.build_alloca(i32_t.into(), "free_idx")?;
        let out = self.build_alloca(i32_t.into(), "free_out")?;
        let start = self.builder.build_int_unsigned_rem(hash, cap, "free_start").unwrap();
        self.builder.build_store(idx, start).unwrap();
        self.builder.build_store(out, i32_t.const_int((-1i32) as u64, false)).unwrap();
        self.builder.build_unconditional_branch(loop_bb).unwrap();
        self.builder.position_at_end(loop_bb);
        let iv = self.builder.build_load(i32_t, idx, "free_iv").unwrap().into_int_value();
        let st = self.map_entry_state(ety, data, e, iv);
        let occ = self.builder.build_int_compare(inkwell::IntPredicate::EQ, st,
            i32_t.const_int(1, false), "free_occ").unwrap();
        self.builder.build_conditional_branch(occ, body_bb, done_bb).unwrap();
        self.builder.position_at_end(body_bb);
        let jv = self.builder.build_load(i32_t, idx, "free_iv").unwrap().into_int_value();
        let ni = self.builder.build_int_add(jv, i32_t.const_int(1, false), "free_ni").unwrap();
        let wi = self.builder.build_int_unsigned_rem(ni, cap, "free_wi").unwrap();
        self.builder.build_store(idx, wi).unwrap();
        self.builder.build_unconditional_branch(loop_bb).unwrap();
        self.builder.position_at_end(done_bb);
        let fv = self.builder.build_load(i32_t, idx, "free_iv").unwrap().into_int_value();
        self.builder.build_store(out, fv).unwrap();
        Ok(self.builder.build_load(i32_t, out, "free_r").unwrap().into_int_value())
    }
}
