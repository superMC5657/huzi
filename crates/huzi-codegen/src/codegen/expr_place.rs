use super::{CodeGen, StructFieldInfo};
use inkwell::values::PointerValue;
use huzi_ast::*;
use huzi_error::{HuziError, Result};


impl<'ctx> CodeGen<'ctx> {
    pub(super) fn compile_assign(
        &mut self,
        expr: &AssignExpr,
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        let value = self.compile_expr(&expr.value)?;

        match &*expr.target {
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

                let value = self.coerce_value(slot.ty, value)?;
                self.builder.build_store(slot.ptr, value).unwrap();
                Ok(value)
            }
            Expr::ArrayIndex(idx_expr) => {
                self.ensure_mutable(&expr.target)?;
                // vec 下标写走动态长度路径。
                if let Expr::Ident(name) = &*idx_expr.array {
                    if let Some(slot) = self.scope_lookup(name) {
                        if Self::is_vec_slot(&slot) {
                            let (elem_ptr, elem_type) =
                                self.vec_index_ptr(name, &idx_expr.index)?;
                            let value = self.coerce_value(elem_type, value)?;
                            self.builder.build_store(elem_ptr, value).unwrap();
                            return Ok(value);
                        }
                    }
                }
                let array_ptr = self.compile_expr(&idx_expr.array)?;
                let array_ptr = if array_ptr.is_pointer_value() {
                    array_ptr.into_pointer_value()
                } else {
                    return Err(HuziError::new_global("Indexed value is not an array"));
                };

                let elem_type = self.resolve_elem_type(&idx_expr.array, Some(value.get_type()))?;

                let index_val = self.compile_expr(&idx_expr.index)?;
                let index_i32 = self.coerce_index(index_val)?;

                if self.is_string_index(&idx_expr.array, elem_type)? {
                    self.emit_str_bounds_check(array_ptr, index_i32)?;
                } else {
                    self.emit_bounds_check(&idx_expr.array, index_i32)?;
                }

                let value = self.coerce_value(elem_type, value)?;

                let elem_ptr = unsafe {
                    self.builder
                        .build_gep(elem_type, array_ptr, &[index_i32], "elem_ptr")
                        .unwrap()
                };
                self.builder.build_store(elem_ptr, value).unwrap();
                Ok(value)
            }
            Expr::FieldAccess(_) => {
                self.ensure_mutable(&expr.target)?;
                let (field_ptr, field_ty) = self.compile_addr(&expr.target)?;
                let value = self.coerce_value(field_ty, value)?;
                self.builder.build_store(field_ptr, value).unwrap();
                Ok(value)
            }
            _ => Err(HuziError::new_global("Invalid assignment target")),
        }
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
                let (base_ptr, base_ty) = self.compile_addr(&fa.base)?;
                self.gep_field(base_ptr, base_ty, &fa.field)
            }
            Expr::ArrayIndex(idx_expr) => {
                if let Expr::Ident(name) = &*idx_expr.array {
                    if let Some(slot) = self.scope_lookup(name) {
                        if Self::is_vec_slot(&slot) {
                            return self.vec_index_ptr(name, &idx_expr.index);
                        }
                    }
                }
                let array_ptr = self.compile_expr(&idx_expr.array)?;
                let array_ptr = if array_ptr.is_pointer_value() {
                    array_ptr.into_pointer_value()
                } else {
                    return Err(HuziError::new_global("Indexed value is not an array"));
                };

                let elem_type = self.resolve_elem_type(&idx_expr.array, None)?;
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
            _ => {
                // Rvalue base (e.g. a function call or enum constructor):
                // spill it to a temporary so it has an address.
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
    /// variables and field chains.
    pub(super) fn struct_def_of_expr(
        &self,
        expr: &Expr,
    ) -> Option<&(inkwell::types::StructType<'ctx>, Vec<StructFieldInfo<'ctx>>)> {
        match expr {
            Expr::Ident(name) => {
                let slot = self.scope_lookup(name)?;
                self.struct_def_by_type(slot.ty)
            }
            Expr::FieldAccess(fa) => {
                let (_, fields) = self.struct_def_of_expr(&fa.base)?;
                let info = fields.iter().find(|info| info.name == fa.field)?;
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
            _ => Ok(()),
        }
    }

    pub(super) fn compile_field_access(
        &mut self,
        expr: &FieldAccessExpr,
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        let (base_ptr, base_ty) = self.compile_addr(&expr.base)?;
        let (field_ptr, field_ty) = self.gep_field(base_ptr, base_ty, &expr.field)?;
        let loaded = self
            .builder
            .build_load(field_ty, field_ptr, "field")
            .unwrap();
        Ok(loaded)
    }
}
