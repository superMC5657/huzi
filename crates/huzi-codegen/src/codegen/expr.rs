use super::CodeGen;
use huzi_ast::*;
use huzi_error::{HuziError, Result};

impl<'ctx> CodeGen<'ctx> {
    pub(super) fn compile_expr(&mut self, expr: &Expr) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        match expr {
            Expr::Literal(lit) => self.compile_literal(lit),
            Expr::Ident(name) => match self.scope_lookup(name) {
                Some(slot) => {
                    let loaded = self
                        .builder
                        .build_load(slot.ty, slot.ptr, "load")
                        .unwrap();
                    Ok(loaded)
                }
                None => Err(self.unknown_variable_error(name)),
            },
            Expr::Binary(bin_expr) => self.compile_binary(bin_expr),
            Expr::Unary(unary_expr) => self.compile_unary(unary_expr),
            Expr::Call(call_expr) => self.compile_call(call_expr),
            Expr::Assign(assign_expr) => self.compile_assign(assign_expr),
            Expr::ArrayIndex(idx_expr) => self.compile_array_index(idx_expr),
            Expr::ArrayLiteral(elements) => self.compile_array_literal(elements),
            Expr::VecEmpty(elem_ty) => self.compile_vec_empty_value(elem_ty),
            Expr::BoxAlloc(inner) => Ok(self.compile_box_alloc(inner, None)?.0),
            Expr::Null => self.compile_null(),
            Expr::TupleLiteral(elements) => self.compile_tuple_literal(elements),
            Expr::If(if_expr) => self.compile_if_expr(if_expr),
            Expr::FieldAccess(fa) => self.compile_field_access(fa),
            Expr::StructLiteral(sl) => self.compile_struct_literal(sl),
            Expr::EnumConstruct(ec) => self.compile_enum_construct(ec),
            Expr::Match(m) => self.compile_match_expr(m),
            Expr::MethodCall(mc) => Err(HuziError::new_global(format!(
                "Unresolved method call '{}'",
                mc.method
            ))),
        }
    }

    pub(super) fn compile_literal(&self, lit: &Literal) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        match lit {
            Literal::Int(n) => {
                // Integers that fit in i32 use i32; larger ones use i64.
                if *n >= i32::MIN as i64 && *n <= i32::MAX as i64 {
                    Ok(self.context.i32_type().const_int(*n as u64, false).into())
                } else {
                    Ok(self.context.i64_type().const_int(*n as u64, false).into())
                }
            }
            Literal::Float(f) => Ok(self.context.f64_type().const_float(*f).into()),
            Literal::Bool(b) => Ok(self.context.bool_type().const_int(*b as u64, false).into()),
            Literal::String(s) => {
                let g = unsafe { self.builder.build_global_string(s, "str").unwrap() };
                Ok(g.as_pointer_value().into())
            }
            Literal::Char(c) => Ok(self.context.i8_type().const_int(*c as u64, false).into()),
        }
    }

    pub(super) fn compile_short_circuit(
        &mut self,
        left: &Expr,
        right: &Expr,
        is_and: bool,
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        let function = self.current_function()?;

        let rhs_block = self.context.append_basic_block(function, "sc_rhs");
        let short_block = self.context.append_basic_block(function, "sc_short");
        let end_block = self.context.append_basic_block(function, "sc_end");

        let result_ptr = self.build_alloca(self.context.bool_type().into(), "sc_result")?;

        let lhs_value = self.compile_expr(left)?;
        let lhs = self.to_i1(lhs_value)?;
        if is_and {
            self.builder
                .build_conditional_branch(lhs, rhs_block, short_block)
                .unwrap();
        } else {
            self.builder
                .build_conditional_branch(lhs, short_block, rhs_block)
                .unwrap();
        }

        // Short-circuit branch: result is false (for &&) or true (for ||).
        self.builder.position_at_end(short_block);
        let short_val = self.context.bool_type().const_int(!is_and as u64, false);
        self.builder.build_store(result_ptr, short_val).unwrap();
        self.builder
            .build_unconditional_branch(end_block)
            .unwrap();

        // Evaluate the right operand only when needed.
        self.builder.position_at_end(rhs_block);
        let rhs_value = self.compile_expr(right)?;
        let rhs = self.to_i1(rhs_value)?;
        self.builder.build_store(result_ptr, rhs).unwrap();
        self.builder
            .build_unconditional_branch(end_block)
            .unwrap();

        self.builder.position_at_end(end_block);
        let result = self
            .builder
            .build_load(self.context.bool_type(), result_ptr, "sc_load")
            .unwrap();

        Ok(result)
    }

    pub(super) fn compile_unary(
        &mut self,
        expr: &UnaryExpr,
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        let operand = self.compile_expr(&expr.operand)?;

        let value = match expr.operator {
            UnOp::Neg => {
                if operand.is_int_value() {
                    self.builder
                        .build_int_neg(operand.into_int_value(), "neg")
                        .unwrap()
                        .into()
                } else {
                    self.builder
                        .build_float_neg(operand.into_float_value(), "fneg")
                        .unwrap()
                        .into()
                }
            }
            UnOp::Not => {
                let cond = self.to_i1(operand)?;
                self.builder.build_not(cond, "not").unwrap().into()
            }
        };

        Ok(value)
    }

    pub(super) fn compile_call(
        &mut self,
        expr: &CallExpr,
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        let callee_name = match &*expr.callee {
            Expr::Ident(name) => name.clone(),
            _ => return Err(HuziError::new_global("Expected function name")),
        };

        // Built-in functions
        match callee_name.as_str() {
            "print" => return self.compile_print(&expr.arguments),
            "read_line" => return self.compile_read_line(),
            "read_int" => return self.compile_read_int(),
            "read_float" => return self.compile_read_float(),
            "len" => return self.compile_len(&expr.arguments),
            "abs" => return self.compile_abs(&expr.arguments),
            "sqrt" => return self.compile_libm_unary("sqrt", &expr.arguments),
            "pow" => return self.compile_pow(&expr.arguments),
            "sin" => return self.compile_libm_unary("sin", &expr.arguments),
            "cos" => return self.compile_libm_unary("cos", &expr.arguments),
            "tan" => return self.compile_libm_unary("tan", &expr.arguments),
            "floor" => return self.compile_libm_unary("floor", &expr.arguments),
            "ceil" => return self.compile_libm_unary("ceil", &expr.arguments),
            "round" => return self.compile_libm_unary("round", &expr.arguments),
            "concat" => return self.compile_concat(&expr.arguments),
            "split" => return self.compile_split(&expr.arguments),
            "substring" => return self.compile_substring(&expr.arguments),
            "trim" => return self.compile_trim(&expr.arguments),
            "contains" => return self.compile_contains(&expr.arguments),
            "to_string" => return self.compile_to_string(&expr.arguments),
            "parse_int" => return self.compile_parse_int(&expr.arguments),
            "parse_float" => return self.compile_parse_float(&expr.arguments),
            "arg_count" => return self.compile_arg_count(),
            "arg" => return self.compile_arg(&expr.arguments),
            "arg_ok" => return self.compile_arg_ok(&expr.arguments),
            "env_get" => return self.compile_env_get(&expr.arguments),
            "is_eof" => return self.compile_is_eof(),
            "rand" => return self.compile_rand(),
            "srand" => return self.compile_srand(&expr.arguments),
            "time" => return self.compile_time(),
            "localtime" => return self.compile_localtime(&expr.arguments),
            "exit" => return self.compile_exit(&expr.arguments),
            "panic" => return self.compile_panic(&expr.arguments),
            "sleep_ms" => return self.compile_sleep_ms(&expr.arguments),
            "read_file" => return self.compile_read_file(&expr.arguments),
            "read_file_ok" => return self.compile_read_file_ok(&expr.arguments),
            "read_file_err" => return self.compile_read_file_err(&expr.arguments),
            "write_file" => return self.compile_write_file(&expr.arguments),
            "vec" => return self.compile_vec_ctor(&expr.arguments),
            "map_new" => return self.compile_map_new(&expr.arguments),
            "map_put" => return self.compile_map_put(&expr.arguments),
            "map_get" => return self.compile_map_get(&expr.arguments),
            "map_has" => return self.compile_map_has(&expr.arguments),
            "map_remove" => return self.compile_map_remove(&expr.arguments),
            "map_len" => return self.compile_map_len(&expr.arguments),
            "map_keys" => return self.compile_map_keys(&expr.arguments),
            "push" => return self.compile_vec_push(&expr.arguments),
            "pop" => return self.compile_vec_pop(&expr.arguments),
            "remove" => return self.compile_vec_remove(&expr.arguments),
            "insert" => return self.compile_vec_insert(&expr.arguments),
            "clear" => return self.compile_vec_clear(&expr.arguments),
            "free_str" => return self.compile_free_str(&expr.arguments),
            "free_vec" => return self.compile_free_vec(&expr.arguments),
            "free_box" => return self.compile_free_box(&expr.arguments),
            "ref_count" => return self.compile_ref_count(&expr.arguments),
            "tcp_connect" => return self.compile_tcp_connect(&expr.arguments),
            "tcp_send" => return self.compile_tcp_send(&expr.arguments),
            "tcp_recv" => return self.compile_tcp_recv(&expr.arguments),
            "tcp_close" => return self.compile_tcp_close(&expr.arguments),
            "tcp_listen" => return self.compile_tcp_listen(&expr.arguments),
            "tcp_accept" => return self.compile_tcp_accept(&expr.arguments),
            "spawn" | "thread_spawn" => return self.compile_spawn(&expr.arguments),
            "join" | "thread_join" => return self.compile_join(&expr.arguments),
            _ => {}
        }

        // 模块内调用同模块函数时按 `模块::名` 查找;主程序中限定名
        // (`mod::fn`,由模块调度传入)本身已带前缀,qualify 原样返回。
        let lookup_key = self.qualify_name(&callee_name);
        let (function, param_types) = self
            .functions
            .get(&lookup_key)
            .cloned()
            .ok_or_else(|| self.unknown_function_error(&callee_name))?;

        if expr.arguments.len() != param_types.len() {
            return Err(HuziError::new_global(format!(
                "Function '{}' expects {} argument(s), got {}",
                callee_name,
                param_types.len(),
                expr.arguments.len()
            )));
        }

        let mut args: Vec<inkwell::values::BasicMetadataValueEnum> = Vec::new();
        // `box(..)`/`null` 实参与 `Box<T>` 形参的 AST 精确校验(LLVM 层面
        // 都是指针,结构名错配只能在这里发现)。
        if let Some(ast_params) = self.fn_param_ast.get(&lookup_key).cloned() {
            for (arg_expr, expected) in expr.arguments.iter().zip(ast_params.iter()) {
                self.check_box_assignable(arg_expr, expected)?;
            }
        }
        let ast_params_opt = self.fn_param_ast.get(&lookup_key).cloned();
        for (idx, (arg_expr, param_type)) in expr.arguments.iter().zip(param_types.iter()).enumerate() {
            let is_container = ast_params_opt.as_ref().map_or(false, |ast_p| {
                idx < ast_p.len() && Self::is_container_handle_type(&ast_p[idx])
            });
            if is_container {
                let ptr_val = match arg_expr {
                    Expr::Ident(name) => {
                        let slot = self
                            .scope_lookup(name)
                            .ok_or_else(|| self.unknown_variable_error(name))?;
                        slot.ptr
                    }
                    Expr::FieldAccess(fa) => {
                        let (base_ptr, base_ty) = self.compile_addr_deref(&fa.base)?;
                        let (field_ptr, _) = self.gep_field(base_ptr, base_ty, &fa.field)?;
                        field_ptr
                    }
                    _ => {
                        let val = self.compile_expr(arg_expr)?;
                        let tmp = self.build_alloca(self.vec_struct_type().into(), "tmp_container_arg")?;
                        self.builder.build_store(tmp, val).unwrap();
                        tmp
                    }
                };
                args.push(ptr_val.into());
            } else {
                let value = self.compile_expr(arg_expr)?;
                let value = self.coerce_value(*param_type, value)?;
                args.push(value.into());
            }
        }

        let call = self.builder.build_call(function, &args, "call").unwrap();

        Ok(call.try_as_basic_value().unwrap_left())
    }
}
