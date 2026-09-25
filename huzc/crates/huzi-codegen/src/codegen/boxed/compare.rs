//! 取址解引用与 Box 比较:逐层自动解引用、`==`/`!=` 判空/比指针。

use super::BoxOperand;
use super::super::CodeGen;
use huzi_ast::*;
use huzi_error::{HuziError, Result};
use inkwell::types::BasicTypeEnum;
use inkwell::values::BasicValueEnum;

impl<'ctx> CodeGen<'ctx> {
    /// 取址后若基址是 Box 则自动解引用:按嵌套层数逐层装载堆指针,
    /// 最终类型切为最内层 pointee(`outer.val` 可穿透 `Box<Box<Node>>`)。
    pub(in super::super) fn compile_addr_deref(
        &mut self,
        expr: &Expr,
    ) -> Result<(
        inkwell::values::PointerValue<'ctx>,
        BasicTypeEnum<'ctx>,
    )> {
        let (mut ptr, slot_ty) = self.compile_addr(expr)?;
        let Some(nest) = self.box_nest_of_expr(expr) else {
            return Ok((ptr, slot_ty));
        };
        let mut load_ty = slot_ty;
        let ptr_ty: BasicTypeEnum<'ctx> = self
            .context
            .ptr_type(inkwell::AddressSpace::default())
            .into();
        for _ in 0..nest.depth {
            let loaded = self
                .builder
                .build_load(load_ty, ptr, "box_deref")
                .unwrap()
                .into_pointer_value();
            ptr = loaded;
            load_ty = ptr_ty;
        }
        if self.is_weak_expr(expr) {
            ptr = self.emit_load_weak(ptr)?;
        }
        Ok((ptr, nest.ultimate))
    }

    /// `==`/`!=` 的 Box 路径:Box vs null 判空,Box vs Box 比指针。
    /// 调用方已确认至少一侧是 Box 或 null。
    pub(in super::super) fn build_box_compare(
        &mut self,
        op: &BinOp,
        left: BoxOperand<'_, 'ctx>,
        right: BoxOperand<'_, 'ctx>,
    ) -> Result<BasicValueEnum<'ctx>> {
        if *op != BinOp::Eq && *op != BinOp::Neq {
            return Err(HuziError::new_global(
                "Box<T> values only support '==' and '!=' (use `x == null` to test for empty)",
            ));
        }
        let l_box = self.is_box_expr(left.expr);
        let r_box = self.is_box_expr(right.expr);
        let l_null = Self::is_null_expr(left.expr);
        let r_null = Self::is_null_expr(right.expr);
        if l_null && r_null {
            return Err(HuziError::new_global(
                "Cannot compare 'null' with 'null'; compare a Box<T> value with null instead",
            ));
        }
        if (l_null && !r_box) || (r_null && !l_box) {
            return Err(HuziError::new_global(
                "null can only be compared with a Box<T> value using '==' or '!='",
            ));
        }
        if (l_box && !r_box && !r_null) || (r_box && !l_box && !l_null) {
            return Err(HuziError::new_global(
                "Cannot compare a Box<T> value with a non-Box value; compare it with null or another Box instead",
            ));
        }
        let i64_ty = self.context.i64_type();
        let l_ptr = match left.value {
            BasicValueEnum::PointerValue(pv) => pv,
            _ => return Err(HuziError::new_global("Box value is not a pointer")),
        };
        let r_ptr = match right.value {
            BasicValueEnum::PointerValue(pv) => pv,
            _ => return Err(HuziError::new_global("Box value is not a pointer")),
        };
        let l_int = self.builder.build_ptr_to_int(l_ptr, i64_ty, "box_l").unwrap();
        let r_int = self.builder.build_ptr_to_int(r_ptr, i64_ty, "box_r").unwrap();
        let pred = if *op == BinOp::Eq {
            inkwell::IntPredicate::EQ
        } else {
            inkwell::IntPredicate::NE
        };
        Ok(self
            .builder
            .build_int_compare(pred, l_int, r_int, "box_cmp")
            .unwrap()
            .into())
    }

    /// 是否走 Box 比较路径(任一侧是 Box 或 null)。
    pub(in super::super) fn is_box_comparison(&self, left: &Expr, right: &Expr) -> bool {
        self.is_box_expr(left) || self.is_box_expr(right) || Self::is_null_expr(left) || Self::is_null_expr(right)
    }
}
