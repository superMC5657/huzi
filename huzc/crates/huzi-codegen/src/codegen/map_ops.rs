//! HashMap 操作:`map_put/get/has/remove/len`(种类感知)。
//!
//! 与 `map.rs`(构造/探测)与 `map_rehash.rs`(扩容/重哈希)同属 map 模块:
//! 本文件只放五个操作入口与命中/缺失分支,条目布局与键/值校验由
//! `MapKind` 决定。写操作仅支持变量槽,读操作支持变量与结构体字段。

use super::vec::VecParts;
use super::{CodeGen, MapKind};
use huzi_ast::*;
use huzi_error::{HuziError, Result};
use inkwell::values::{BasicValueEnum, IntValue, PointerValue};

impl<'ctx> CodeGen<'ctx> {
    /// `map_put(m, k, v)` — 需 `let mut`;存在则覆盖,不存在则插入。
    pub(super) fn compile_map_put(&mut self, arguments: &[Expr]) -> Result<BasicValueEnum<'ctx>> {
        if arguments.len() != 3 {
            return Err(HuziError::new_global("map_put() requires exactly 3 arguments (map, key, value)"));
        }
        let slot = self.map_first_slot("map_put", &arguments[0])?;
        self.ensure_mutable(&arguments[0])?;
        let kind = slot.map_kind.unwrap_or(MapKind::StrI32);
        let key = self.compile_map_key(&arguments[1], kind, "map_put")?;
        let val = self.compile_map_val(&arguments[2], kind, "map_put")?;
        let parts = self.load_vec_parts(&slot)?;
        let grown = self.map_ensure_capacity(&slot, &parts, kind)?;
        let (klen, hash) = self.map_key_hash(key, kind)?;
        let found = self.map_find_index_typed(grown.data, grown.cap, hash, key, kind)?;
        self.emit_put_branch(&slot, &grown, hash, key, klen, val, kind, found)
    }

    /// 由键求 `(klen, hash)`:`str` 键需长度,`i32` 键 klen 置零。
    fn map_key_hash(
        &mut self,
        key: BasicValueEnum<'ctx>,
        kind: MapKind,
    ) -> Result<(IntValue<'ctx>, IntValue<'ctx>)> {
        if kind.key_is_str() {
            let ptr = match key {
                BasicValueEnum::PointerValue(p) => p,
                _ => return Err(HuziError::new_global("Map str key must be a pointer")),
            };
            let klen = self.str_len_of(ptr, "put_klen")?;
            let hash = self.map_hash(ptr, klen)?;
            return Ok((klen, hash));
        }
        let zero = self.context.i32_type().const_int(0, false);
        let hash = self.map_hash_typed(key, kind)?;
        Ok((zero, hash))
    }

    /// put 的命中/缺失分支。
    #[allow(clippy::too_many_arguments)]
    fn emit_put_branch(
        &mut self,
        slot: &super::VarSlot<'ctx>,
        grown: &VecParts<'ctx>,
        hash: IntValue<'ctx>,
        key: BasicValueEnum<'ctx>,
        klen: IntValue<'ctx>,
        val: BasicValueEnum<'ctx>,
        kind: MapKind,
        found: IntValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>> {
        let f = self.current_function()?;
        let hit_bb = self.context.append_basic_block(f, "put_hit");
        let miss_bb = self.context.append_basic_block(f, "put_miss");
        let done_bb = self.context.append_basic_block(f, "put_done");
        let absent = self.builder.build_int_compare(inkwell::IntPredicate::EQ, found,
            self.context.i32_type().const_int((-1i32) as u64, false), "put_abs").unwrap();
        self.builder.build_conditional_branch(absent, miss_bb, hit_bb).unwrap();
        self.builder.position_at_end(hit_bb);
        self.map_put_update(grown.data, found, val, kind, done_bb)?;
        self.builder.position_at_end(miss_bb);
        self.map_put_insert(slot, grown, hash, key, klen, val, kind, done_bb)?;
        self.builder.position_at_end(done_bb);
        Ok(self.context.i32_type().const_int(0, false).into())
    }

    /// 命中路径:原位覆盖 `val` 字段。
    fn map_put_update(
        &mut self,
        data: PointerValue<'ctx>,
        found: IntValue<'ctx>,
        val: BasicValueEnum<'ctx>,
        kind: MapKind,
        done: inkwell::basic_block::BasicBlock<'ctx>,
    ) -> Result<()> {
        let ety = self.map_entry_type_of(kind);
        let e: inkwell::types::BasicTypeEnum<'ctx> = ety.into();
        let ep = unsafe { self.builder.build_gep(e, data, &[found], "put_ep").unwrap() };
        let vp = self.builder.build_struct_gep(ety, ep, 4, "put_vp").unwrap();
        self.builder.build_store(vp, val).unwrap();
        self.builder.build_unconditional_branch(done).unwrap();
        Ok(())
    }

    /// 缺失路径:找空/墓碑位写入并 `len+1`。
    #[allow(clippy::too_many_arguments)]
    fn map_put_insert(
        &mut self,
        slot: &super::VarSlot<'ctx>,
        grown: &VecParts<'ctx>,
        hash: IntValue<'ctx>,
        key: BasicValueEnum<'ctx>,
        klen: IntValue<'ctx>,
        val: BasicValueEnum<'ctx>,
        kind: MapKind,
        done: inkwell::basic_block::BasicBlock<'ctx>,
    ) -> Result<()> {
        let i32_t = self.context.i32_type();
        let ety = self.map_entry_type_of(kind);
        let free_idx = self.map_free_index_typed(grown.data, grown.cap, hash, kind)?;
        let idx_slot = self.build_alloca(i32_t.into(), "put_free")?;
        self.builder.build_store(idx_slot, free_idx).unwrap();
        self.map_store_new_entry(grown.data, ety, idx_slot, hash, key, klen, val, kind)?;
        let nlen = self.builder.build_int_add(grown.len, i32_t.const_int(1, false), "put_nlen").unwrap();
        self.store_vec_parts(slot, self.vec_struct_type(), &VecParts {
            data: grown.data, len: nlen, cap: grown.cap,
        });
        self.builder.build_unconditional_branch(done).unwrap();
        Ok(())
    }

    /// 组装 `(found: bool, val)` 元组值(值类型按种类)。
    fn map_found_tuple(
        &mut self,
        found: IntValue<'ctx>,
        val: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>> {
        let tup = self.context.struct_type(&[self.context.bool_type().into(), val.get_type()], false);
        let tmp = self.build_alloca(tup.into(), "get_tup")?;
        let f0 = self.builder.build_struct_gep(tup, tmp, 0, "get_f0").unwrap();
        self.builder.build_store(f0, found).unwrap();
        let f1 = self.builder.build_struct_gep(tup, tmp, 1, "get_f1").unwrap();
        self.builder.build_store(f1, val).unwrap();
        Ok(self.builder.build_load(tup, tmp, "get_tv").unwrap())
    }

    /// `map_get(m, k)` — 缺键返回 `(false, 零值)` 不 abort。
    pub(super) fn compile_map_get(&mut self, arguments: &[Expr]) -> Result<BasicValueEnum<'ctx>> {
        if arguments.len() != 2 {
            return Err(HuziError::new_global("map_get() requires exactly 2 arguments (map, key)"));
        }
        let (parts, kind) = self.resolve_map_parts_typed("map_get", &arguments[0])?;
        let key = self.compile_map_key(&arguments[1], kind, "map_get")?;
        let f = self.current_function()?;
        let probe_bb = self.context.append_basic_block(f, "get_probe");
        let done_bb = self.context.append_basic_block(f, "get_done");
        let found_s = self.build_alloca(self.context.bool_type().into(), "get_found")?;
        let val_ty = self.map_val_llvm(kind);
        let val_s = self.build_alloca(val_ty, "get_val")?;
        self.builder.build_store(found_s, self.context.bool_type().const_int(0, false)).unwrap();
        self.builder.build_store(val_s, val_ty.const_zero()).unwrap();
        let empty = self.builder.build_int_compare(inkwell::IntPredicate::EQ, parts.cap,
            self.context.i32_type().const_int(0, false), "get_empty").unwrap();
        self.builder.build_conditional_branch(empty, done_bb, probe_bb).unwrap();
        self.builder.position_at_end(probe_bb);
        self.map_get_fill(parts.data, parts.cap, key, kind, found_s, val_s, done_bb)?;
        self.builder.position_at_end(done_bb);
        let fb = self.builder.build_load(self.context.bool_type(), found_s, "get_fb").unwrap().into_int_value();
        let vb = self.builder.build_load(val_ty, val_s, "get_vb").unwrap();
        self.map_found_tuple(fb, vb)
    }

    /// 探测命中时回填 `(true, val)`。
    #[allow(clippy::too_many_arguments)]
    fn map_get_fill(
        &mut self,
        data: PointerValue<'ctx>,
        cap: IntValue<'ctx>,
        key: BasicValueEnum<'ctx>,
        kind: MapKind,
        found_s: PointerValue<'ctx>,
        val_s: PointerValue<'ctx>,
        done: inkwell::basic_block::BasicBlock<'ctx>,
    ) -> Result<()> {
        let i32_t = self.context.i32_type();
        let ety = self.map_entry_type_of(kind);
        let e: inkwell::types::BasicTypeEnum<'ctx> = ety.into();
        let hash = self.map_hash_typed(key, kind)?;
        let idx = self.map_find_index_typed(data, cap, hash, key, kind)?;
        let hit = self.builder.build_int_compare(inkwell::IntPredicate::NE, idx,
            i32_t.const_int((-1i32) as u64, false), "get_hit").unwrap();
        let f = self.current_function()?;
        let hit_bb = self.context.append_basic_block(f, "get_hit");
        self.builder.build_conditional_branch(hit, hit_bb, done).unwrap();
        self.builder.position_at_end(hit_bb);
        let vv = self.map_load_val(ety, data, e, idx, kind);
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
        let (parts, kind) = self.resolve_map_parts_typed("map_has", &arguments[0])?;
        let key = self.compile_map_key(&arguments[1], kind, "map_has")?;
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
        let hash = self.map_hash_typed(key, kind)?;
        let idx = self.map_find_index_typed(parts.data, parts.cap, hash, key, kind)?;
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
        let kind = slot.map_kind.unwrap_or(MapKind::StrI32);
        let key = self.compile_map_key(&arguments[1], kind, "map_remove")?;
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
        self.map_remove_hit(&slot, &parts, key, kind, out, done_bb)?;
        self.builder.position_at_end(done_bb);
        Ok(self.builder.build_load(self.context.bool_type(), out, "rm_r").unwrap())
    }

    /// 命中则置墓碑、`len-1`、返回 true。
    fn map_remove_hit(
        &mut self,
        slot: &super::VarSlot<'ctx>,
        parts: &VecParts<'ctx>,
        key: BasicValueEnum<'ctx>,
        kind: MapKind,
        out: PointerValue<'ctx>,
        done: inkwell::basic_block::BasicBlock<'ctx>,
    ) -> Result<()> {
        let i32_t = self.context.i32_type();
        let ety = self.map_entry_type_of(kind);
        let e: inkwell::types::BasicTypeEnum<'ctx> = ety.into();
        let hash = self.map_hash_typed(key, kind)?;
        let idx = self.map_find_index_typed(parts.data, parts.cap, hash, key, kind)?;
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
        let (parts, _) = self.resolve_map_parts_typed("map_len", &arguments[0])?;
        Ok(parts.len.into())
    }
}
