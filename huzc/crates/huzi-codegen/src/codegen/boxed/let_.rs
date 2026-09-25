//! `let` 绑定 Box/null:标注校验、堆分配存槽、槽位注册。

use super::super::{box_nest::BoxNest, CodeGen, VarSlot};
use huzi_ast::*;
use huzi_error::{HuziError, Result};
use inkwell::types::BasicTypeEnum;
use inkwell::values::BasicValueEnum;

impl<'ctx> CodeGen<'ctx> {
    /// `let x[: Box<T>] = null` — 标注须为 Box(裸 `let x = null` 无法推导)。
    pub(in super::super) fn compile_let_null(&mut self, stmt: &LetStmt, span: Span) -> Result<()> {
        let ann = stmt.type_annotation.as_ref().ok_or_else(|| {
            HuziError::new_global(
                "Cannot infer the type of 'null'; add a `: Box<T>` annotation (e.g. `let x: Box<Node> = null`)",
            )
        })?;
        if !Self::is_box_or_weak_ast(ann) {
            return Err(HuziError::new_global(format!(
                "null can only be assigned to a Box<T> or weak Box<T> slot (found '{}')",
                ann
            )));
        }
        let is_weak = Self::is_weak_ast(ann);
        let ptr_ty = self.type_to_llvm(ann)?;
        let box_inner = self.box_nest_of_ast(ann)?;
        let alloca = self.build_box_alloca(ptr_ty, &stmt.name)?;
        let drop_kind = if is_weak {
            super::super::drop::DropKind::WeakBox
        } else {
            super::super::drop::DropKind::Box
        };
        self.register_droppable(alloca, ptr_ty, drop_kind);
        self.builder.build_store(alloca, ptr_ty.const_zero()).unwrap();
        self.scope_insert(
            stmt.name.clone(),
            VarSlot {
                ptr: alloca,
                ty: ptr_ty,
                elem: None,
                array_len: None,
                mutable: stmt.mutable,
                box_inner,
                map_kind: None,
            },
        );
        self.local_ast.insert(stmt.name.clone(), ann.clone());
        self.declare_local(&stmt.name, alloca, ptr_ty, span);
        Ok(())
    }

    /// `let x[: Box<T>] = box(inner)` — 有标注时校验 inner 与 T,无标注时推导。
    /// 嵌套标注(`Box<Box<Node>>`)记录层数 + 最内层,供后续逐层解引用。
    pub(in super::super) fn compile_let_box(&mut self, stmt: &LetStmt, inner: &Expr, span: Span) -> Result<()> {
        if let Some(ann) = &stmt.type_annotation {
            self.check_box_assignable(&Expr::BoxAlloc(Box::new(inner.clone())), ann)?;
            let (val, nest) = self.compile_box_alloc(inner, Some(ann))?;
            let ptr_ty = self.type_to_llvm(ann)?;
            self.insert_box_slot(&stmt.name, ptr_ty, val, nest, stmt.mutable, span)?;
            return Ok(());
        }
        let (val, nest) = self.compile_box_alloc(inner, None)?;
        let var_ty = val.get_type();
        self.insert_box_slot(&stmt.name, var_ty, val, nest, stmt.mutable, span)?;
        Ok(())
    }

    /// Box 变量槽插入(标注/推导路径共用):指针类型 + 嵌套描述。
    fn insert_box_slot(
        &mut self,
        name: &str,
        slot_ty: BasicTypeEnum<'ctx>,
        val: BasicValueEnum<'ctx>,
        nest: BoxNest<'ctx>,
        mutable: bool,
        span: Span,
    ) -> Result<()> {
        let alloca = self.build_box_alloca(slot_ty, name)?;
        self.register_droppable(alloca, slot_ty, super::super::drop::DropKind::Box);
        self.builder.build_store(alloca, val).unwrap();
        self.scope_insert(
            name.to_string(),
            VarSlot {
                ptr: alloca,
                ty: slot_ty,
                elem: None,
                array_len: None,
                mutable,
                box_inner: Some(nest),
                map_kind: None,
            },
        );
        self.declare_local(name, alloca, slot_ty, span);
        Ok(())
    }
}
