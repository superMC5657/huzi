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
                None => {
                    let qname = self.qualify_name(name);
                    if self.functions.contains_key(&qname) {
                        self.get_named_function_closure(name)
                    } else {
                        Err(self.unknown_variable_error(name))
                    }
                }
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
            Expr::Try(t) => self.compile_try(t),
            Expr::Closure(c) => self.compile_closure(c),
            Expr::FString(_) => unreachable!("f-string must be desugared before codegen"),
        }
    }

    pub(super) fn compile_literal(&self, lit: &Literal) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        match lit {
            Literal::Int(n) => {
                // 能容纳在 i32 内的整数使用 i32；更大的使用 i64。
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

        // 短路分支：对于 && 结果为 false，对于 || 结果为 true。
        self.builder.position_at_end(short_block);
        let short_val = self.context.bool_type().const_int(!is_and as u64, false);
        self.builder.build_store(result_ptr, short_val).unwrap();
        self.builder
            .build_unconditional_branch(end_block)
            .unwrap();

        // 仅在必要时才对右操作数求值。
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
        // 解引用独占路径(内部自行编译操作数并逐层空检查)。
        if expr.operator == UnOp::Deref {
            return self.compile_deref(&expr.operand);
        }
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
            // 解引用已在函数入口独占返回,此处不可达。
            UnOp::Deref => unreachable!("deref handled above"),
        };

        Ok(value)
    }

    pub(super) fn compile_call(
        &mut self,
        expr: &CallExpr,
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        let callee_name = match &*expr.callee {
            Expr::Ident(name) => {
                if self.scope_lookup(name).is_some() {
                    return self.compile_closure_call(&expr.callee, &expr.arguments);
                }
                name.clone()
            }
            _ => return self.compile_closure_call(&expr.callee, &expr.arguments),
        };

        if let Some(res) = self.compile_builtin_call(&callee_name, &expr.arguments) {
            return res;
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
            let is_container = ast_params_opt.as_ref().is_some_and(|ast_p| {
                idx < ast_p.len() && Self::is_container_handle_type(&ast_p[idx])
            });
            if is_container {
                let expected_ast = &ast_params_opt.as_ref().unwrap()[idx];
                let ptr_val = self.compile_container_arg_ptr(arg_expr, expected_ast)?;
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

    /// 编译并校验传给容器句柄形参的指针实参。
    fn compile_container_arg_ptr(
        &mut self,
        arg_expr: &Expr,
        expected_ty: &Type,
    ) -> Result<inkwell::values::PointerValue<'ctx>> {
        let is_map = matches!(expected_ty, Type::Named(ref n) | Type::Applied(ref n, _) if n == "Map" || n == "HashMap" || n == "map");
        match arg_expr {
            Expr::Ident(name) => {
                let slot = self
                    .scope_lookup(name)
                    .ok_or_else(|| self.unknown_variable_error(name))?;
                if is_map {
                    if !Self::is_map_slot(&slot) {
                        return Err(HuziError::new_global(format!(
                            "Type mismatch: expected Map, found '{}'",
                            name
                        )));
                    }
                    // Map 键值特化须一致(`Map<str,str>` 实参不可进 `Map` 形参)。
                    if let Some(expected_kind) = super::MapKind::from_ast(expected_ty) {
                        let actual_kind = slot.map_kind.unwrap_or(super::MapKind::StrI32);
                        if actual_kind != expected_kind {
                            return Err(HuziError::new_global(format!(
                                "Type mismatch: expected Map({}), found Map({})",
                                expected_kind.display(),
                                actual_kind.display()
                            )));
                        }
                    }
                } else {
                    if !Self::is_vec_slot(&slot) {
                        return Err(HuziError::new_global(format!(
                            "Type mismatch: expected vec, found '{}'",
                            name
                        )));
                    }
                    if let Type::Applied(_, ref type_args) = expected_ty {
                        if let Some(expected_elem_ast) = type_args.first() {
                            let expected_elem_ty = self.type_to_llvm(expected_elem_ast)?;
                            if slot.elem != Some(expected_elem_ty) {
                                return Err(HuziError::new_global(
                                    "Type mismatch: vec element type does not match",
                                ));
                            }
                        }
                    }
                }
                Ok(slot.ptr)
            }
            Expr::FieldAccess(fa) => {
                let (base_ptr, base_ty) = self.compile_addr_deref(&fa.base)?;
                let (field_ptr, field_ty) = self.gep_field(base_ptr, base_ty, &fa.field)?;
                if field_ty != self.vec_struct_type().into() {
                    return Err(HuziError::new_global(
                        "Type mismatch: field is not a container",
                    ));
                }
                // 字段为 map 时校验键值特化一致。
                if is_map {
                    if let Some(field_ast) = self.field_ast_type(&fa.base, &fa.field) {
                        if let (Some(exp_k), Some(act_k)) = (
                            super::MapKind::from_ast(expected_ty),
                            super::MapKind::from_ast(&field_ast),
                        ) {
                            if exp_k != act_k {
                                return Err(HuziError::new_global(format!(
                                    "Type mismatch: expected Map({}), found Map({})",
                                    exp_k.display(),
                                    act_k.display()
                                )));
                            }
                        }
                    }
                }
                Ok(field_ptr)
            }
            _ => {
                let val = self.compile_expr(arg_expr)?;
                if !val.is_struct_value() {
                    return Err(HuziError::new_global("Type mismatch: expected container value"));
                }
                let tmp = self.build_alloca(self.vec_struct_type().into(), "tmp_container_arg")?;
                self.builder.build_store(tmp, val).unwrap();
                Ok(tmp)
            }
        }
    }

    /// 内置函数调用分派。如果匹配到内置函数则返回 Some(Result)，否则返回 None。
    fn compile_builtin_call(
        &mut self,
        name: &str,
        arguments: &[Expr],
    ) -> Option<Result<inkwell::values::BasicValueEnum<'ctx>>> {
        match name {
            "print" => Some(self.compile_print(arguments)),
            "read_line" => Some(self.compile_read_line()),
            "read_int" => Some(self.compile_read_int()),
            "read_float" => Some(self.compile_read_float()),
            "len" => Some(self.compile_len(arguments)),
            "abs" => Some(self.compile_abs(arguments)),
            "sqrt" => Some(self.compile_libm_unary("sqrt", arguments)),
            "pow" => Some(self.compile_pow(arguments)),
            "sin" => Some(self.compile_libm_unary("sin", arguments)),
            "cos" => Some(self.compile_libm_unary("cos", arguments)),
            "tan" => Some(self.compile_libm_unary("tan", arguments)),
            "floor" => Some(self.compile_libm_unary("floor", arguments)),
            "ceil" => Some(self.compile_libm_unary("ceil", arguments)),
            "round" => Some(self.compile_libm_unary("round", arguments)),
            "concat" => Some(self.compile_concat(arguments)),
            "split" => Some(self.compile_split(arguments)),
            "substring" => Some(self.compile_substring(arguments)),
            "trim" => Some(self.compile_trim(arguments)),
            "contains" => Some(self.compile_contains(arguments)),
            "to_string" => Some(self.compile_to_string(arguments)),
            "parse_int" => Some(self.compile_parse_int(arguments)),
            "parse_float" => Some(self.compile_parse_float(arguments)),
            "arg_count" => Some(self.compile_arg_count()),
            "arg" => Some(self.compile_arg(arguments)),
            "arg_ok" => Some(self.compile_arg_ok(arguments)),
            "env_get" => Some(self.compile_env_get(arguments)),
            "is_eof" => Some(self.compile_is_eof()),
            "rand" => Some(self.compile_rand()),
            "srand" => Some(self.compile_srand(arguments)),
            "time" => Some(self.compile_time()),
            "localtime" => Some(self.compile_localtime(arguments)),
            "exit" => Some(self.compile_exit(arguments)),
            "panic" => Some(self.compile_panic(arguments)),
            "sleep_ms" => Some(self.compile_sleep_ms(arguments)),
            "read_file" => Some(self.compile_read_file(arguments)),
            "read_file_ok" => Some(self.compile_read_file_ok(arguments)),
            "read_file_err" => Some(self.compile_read_file_err(arguments)),
            "write_file" => Some(self.compile_write_file(arguments)),
            "vec" => Some(self.compile_vec_ctor(arguments)),
            "map_new" => Some(self.compile_map_new(arguments)),
            "map_put" => Some(self.compile_map_put(arguments)),
            "map_get" => Some(self.compile_map_get(arguments)),
            "map_has" => Some(self.compile_map_has(arguments)),
            "map_remove" => Some(self.compile_map_remove(arguments)),
            "map_len" => Some(self.compile_map_len(arguments)),
            "map_keys" => Some(self.compile_map_keys(arguments)),
            "push" => Some(self.compile_vec_push(arguments)),
            "pop" => Some(self.compile_vec_pop(arguments)),
            "remove" => Some(self.compile_vec_remove(arguments)),
            "insert" => Some(self.compile_vec_insert(arguments)),
            "clear" => Some(self.compile_vec_clear(arguments)),
            "free_str" => Some(self.compile_free_str(arguments)),
            "free_vec" => Some(self.compile_free_vec(arguments)),
            "free_box" => Some(self.compile_free_box(arguments)),
            "ref_count" => Some(self.compile_ref_count(arguments)),
            "tcp_connect" => Some(self.compile_tcp_connect(arguments)),
            "tcp_send" => Some(self.compile_tcp_send(arguments)),
            "tcp_recv" => Some(self.compile_tcp_recv(arguments)),
            "tcp_close" => Some(self.compile_tcp_close(arguments)),
            "tcp_listen" => Some(self.compile_tcp_listen(arguments)),
            "tcp_accept" => Some(self.compile_tcp_accept(arguments)),
            "spawn" | "thread_spawn" => Some(self.compile_spawn(arguments)),
            "join" | "thread_join" => Some(self.compile_join(arguments)),
            "chan_new" => Some(self.compile_chan_new(arguments)),
            "chan_send" => Some(self.compile_chan_send(arguments)),
            "chan_recv" => Some(self.compile_chan_recv(arguments)),
            "str_from_bytes" => Some(self.compile_str_from_bytes(arguments)),
            _ => None,
        }
    }
}
