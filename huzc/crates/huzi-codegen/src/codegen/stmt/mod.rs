//! 语句编译:语句分派、函数/返回/块编译,以及值位置调用与静态
//! AST 类型查询。`let` 各形态的编译在子模块 `let` 中。

mod let_;

use super::{CodeGen, VarSlot};
use super::qname;
use std::collections::HashMap;
use inkwell::values::PointerValue;
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
            // import/export 在加载阶段已处理或由模块重导出处理,此处跳过
            Stmt::Import(_) | Stmt::Export(_) => Ok(()),
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

        // 程序入口点首先初始化 UTF-8 控制台输出。
        if stmt.name == "main" {
            self.emit_console_utf8_setup();
            // 无参的 Huzi `fn main` 编译为带有 C `main(argc, argv)` 签名的函数；
            // 捕获这两个隐藏形参。
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
        self.droppable_scopes = vec![Vec::new()];
        self.defer_stack.clear();
        self.local_ast.clear();

        for (i, param) in stmt.params.iter().enumerate() {
            let arg = function.get_nth_param(i as u32).unwrap();
            let arg_type = arg.get_type();
            let box_inner = self.box_nest_of_ast(&param.param_type)?;

            let is_weak = Self::is_weak_ast(&param.param_type);
            let (alloca, slot_ty) = if Self::is_container_handle_type(&param.param_type) {
                (arg.into_pointer_value(), self.vec_struct_type().into())
            } else if is_weak {
                let a = self.build_box_alloca(arg_type, &param.name)?;
                self.register_droppable(a, arg_type, super::drop::DropKind::WeakBox);
                if arg.is_pointer_value() {
                    self.emit_retain_weak(arg.into_pointer_value())?;
                }
                self.builder.build_store(a, arg).unwrap();
                (a, arg_type)
            } else if box_inner.is_some() {
                let a = self.build_box_alloca(arg_type, &param.name)?;
                self.register_droppable(a, arg_type, super::drop::DropKind::Box);
                if arg.is_pointer_value() {
                    self.emit_retain_box(arg.into_pointer_value())?;
                }
                self.builder.build_store(a, arg).unwrap();
                (a, arg_type)
            } else {
                let a = self.build_alloca(arg_type, &param.name)?;
                self.builder.build_store(a, arg).unwrap();
                (a, arg_type)
            };
            self.declare_param(&param.name, alloca, slot_ty, i as u32 + 1, span.start_line() as u32);

            // 数组退化为指针；记住元素类型以供索引。
            let (vec_elem, map_mark) = self.elem_and_mark_from_ast(&param.param_type)?;
            let elem = match &param.param_type {
                Type::Array(elem_ty, _) => Some(self.type_to_llvm(elem_ty)?),
                // `s: str` 解析为 Named("str")（parse_type 保留内置类型名称为 Named），
                // 因此两种形式都需要字符索引元数据。
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
                    ty: slot_ty,
                    elem,
                    array_len,
                    mutable: true,
                    box_inner,
                    map_kind: super::MapKind::from_ast(&param.param_type),
                },
            );
            self.local_ast.insert(param.name.clone(), param.param_type.clone());
        }

        self.compile_block(&stmt.body)?;

        // 没有显式 return 的函数在末尾 fallthrough 返回所声明返回类型的零值。
        if self.at_open_end() {
            self.emit_defers()?;
            self.emit_release_active_boxes(&[])?;
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

                let skip_ptrs = self.resolve_return_droppables(value_expr, value)?;
                self.emit_release_active_boxes(&skip_ptrs)?;
                self.builder.build_return(Some(&value)).unwrap();
            }
            None => {
                self.emit_defers()?;
                self.emit_release_active_boxes(&[])?;
                self.builder
                    .build_return(Some(&ret_type.const_zero()))
                    .unwrap();
            }
        }
        Ok(())
    }

    fn resolve_return_droppables(
        &mut self,
        value_expr: &Expr,
        value: inkwell::values::BasicValueEnum<'ctx>,
    ) -> Result<Vec<PointerValue<'ctx>>> {
        let mut skip_ptrs = Vec::new();
        let ret_ast = self.current_return_ast.clone();

        match value_expr {
            Expr::Ident(name) => {
                let slot_ptr = self.scope_lookup(name).map(|s| s.ptr);
                let is_local = slot_ptr.map_or(false, |p| {
                    self.droppable_scopes.iter().any(|sc| sc.iter().any(|s| s.ptr == p))
                });
                if is_local {
                    if let Some(p) = slot_ptr {
                        skip_ptrs.push(p);
                    }
                } else {
                    let is_ret_box = ret_ast.as_ref().map(Self::is_box_ast).unwrap_or(false);
                    let is_ret_vec = ret_ast.as_ref().map(Self::is_vec_ast).unwrap_or(false);
                    if is_ret_box && value.is_pointer_value() {
                        self.emit_retain_box(value.into_pointer_value())?;
                    } else if is_ret_vec && value.is_struct_value() {
                        let data = self
                            .builder
                            .build_extract_value(value.into_struct_value(), 0, "ret_vec_data")
                            .unwrap()
                            .into_pointer_value();
                        self.emit_retain_box(data)?;
                    }
                }
            }
            Expr::TupleLiteral(elements) => {
                let elem_types = match &ret_ast {
                    Some(Type::Tuple(ts)) => ts.clone(),
                    _ => Vec::new(),
                };
                for (i, elem_expr) in elements.iter().enumerate() {
                    let ast_ty = elem_types.get(i);
                    let is_box = ast_ty.map(Self::is_box_ast).unwrap_or(false);
                    let is_weak = ast_ty.map(Self::is_weak_ast).unwrap_or(false);
                    let is_vec = ast_ty.map(Self::is_vec_ast).unwrap_or(false);
                    if !is_box && !is_weak && !is_vec {
                        continue;
                    }
                    if let Expr::Ident(name) = elem_expr {
                        let slot_ptr = self.scope_lookup(name).map(|s| s.ptr);
                        let is_local = slot_ptr.map_or(false, |p| {
                            self.droppable_scopes.iter().any(|sc| sc.iter().any(|s| s.ptr == p))
                        });
                        if is_local {
                            if let Some(p) = slot_ptr {
                                skip_ptrs.push(p);
                            }
                            continue;
                        }
                    }
                    if !matches!(elem_expr, Expr::Call(_) | Expr::VecEmpty(_) | Expr::BoxAlloc(_) | Expr::Null)
                        && value.is_struct_value()
                    {
                        let fld = self
                            .builder
                            .build_extract_value(value.into_struct_value(), i as u32, "tup_elem")
                            .unwrap();
                        if is_weak && fld.is_pointer_value() {
                            self.emit_retain_weak(fld.into_pointer_value())?;
                        } else if is_box && fld.is_pointer_value() {
                            self.emit_retain_box(fld.into_pointer_value())?;
                        } else if is_vec && fld.is_struct_value() {
                            let data = self
                                .builder
                                .build_extract_value(fld.into_struct_value(), 0, "tup_vec_data")
                                .unwrap()
                                .into_pointer_value();
                            self.emit_retain_box(data)?;
                        }
                    }
                }
            }
            _ => {
                let is_ret_box = ret_ast.as_ref().map(Self::is_box_ast).unwrap_or(false);
                let is_ret_weak = ret_ast.as_ref().map(Self::is_weak_ast).unwrap_or(false);
                let is_ret_vec = ret_ast.as_ref().map(Self::is_vec_ast).unwrap_or(false);
                if is_ret_weak {
                    if !matches!(value_expr, Expr::Null) && value.is_pointer_value() {
                        self.emit_retain_weak(value.into_pointer_value())?;
                    }
                } else if is_ret_box {
                    if !matches!(value_expr, Expr::BoxAlloc(_) | Expr::Call(_) | Expr::Null)
                        && value.is_pointer_value()
                    {
                        self.emit_retain_box(value.into_pointer_value())?;
                    }
                } else if is_ret_vec {
                    if !matches!(value_expr, Expr::Call(_) | Expr::VecEmpty(_))
                        && value.is_struct_value()
                    {
                        let data = self
                            .builder
                            .build_extract_value(value.into_struct_value(), 0, "ret_vec_data")
                            .unwrap()
                            .into_pointer_value();
                        self.emit_retain_box(data)?;
                    }
                }
            }
        }

        Ok(skip_ptrs)
    }

    pub(super) fn compile_block(&mut self, block: &Block) -> Result<()> {
        self.push_scope();
        for stmt in &block.statements {
            self.compile_stmt(&stmt.node, stmt.span)?;
        }
        if self.at_open_end() {
            self.emit_drop_current_scope()?;
        }
        self.pop_scope();
        Ok(())
    }

    /// AST 类型 → (元素 LLVM 类型, 容器标记):`vec<T>` 取元素类型,
    /// map 族取 MAP_MARK 标记,其余无。供 let/形参/for-in/字段访问
    /// 共用的容器元数据解析。
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

    /// 值表达式的静态 AST 类型(能从函数签名得知时):调用与
    /// 模块限定调用取声明的返回类型。
    pub(super) fn static_type_of_value(&self, expr: &Expr) -> Option<Type> {
        match expr {
            Expr::Literal(Literal::Int(_)) => Some(Type::Named("i32".to_string())),
            Expr::Literal(Literal::Float(_)) => Some(Type::Named("f64".to_string())),
            Expr::Literal(Literal::String(_)) => Some(Type::Named("str".to_string())),
            Expr::Literal(Literal::Bool(_)) => Some(Type::Named("bool".to_string())),
            Expr::Literal(Literal::Char(_)) => Some(Type::Named("char".to_string())),
            Expr::Ident(name) => self.local_ast.get(name).cloned(),
            Expr::TupleLiteral(elems) => {
                let mut types = Vec::with_capacity(elems.len());
                for e in elems {
                    types.push(self.static_type_of_value(e)?);
                }
                Some(Type::Tuple(types))
            }
            Expr::Call(c) => match &*c.callee {
                Expr::Ident(fname) => {
                    let key = self.qualify_name(fname);
                    self.fn_return_ast.get(&key).cloned()
                }
                _ => None,
            },
            Expr::EnumConstruct(ec) => {
                let key = qname::qualified(&ec.enum_name, &ec.variant);
                self.fn_return_ast.get(&key).cloned()
            }
            Expr::Closure(c) => {
                let mut param_types = Vec::new();
                for p in &c.params {
                    param_types.push(p.param_type.clone().unwrap_or(Type::Named("i32".to_string())));
                }
                let ret = c.return_type.clone().unwrap_or(Type::Named("i32".to_string()));
                Some(Type::Fn(param_types, Box::new(ret)))
            }
            _ => None,
        }
    }

    /// 将语句块作为表达式编译：块的值等于其最后一条表达式语句的值。
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
