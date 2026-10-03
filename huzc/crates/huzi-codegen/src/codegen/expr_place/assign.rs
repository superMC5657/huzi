//! 赋值分发:`=`/`+=` 等复合赋值、标识符/字段/解引用目标。(自 `expr_place.rs` 纯搬移,零逻辑变化)。

use super::super::CodeGen;
use huzi_ast::*;
use huzi_error::{HuziError, Result};

impl<'ctx> CodeGen<'ctx> {
    fn compile_assign_ident(
        &mut self,
        name: &str,
        expr: &AssignExpr,
        value: inkwell::values::BasicValueEnum<'ctx>,
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        let slot = self
            .scope_lookup(name)
            .ok_or_else(|| self.unknown_variable_error(name))?;

        if !slot.mutable {
            return Err(HuziError::new_global(format!(
                "Cannot assign to immutable variable '{}'; declare it with `let mut`",
                name
            )));
        }
        // 门卫:按值含堆结构体变量禁整体赋值,引导改 `Box` 或拆参。
        if let Some(ty) = self.local_ast.get(name).cloned() {
            self.reject_heap_struct_byvalue(&ty, "按值赋值")?;
        }

        let is_weak = self.is_weak_var(name);
        let is_box = Self::is_box_slot(&slot);

        if Self::is_null_expr(&expr.value) && !is_box && !is_weak {
            return Err(HuziError::new_global(format!(
                "null can only be assigned to a Box<T> or weak Box<T> slot (variable '{}' is not a Box)",
                name
            )));
        }
        if matches!(&*expr.value, Expr::BoxAlloc(_)) && !is_box && !is_weak {
            return Err(HuziError::new_global(format!(
                "Cannot assign a Box value to non-Box variable '{}'",
                name
            )));
        }

        if is_weak {
            let old_ptr = self
                .builder
                .build_load(slot.ty, slot.ptr, "rc_old_weak")
                .unwrap()
                .into_pointer_value();
            if !matches!(&*expr.value, Expr::Null) && value.is_pointer_value() {
                self.emit_retain_weak(value.into_pointer_value())?;
            }
            self.emit_release_weak(old_ptr)?;
        } else if is_box {
            let old_ptr = self
                .builder
                .build_load(slot.ty, slot.ptr, "rc_old")
                .unwrap()
                .into_pointer_value();
            if !matches!(&*expr.value, Expr::BoxAlloc(_) | Expr::Call(_) | Expr::Null)
                && value.is_pointer_value() {
                    self.emit_retain_box(value.into_pointer_value())?;
                }
            self.emit_release_box(old_ptr)?;
        }

        let value = self.coerce_value(slot.ty, value)?;
        self.builder.build_store(slot.ptr, value).unwrap();
        Ok(value)
    }

    fn compile_assign_field(
        &mut self,
        fa: &FieldAccessExpr,
        expr: &AssignExpr,
        value: inkwell::values::BasicValueEnum<'ctx>,
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        self.ensure_mutable(&expr.target)?;
        let mut is_box_field = false;
        let mut is_weak_field = false;
        if let Some(expected) = self.field_ast_type(&fa.base, &fa.field) {
            self.check_box_assignable(&expr.value, &expected)?;
            // 门卫:含堆结构体字段禁按值整体赋值。
            self.reject_heap_struct_byvalue(&expected, "按值赋值")?;
            is_box_field = Self::is_box_ast(&expected);
            is_weak_field = Self::is_weak_ast(&expected);
        }
        let (field_ptr, field_ty) = self.compile_addr(&expr.target)?;
        if is_weak_field {
            let old_ptr = self
                .builder
                .build_load(field_ty, field_ptr, "rc_old_weak_field")
                .unwrap()
                .into_pointer_value();
            if !matches!(&*expr.value, Expr::Null) && value.is_pointer_value() {
                self.emit_retain_weak(value.into_pointer_value())?;
            }
            self.emit_release_weak(old_ptr)?;
        } else if is_box_field {
            let old_ptr = self
                .builder
                .build_load(field_ty, field_ptr, "rc_old_field")
                .unwrap()
                .into_pointer_value();
            if !matches!(&*expr.value, Expr::BoxAlloc(_) | Expr::Call(_) | Expr::Null)
                && value.is_pointer_value() {
                    self.emit_retain_box(value.into_pointer_value())?;
                }
            self.emit_release_box(old_ptr)?;
        }
        let value = self.coerce_value(field_ty, value)?;
        self.builder.build_store(field_ptr, value).unwrap();
        Ok(value)
    }

    fn compile_compound_assign(
        &mut self,
        expr: &AssignExpr,
        bin_op: BinOp,
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        let (ptr, ty) = match &*expr.target {
            Expr::Ident(name) => {
                let slot = self
                    .scope_lookup(name)
                    .ok_or_else(|| self.unknown_variable_error(name))?;
                if !slot.mutable {
                    return Err(HuziError::new_global(format!(
                        "Cannot assign to immutable variable '{}'; declare it with `let mut`",
                        name
                    )));
                }
                (slot.ptr, slot.ty)
            }
            Expr::ArrayIndex(idx_expr) => {
                self.ensure_mutable(&expr.target)?;
                self.compile_array_index_addr(idx_expr, None)?
            }
            Expr::FieldAccess(_) => {
                self.ensure_mutable(&expr.target)?;
                self.compile_addr(&expr.target)?
            }
            Expr::Unary(u) if u.operator == UnOp::Deref => {
                if Self::is_null_expr(&u.operand) {
                    return Err(HuziError::new_global(
                        "Cannot dereference 'null'; assign through a Box<T> variable instead",
                    ));
                }
                let nest = self.box_nest_of_expr(&u.operand).ok_or_else(|| {
                    HuziError::new_global(
                        "Dereference '*' requires a Box<T> operand (found a non-Box value)",
                    )
                })?;
                self.ensure_mutable(&u.operand)?;
                let ptr = self.compile_box_ptr(&u.operand)?;
                self.walk_box_chain(ptr, &nest)?
            }
            _ => return Err(HuziError::new_global("Invalid assignment target")),
        };

        let cur_val = self.builder.build_load(ty, ptr, "compound_lhs").unwrap();
        let rhs_val = self.compile_expr(&expr.value)?;
        let mut left = cur_val;
        let mut right = rhs_val;
        self.coerce_binary_operands(&mut left, &mut right);
        let new_val = self.build_arithmetic(&bin_op, &left, &right)?;
        let value = self.coerce_value(ty, new_val)?;
        self.builder.build_store(ptr, value).unwrap();
        Ok(value)
    }

    pub(in crate::codegen) fn compile_assign(
        &mut self,
        expr: &AssignExpr,
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        if let Some(name) = self.unit_call_name(&expr.value) {
            return Err(HuziError::new_global(format!(
                "Function '{}' has no return value and cannot be used as a value; call it as a statement instead",
                name
            )));
        }
        if let Some(bin_op) = expr.operator.to_bin_op() {
            return self.compile_compound_assign(expr, bin_op);
        }
        // 门卫:右值按值含堆结构体禁赋值(含行列;`box`/`null` 已放行)。
        self.reject_heap_value_expr(&expr.value, "按值赋值")?;
        let value = self.compile_expr(&expr.value)?;

        match &*expr.target {
            Expr::Ident(name) => self.compile_assign_ident(name, expr, value),
            Expr::Unary(u) if u.operator == UnOp::Deref => self.compile_deref_store(u, value),
            Expr::ArrayIndex(idx_expr) => {
                self.ensure_mutable(&expr.target)?;
                let (elem_ptr, elem_type) =
                    self.compile_array_index_addr(idx_expr, Some(value.get_type()))?;
                let value = self.coerce_value(elem_type, value)?;
                self.builder.build_store(elem_ptr, value).unwrap();
                Ok(value)
            }
            Expr::FieldAccess(fa) => self.compile_assign_field(fa, expr, value),
            _ => Err(HuziError::new_global("Invalid assignment target")),
        }
    }
}
