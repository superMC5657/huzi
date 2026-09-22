//! HashMap 键提取:`map_keys(m) -> vec<K>`(K 按种类为 `str` 或 `i32`)。

use super::{CodeGen, MapKind};
use super::VarSlot;
use huzi_ast::*;
use huzi_error::{HuziError, Result};
use inkwell::types::BasicTypeEnum;
use inkwell::values::{BasicValueEnum, IntValue, PointerValue};

impl<'ctx> CodeGen<'ctx> {
    /// 是否为 `map_keys(...)` 调用(供 `let` 分发)。
    pub(super) fn is_map_keys_ctor(call: &CallExpr) -> bool {
        matches!(&*call.callee, Expr::Ident(name) if name == "map_keys")
    }

    /// `let keys = map_keys(m)` — 元素类型随键种类,存槽 elem 标记。
    pub(super) fn compile_let_map_keys(
        &mut self,
        stmt: &LetStmt,
        arguments: &[Expr],
        span: Span,
    ) -> Result<()> {
        if stmt.type_annotation.is_some() {
            return Err(HuziError::new_global(
                "map_keys() infers its vec<K> type from the arguments; remove the `let` type annotation",
            ));
        }
        let (vec_val, key_ty) = self.compile_map_keys_typed(arguments)?;
        let vec_ty = vec_val.get_type();
        let alloca = self.build_alloca(vec_ty, &stmt.name)?;
        self.builder.build_store(alloca, vec_val).unwrap();
        self.scope_insert(
            stmt.name.clone(),
            VarSlot {
                ptr: alloca,
                ty: vec_ty,
                elem: Some(key_ty),
                array_len: None,
                mutable: stmt.mutable,
                box_inner: None,
                map_kind: None,
            },
        );
        self.declare_local(&stmt.name, alloca, vec_ty, span);
        Ok(())
    }

    /// `map_keys(m)` 旧入口(供表达式位置复用,键类型由种类决定)。
    pub(super) fn compile_map_keys(
        &mut self,
        arguments: &[Expr],
    ) -> Result<BasicValueEnum<'ctx>> {
        let (val, _) = self.compile_map_keys_typed(arguments)?;
        Ok(val)
    }

    /// `map_keys(m)` 类型化实现,返回 `(vec 值, 键 LLVM 类型)`。
    fn compile_map_keys_typed(
        &mut self,
        arguments: &[Expr],
    ) -> Result<(BasicValueEnum<'ctx>, BasicTypeEnum<'ctx>)> {
        if arguments.len() != 1 {
            return Err(HuziError::new_global(
                "map_keys() requires exactly 1 argument (map)",
            ));
        }
        let (parts, kind) = self.resolve_map_parts_typed("map_keys", &arguments[0])?;
        let key_ty = self.map_key_llvm(kind);
        let i32_t = self.context.i32_type();

        let count = parts.len;
        let function = self.current_function()?;
        let empty_bb = self.context.append_basic_block(function, "mk_empty");
        let fill_bb = self.context.append_basic_block(function, "mk_fill");
        let done_bb = self.context.append_basic_block(function, "mk_done");

        let res_vec_ptr = self.build_alloca(self.vec_struct_type().into(), "mk_res")?;
        let is_zero = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                count,
                i32_t.const_int(0, false),
                "mk_is_zero",
            )
            .unwrap();
        self.builder
            .build_conditional_branch(is_zero, empty_bb, fill_bb)
            .unwrap();

        self.builder.position_at_end(empty_bb);
        let empty_val = self.vec_assemble_empty(key_ty)?;
        self.builder.build_store(res_vec_ptr, empty_val).unwrap();
        self.builder.build_unconditional_branch(done_bb).unwrap();

        self.builder.position_at_end(fill_bb);
        let data = self
            .builder
            .build_array_malloc(key_ty, count, "mk_data")
            .map_err(|_| HuziError::new_global("Failed to allocate map_keys storage"))?;

        self.emit_map_keys_loop(parts.data, parts.cap, data, count, kind, done_bb, res_vec_ptr)?;

        self.builder.position_at_end(done_bb);
        let ret = self
            .builder
            .build_load(self.vec_struct_type(), res_vec_ptr, "mk_ret")
            .unwrap();
        Ok((ret, key_ty))
    }

    /// 遍历哈希表桶数组,复制 active(state==1) 键到新数组(类型按种类)。
    fn emit_map_keys_loop(
        &mut self,
        map_data: PointerValue<'ctx>,
        cap: IntValue<'ctx>,
        out_data: PointerValue<'ctx>,
        count: IntValue<'ctx>,
        kind: MapKind,
        done_bb: inkwell::basic_block::BasicBlock<'ctx>,
        res_vec_ptr: PointerValue<'ctx>,
    ) -> Result<()> {
        let key_ty = self.map_key_llvm(kind);
        let i32_t = self.context.i32_type();
        let ety = self.map_entry_type_of(kind);
        let e: BasicTypeEnum<'ctx> = ety.into();
        let function = self.current_function()?;

        let loop_bb = self.context.append_basic_block(function, "mk_loop");
        let body_bb = self.context.append_basic_block(function, "mk_body");
        let push_bb = self.context.append_basic_block(function, "mk_push");
        let next_bb = self.context.append_basic_block(function, "mk_next");
        let finish_bb = self.context.append_basic_block(function, "mk_finish");

        let cur_i = self.build_alloca(i32_t.into(), "mk_i")?;
        let out_idx = self.build_alloca(i32_t.into(), "mk_out_idx")?;
        self.builder.build_store(cur_i, i32_t.const_int(0, false)).unwrap();
        self.builder.build_store(out_idx, i32_t.const_int(0, false)).unwrap();

        self.builder.build_unconditional_branch(loop_bb).unwrap();

        self.builder.position_at_end(loop_bb);
        let iv = self.builder.build_load(i32_t, cur_i, "mk_iv").unwrap().into_int_value();
        let go = self.builder.build_int_compare(inkwell::IntPredicate::ULT, iv, cap, "mk_go").unwrap();
        self.builder.build_conditional_branch(go, body_bb, finish_bb).unwrap();

        self.builder.position_at_end(body_bb);
        let st = self.map_entry_state(ety, map_data, e, iv);
        let is_occ = self.builder.build_int_compare(
            inkwell::IntPredicate::EQ,
            st,
            i32_t.const_int(1, false),
            "mk_occ",
        ).unwrap();
        self.builder.build_conditional_branch(is_occ, push_bb, next_bb).unwrap();

        self.builder.position_at_end(push_bb);
        let key_val = self.map_load_key(ety, map_data, e, iv, kind);

        let oi = self.builder.build_load(i32_t, out_idx, "mk_oiv").unwrap().into_int_value();
        let out_slot = unsafe { self.builder.build_gep(key_ty, out_data, &[oi], "mk_slot").unwrap() };
        self.builder.build_store(out_slot, key_val).unwrap();
        let next_oi = self.builder.build_int_add(oi, i32_t.const_int(1, false), "mk_noi").unwrap();
        self.builder.build_store(out_idx, next_oi).unwrap();
        self.builder.build_unconditional_branch(next_bb).unwrap();

        self.builder.position_at_end(next_bb);
        let next_iv = self.builder.build_int_add(iv, i32_t.const_int(1, false), "mk_niv").unwrap();
        self.builder.build_store(cur_i, next_iv).unwrap();
        self.builder.build_unconditional_branch(loop_bb).unwrap();

        self.builder.position_at_end(finish_bb);
        let vec_val = self.assemble_str_vec(out_data, count)?;
        self.builder.build_store(res_vec_ptr, vec_val).unwrap();
        self.builder.build_unconditional_branch(done_bb).unwrap();
        Ok(())
    }
}
