//! HashMap 操作:扩容重哈希 + `map_put/get/has/remove/len`。
//!
//! 与 `map.rs` 同属 map 模块:构造/哈希/探测留在 `map.rs`,本文件只放
//! 扩容与六函数中的五个操作(`map_new` 见 `map.rs`)。槽布局复用
//! `vec.rs` 的 `{ data, len, cap }` 三元组与 `load/store_vec_parts`;
//! 扩散条件 `(len+1)*10 > cap*7`(含空表首扩);新表首容量 8 翻倍。

use super::vec::VecParts;
use super::CodeGen;
use huzi_ast::*;
use huzi_error::{HuziError, Result};
use inkwell::values::{BasicValueEnum, IntValue, PointerValue};

impl<'ctx> CodeGen<'ctx> {
    /// 新表条目清零 `{ 0, 0, null, 0, 0 }`。
    fn map_zero_entries(
        &mut self,
        data: PointerValue<'ctx>,
        cap: IntValue<'ctx>,
    ) -> Result<()> {
        let i32_t = self.context.i32_type();
        let ety = self.map_entry_type();
        let e: inkwell::types::BasicTypeEnum<'ctx> = ety.into();
        let f = self.current_function()?;
        let cond_bb = self.context.append_basic_block(f, "zero_cond");
        let body_bb = self.context.append_basic_block(f, "zero_body");
        let done_bb = self.context.append_basic_block(f, "zero_done");
        let cur = self.build_alloca(i32_t.into(), "zero_i")?;
        self.builder.build_store(cur, i32_t.const_int(0, false)).unwrap();
        self.builder.build_unconditional_branch(cond_bb).unwrap();
        self.builder.position_at_end(cond_bb);
        let cv = self.builder.build_load(i32_t, cur, "zero_cv").unwrap().into_int_value();
        let go = self.builder.build_int_compare(inkwell::IntPredicate::ULT, cv, cap, "zero_go").unwrap();
        self.builder.build_conditional_branch(go, body_bb, done_bb).unwrap();
        self.builder.position_at_end(body_bb);
        self.map_zero_one(data, e, ety, cur)?;
        self.builder.build_unconditional_branch(cond_bb).unwrap();
        self.builder.position_at_end(done_bb);
        Ok(())
    }

    /// 清零单个条目并推进下标。
    fn map_zero_one(
        &mut self,
        data: PointerValue<'ctx>,
        e: inkwell::types::BasicTypeEnum<'ctx>,
        ety: inkwell::types::StructType<'ctx>,
        cur: PointerValue<'ctx>,
    ) -> Result<()> {
        let i32_t = self.context.i32_type();
        let iv = self.builder.build_load(i32_t, cur, "zero_cv").unwrap().into_int_value();
        let ep = unsafe { self.builder.build_gep(e, data, &[iv], "zero_ep").unwrap() };
        let vals: [BasicValueEnum<'ctx>; 5] = [
            i32_t.const_int(0, false).into(),
            i32_t.const_int(0, false).into(),
            self.context.ptr_type(inkwell::AddressSpace::default()).const_null().into(),
            i32_t.const_int(0, false).into(),
            i32_t.const_int(0, false).into(),
        ];
        for (k, v) in vals.iter().enumerate() {
            let fp = self.builder.build_struct_gep(ety, ep, k as u32, "zero_fp").unwrap();
            self.builder.build_store(fp, *v).unwrap();
        }
        let nx = self.builder.build_int_add(iv, i32_t.const_int(1, false), "zero_nx").unwrap();
        self.builder.build_store(cur, nx).unwrap();
        Ok(())
    }

    /// 重哈希单条:新表内探测首个空桶并写入(新表无墓碑)。
    fn map_rehash_insert(
        &mut self,
        new_data: PointerValue<'ctx>,
        new_cap: IntValue<'ctx>,
        hash: IntValue<'ctx>,
        key: PointerValue<'ctx>,
        klen: IntValue<'ctx>,
        val: IntValue<'ctx>,
    ) -> Result<()> {
        let i32_t = self.context.i32_type();
        let ety = self.map_entry_type();
        let e: inkwell::types::BasicTypeEnum<'ctx> = ety.into();
        let f = self.current_function()?;
        let loop_bb = self.context.append_basic_block(f, "rh_loop");
        let body_bb = self.context.append_basic_block(f, "rh_body");
        let done_bb = self.context.append_basic_block(f, "rh_done");
        let idx = self.build_alloca(i32_t.into(), "rh_idx")?;
        let st0 = self.builder.build_int_unsigned_rem(hash, new_cap, "rh_start").unwrap();
        self.builder.build_store(idx, st0).unwrap();
        self.builder.build_unconditional_branch(loop_bb).unwrap();
        self.builder.position_at_end(loop_bb);
        let iv = self.builder.build_load(i32_t, idx, "rh_iv").unwrap().into_int_value();
        let ep = unsafe { self.builder.build_gep(e, new_data, &[iv], "rh_ep").unwrap() };
        let sp = self.builder.build_struct_gep(ety, ep, 1, "rh_sp").unwrap();
        let sv = self.builder.build_load(i32_t, sp, "rh_sv").unwrap().into_int_value();
        let empty = self.builder.build_int_compare(inkwell::IntPredicate::EQ, sv,
            i32_t.const_int(0, false), "rh_empty").unwrap();
        self.builder.build_conditional_branch(empty, done_bb, body_bb).unwrap();
        self.builder.position_at_end(body_bb);
        let jv = self.builder.build_load(i32_t, idx, "rh_iv").unwrap().into_int_value();
        let ni = self.builder.build_int_add(jv, i32_t.const_int(1, false), "rh_ni").unwrap();
        let wi = self.builder.build_int_unsigned_rem(ni, new_cap, "rh_wi").unwrap();
        self.builder.build_store(idx, wi).unwrap();
        self.builder.build_unconditional_branch(loop_bb).unwrap();
        self.builder.position_at_end(done_bb);
        self.map_store_new_entry(new_data, ety, idx, hash, key, klen, val)?;
        Ok(())
    }

    /// 旧表已占条目逐条重插新表。
    fn map_rehash(
        &mut self,
        old_data: PointerValue<'ctx>,
        old_cap: IntValue<'ctx>,
        new_data: PointerValue<'ctx>,
        new_cap: IntValue<'ctx>,
    ) -> Result<()> {
        let i32_t = self.context.i32_type();
        let ety = self.map_entry_type();
        let e: inkwell::types::BasicTypeEnum<'ctx> = ety.into();
        let f = self.current_function()?;
        let cond_bb = self.context.append_basic_block(f, "reh_cond");
        let body_bb = self.context.append_basic_block(f, "reh_body");
        let copy_bb = self.context.append_basic_block(f, "reh_copy");
        let incr_bb = self.context.append_basic_block(f, "reh_incr");
        let done_bb = self.context.append_basic_block(f, "reh_done");
        let cur = self.build_alloca(i32_t.into(), "reh_i")?;
        self.builder.build_store(cur, i32_t.const_int(0, false)).unwrap();
        self.builder.build_unconditional_branch(cond_bb).unwrap();
        self.builder.position_at_end(cond_bb);
        let cv = self.builder.build_load(i32_t, cur, "reh_cv").unwrap().into_int_value();
        let go = self.builder.build_int_compare(inkwell::IntPredicate::ULT, cv, old_cap, "reh_go").unwrap();
        self.builder.build_conditional_branch(go, body_bb, done_bb).unwrap();
        self.builder.position_at_end(body_bb);
        let iv = self.builder.build_load(i32_t, cur, "reh_iv").unwrap().into_int_value();
        let ep = unsafe { self.builder.build_gep(e, old_data, &[iv], "reh_ep").unwrap() };
        let sp = self.builder.build_struct_gep(ety, ep, 1, "reh_sp").unwrap();
        let sv = self.builder.build_load(i32_t, sp, "reh_sv").unwrap().into_int_value();
        let occ = self.builder.build_int_compare(inkwell::IntPredicate::EQ, sv,
            i32_t.const_int(1, false), "reh_occ").unwrap();
        self.builder.build_conditional_branch(occ, copy_bb, incr_bb).unwrap();
        self.builder.position_at_end(copy_bb);
        self.map_rehash_copy(old_data, e, ety, cur, new_data, new_cap, incr_bb)?;
        self.builder.position_at_end(incr_bb);
        let jv = self.builder.build_load(i32_t, cur, "reh_iv").unwrap().into_int_value();
        let nx = self.builder.build_int_add(jv, i32_t.const_int(1, false), "reh_nx").unwrap();
        self.builder.build_store(cur, nx).unwrap();
        self.builder.build_unconditional_branch(cond_bb).unwrap();
        self.builder.position_at_end(done_bb);
        Ok(())
    }

    /// 载入旧条目五字段并插入新表,随后回到 `next`。
    fn map_rehash_copy(
        &mut self,
        old_data: PointerValue<'ctx>,
        e: inkwell::types::BasicTypeEnum<'ctx>,
        ety: inkwell::types::StructType<'ctx>,
        cur: PointerValue<'ctx>,
        new_data: PointerValue<'ctx>,
        new_cap: IntValue<'ctx>,
        next: inkwell::basic_block::BasicBlock<'ctx>,
    ) -> Result<()> {
        let i32_t = self.context.i32_type();
        let iv = self.builder.build_load(i32_t, cur, "reh_iv").unwrap().into_int_value();
        let ep = unsafe { self.builder.build_gep(e, old_data, &[iv], "reh_ep").unwrap() };
        let hp = self.builder.build_struct_gep(ety, ep, 0, "reh_hp").unwrap();
        let hv = self.builder.build_load(i32_t, hp, "reh_hv").unwrap().into_int_value();
        let kp = self.builder.build_struct_gep(ety, ep, 2, "reh_kp").unwrap();
        let kv = self.builder.build_load(self.context.ptr_type(inkwell::AddressSpace::default()),
            kp, "reh_kv").unwrap().into_pointer_value();
        let lp = self.builder.build_struct_gep(ety, ep, 3, "reh_lp").unwrap();
        let lv = self.builder.build_load(i32_t, lp, "reh_lv").unwrap().into_int_value();
        let vp = self.builder.build_struct_gep(ety, ep, 4, "reh_vp").unwrap();
        let vv = self.builder.build_load(i32_t, vp, "reh_vv").unwrap().into_int_value();
        self.map_rehash_insert(new_data, new_cap, hv, kv, lv, vv)?;
        self.builder.build_unconditional_branch(next).unwrap();
        Ok(())
    }

    /// 按负载 `(len+1)*10 > cap*7` 翻倍(空表首扩 8),重哈希后返回新 parts。
    fn map_ensure_capacity(
        &mut self,
        slot: &super::VarSlot<'ctx>,
        parts: &VecParts<'ctx>,
    ) -> Result<VecParts<'ctx>> {
        let i32_t = self.context.i32_type();
        let vec_ty = self.vec_struct_type();
        let f = self.current_function()?;
        let grow_bb = self.context.append_basic_block(f, "map_grow");
        let done_bb = self.context.append_basic_block(f, "map_cap_done");
        let plus1 = self.builder.build_int_add(parts.len, i32_t.const_int(1, false), "map_p1").unwrap();
        let lhs = self.builder.build_int_mul(plus1, i32_t.const_int(10, false), "map_lhs").unwrap();
        let rhs = self.builder.build_int_mul(parts.cap, i32_t.const_int(7, false), "map_rhs").unwrap();
        let full = self.builder.build_int_compare(inkwell::IntPredicate::UGT, lhs, rhs, "map_full").unwrap();
        self.builder.build_conditional_branch(full, grow_bb, done_bb).unwrap();
        self.builder.position_at_end(grow_bb);
        self.map_grow(slot, parts, vec_ty)?;
        self.builder.build_unconditional_branch(done_bb).unwrap();
        self.builder.position_at_end(done_bb);
        self.load_vec_parts(slot)
    }

    /// 分配新表、清零、重哈希、回写槽。
    fn map_grow(
        &mut self,
        slot: &super::VarSlot<'ctx>,
        parts: &VecParts<'ctx>,
        vec_ty: inkwell::types::StructType<'ctx>,
    ) -> Result<()> {
        let i32_t = self.context.i32_type();
        let ety = self.map_entry_type();
        let dbl = self.builder.build_int_mul(parts.cap, i32_t.const_int(2, false), "map_dbl").unwrap();
        let is0 = self.builder.build_int_compare(inkwell::IntPredicate::EQ, parts.cap,
            i32_t.const_int(0, false), "map_is0").unwrap();
        let ncap = self.builder.build_select(is0, i32_t.const_int(8, false),
            dbl, "map_ncap").unwrap().into_int_value();
        let et: inkwell::types::BasicTypeEnum<'ctx> = ety.into();
        let ndata = self.builder.build_array_malloc(et, ncap, "map_ndata")
            .map_err(|_| HuziError::new_global("Failed to allocate HashMap storage"))?;
        self.map_zero_entries(ndata, ncap)?;
        self.map_rehash(parts.data, parts.cap, ndata, ncap)?;
        self.store_vec_parts(slot, vec_ty, &VecParts { data: ndata, len: parts.len, cap: ncap });
        Ok(())
    }

    /// 在 `idx` 处写入新条目 `{ hash, 1, key, klen, val }`。
    fn map_store_new_entry(
        &mut self,
        data: PointerValue<'ctx>,
        ety: inkwell::types::StructType<'ctx>,
        idx_slot: PointerValue<'ctx>,
        hash: IntValue<'ctx>,
        key: PointerValue<'ctx>,
        klen: IntValue<'ctx>,
        val: IntValue<'ctx>,
    ) -> Result<()> {
        let i32_t = self.context.i32_type();
        let e: inkwell::types::BasicTypeEnum<'ctx> = ety.into();
        let iv = self.builder.build_load(i32_t, idx_slot, "put_iv").unwrap().into_int_value();
        let ep = unsafe { self.builder.build_gep(e, data, &[iv], "put_ep").unwrap() };
        let parts: [(u32, BasicValueEnum<'ctx>); 5] = [
            (0, hash.into()), (1, i32_t.const_int(1, false).into()),
            (2, key.into()), (3, klen.into()), (4, val.into()),
        ];
        for (k, v) in parts {
            let fp = self.builder.build_struct_gep(ety, ep, k, "put_fp").unwrap();
            self.builder.build_store(fp, v).unwrap();
        }
        Ok(())
    }

    /// `map_put(m, k, v)` — 需 `let mut`;存在则覆盖,不存在则插入。
    pub(super) fn compile_map_put(&mut self, arguments: &[Expr]) -> Result<BasicValueEnum<'ctx>> {
        if arguments.len() != 3 {
            return Err(HuziError::new_global("map_put() requires exactly 3 arguments (map, key, value)"));
        }
        let slot = self.map_first_slot("map_put", &arguments[0])?;
        self.ensure_mutable(&arguments[0])?;
        let key = self.map_key_ptr(&arguments[1], "map_put")?;
        let val = self.map_val_i32(&arguments[2])?;
        let parts = self.load_vec_parts(&slot)?;
        let grown = self.map_ensure_capacity(&slot, &parts)?;
        let klen = self.str_len_of(key, "put_klen")?;
        let hash = self.map_hash(key, klen)?;
        let found = self.map_find_index(grown.data, grown.cap, hash, key)?;
        let f = self.current_function()?;
        let hit_bb = self.context.append_basic_block(f, "put_hit");
        let miss_bb = self.context.append_basic_block(f, "put_miss");
        let done_bb = self.context.append_basic_block(f, "put_done");
        let absent = self.builder.build_int_compare(inkwell::IntPredicate::EQ, found,
            self.context.i32_type().const_int((-1i32) as u64, false), "put_abs").unwrap();
        self.builder.build_conditional_branch(absent, miss_bb, hit_bb).unwrap();
        self.builder.position_at_end(hit_bb);
        self.map_put_update(grown.data, found, val, done_bb)?;
        self.builder.position_at_end(miss_bb);
        self.map_put_insert(&slot, &grown, hash, key, klen, val, done_bb)?;
        self.builder.position_at_end(done_bb);
        Ok(self.context.i32_type().const_int(0, false).into())
    }

    /// 命中路径:原位覆盖 `val` 字段。
    fn map_put_update(
        &mut self,
        data: PointerValue<'ctx>,
        found: IntValue<'ctx>,
        val: IntValue<'ctx>,
        done: inkwell::basic_block::BasicBlock<'ctx>,
    ) -> Result<()> {
        let ety = self.map_entry_type();
        let e: inkwell::types::BasicTypeEnum<'ctx> = ety.into();
        let ep = unsafe { self.builder.build_gep(e, data, &[found], "put_ep").unwrap() };
        let vp = self.builder.build_struct_gep(ety, ep, 4, "put_vp").unwrap();
        self.builder.build_store(vp, val).unwrap();
        self.builder.build_unconditional_branch(done).unwrap();
        Ok(())
    }

    /// 缺失路径:找空/墓碑位写入并 `len+1`。
    fn map_put_insert(
        &mut self,
        slot: &super::VarSlot<'ctx>,
        grown: &VecParts<'ctx>,
        hash: IntValue<'ctx>,
        key: PointerValue<'ctx>,
        klen: IntValue<'ctx>,
        val: IntValue<'ctx>,
        done: inkwell::basic_block::BasicBlock<'ctx>,
    ) -> Result<()> {
        let i32_t = self.context.i32_type();
        let ety = self.map_entry_type();
        let free_idx = self.map_free_index(grown.data, grown.cap, hash)?;
        let idx_slot = self.build_alloca(i32_t.into(), "put_free")?;
        self.builder.build_store(idx_slot, free_idx).unwrap();
        self.map_store_new_entry(grown.data, ety, idx_slot, hash, key, klen, val)?;
        let nlen = self.builder.build_int_add(grown.len, i32_t.const_int(1, false), "put_nlen").unwrap();
        self.store_vec_parts(slot, self.vec_struct_type(), &VecParts {
            data: grown.data, len: nlen, cap: grown.cap,
        });
        self.builder.build_unconditional_branch(done).unwrap();
        Ok(())
    }

    /// 组装 `(found: bool, val: i32)` 元组值。
    fn map_found_tuple(&mut self, found: IntValue<'ctx>, val: IntValue<'ctx>) -> Result<BasicValueEnum<'ctx>> {
        let tup = self.context.struct_type(&[self.context.bool_type().into(),
            self.context.i32_type().into()], false);
        let tmp = self.build_alloca(tup.into(), "get_tup")?;
        let f0 = self.builder.build_struct_gep(tup, tmp, 0, "get_f0").unwrap();
        self.builder.build_store(f0, found).unwrap();
        let f1 = self.builder.build_struct_gep(tup, tmp, 1, "get_f1").unwrap();
        self.builder.build_store(f1, val).unwrap();
        Ok(self.builder.build_load(tup, tmp, "get_tv").unwrap())
    }

    /// `map_get(m, k)` — 缺键返回 `(false, 0)` 不 abort。
    pub(super) fn compile_map_get(&mut self, arguments: &[Expr]) -> Result<BasicValueEnum<'ctx>> {
        if arguments.len() != 2 {
            return Err(HuziError::new_global("map_get() requires exactly 2 arguments (map, key)"));
        }
        let parts = self.resolve_map_parts("map_get", &arguments[0])?;
        let key = self.map_key_ptr(&arguments[1], "map_get")?;
        let i32_t = self.context.i32_type();
        let f = self.current_function()?;
        let probe_bb = self.context.append_basic_block(f, "get_probe");
        let done_bb = self.context.append_basic_block(f, "get_done");
        let found_s = self.build_alloca(self.context.bool_type().into(), "get_found")?;
        let val_s = self.build_alloca(i32_t.into(), "get_val")?;
        self.builder.build_store(found_s, self.context.bool_type().const_int(0, false)).unwrap();
        self.builder.build_store(val_s, i32_t.const_int(0, false)).unwrap();
        let empty = self.builder.build_int_compare(inkwell::IntPredicate::EQ, parts.cap,
            i32_t.const_int(0, false), "get_empty").unwrap();
        self.builder.build_conditional_branch(empty, done_bb, probe_bb).unwrap();
        self.builder.position_at_end(probe_bb);
        self.map_get_fill(parts.data, parts.cap, key, found_s, val_s, done_bb)?;
        self.builder.position_at_end(done_bb);
        let fb = self.builder.build_load(self.context.bool_type(), found_s, "get_fb").unwrap().into_int_value();
        let vb = self.builder.build_load(i32_t, val_s, "get_vb").unwrap().into_int_value();
        self.map_found_tuple(fb, vb)
    }

    /// 探测命中时回填 `(true, val)`。
    fn map_get_fill(
        &mut self,
        data: PointerValue<'ctx>,
        cap: IntValue<'ctx>,
        key: PointerValue<'ctx>,
        found_s: PointerValue<'ctx>,
        val_s: PointerValue<'ctx>,
        done: inkwell::basic_block::BasicBlock<'ctx>,
    ) -> Result<()> {
        let i32_t = self.context.i32_type();
        let ety = self.map_entry_type();
        let e: inkwell::types::BasicTypeEnum<'ctx> = ety.into();
        let klen = self.str_len_of(key, "get_klen")?;
        let hash = self.map_hash(key, klen)?;
        let idx = self.map_find_index(data, cap, hash, key)?;
        let hit = self.builder.build_int_compare(inkwell::IntPredicate::NE, idx,
            i32_t.const_int((-1i32) as u64, false), "get_hit").unwrap();
        let f = self.current_function()?;
        let hit_bb = self.context.append_basic_block(f, "get_hit");
        self.builder.build_conditional_branch(hit, hit_bb, done).unwrap();
        self.builder.position_at_end(hit_bb);
        let ep = unsafe { self.builder.build_gep(e, data, &[idx], "get_ep").unwrap() };
        let vp = self.builder.build_struct_gep(ety, ep, 4, "get_vp").unwrap();
        let vv = self.builder.build_load(i32_t, vp, "get_vv").unwrap();
        self.builder.build_store(found_s, self.context.bool_type().const_int(1, false)).unwrap();
        self.builder.build_store(val_s, vv).unwrap();
        self.builder.build_unconditional_branch(done).unwrap();
        Ok(())
    }

    /// `map_has(m, k)` — 存在返回 true(含空表 false)。
    pub(super) fn compile_map_has(&mut self, arguments: &[Expr]) -> Result<BasicValueEnum<'ctx>> {
        if arguments.len() != 2 {
            return Err(HuziError::new_global("map_has() requires exactly 2 arguments (map, key)"));
        }
        let parts = self.resolve_map_parts("map_has", &arguments[0])?;
        let key = self.map_key_ptr(&arguments[1], "map_has")?;
        let i32_t = self.context.i32_type();
        let is0 = self.builder.build_int_compare(inkwell::IntPredicate::EQ, parts.cap,
            i32_t.const_int(0, false), "has_empty").unwrap();
        let f = self.current_function()?;
        let probe_bb = self.context.append_basic_block(f, "has_probe");
        let done_bb = self.context.append_basic_block(f, "has_done");
        let out = self.build_alloca(self.context.bool_type().into(), "has_out")?;
        self.builder.build_store(out, self.context.bool_type().const_int(0, false)).unwrap();
        self.builder.build_conditional_branch(is0, done_bb, probe_bb).unwrap();
        self.builder.position_at_end(probe_bb);
        let klen = self.str_len_of(key, "has_klen")?;
        let hash = self.map_hash(key, klen)?;
        let idx = self.map_find_index(parts.data, parts.cap, hash, key)?;
        let hit = self.builder.build_int_compare(inkwell::IntPredicate::NE, idx,
            i32_t.const_int((-1i32) as u64, false), "has_hit").unwrap();
        self.builder.build_store(out, hit).unwrap();
        self.builder.build_unconditional_branch(done_bb).unwrap();
        self.builder.position_at_end(done_bb);
        Ok(self.builder.build_load(self.context.bool_type(), out, "has_r").unwrap())
    }

    /// `map_remove(m, k)` — 需 `let mut`;命中置墓碑并 `len-1`。
    pub(super) fn compile_map_remove(&mut self, arguments: &[Expr]) -> Result<BasicValueEnum<'ctx>> {
        if arguments.len() != 2 {
            return Err(HuziError::new_global("map_remove() requires exactly 2 arguments (map, key)"));
        }
        let slot = self.map_first_slot("map_remove", &arguments[0])?;
        self.ensure_mutable(&arguments[0])?;
        let key = self.map_key_ptr(&arguments[1], "map_remove")?;
        let parts = self.load_vec_parts(&slot)?;
        let i32_t = self.context.i32_type();
        let is0 = self.builder.build_int_compare(inkwell::IntPredicate::EQ, parts.cap,
            i32_t.const_int(0, false), "rm_empty").unwrap();
        let f = self.current_function()?;
        let probe_bb = self.context.append_basic_block(f, "rm_probe");
        let done_bb = self.context.append_basic_block(f, "rm_done");
        let out = self.build_alloca(self.context.bool_type().into(), "rm_out")?;
        self.builder.build_store(out, self.context.bool_type().const_int(0, false)).unwrap();
        self.builder.build_conditional_branch(is0, done_bb, probe_bb).unwrap();
        self.builder.position_at_end(probe_bb);
        self.map_remove_hit(&slot, &parts, key, out, done_bb)?;
        self.builder.position_at_end(done_bb);
        Ok(self.builder.build_load(self.context.bool_type(), out, "rm_r").unwrap())
    }

    /// 命中则置墓碑、`len-1`、返回 true。
    fn map_remove_hit(
        &mut self,
        slot: &super::VarSlot<'ctx>,
        parts: &VecParts<'ctx>,
        key: PointerValue<'ctx>,
        out: PointerValue<'ctx>,
        done: inkwell::basic_block::BasicBlock<'ctx>,
    ) -> Result<()> {
        let i32_t = self.context.i32_type();
        let ety = self.map_entry_type();
        let e: inkwell::types::BasicTypeEnum<'ctx> = ety.into();
        let klen = self.str_len_of(key, "rm_klen")?;
        let hash = self.map_hash(key, klen)?;
        let idx = self.map_find_index(parts.data, parts.cap, hash, key)?;
        let miss = self.builder.build_int_compare(inkwell::IntPredicate::EQ, idx,
            i32_t.const_int((-1i32) as u64, false), "rm_miss").unwrap();
        let f = self.current_function()?;
        let hit_bb = self.context.append_basic_block(f, "rm_hit");
        self.builder.build_conditional_branch(miss, done, hit_bb).unwrap();
        self.builder.position_at_end(hit_bb);
        let ep = unsafe { self.builder.build_gep(e, parts.data, &[idx], "rm_ep").unwrap() };
        let sp = self.builder.build_struct_gep(ety, ep, 1, "rm_sp").unwrap();
        self.builder.build_store(sp, i32_t.const_int(2, false)).unwrap();
        let nl = self.builder.build_int_sub(parts.len, i32_t.const_int(1, false), "rm_nl").unwrap();
        self.store_vec_parts(slot, self.vec_struct_type(), &VecParts {
            data: parts.data, len: nl, cap: parts.cap,
        });
        self.builder.build_store(out, self.context.bool_type().const_int(1, false)).unwrap();
        self.builder.build_unconditional_branch(done).unwrap();
        Ok(())
    }

    /// `map_len(m)` — 已占条目数。
    pub(super) fn compile_map_len(&mut self, arguments: &[Expr]) -> Result<BasicValueEnum<'ctx>> {
        if arguments.len() != 1 {
            return Err(HuziError::new_global("map_len() requires exactly 1 argument (map)"));
        }
        let parts = self.resolve_map_parts("map_len", &arguments[0])?;
        Ok(parts.len.into())
    }
}
