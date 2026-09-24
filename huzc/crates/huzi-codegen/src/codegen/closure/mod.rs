mod free_vars;
mod compile;

use super::CodeGen;
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
