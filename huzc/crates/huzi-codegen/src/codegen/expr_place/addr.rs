//! 取址与字段定位:数组下标地址、左值地址、结构体/元组字段 GEP。(自 `expr_place.rs` 纯搬移,零逻辑变化)。

use super::super::{CodeGen, StructFieldInfo};
use huzi_ast::*;
use huzi_error::{HuziError, Result};
use inkwell::values::PointerValue;

impl<'ctx> CodeGen<'ctx> {
    pub(super) fn compile_array_index_addr(
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

    pub(in crate::codegen) fn compile_addr(
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

    /// 对存储在 `base_ptr` 的结构体执行 GEP 以获取具名字段；
    /// 若基类为元组且字段为数字索引（`t.0`），则定位至该元素。
    pub(in crate::codegen) fn gep_field(
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

    /// 根据 LLVM 类型查找已注册的结构体定义。
    pub(in crate::codegen) fn struct_def_by_type(
        &self,
        ty: inkwell::types::BasicTypeEnum<'ctx>,
    ) -> Option<&(inkwell::types::StructType<'ctx>, Vec<StructFieldInfo<'ctx>>)> {
        let st = match ty {
            inkwell::types::BasicTypeEnum::StructType(st) => st,
            _ => return None,
        };
        self.structs.values().find(|(def_st, _)| *def_st == st)
    }

    /// 尽力推导表达式对应的结构体定义，顺着变量与字段链追踪
    /// （对 Box 层级执行自动解引用）。
    pub(in crate::codegen) fn struct_def_of_expr(
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
                if Self::is_box_or_weak_ast(&info.ast_ty) {
                    let nest = self.box_nest_of_ast(&info.ast_ty).ok()??;
                    return self.struct_def_by_type(nest.ultimate);
                }
                self.struct_def_by_type(info.ty)
            }
            _ => None,
        }
    }

    /// 左值链的根部必须是一个可变变量（mutable variable）。
    pub(in crate::codegen) fn ensure_mutable(&self, expr: &Expr) -> Result<()> {
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

    pub(in crate::codegen) fn compile_field_access(
        &mut self,
        expr: &FieldAccessExpr,
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        let is_weak = self
            .field_ast_type(&expr.base, &expr.field)
            .map(|t| Self::is_weak_ast(&t))
            .unwrap_or(false);

        // 基址是 Box 时先自动解引用,逐层生效(`head.next.val`)。
        let loaded = match self.compile_addr_deref(&expr.base) {
            Ok((base_ptr, base_ty)) => {
                let (field_ptr, field_ty) = self.gep_field(base_ptr, base_ty, &expr.field)?;
                self.builder
                    .build_load(field_ty, field_ptr, "field")
                    .unwrap()
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
                self.builder
                    .build_load(field_ty, field_ptr, "field")
                    .unwrap()
            }
        };

        if is_weak && loaded.is_pointer_value() {
            let valid = self.emit_load_weak(loaded.into_pointer_value())?;
            return Ok(valid.into());
        }
        Ok(loaded)
    }
}
