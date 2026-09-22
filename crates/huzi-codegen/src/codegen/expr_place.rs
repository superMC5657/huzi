use super::{CodeGen, StructFieldInfo};
use inkwell::values::PointerValue;
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

        if Self::is_null_expr(&expr.value) && !Self::is_box_slot(&slot) {
            return Err(HuziError::new_global(format!(
                "null can only be assigned to a Box<T> slot (variable '{}' is not a Box)",
                name
            )));
        }
        if matches!(&*expr.value, Expr::BoxAlloc(_)) && !Self::is_box_slot(&slot) {
            return Err(HuziError::new_global(format!(
                "Cannot assign a Box value to non-Box variable '{}'",
                name
            )));
        }

        if Self::is_box_slot(&slot) {
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
        if let Some(expected) = self.field_ast_type(&fa.base, &fa.field) {
            self.check_box_assignable(&expr.value, &expected)?;
            is_box_field = Self::is_box_ast(&expected);
        }
        let (field_ptr, field_ty) = self.compile_addr(&expr.target)?;
        if is_box_field {
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

    pub(super) fn compile_assign(
        &mut self,
        expr: &AssignExpr,
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        if let Some(name) = self.unit_call_name(&expr.value) {
            return Err(HuziError::new_global(format!(
                "Function '{}' has no return value and cannot be used as a value; call it as a statement instead",
                name
            )));
        }
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

    fn compile_array_index_addr(
        &mut self,
        idx_expr: &ArrayIndexExpr,
        expected_elem: Option<inkwell::types::BasicTypeEnum<'ctx>>,
    ) -> Result<(PointerValue<'ctx>, inkwell::types::BasicTypeEnum<'ctx>)> {
        if let Expr::Ident(name) = &*idx_expr.array {
            if let Some(slot) = self.scope_lookup(name) {
                if Self::is_vec_slot(&slot) {
                    return self.vec_index_ptr(name, &idx_expr.index);
                }
            }
        }
        if let Expr::FieldAccess(fa) = &*idx_expr.array {
            if let Some(field_ty) = self.field_ast_type(&fa.base, &fa.field) {
                if matches!(field_ty, Type::Applied(ref n, _) if n == "vec") {
                    let elem_type = self.elem_and_mark_from_ast(&field_ty)?.0.unwrap();
                    let (vec_ptr, vec_ty) = self.compile_addr(&idx_expr.array)?;
                    let parts = self.load_vec_parts_from_ptr(vec_ptr, vec_ty)?;
                    let index_val = self.compile_expr(&idx_expr.index)?;
                    let index_i32 = self.coerce_index(index_val)?;
                    self.emit_vec_bounds_check(parts.len, index_i32)?;
                    let elem_ptr = unsafe {
                        self.builder
                            .build_gep(elem_type, parts.data, &[index_i32], "vec_elem_ptr")
                            .unwrap()
                    };
                    return Ok((elem_ptr, elem_type));
                }
            }
        }
        let array_ptr = self.compile_expr(&idx_expr.array)?;
        let array_ptr = if array_ptr.is_pointer_value() {
            array_ptr.into_pointer_value()
        } else {
            return Err(HuziError::new_global("Indexed value is not an array"));
        };

        let elem_type = self.resolve_elem_type(&idx_expr.array, expected_elem)?;
        let index_val = self.compile_expr(&idx_expr.index)?;
        let index_i32 = self.coerce_index(index_val)?;

        if self.is_string_index(&idx_expr.array, elem_type)? {
            self.emit_str_bounds_check(array_ptr, index_i32)?;
        } else {
            self.emit_bounds_check(&idx_expr.array, index_i32)?;
        }

        let elem_ptr = unsafe {
            self.builder
                .build_gep(elem_type, array_ptr, &[index_i32], "elem_ptr")
                .unwrap()
        };
        Ok((elem_ptr, elem_type))
    }

    pub(super) fn compile_addr(
        &mut self,
        expr: &Expr,
    ) -> Result<(PointerValue<'ctx>, inkwell::types::BasicTypeEnum<'ctx>)> {
        match expr {
            Expr::Ident(name) => {
                let slot = self
                    .scope_lookup(name)
                    .ok_or_else(|| self.unknown_variable_error(name))?;
                Ok((slot.ptr, slot.ty))
            }
            Expr::FieldAccess(fa) => {
                let (base_ptr, base_ty) = self.compile_addr_deref(&fa.base)?;
                self.gep_field(base_ptr, base_ty, &fa.field)
            }
            Expr::ArrayIndex(idx_expr) => self.compile_array_index_addr(idx_expr, None),
            _ => {
                let value = self.compile_expr(expr)?;
                let ty = value.get_type();
                let tmp = self.build_alloca(ty, "rvalue_tmp")?;
                self.builder.build_store(tmp, value).unwrap();
                Ok((tmp, ty))
            }
        }
    }

    /// GEP to a named field of the struct value stored at `base_ptr`, or to
    /// element `field` when the base is a tuple and the field is an index
    /// (`t.0`).
    pub(super) fn gep_field(
        &self,
        base_ptr: PointerValue<'ctx>,
        base_ty: inkwell::types::BasicTypeEnum<'ctx>,
        field: &str,
    ) -> Result<(PointerValue<'ctx>, inkwell::types::BasicTypeEnum<'ctx>)> {
        if let Ok(index) = field.parse::<usize>() {
            if let inkwell::types::BasicTypeEnum::StructType(st) = base_ty {
                if self.is_tuple_type(st) {
                    return self.gep_tuple_field(base_ptr, st, index);
                }
            }
        }

        let (_, fields) = self
            .struct_def_by_type(base_ty)
            .ok_or_else(|| HuziError::new_global("Value has no fields (not a struct)"))?;

        let (index, info) = fields
            .iter()
            .enumerate()
            .find(|(_, info)| info.name == field)
            .ok_or_else(|| HuziError::new_global(format!("Struct has no field '{}'", field)))?;

        let field_ptr = self
            .builder
            .build_struct_gep(base_ty.into_struct_type(), base_ptr, index as u32, "field_ptr")
            .unwrap();
        Ok((field_ptr, info.ty))
    }

    /// Find a registered struct definition by its LLVM type.
    pub(super) fn struct_def_by_type(
        &self,
        ty: inkwell::types::BasicTypeEnum<'ctx>,
    ) -> Option<&(inkwell::types::StructType<'ctx>, Vec<StructFieldInfo<'ctx>>)> {
        let st = match ty {
            inkwell::types::BasicTypeEnum::StructType(st) => st,
            _ => return None,
        };
        self.structs.values().find(|(def_st, _)| *def_st == st)
    }

    /// Best-effort struct definition lookup for an expression, following
    /// variables and field chains (Box layers are auto-dereferenced).
    pub(super) fn struct_def_of_expr(
        &self,
        expr: &Expr,
    ) -> Option<&(inkwell::types::StructType<'ctx>, Vec<StructFieldInfo<'ctx>>)> {
        match expr {
            Expr::Ident(name) => {
                let slot = self.scope_lookup(name)?;
                if let Some(nest) = slot.box_inner {
                    return self.struct_def_by_type(nest.ultimate);
                }
                self.struct_def_by_type(slot.ty)
            }
            Expr::FieldAccess(fa) => {
                let (_, fields) = self.struct_def_of_expr(&fa.base)?;
                let info = fields.iter().find(|info| info.name == fa.field)?;
                // Box(含嵌套)字段:最内层 pointee 才是下一层结构体。
                if Self::is_box_ast(&info.ast_ty) {
                    let nest = self.box_nest_of_ast(&info.ast_ty).ok()??;
                    return self.struct_def_by_type(nest.ultimate);
                }
                self.struct_def_by_type(info.ty)
            }
            _ => None,
        }
    }

    /// The root of an lvalue chain must be a mutable variable.
    pub(super) fn ensure_mutable(&self, expr: &Expr) -> Result<()> {
        match expr {
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
                Ok(())
            }
            Expr::FieldAccess(fa) => self.ensure_mutable(&fa.base),
            Expr::ArrayIndex(idx) => self.ensure_mutable(&idx.array),
            Expr::Unary(u) if u.operator == UnOp::Deref => self.ensure_mutable(&u.operand),
            _ => Ok(()),
        }
    }

    pub(super) fn compile_field_access(
        &mut self,
        expr: &FieldAccessExpr,
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        // 基址是 Box 时先自动解引用,逐层生效(`head.next.val`)。
        match self.compile_addr_deref(&expr.base) {
            Ok((base_ptr, base_ty)) => {
                let (field_ptr, field_ty) = self.gep_field(base_ptr, base_ty, &expr.field)?;
                let loaded = self
                    .builder
                    .build_load(field_ty, field_ptr, "field")
                    .unwrap();
                Ok(loaded)
            }
            Err(base_err) => {
                // 右值基座(调用结果等值位置):暂存临时槽再取字段,
                // 支持 `parse_int(s).1`、`json::get_int(d, "k").0` 写法。
                let val = match self.compile_expr(&expr.base) {
                    Ok(v) => v,
                    Err(_) => return Err(base_err),
                };
                if !val.is_struct_value() {
                    return Err(base_err);
                }
                let sv = val.into_struct_value();
                let sty = sv.get_type();
                let tmp = self.build_alloca(sty.into(), "field_rvalue_tmp")?;
                self.builder.build_store(tmp, sv).unwrap();
                let (field_ptr, field_ty) = self.gep_field(tmp, sty.into(), &expr.field)?;
                let loaded = self
                    .builder
                    .build_load(field_ty, field_ptr, "field")
                    .unwrap();
                Ok(loaded)
            }
        }
    }
}
