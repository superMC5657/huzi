use super::CodeGen;
use super::VarSlot;
use huzi_ast::*;
use huzi_error::{HuziError, Result};
use inkwell::types::{BasicMetadataTypeEnum, BasicType, BasicTypeEnum};
use inkwell::values::{BasicMetadataValueEnum, BasicValueEnum, FunctionValue};
use inkwell::AddressSpace;
use std::collections::HashSet;

impl<'ctx> CodeGen<'ctx> {
    /// 静态查找闭包中引用的自由变量（非闭包形参、非内部 let 定义的外层局部变量）。
    pub(super) fn find_closure_free_variables(
        &self,
        closure: &ClosureExpr,
    ) -> Vec<(String, Type, BasicTypeEnum<'ctx>)> {
        let mut known: HashSet<String> =
            closure.params.iter().map(|p| p.name.clone()).collect();
        let mut free_vars = Vec::new();

        self.collect_free_vars_body(&closure.body, &mut known, &mut free_vars);
        free_vars
    }

    fn collect_free_vars_body(
        &self,
        body: &ClosureBody,
        known: &mut HashSet<String>,
        free_vars: &mut Vec<(String, Type, BasicTypeEnum<'ctx>)>,
    ) {
        match body {
            ClosureBody::Expr(e) => self.collect_free_vars_expr(e, known, free_vars),
            ClosureBody::Block(b) => self.collect_free_vars_block(b, known, free_vars),
        }
    }

    fn collect_free_vars_block(
        &self,
        block: &Block,
        known: &mut HashSet<String>,
        free_vars: &mut Vec<(String, Type, BasicTypeEnum<'ctx>)>,
    ) {
        for stmt in &block.statements {
            match &stmt.node {
                Stmt::Let(l) => {
                    if let Some(val) = &l.value {
                        self.collect_free_vars_expr(val, known, free_vars);
                    }
                    for (name, _) in l.bound_names() {
                        if name != "_" {
                            known.insert(name.to_string());
                        }
                    }
                }
                Stmt::Expr(e) => self.collect_free_vars_expr(&e.expr, known, free_vars),
                Stmt::Return(r) => {
                    if let Some(val) = &r.value {
                        self.collect_free_vars_expr(val, known, free_vars);
                    }
                }
                Stmt::If(i) => {
                    self.collect_free_vars_expr(&i.condition, known, free_vars);
                    self.collect_free_vars_block(&i.then_branch, known, free_vars);
                    for (cond, blk) in &i.elif_branches {
                        self.collect_free_vars_expr(cond, known, free_vars);
                        self.collect_free_vars_block(blk, known, free_vars);
                    }
                    if let Some(eb) = &i.else_branch {
                        self.collect_free_vars_block(eb, known, free_vars);
                    }
                }
                Stmt::While(w) => {
                    self.collect_free_vars_expr(&w.condition, known, free_vars);
                    self.collect_free_vars_block(&w.body, known, free_vars);
                }
                Stmt::For(f) => {
                    match &f.source {
                        ForSource::Range { start, end } => {
                            self.collect_free_vars_expr(start, known, free_vars);
                            self.collect_free_vars_expr(end, known, free_vars);
                        }
                        ForSource::Array(a) => {
                            self.collect_free_vars_expr(a, known, free_vars);
                        }
                    }
                    let mut for_known = known.clone();
                    for_known.insert(f.var_name.clone());
                    self.collect_free_vars_block(&f.body, &mut for_known, free_vars);
                }
                _ => {}
            }
        }
    }

    fn collect_free_vars_expr(
        &self,
        expr: &Expr,
        known: &mut HashSet<String>,
        free_vars: &mut Vec<(String, Type, BasicTypeEnum<'ctx>)>,
    ) {
        match expr {
            Expr::Ident(name) => {
                if !known.contains(name) && !free_vars.iter().any(|(n, _, _)| n == name) {
                    if let Some(slot) = self.scope_lookup(name) {
                        let ast_ty = self
                            .local_ast
                            .get(name)
                            .cloned()
                            .unwrap_or(Type::Named("i32".to_string()));
                        free_vars.push((name.clone(), ast_ty, slot.ty));
                    }
                }
            }
            Expr::Binary(b) => {
                self.collect_free_vars_expr(&b.left, known, free_vars);
                self.collect_free_vars_expr(&b.right, known, free_vars);
            }
            Expr::Unary(u) => self.collect_free_vars_expr(&u.operand, known, free_vars),
            Expr::Call(c) => {
                self.collect_free_vars_expr(&c.callee, known, free_vars);
                for arg in &c.arguments {
                    self.collect_free_vars_expr(arg, known, free_vars);
                }
            }
            Expr::MethodCall(m) => {
                self.collect_free_vars_expr(&m.receiver, known, free_vars);
                for arg in &m.arguments {
                    self.collect_free_vars_expr(arg, known, free_vars);
                }
            }
            Expr::ArrayIndex(a) => {
                self.collect_free_vars_expr(&a.array, known, free_vars);
                self.collect_free_vars_expr(&a.index, known, free_vars);
            }
            Expr::Assign(a) => {
                self.collect_free_vars_expr(&a.target, known, free_vars);
                self.collect_free_vars_expr(&a.value, known, free_vars);
            }
            Expr::If(i) => {
                self.collect_free_vars_expr(&i.condition, known, free_vars);
                self.collect_free_vars_block(&i.then_branch, known, free_vars);
                self.collect_free_vars_block(&i.else_branch, known, free_vars);
            }
            Expr::Closure(c) => {
                let mut inner_known = known.clone();
                for p in &c.params {
                    inner_known.insert(p.name.clone());
                }
                self.collect_free_vars_body(&c.body, &mut inner_known, free_vars);
            }
            Expr::ArrayLiteral(elems) | Expr::TupleLiteral(elems) => {
                for e in elems {
                    self.collect_free_vars_expr(e, known, free_vars);
                }
            }
            _ => {}
        }
    }

    /// 编译闭包构造表达式，返回 `{ fn_ptr, env_ptr }` 胖指针值。
    pub(super) fn compile_closure(
        &mut self,
        closure: &ClosureExpr,
    ) -> Result<BasicValueEnum<'ctx>> {
        let free_vars = self.find_closure_free_variables(closure);

        // 1. 确定闭包参数与返回值 LLVM 类型
        let ret_llvm_ty = if let Some(ret_ast) = &closure.return_type {
            self.type_to_llvm(ret_ast)?
        } else {
            self.infer_closure_body_return_type(&closure.body)?
        };

        let mut param_llvms: Vec<BasicMetadataTypeEnum<'ctx>> =
            Vec::with_capacity(closure.params.len() + 1);
        // 第一个形参为环境指针 `env: ptr`
        param_llvms.push(self.context.ptr_type(AddressSpace::default()).into());
        for p in &closure.params {
            let ty = if let Some(pt) = &p.param_type {
                self.type_to_llvm(pt)?
            } else {
                self.context.i32_type().into()
            };
            param_llvms.push(ty.into());
        }

        let fn_name = format!("__huzi_closure_{}", self.closure_counter);
        self.closure_counter += 1;
        let fn_ty = ret_llvm_ty.fn_type(&param_llvms, false);
        let closure_fn = self.module.add_function(&fn_name, fn_ty, None);

        // 2. 编译闭包函数体代码
        let cur_bb = self.builder.get_insert_block();
        let prev_ret_ty = self.current_return_type;
        let prev_ret_ast = self.current_return_ast.clone();
        self.current_return_type = Some(ret_llvm_ty);
        self.current_return_ast = closure.return_type.clone();

        let entry_bb = self.context.append_basic_block(closure_fn, "entry");
        self.builder.position_at_end(entry_bb);
        self.scopes.push(std::collections::HashMap::new());

        // 绑定捕获的自由变量
        let env_fields: Vec<BasicTypeEnum<'ctx>> = free_vars.iter().map(|(_, _, ty)| *ty).collect();
        let env_struct_ty = self.context.struct_type(&env_fields, false);
        if !free_vars.is_empty() {
            let env_param = closure_fn.get_first_param().unwrap().into_pointer_value();
            for (i, (name, ast_ty, llvm_ty)) in free_vars.iter().enumerate() {
                let gep = self
                    .builder
                    .build_struct_gep(env_struct_ty, env_param, i as u32, &format!("gep_{}", name))
                    .map_err(|_| HuziError::new_global("Failed to GEP closure env"))?;
                let loaded = self
                    .builder
                    .build_load(*llvm_ty, gep, &format!("cap_{}", name))
                    .map_err(|_| HuziError::new_global("Failed to load capture"))?;
                let slot = self.build_alloca(*llvm_ty, name)?;
                self.builder.build_store(slot, loaded).map_err(|_| HuziError::new_global("Failed to store capture"))?;
                self.scope_insert(
                    name.clone(),
                    VarSlot {
                        ptr: slot,
                        ty: *llvm_ty,
                        elem: None,
                        array_len: None,
                        mutable: true,
                        box_inner: None,
                        map_kind: None,
                    },
                );
                self.local_ast.insert(name.clone(), ast_ty.clone());
            }
        }

        // 绑定形参
        for (i, p) in closure.params.iter().enumerate() {
            let param_val = closure_fn.get_nth_param((i + 1) as u32).unwrap();
            let param_ty = param_val.get_type();
            let slot = self.build_alloca(param_ty, &p.name)?;
            self.builder.build_store(slot, param_val).map_err(|_| HuziError::new_global("Failed to store param"))?;
            self.scope_insert(
                p.name.clone(),
                VarSlot {
                    ptr: slot,
                    ty: param_ty,
                    elem: None,
                    array_len: None,
                    mutable: true,
                    box_inner: None,
                    map_kind: None,
                },
            );
            if let Some(pt) = &p.param_type {
                self.local_ast.insert(p.name.clone(), pt.clone());
            }
        }

        // 编译主体
        match &closure.body {
            ClosureBody::Expr(e) => {
                let ret_val = self.compile_expr(e)?;
                self.builder.build_return(Some(&ret_val)).map_err(|_| HuziError::new_global("Failed to build return"))?;
            }
            ClosureBody::Block(b) => {
                self.compile_block(b)?;
                if let Some(bb) = self.builder.get_insert_block() {
                    if bb.get_terminator().is_none() {
                        let zero = self.context.i32_type().const_zero();
                        self.builder.build_return(Some(&zero)).map_err(|_| HuziError::new_global("Failed to build return"))?;
                    }
                }
            }
        }

        self.scopes.pop();
        self.current_return_type = prev_ret_ty;
        self.current_return_ast = prev_ret_ast;
        if let Some(bb) = cur_bb {
            self.builder.position_at_end(bb);
        }

        // 3. 在调用点创建闭包环境并构造 `{ fn_ptr, env_ptr }`
        let env_ptr = if free_vars.is_empty() {
            self.context.ptr_type(AddressSpace::default()).const_null()
        } else {
            let env_alloc = self
                .builder
                .build_malloc(env_struct_ty, "env_alloc")
                .map_err(|_| HuziError::new_global("Failed to malloc closure env"))?;
            for (i, (name, _, _)) in free_vars.iter().enumerate() {
                let val = self.compile_expr(&Expr::Ident(name.clone()))?;
                let gep = self
                    .builder
                    .build_struct_gep(env_struct_ty, env_alloc, i as u32, &format!("env_store_{}", name))
                    .map_err(|_| HuziError::new_global("Failed to GEP env store"))?;
                self.builder.build_store(gep, val).map_err(|_| HuziError::new_global("Failed to store env field"))?;
            }
            env_alloc
        };

        let closure_ty = self.closure_struct_type();
        let mut val = closure_ty.get_undef();
        val = self
            .builder
            .build_insert_value(val, closure_fn.as_global_value().as_pointer_value(), 0, "fn_ptr")
            .map_err(|_| HuziError::new_global("Failed to insert fn_ptr"))?
            .into_struct_value();
        val = self
            .builder
            .build_insert_value(val, env_ptr, 1, "env_ptr")
            .map_err(|_| HuziError::new_global("Failed to insert env_ptr"))?
            .into_struct_value();

        Ok(val.into())
    }

    fn infer_closure_body_return_type(&self, body: &ClosureBody) -> Result<BasicTypeEnum<'ctx>> {
        match body {
            ClosureBody::Expr(e) => match &**e {
                Expr::Literal(Literal::Bool(_)) => Ok(self.context.bool_type().into()),
                Expr::Literal(Literal::Float(_)) => Ok(self.context.f64_type().into()),
                Expr::Literal(Literal::String(_)) => {
                    Ok(self.context.ptr_type(AddressSpace::default()).into())
                }
                _ => Ok(self.context.i32_type().into()),
            },
            ClosureBody::Block(_) => Ok(self.context.i32_type().into()),
        }
    }

    /// 执行闭包间接调用：提取 `{ fn_ptr, env_ptr }`，将 `env_ptr` 作为首参数调用。
    pub(super) fn compile_closure_call(
        &mut self,
        callee_expr: &Expr,
        arguments: &[Expr],
    ) -> Result<BasicValueEnum<'ctx>> {
        let callee_val = self.compile_expr(callee_expr)?;
        let closure_struct = match callee_val {
            BasicValueEnum::StructValue(s) => s,
            _ => return Err(HuziError::new_global("Expected closure or function value")),
        };

        if let Expr::Ident(name) = callee_expr {
            if let Some(Type::Fn(expected_params, _)) = self.local_ast.get(name) {
                if arguments.len() != expected_params.len() {
                    return Err(HuziError::new_global(format!(
                        "Closure '{}' expects {} argument(s), got {}",
                        name,
                        expected_params.len(),
                        arguments.len()
                    )));
                }
            }
        }

        let fn_ptr = self
            .builder
            .build_extract_value(closure_struct, 0, "fn_ptr")
            .map_err(|_| HuziError::new_global("Failed to extract fn_ptr"))?
            .into_pointer_value();
        let env_ptr = self
            .builder
            .build_extract_value(closure_struct, 1, "env_ptr")
            .map_err(|_| HuziError::new_global("Failed to extract env_ptr"))?
            .into_pointer_value();

        let mut call_args: Vec<BasicMetadataValueEnum<'ctx>> = Vec::with_capacity(arguments.len() + 1);
        call_args.push(env_ptr.into());
        for arg in arguments {
            let val = self.compile_expr(arg)?;
            call_args.push(val.into());
        }

        let (ret_llvm, param_llvms) = self.resolve_closure_call_signature(callee_expr, &call_args)?;
        let fn_type = ret_llvm.fn_type(&param_llvms, false);

        let call_res = self
            .builder
            .build_indirect_call(fn_type, fn_ptr, &call_args, "closure_call")
            .map_err(|_| HuziError::new_global("Failed to build closure indirect call"))?;

        Ok(call_res
            .try_as_basic_value()
            .left()
            .unwrap_or_else(|| self.context.i32_type().const_zero().into()))
    }

    fn resolve_closure_call_signature(
        &self,
        callee_expr: &Expr,
        call_args: &[BasicMetadataValueEnum<'ctx>],
    ) -> Result<(BasicTypeEnum<'ctx>, Vec<BasicMetadataTypeEnum<'ctx>>)> {
        let ptr_ty: BasicMetadataTypeEnum<'ctx> = self.context.ptr_type(AddressSpace::default()).into();
        let mut param_llvms: Vec<BasicMetadataTypeEnum<'ctx>> = vec![ptr_ty];
        for a in &call_args[1..] {
            let ty: BasicMetadataTypeEnum<'ctx> = if a.is_int_value() {
                a.into_int_value().get_type().into()
            } else if a.is_float_value() {
                a.into_float_value().get_type().into()
            } else if a.is_pointer_value() {
                a.into_pointer_value().get_type().into()
            } else if a.is_struct_value() {
                a.into_struct_value().get_type().into()
            } else {
                self.context.i32_type().into()
            };
            param_llvms.push(ty);
        }

        let ret_llvm = if let Expr::Ident(name) = callee_expr {
            if let Some(Type::Fn(_, ret_ast)) = self.local_ast.get(name) {
                self.type_to_llvm(ret_ast)?
            } else {
                self.context.i32_type().into()
            }
        } else {
            self.context.i32_type().into()
        };

        Ok((ret_llvm, param_llvms))
    }

    /// 为顶层命名函数合成 thunk 适配器：`__huzi_thunk_<name>(env: ptr, args...) -> ret`
    pub(super) fn ensure_function_thunk(&mut self, fn_name: &str) -> Result<FunctionValue<'ctx>> {
        let lookup_key = self.qualify_name(fn_name);
        if let Some(thunk) = self.thunk_cache.get(&lookup_key) {
            return Ok(*thunk);
        }

        let (target_fn, param_types) = self
            .functions
            .get(&lookup_key)
            .cloned()
            .ok_or_else(|| self.unknown_function_error(fn_name))?;

        let thunk_name = format!("__huzi_thunk_{}", lookup_key.replace("::", "_"));
        let mut thunk_params: Vec<BasicMetadataTypeEnum<'ctx>> =
            vec![self.context.ptr_type(AddressSpace::default()).into()];
        for p in &param_types {
            thunk_params.push((*p).into());
        }

        let ret_ty = target_fn.get_type().get_return_type().unwrap_or_else(|| self.context.i32_type().into());
        let thunk_fn_ty = ret_ty.fn_type(&thunk_params, false);
        let thunk_fn = self.module.add_function(&thunk_name, thunk_fn_ty, None);

        let cur_bb = self.builder.get_insert_block();
        let entry_bb = self.context.append_basic_block(thunk_fn, "entry");
        self.builder.position_at_end(entry_bb);

        let mut call_args = Vec::with_capacity(param_types.len());
        for i in 0..param_types.len() {
            let param_val = thunk_fn.get_nth_param((i + 1) as u32).unwrap();
            call_args.push(param_val.into());
        }

        let call_res = self
            .builder
            .build_call(target_fn, &call_args, "f_call")
            .map_err(|_| HuziError::new_global("Failed to build thunk call"))?;

        if let Some(ret_val) = call_res.try_as_basic_value().left() {
            self.builder.build_return(Some(&ret_val)).map_err(|_| HuziError::new_global("Failed to build thunk ret"))?;
        } else {
            let zero = self.context.i32_type().const_zero();
            self.builder.build_return(Some(&zero)).map_err(|_| HuziError::new_global("Failed to build thunk ret"))?;
        }

        if let Some(bb) = cur_bb {
            self.builder.position_at_end(bb);
        }

        self.thunk_cache.insert(lookup_key, thunk_fn);
        Ok(thunk_fn)
    }

    /// 将命名函数名升格为一等闭包值：`{ thunk_ptr, null }`
    pub(super) fn get_named_function_closure(&mut self, fn_name: &str) -> Result<BasicValueEnum<'ctx>> {
        let thunk = self.ensure_function_thunk(fn_name)?;
        let env_ptr = self.context.ptr_type(AddressSpace::default()).const_null();

        let closure_ty = self.closure_struct_type();
        let mut val = closure_ty.get_undef();
        val = self
            .builder
            .build_insert_value(val, thunk.as_global_value().as_pointer_value(), 0, "fn_ptr")
            .map_err(|_| HuziError::new_global("Failed to insert thunk fn_ptr"))?
            .into_struct_value();
        val = self
            .builder
            .build_insert_value(val, env_ptr, 1, "env_ptr")
            .map_err(|_| HuziError::new_global("Failed to insert thunk env_ptr"))?
            .into_struct_value();

        Ok(val.into())
    }
}
