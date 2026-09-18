use super::{CodeGen, VarSlot};
use inkwell::AddressSpace;
use inkwell::types::BasicType;
use std::collections::HashMap;
use huzi_ast::*;
use huzi_error::{HuziError, Result};

impl<'ctx> CodeGen<'ctx> {
    pub(super) fn compile_stmt(&mut self, stmt: &Stmt, span: Span) -> Result<()> {
        // 之后生成的指令都归属该语句所在行(未启用调试时为 no-op)。
        self.set_current_debug_span(span);
        match stmt {
            Stmt::Let(let_stmt) => self.compile_let(let_stmt, span),
            Stmt::Struct(_) => Err(HuziError::new_global(
                "Struct definitions are only allowed at the top level",
            )),
            Stmt::Enum(_) => Err(HuziError::new_global(
                "Enum definitions are only allowed at the top level",
            )),
            Stmt::Fn(fn_stmt) => self.compile_fn(fn_stmt, span),
            // import 在加载阶段已处理,编译期不再出现
            Stmt::Import(_) => Ok(()),
            Stmt::Expr(expr_stmt) => {
                self.compile_expr(&expr_stmt.expr)?;
                Ok(())
            }
            Stmt::Return(return_stmt) => self.compile_return(return_stmt),
            Stmt::Break => self.compile_break(),
            Stmt::Continue => self.compile_continue(),
            Stmt::Block(block) => self.compile_block(block),
            Stmt::If(if_stmt) => self.compile_if(if_stmt, span),
            Stmt::For(for_stmt) => self.compile_for(for_stmt, span),
            Stmt::While(while_stmt) => self.compile_while(while_stmt),
            Stmt::Defer(inner) => self.compile_defer(inner),
            Stmt::Trait(_) | Stmt::Impl(_) => Ok(()),
        }
    }

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

        let alloca = if box_inner.is_some() {
            let a = self.build_box_alloca(var_type, &stmt.name)?;
            self.box_slots.push((a, var_type));
            a
        } else {
            self.build_alloca(var_type, &stmt.name)?
        };
        self.builder.build_store(alloca, value).unwrap();

        if box_inner.is_some() {
            if !matches!(value_expr, Expr::BoxAlloc(_) | Expr::Call(_) | Expr::Null) {
                if value.is_pointer_value() {
                    self.emit_retain_box(value.into_pointer_value())?;
                }
            }
        }

        self.scope_insert(
            stmt.name.clone(),
            VarSlot {
                ptr: alloca,
                ty: var_type,
                elem,
                array_len,
                mutable: stmt.mutable,
                box_inner,
            },
        );
        self.declare_local(&stmt.name, alloca, var_type, span);
        Ok(())
    }

    pub(super) fn elem_and_mark_from_ast(
        &self,
        ty: &Type,
    ) -> Result<(Option<inkwell::types::BasicTypeEnum<'ctx>>, Option<u32>)> {
        match ty {
            Type::Applied(name, args) if name == "vec" => {
                let elem = match args.first() {
                    Some(first) => Some(self.type_to_llvm(first)?),
                    None => None,
                };
                Ok((elem, None))
            }
            Type::Named(n) | Type::Applied(n, _) if n == "map" || n == "Map" || n == "HashMap" => {
                Ok((None, Some(crate::codegen::map::MAP_MARK)))
            }
            _ => Ok((None, None)),
        }
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
        }
        if let Expr::Call(c) = value_expr {
            if let Expr::Ident(fname) = &*c.callee {
                let key = self.qualify_name(fname);
                if let Some(ret_ty) = self.fn_return_ast.get(&key) {
                    return self.elem_and_mark_from_ast(ret_ty);
                }
            }
        }
        if let Expr::EnumConstruct(ec) = value_expr {
            let key = format!("{}::{}", ec.enum_name, ec.variant);
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

        self.scope_insert(
            stmt.name.clone(),
            VarSlot {
                ptr: alloca,
                ty,
                elem,
                array_len: None,
                mutable: stmt.mutable,
                box_inner,
            },
        );
        self.declare_local(&stmt.name, alloca, ty, span);
        Ok(())
    }

    pub(super) fn compile_fn(&mut self, stmt: &FnStmt, span: Span) -> Result<()> {
        let (function, _) = self
            .functions
            .get(&self.qualify_name(&stmt.name))
            .cloned()
            .ok_or_else(|| HuziError::new_global(format!("Unknown function: {}", stmt.name)))?;

        let entry = self.context.append_basic_block(function, "entry");
        self.builder.position_at_end(entry);
        self.current_subprogram = function.get_subprogram();
        self.clear_debug_location();

        // The program entry point sets up UTF-8 console output first.
        if stmt.name == "main" {
            self.emit_console_utf8_setup();
            // A parameterless Huzi `fn main` is compiled with the C
            // `main(argc, argv)` signature; capture the hidden params.
            if stmt.params.is_empty() && function.count_params() == 2 {
                self.store_main_args(function);
            }
        }

        let return_type = match &stmt.return_type {
            Some(t) => self.type_to_llvm(t)?,
            None => self.context.i32_type().into(),
        };
        self.current_return_type = Some(return_type);
        self.current_return_ast = stmt.return_type.clone();
        self.scopes = vec![HashMap::new()];
        self.defer_stack.clear();
        self.box_slots.clear();

        for (i, param) in stmt.params.iter().enumerate() {
            let arg = function.get_nth_param(i as u32).unwrap();
            let arg_type = arg.get_type();
            let box_inner = self.box_nest_of_ast(&param.param_type)?;

            let alloca = if box_inner.is_some() {
                let a = self.build_box_alloca(arg_type, &param.name)?;
                self.box_slots.push((a, arg_type));
                if arg.is_pointer_value() {
                    self.emit_retain_box(arg.into_pointer_value())?;
                }
                a
            } else {
                self.build_alloca(arg_type, &param.name)?
            };
            self.builder.build_store(alloca, arg).unwrap();
            self.declare_param(&param.name, alloca, arg_type, i as u32 + 1, span.start_line() as u32);

            // Arrays decay to pointers; remember the element type for indexing.
            let (vec_elem, map_mark) = self.elem_and_mark_from_ast(&param.param_type)?;
            let elem = match &param.param_type {
                Type::Array(elem_ty, _) => Some(self.type_to_llvm(elem_ty)?),
                // `s: str` parses as Named("str") (parse_type keeps builtin
                // names as Named), so both forms need char-index metadata.
                Type::Str | Type::Named(_) if param.param_type == Type::Named("str".to_string()) => {
                    Some(self.context.i8_type().into())
                }
                _ => vec_elem,
            };

            let array_len = match &param.param_type {
                Type::Array(_, size) => Some(*size as u32),
                _ => map_mark,
            };

            self.scopes.last_mut().unwrap().insert(
                param.name.clone(),
                VarSlot {
                    ptr: alloca,
                    ty: arg_type,
                    elem,
                    array_len,
                    mutable: true,
                    box_inner,
                },
            );
        }

        self.compile_block(&stmt.body)?;

        // Functions without an explicit return fall through with a zero value
        // of the declared return type.
        if self.at_open_end() {
            self.emit_defers()?;
            self.emit_release_active_boxes(None)?;
            self.builder
                .build_return(Some(&return_type.const_zero()))
                .unwrap();
        }

        self.current_return_type = None;
        self.current_return_ast = None;

        Ok(())
    }

    pub(super) fn compile_return(&mut self, stmt: &ReturnStmt) -> Result<()> {
        let ret_type = self
            .current_return_type
            .unwrap_or_else(|| self.context.i32_type().into());

        match &stmt.value {
            Some(value_expr) => {
                // `return null` 只能出现在 Box 返回位置。
                if let Some(ast) = self.current_return_ast.clone() {
                    self.check_box_assignable(value_expr, &ast)?;
                }
                let value = self.compile_expr(value_expr)?;
                let value = self.coerce_value(ret_type, value)?;
                self.emit_defers()?;

                let is_ret_box = self
                    .current_return_ast
                    .as_ref()
                    .map(|t| Self::is_box_ast(t))
                    .unwrap_or(false);
                let ret_slot_ptr = if is_ret_box {
                    if let Expr::Ident(name) = value_expr {
                        self.scope_lookup(name).map(|s| s.ptr)
                    } else {
                        if !matches!(value_expr, Expr::BoxAlloc(_) | Expr::Call(_) | Expr::Null) {
                            if value.is_pointer_value() {
                                self.emit_retain_box(value.into_pointer_value())?;
                            }
                        }
                        None
                    }
                } else {
                    None
                };
                self.emit_release_active_boxes(ret_slot_ptr)?;
                self.builder.build_return(Some(&value)).unwrap();
            }
            None => {
                self.emit_defers()?;
                self.emit_release_active_boxes(None)?;
                self.builder
                    .build_return(Some(&ret_type.const_zero()))
                    .unwrap();
            }
        }
        Ok(())
    }

    pub(super) fn compile_block(&mut self, block: &Block) -> Result<()> {
        self.push_scope();
        for stmt in &block.statements {
            self.compile_stmt(&stmt.node, stmt.span)?;
        }
        self.pop_scope();
        Ok(())
    }

    /// 无返回值函数(`fn foo() {...}`)的调用名:值位置(`let x = foo()`,
    /// `x = foo()`)须拒绝,语句位置(`foo()`)放行。非调用返回 None。
    pub(super) fn unit_call_name(&self, expr: &Expr) -> Option<String> {
        if let Expr::Call(call) = expr {
            if let Expr::Ident(name) = &*call.callee {
                if self.fn_no_return.get(&self.qualify_name(name)).copied().unwrap_or(false) {
                    return Some(name.clone());
                }
            }
        }
        None
    }

    /// Compile a block as an expression: the block's value is the value of its
    /// last expression statement.
    pub(super) fn compile_block_value(&mut self, block: &Block) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        self.push_scope();
        let mut last: Option<inkwell::values::BasicValueEnum<'ctx>> = None;
        for stmt in &block.statements {
            match &stmt.node {
                Stmt::Expr(es) => last = Some(self.compile_expr(&es.expr)?),
                other => self.compile_stmt(other, stmt.span)?,
            }
        }
        self.pop_scope();
        last.ok_or_else(|| HuziError::new_global("Block used as an expression must end with a value"))
    }

}
