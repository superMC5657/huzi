//! `split`/`substring`:vec<str> 切分构造与字节区间拷贝(自 `builtins_string/mod.rs` 纯搬移,零逻辑变化)。

use super::super::{CodeGen, VarSlot};
use huzi_ast::*;
use huzi_error::{HuziError, Result};
use inkwell::AddressSpace;

impl<'ctx> CodeGen<'ctx> {
    /// AST 层面判断是否为 `split(...)` 构造(供 `let` 分发,不生成指令)。
    pub(in crate::codegen) fn is_split_ctor(call: &CallExpr) -> bool {
        matches!(&*call.callee, Expr::Ident(name) if name == "split")
    }

    /// `let parts = split(s, d)` — vec<str> 存槽,elem 标记 str 指针类型,
    /// 使下标/`len`/`print`/`for-in`/`push` 与 `vec(...)` 完全互通。
    pub(in crate::codegen) fn compile_let_split(
        &mut self,
        stmt: &LetStmt,
        arguments: &[Expr],
        span: Span,
    ) -> Result<()> {
        if stmt.type_annotation.is_some() {
            return Err(HuziError::new_global(
                "split() infers its vec<str> type from the arguments; remove the type annotation",
            ));
        }
        let vec_val = self.compile_split(arguments)?;
        let vec_ty = vec_val.get_type();
        let alloca = self.build_alloca(vec_ty, &stmt.name)?;
        self.register_droppable(alloca, vec_ty, crate::codegen::drop::DropKind::Vec);
        self.builder.build_store(alloca, vec_val).unwrap();
        let str_ty = self.context.ptr_type(AddressSpace::default()).into();
        self.scope_insert(
            stmt.name.clone(),
            VarSlot {
                ptr: alloca,
                ty: vec_ty,
                elem: Some(str_ty),
                array_len: None,
                mutable: stmt.mutable,
                box_inner: None,
                map_kind: None,
            },
        );
        self.declare_local(&stmt.name, alloca, vec_ty, span);
        Ok(())
    }

    /// `split(s, delim)` — 按单字符或多字符分隔符切分,返回 vec<str>;
    /// 空分隔符时整体作为唯一一段(均为堆拷贝,与 concat 同分配模式)。
    pub(in crate::codegen) fn compile_split(
        &mut self,
        arguments: &[Expr],
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        self.expect_arg_count("split", arguments, 2)?;
        let s = self.compile_str_arg(&arguments[0], "split")?;
        let d = self.compile_str_arg(&arguments[1], "split")?;
        let s_len = self.str_len_of(s, "split_slen")?;
        let d_len = self.str_len_of(d, "split_dlen")?;
        let i32_type = self.context.i32_type();
        let str_ty: inkwell::types::BasicTypeEnum<'ctx> =
            self.context.ptr_type(AddressSpace::default()).into();
        let function = self.current_function()?;
        let one_bb = self.context.append_basic_block(function, "split_one");
        let many_bb = self.context.append_basic_block(function, "split_many");
        let done_bb = self.context.append_basic_block(function, "split_done");
        let result = self.build_alloca(self.vec_struct_type().into(), "split_result")?;
        let empty = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                d_len,
                i32_type.const_int(0, false),
                "split_empty",
            )
            .unwrap();
        self.builder.build_conditional_branch(empty, one_bb, many_bb).unwrap();
        self.builder.position_at_end(one_bb);
        self.emit_split_single(s, s_len, str_ty, result, done_bb)?;
        self.builder.position_at_end(many_bb);
        self.emit_split_many(s, d, s_len, d_len, str_ty, result, done_bb)?;
        self.builder.position_at_end(done_bb);
        Ok(self.builder.build_load(self.vec_struct_type(), result, "split_val").unwrap())
    }

    // 空分隔符分支:整体拷贝为唯一一段。
    // 正常分支:先计数分配指针数组,再逐段拷贝填充。
    // (实现见 builtins_string_util.rs 的同名助手,保持本文件在 500 行内。)

    /// `substring(s, start, end)` — 字节区间 `[start, end)` 拷贝;
    /// 越界(含负数)报运行时错误。仍按字节语义,不做字符语义。
    pub(in crate::codegen) fn compile_substring(
        &mut self,
        arguments: &[Expr],
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        self.expect_arg_count("substring", arguments, 3)?;
        let s = self.compile_str_arg(&arguments[0], "substring")?;
        let start_expr = self.compile_expr(&arguments[1])?;
        let start = self.coerce_index(start_expr)?;
        let end_expr = self.compile_expr(&arguments[2])?;
        let end = self.coerce_index(end_expr)?;
        let s_len = self.str_len_of(s, "sub_slen")?;
        // 无符号比较:0 <= start <= end <= len,负数自然落入失败分支。
        let ok_lo = self
            .builder
            .build_int_compare(inkwell::IntPredicate::ULE, start, end, "sub_se")
            .unwrap();
        self.emit_runtime_check(ok_lo, "Runtime error: substring out of bounds\n\0", &[])?;
        let ok_hi = self
            .builder
            .build_int_compare(inkwell::IntPredicate::ULE, end, s_len, "sub_el")
            .unwrap();
        self.emit_runtime_check(ok_hi, "Runtime error: substring out of bounds\n\0", &[])?;
        Ok(self.str_copy_range(s, start, end)?.into())
    }
}
