//! `let` 语句编译:按右值形态分派(数组/元组字面量、vec/split/map
//! 构造、Box、null、普通值),并负责元素类型/容器标记的解析。

use super::{CodeGen, VarSlot};
use super::qname;
use inkwell::AddressSpace;
use inkwell::types::BasicType;
use huzi_ast::*;
use huzi_error::{HuziError, Result};

impl<'ctx> CodeGen<'ctx> {
    pub(super) fn compile_let(&mut self, stmt: &LetStmt, span: Span) -> Result<()> {
        match &stmt.value {
            Some(Expr::ArrayLiteral(elements)) => self.compile_let_array(stmt, elements, span),
            Some(Expr::TupleLiteral(elements)) => self.compile_let_tuple(stmt, elements, span),
            Some(Expr::Call(call)) if Self::is_vec_ctor(call) => {
                self.compile_let_vec(stmt, &call.arguments, span)
            }
            Some(Expr::Call(call)) if Self::is_split_ctor(call) => {
                self.compile_let_split(stmt, &call.arguments, span)
            }
            Some(Expr::Call(call)) if Self::is_map_keys_ctor(call) => {
                self.compile_let_map_keys(stmt, &call.arguments, span)
            }
            Some(Expr::Call(call)) if Self::is_map_ctor(call) => {
                self.compile_let_map(stmt, &call.arguments, span)
            }
            Some(Expr::VecEmpty(elem_ty)) => self.compile_let_vec_empty(stmt, elem_ty, span),
            Some(Expr::Null) => self.compile_let_null(stmt, span),
            Some(Expr::BoxAlloc(inner)) => self.compile_let_box(stmt, inner, span),
            Some(value_expr) => self.compile_let_with_value(stmt, value_expr, span),
            None => self.compile_let_uninitialized(stmt, span),
        }
    }

    /// `let name = [a, b, c]` — build a fixed-size array and store its
    /// address in a pointer slot so loading the variable yields the array
    /// address.
    fn compile_let_array(&mut self, stmt: &LetStmt, elements: &[Expr], span: Span) -> Result<()> {
        if elements.is_empty() {
            return Err(HuziError::new_global("Empty array literal not supported"));
        }

        let mut values = Vec::with_capacity(elements.len());
        for e in elements {
            values.push(self.compile_expr(e)?);
        }

        let elem_type = values[0].get_type();
        let array_type = elem_type.array_type(values.len() as u32);
        let array_ptr = self.build_alloca(array_type.into(), &stmt.name)?;

        for (i, val) in values.iter().enumerate() {
            let val = self.coerce_value(elem_type, *val)?;
            let index = self.context.i32_type().const_int(i as u64, false);
            let elem_ptr = unsafe {
                self.builder
                    .build_gep(elem_type, array_ptr, &[index], "arr_elem")
                    .unwrap()
            };
            self.builder.build_store(elem_ptr, val).unwrap();
        }

        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let slot_ptr = self.build_alloca(ptr_ty.into(), &format!("{}.ptr", stmt.name))?;
        self.builder.build_store(slot_ptr, array_ptr).unwrap();
        self.scope_insert(
            stmt.name.clone(),
            VarSlot {
                ptr: slot_ptr,
                ty: ptr_ty.into(),
                elem: Some(elem_type),
                array_len: Some(values.len() as u32),
                mutable: stmt.mutable,
                box_inner: None,
                map_kind: None,
            },
        );
        self.declare_local(&stmt.name, slot_ptr, ptr_ty.into(), span);
        Ok(())
    }

    /// `let name[: T] = value`.
    fn compile_let_with_value(&mut self, stmt: &LetStmt, value_expr: &Expr, span: Span) -> Result<()> {
        // 无返回值函数的调用值不可赋给变量(语句位置调用仍放行)。
        if let Some(name) = self.unit_call_name(value_expr) {
            return Err(HuziError::new_global(format!(
                "Function '{}' has no return value and cannot be used as a value; call it as a statement instead",
                name
            )));
        }
        // 有标注时先做 `box`/`null` 的 AST 校验(LLVM 指针无法区分 Box 内外层)。
        if let Some(ann) = &stmt.type_annotation {
            self.check_box_assignable(value_expr, ann)?;
        }
        let mut value = self.compile_expr(value_expr)?;

        let var_type = match &stmt.type_annotation {
            Some(t) => {
                let ty = self.type_to_llvm(t)?;
                value = self.coerce_value(ty, value)?;
                ty
            }
            None => value.get_type(),
        };

        self.record_let_ast_type(stmt, value_expr);

        // Box 槽记录 pointee(无标注时由值推导);Box 指针不做字符串元数据标记。
        let box_inner = match &stmt.type_annotation {
            Some(ann) => self.box_nest_of_ast(ann)?,
            None => self.box_nest_of_expr(value_expr),
        };
        let (elem, array_len) = self.resolve_let_elem_and_mark(
            stmt,
            value_expr,
            var_type,
            box_inner.is_some(),
        )?;

        let alloca = self.alloc_let_slot(stmt, value_expr, value, var_type, box_inner.is_some())?;

        // Map 拷贝透传种类:标注优先,否则随源变量/字段/返回类型。
        let map_kind = self.infer_let_map_kind(stmt, value_expr);
        self.scope_insert(
            stmt.name.clone(),
            VarSlot {
                ptr: alloca,
                ty: var_type,
                elem,
                array_len,
                mutable: stmt.mutable,
                box_inner,
                map_kind,
            },
        );
        self.declare_local(&stmt.name, alloca, var_type, span);
        Ok(())
    }

    /// 记录局部变量的 AST 类型(供 `r.1` 元组字段访问推断元素类型)。
    fn record_let_ast_type(&mut self, stmt: &LetStmt, value_expr: &Expr) {
        let ast_ty = if let Some(ann) = &stmt.type_annotation {
            Some(ann.clone())
        } else {
            self.static_type_of_value(value_expr)
        };
        if let Some(t) = ast_ty {
            self.local_ast.insert(stmt.name.clone(), t);
        }
    }

    /// 分配 let 的存储槽并写入初值:Box 变量走 box alloca 并登记统一
    /// 释放槽,其余按类型 alloca;非常量来源(BoxAlloc/调用/null 之外)
    /// 的 Box 值补 retain。
    fn alloc_let_slot(
        &mut self,
        stmt: &LetStmt,
        value_expr: &Expr,
        value: inkwell::values::BasicValueEnum<'ctx>,
        var_type: inkwell::types::BasicTypeEnum<'ctx>,
        is_box: bool,
    ) -> Result<inkwell::values::PointerValue<'ctx>> {
        let alloca = if is_box {
            let a = self.build_box_alloca(var_type, &stmt.name)?;
            self.box_slots.push((a, var_type));
            a
        } else {
            self.build_alloca(var_type, &stmt.name)?
        };
        self.builder.build_store(alloca, value).unwrap();

        if is_box
            && !matches!(value_expr, Expr::BoxAlloc(_) | Expr::Call(_) | Expr::Null)
                && value.is_pointer_value() {
                    self.emit_retain_box(value.into_pointer_value())?;
                }
        Ok(alloca)
    }

    fn resolve_let_elem_and_mark(
        &self,
        stmt: &LetStmt,
        value_expr: &Expr,
        var_type: inkwell::types::BasicTypeEnum<'ctx>,
        box_inner: bool,
    ) -> Result<(Option<inkwell::types::BasicTypeEnum<'ctx>>, Option<u32>)> {
        if box_inner {
            return Ok((None, None));
        }
        if var_type.is_pointer_type() {
            return Ok((Some(self.context.i8_type().into()), None));
        }
        if let Some(ann) = &stmt.type_annotation {
            return self.elem_and_mark_from_ast(ann);
        }
        if let Expr::Ident(id) = value_expr {
            if let Some(slot) = self.scope_lookup(id) {
                if Self::is_vec_slot(&slot) {
                    return Ok((slot.elem, None));
                }
                if Self::is_map_slot(&slot) {
                    return Ok((None, Some(crate::codegen::map::MAP_MARK)));
                }
            }
        }
        if let Expr::FieldAccess(fa) = value_expr {
            if let Some(field_ty) = self.field_ast_type(&fa.base, &fa.field) {
                return self.elem_and_mark_from_ast(&field_ty);
            }
            // 元组右值字段:`let kinds = r.1` / `let kinds = f(s).1`。
            // 字段名为数字且基座的元组类型可知时,取对应元素类型。
            if let Ok(idx) = fa.field.parse::<usize>() {
                let base_ty = match &*fa.base {
                    Expr::Ident(id) => self.local_ast.get(id).cloned(),
                    other => self.static_type_of_value(other),
                };
                if let Some(Type::Tuple(elems)) = base_ty {
                    if let Some(et) = elems.get(idx) {
                        return self.elem_and_mark_from_ast(et);
                    }
                }
            }
        }
        if let Expr::Call(c) = value_expr {
            if let Expr::Ident(fname) = &*c.callee {
                if fname == "map_keys" {
                    return Ok((
                        Some(self.context.ptr_type(inkwell::AddressSpace::default()).into()),
                        None,
                    ));
                }
                let key = self.qualify_name(fname);
                if let Some(ret_ty) = self.fn_return_ast.get(&key) {
                    return self.elem_and_mark_from_ast(ret_ty);
                }
            }
        }
        if let Expr::EnumConstruct(ec) = value_expr {
            let key = qname::qualified(&ec.enum_name, &ec.variant);
            if let Some(ret_ty) = self.fn_return_ast.get(&key) {
                return self.elem_and_mark_from_ast(ret_ty);
            }
        }
        Ok((None, None))
    }

    /// `let name: T;` — declaration without initializer, zero-initialized.
    fn compile_let_uninitialized(&mut self, stmt: &LetStmt, span: Span) -> Result<()> {
        // Requires a type annotation.
        let ty = match &stmt.type_annotation {
            Some(t) => self.type_to_llvm(t)?,
            None => {
                return Err(HuziError::new_global(format!(
                    "Variable '{}' declared without a value or a type annotation",
                    stmt.name
                )))
            }
        };

        let box_inner = match &stmt.type_annotation {
            Some(ann) => self.box_nest_of_ast(ann)?,
            None => None,
        };
        let elem = if box_inner.is_some() {
            None
        } else if ty.is_pointer_type() {
            Some(self.context.i8_type().into())
        } else {
            None
        };

        let alloca = if box_inner.is_some() {
            let a = self.build_box_alloca(ty, &stmt.name)?;
            self.box_slots.push((a, ty));
            a
        } else {
            self.build_alloca(ty, &stmt.name)?
        };
        self.builder.build_store(alloca, ty.const_zero()).unwrap();

        // 无初值声明的 map 种类由标注决定。
        let map_kind = stmt
            .type_annotation
            .as_ref()
            .and_then(super::super::MapKind::from_ast);
        self.scope_insert(
            stmt.name.clone(),
            VarSlot {
                ptr: alloca,
                ty,
                elem,
                array_len: None,
                mutable: stmt.mutable,
                box_inner,
                map_kind,
            },
        );
        self.declare_local(&stmt.name, alloca, ty, span);
        Ok(())
    }

    /// 推断 `let` 的 map 种类:显式标注优先,否则随源变量/字段/函数返回。
    fn infer_let_map_kind(
        &self,
        stmt: &LetStmt,
        value_expr: &Expr,
    ) -> Option<super::super::MapKind> {
        if let Some(ann) = &stmt.type_annotation {
            if let Some(k) = super::super::MapKind::from_ast(ann) {
                return Some(k);
            }
        }
        match value_expr {
            Expr::Ident(name) => self.scope_lookup(name).and_then(|s| s.map_kind),
            Expr::FieldAccess(fa) => self
                .field_ast_type(&fa.base, &fa.field)
                .and_then(|t| super::super::MapKind::from_ast(&t)),
            Expr::Call(c) => match &*c.callee {
                Expr::Ident(fname) => {
                    let key = self.qualify_name(fname);
                    self.fn_return_ast
                        .get(&key)
                        .and_then(super::super::MapKind::from_ast)
                }
                _ => None,
            },
            _ => None,
        }
    }
}
