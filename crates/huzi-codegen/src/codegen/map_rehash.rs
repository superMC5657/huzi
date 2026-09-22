//! HashMap 扩容与重哈希(种类感知):清零、新表分配、旧条目重插。
//!
//! 与 `map.rs`(构造/探测)与 `map_ops.rs`(put/get/has/remove/len)同属 map 模块:
//! 本文件只放容量路径,保持各文件 500 行以内。条目布局与键/值类型由
//! `MapKind` 决定,槽布局复用 `vec.rs` 的 `{ data, len, cap }`。

use super::map::MAP_MARK;
use super::vec::VecParts;
use super::{CodeGen, MapKind, VarSlot};
use huzi_error::{HuziError, Result};
use inkwell::values::{BasicValueEnum, IntValue, PointerValue};

impl<'ctx> CodeGen<'ctx> {
    /// 新表条目清零(键/值零值按种类取空指针或 0)。
    pub(super) fn map_zero_entries(
        &mut self,
        data: PointerValue<'ctx>,
        cap: IntValue<'ctx>,
        kind: MapKind,
    ) -> Result<()> {
        let i32_t = self.context.i32_type();
        let ety = self.map_entry_type_of(kind);
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
        self.map_zero_one(data, e, ety, cur, kind)?;
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
        kind: MapKind,
    ) -> Result<()> {
        let i32_t = self.context.i32_type();
        let null_ptr: BasicValueEnum<'ctx> = self
            .context
            .ptr_type(inkwell::AddressSpace::default())
            .const_null()
            .into();
        let zero_i32: BasicValueEnum<'ctx> = i32_t.const_int(0, false).into();
        let key_zero = if kind.key_is_str() { null_ptr } else { zero_i32 };
        let val_zero = if kind.val_is_str() { null_ptr } else { zero_i32 };
        let iv = self.builder.build_load(i32_t, cur, "zero_cv").unwrap().into_int_value();
        let ep = unsafe { self.builder.build_gep(e, data, &[iv], "zero_ep").unwrap() };
        let vals: [BasicValueEnum<'ctx>; 5] = [
            zero_i32,
            zero_i32,
            key_zero,
            zero_i32,
            val_zero,
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
        key: BasicValueEnum<'ctx>,
        klen: IntValue<'ctx>,
        val: BasicValueEnum<'ctx>,
        kind: MapKind,
    ) -> Result<()> {
        let i32_t = self.context.i32_type();
        let ety = self.map_entry_type_of(kind);
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
        self.emit_rehash_probe(new_data, e, ety, idx, new_cap, loop_bb, body_bb, done_bb)?;
        self.builder.position_at_end(done_bb);
        self.map_store_new_entry(new_data, ety, idx, hash, key, klen, val, kind)?;
        Ok(())
    }

    /// 重哈希探测单步:空桶停,已占继续下一桶。
    #[allow(clippy::too_many_arguments)]
    fn emit_rehash_probe(
        &mut self,
        new_data: PointerValue<'ctx>,
        e: inkwell::types::BasicTypeEnum<'ctx>,
        ety: inkwell::types::StructType<'ctx>,
        idx: PointerValue<'ctx>,
        new_cap: IntValue<'ctx>,
        loop_bb: inkwell::basic_block::BasicBlock<'ctx>,
        body_bb: inkwell::basic_block::BasicBlock<'ctx>,
        done_bb: inkwell::basic_block::BasicBlock<'ctx>,
    ) -> Result<()> {
        let i32_t = self.context.i32_type();
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
        Ok(())
    }

    /// 旧表已占条目逐条重插新表。
    pub(super) fn map_rehash(
        &mut self,
        old_data: PointerValue<'ctx>,
        old_cap: IntValue<'ctx>,
        new_data: PointerValue<'ctx>,
        new_cap: IntValue<'ctx>,
        kind: MapKind,
    ) -> Result<()> {
        let i32_t = self.context.i32_type();
        let ety = self.map_entry_type_of(kind);
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
        self.emit_rehash_filter(old_data, e, ety, cur, copy_bb, incr_bb)?;
        self.builder.position_at_end(copy_bb);
        self.map_rehash_copy(old_data, e, ety, cur, new_data, new_cap, kind, incr_bb)?;
        self.builder.position_at_end(incr_bb);
        let jv = self.builder.build_load(i32_t, cur, "reh_iv").unwrap().into_int_value();
        let nx = self.builder.build_int_add(jv, i32_t.const_int(1, false), "reh_nx").unwrap();
        self.builder.build_store(cur, nx).unwrap();
        self.builder.build_unconditional_branch(cond_bb).unwrap();
        self.builder.position_at_end(done_bb);
        Ok(())
    }

    /// 重哈希过滤:已占进拷贝分支,否则直接下一条。
    fn emit_rehash_filter(
        &mut self,
        old_data: PointerValue<'ctx>,
        e: inkwell::types::BasicTypeEnum<'ctx>,
        ety: inkwell::types::StructType<'ctx>,
        cur: PointerValue<'ctx>,
        copy_bb: inkwell::basic_block::BasicBlock<'ctx>,
        incr_bb: inkwell::basic_block::BasicBlock<'ctx>,
    ) -> Result<()> {
        let i32_t = self.context.i32_type();
        let iv = self.builder.build_load(i32_t, cur, "reh_iv").unwrap().into_int_value();
        let ep = unsafe { self.builder.build_gep(e, old_data, &[iv], "reh_ep").unwrap() };
        let sp = self.builder.build_struct_gep(ety, ep, 1, "reh_sp").unwrap();
        let sv = self.builder.build_load(i32_t, sp, "reh_sv").unwrap().into_int_value();
        let occ = self.builder.build_int_compare(inkwell::IntPredicate::EQ, sv,
            i32_t.const_int(1, false), "reh_occ").unwrap();
        self.builder.build_conditional_branch(occ, copy_bb, incr_bb).unwrap();
        Ok(())
    }

    /// 载入旧条目五字段并插入新表,随后回到 `next`。
    #[allow(clippy::too_many_arguments)]
    fn map_rehash_copy(
        &mut self,
        old_data: PointerValue<'ctx>,
        e: inkwell::types::BasicTypeEnum<'ctx>,
        ety: inkwell::types::StructType<'ctx>,
        cur: PointerValue<'ctx>,
        new_data: PointerValue<'ctx>,
        new_cap: IntValue<'ctx>,
        kind: MapKind,
        next: inkwell::basic_block::BasicBlock<'ctx>,
    ) -> Result<()> {
        let i32_t = self.context.i32_type();
        let iv = self.builder.build_load(i32_t, cur, "reh_iv").unwrap().into_int_value();
        let ep = unsafe { self.builder.build_gep(e, old_data, &[iv], "reh_ep").unwrap() };
        let hv = self.load_entry_field(ety, ep, 0, i32_t.into(), "reh_hv")?.into_int_value();
        let kv = self.load_entry_field(ety, ep, 2, self.map_key_llvm(kind), "reh_kv")?;
        let lv = self.load_entry_field(ety, ep, 3, i32_t.into(), "reh_lv")?.into_int_value();
        let vv = self.load_entry_field(ety, ep, 4, self.map_val_llvm(kind), "reh_vv")?;
        self.map_rehash_insert(new_data, new_cap, hv, kv, lv, vv, kind)?;
        self.builder.build_unconditional_branch(next).unwrap();
        Ok(())
    }

    /// 读条目指定字段(供重哈希拷贝复用)。
    fn load_entry_field(
        &self,
        ety: inkwell::types::StructType<'ctx>,
        ep: PointerValue<'ctx>,
        idx: u32,
        ty: inkwell::types::BasicTypeEnum<'ctx>,
        name: &str,
    ) -> Result<BasicValueEnum<'ctx>> {
        let fp = self.builder.build_struct_gep(ety, ep, idx, name).unwrap();
        Ok(self.builder.build_load(ty, fp, name).unwrap())
    }

    /// 按负载 `(len+1)*10 > cap*7` 翻倍(空表首扩 8),重哈希后返回新 parts。
    pub(super) fn map_ensure_capacity(
        &mut self,
        slot: &VarSlot<'ctx>,
        parts: &VecParts<'ctx>,
        kind: MapKind,
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
        self.map_grow(slot, parts, vec_ty, kind)?;
        self.builder.build_unconditional_branch(done_bb).unwrap();
        self.builder.position_at_end(done_bb);
        self.load_vec_parts(slot)
    }

    /// 分配新表、清零、重哈希、回写槽。
    fn map_grow(
        &mut self,
        slot: &VarSlot<'ctx>,
        parts: &VecParts<'ctx>,
        vec_ty: inkwell::types::StructType<'ctx>,
        kind: MapKind,
    ) -> Result<()> {
        let i32_t = self.context.i32_type();
        let ety = self.map_entry_type_of(kind);
        let dbl = self.builder.build_int_mul(parts.cap, i32_t.const_int(2, false), "map_dbl").unwrap();
        let is0 = self.builder.build_int_compare(inkwell::IntPredicate::EQ, parts.cap,
            i32_t.const_int(0, false), "map_is0").unwrap();
        let ncap = self.builder.build_select(is0, i32_t.const_int(8, false),
            dbl, "map_ncap").unwrap().into_int_value();
        let et: inkwell::types::BasicTypeEnum<'ctx> = ety.into();
        let ndata = self.builder.build_array_malloc(et, ncap, "map_ndata")
            .map_err(|_| HuziError::new_global("Failed to allocate HashMap storage"))?;
        self.map_zero_entries(ndata, ncap, kind)?;
        self.map_rehash(parts.data, parts.cap, ndata, ncap, kind)?;
        self.store_vec_parts(slot, vec_ty, &VecParts { data: ndata, len: parts.len, cap: ncap });
        // 保留哨兵常量引用,避免重构后死代码告警。
        let _ = MAP_MARK;
        Ok(())
    }

    /// 在 `idx` 处写入新条目 `{ hash, 1, key, klen, val }`。
    #[allow(clippy::too_many_arguments)]
    pub(super) fn map_store_new_entry(
        &mut self,
        data: PointerValue<'ctx>,
        ety: inkwell::types::StructType<'ctx>,
        idx_slot: PointerValue<'ctx>,
        hash: IntValue<'ctx>,
        key: BasicValueEnum<'ctx>,
        klen: IntValue<'ctx>,
        val: BasicValueEnum<'ctx>,
        kind: MapKind,
    ) -> Result<()> {
        let i32_t = self.context.i32_type();
        let e: inkwell::types::BasicTypeEnum<'ctx> = ety.into();
        let iv = self.builder.build_load(i32_t, idx_slot, "put_iv").unwrap().into_int_value();
        let ep = unsafe { self.builder.build_gep(e, data, &[iv], "put_ep").unwrap() };
        // `i32` 键的 klen 槽填 0,保持五字段布局统一。
        let klen_val: BasicValueEnum<'ctx> = if kind.key_is_str() {
            klen.into()
        } else {
            i32_t.const_int(0, false).into()
        };
        let parts: [(u32, BasicValueEnum<'ctx>); 5] = [
            (0, hash.into()), (1, i32_t.const_int(1, false).into()),
            (2, key), (3, klen_val), (4, val),
        ];
        for (k, v) in parts {
            let fp = self.builder.build_struct_gep(ety, ep, k, "put_fp").unwrap();
            self.builder.build_store(fp, v).unwrap();
        }
        Ok(())
    }
}
