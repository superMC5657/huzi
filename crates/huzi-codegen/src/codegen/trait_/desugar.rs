use super::TraitDesugarer;
use huzi_ast::*;
use huzi_error::{HuziError, Result, did_you_mean};
use std::collections::HashMap;

impl TraitDesugarer {
    pub(super) fn resolve_stmt(&self, stmt: &mut Stmt) -> Result<()> {
        let mut local_env = HashMap::new();
        self.resolve_stmt_scoped(stmt, &mut local_env)
    }

    pub(super) fn resolve_stmt_scoped(
        &self,
        stmt: &mut Stmt,
        env: &mut HashMap<String, Type>,
    ) -> Result<()> {
        match stmt {
            Stmt::Let(l) => {
                if let Some(val) = &mut l.value {
                    self.resolve_expr(val, env)?;
                }
                let ty = if let Some(ann) = &l.type_annotation {
                    ann.clone()
                } else if let Some(val) = &l.value {
                    self.infer_expr_type(val, env).unwrap_or(Type::I32)
                } else {
                    Type::I32
                };
                env.insert(l.name.clone(), ty);
            }
            Stmt::Expr(e) => self.resolve_expr(&mut e.expr, env)?,
            Stmt::Return(r) => {
                if let Some(val) = &mut r.value {
                    self.resolve_expr(val, env)?;
                }
            }
            Stmt::Block(b) => self.resolve_block(b, env)?,
            Stmt::If(i) => {
                self.resolve_expr(&mut i.condition, env)?;
                self.resolve_block(&mut i.then_branch, env)?;
                for (cond, blk) in &mut i.elif_branches {
                    self.resolve_expr(cond, env)?;
                    self.resolve_block(blk, env)?;
                }
                if let Some(b) = &mut i.else_branch {
                    self.resolve_block(b, env)?;
                }
            }
            Stmt::For(f) => {
                match &mut f.source {
                    ForSource::Range { start, end } => {
                        self.resolve_expr(start, env)?;
                        self.resolve_expr(end, env)?;
                        env.insert(f.var_name.clone(), Type::I32);
                    }
                    ForSource::Array(arr) => {
                        self.resolve_expr(arr, env)?;
                        let elem_ty = match self.infer_expr_type(arr, env) {
                            Some(Type::Array(elem, _)) => *elem,
                            Some(Type::Applied(name, args))
                                if name == "vec" && !args.is_empty() =>
                            {
                                args[0].clone()
                            }
                            _ => Type::I32,
                        };
                        env.insert(f.var_name.clone(), elem_ty);
                    }
                }
                self.resolve_block(&mut f.body, env)?;
            }
            Stmt::While(w) => {
                self.resolve_expr(&mut w.condition, env)?;
                self.resolve_block(&mut w.body, env)?;
            }
            Stmt::Defer(d) => self.resolve_stmt_scoped(&mut d.node, env)?,
            Stmt::Fn(f) => {
                let mut fn_env = env.clone();
                for p in &f.params {
                    fn_env.insert(p.name.clone(), p.param_type.clone());
                }
                self.resolve_block(&mut f.body, &fn_env)?;
            }
            _ => {}
        }
        Ok(())
    }

    pub(super) fn resolve_block(
        &self,
        block: &mut Block,
        env: &HashMap<String, Type>,
    ) -> Result<()> {
        let mut scope_env = env.clone();
        for stmt in &mut block.statements {
            let span = stmt.span;
            self.resolve_stmt_scoped(&mut stmt.node, &mut scope_env)
                .map_err(|e| e.with_position(span.line, span.column))?;
        }
        Ok(())
    }

    fn resolve_expr(&self, expr: &mut Expr, env: &HashMap<String, Type>) -> Result<()> {
        match expr {
            Expr::MethodCall(mc) => {
                self.resolve_expr(&mut mc.receiver, env)?;
                for arg in &mut mc.arguments {
                    self.resolve_expr(arg, env)?;
                }

                let receiver_ty = self.infer_expr_type(&mc.receiver, env).ok_or_else(|| {
                    HuziError::new_global(format!(
                        "无法确定方法 '{}' 接收者类型:期望具名结构体变量,实际推导失败;帮助:检查接收者变量是否已定义并标注类型",
                        mc.method
                    ))
                })?;

                let type_name = match &receiver_ty {
                    Type::Named(n) => n.clone(),
                    _ => {
                        return Err(HuziError::new_global(format!(
                            "方法调用 '{}' 需要具名结构体接收者:期望如 'Point',实际为 '{}'",
                            mc.method, receiver_ty
                        )));
                    }
                };

                let implemented = self.implemented_methods.get(&type_name);
                let has_method = implemented
                    .and_then(|m| m.get(&mc.method))
                    .is_some();

                if !has_method {
                    let available: Vec<&str> = implemented
                        .map(|m| m.keys().map(|k| k.as_str()).collect())
                        .unwrap_or_default();
                    let hint = did_you_mean(&mc.method, available.iter().copied());
                    let mut message = if available.is_empty() {
                        format!(
                            "类型 '{}' 没有方法 '{}':期望已实现的方法,实际该类型尚未实现任何方法",
                            type_name, mc.method
                        )
                    } else {
                        format!(
                            "类型 '{}' 没有方法 '{}':期望为 [{}] 之一,实际未找到",
                            type_name,
                            mc.method,
                            available.join(", ")
                        )
                    };
                    if let Some(h) = hint {
                        message.push_str(&format!(";帮助:{}", h));
                    }
                    return Err(HuziError::new_global(message));
                }

                let mangled_callee = format!("{}__{}", type_name, mc.method);
                let mut args = vec![*mc.receiver.clone()];
                args.append(&mut mc.arguments);

                *expr = Expr::Call(CallExpr {
                    callee: Box::new(Expr::Ident(mangled_callee)),
                    arguments: args,
                    type_args: Vec::new(),
                });
            }
            Expr::Call(c) => {
                self.resolve_expr(&mut c.callee, env)?;
                for a in &mut c.arguments {
                    self.resolve_expr(a, env)?;
                }
            }
            Expr::Binary(b) => {
                self.resolve_expr(&mut b.left, env)?;
                self.resolve_expr(&mut b.right, env)?;
            }
            Expr::Unary(u) => self.resolve_expr(&mut u.operand, env)?,
            Expr::Assign(a) => {
                self.resolve_expr(&mut a.target, env)?;
                self.resolve_expr(&mut a.value, env)?;
            }
            Expr::ArrayIndex(a) => {
                self.resolve_expr(&mut a.array, env)?;
                self.resolve_expr(&mut a.index, env)?;
            }
            Expr::ArrayLiteral(elems) | Expr::TupleLiteral(elems) => {
                for e in elems {
                    self.resolve_expr(e, env)?;
                }
            }
            Expr::BoxAlloc(inner) => self.resolve_expr(inner, env)?,
            Expr::If(i) => {
                self.resolve_expr(&mut i.condition, env)?;
                self.resolve_block(&mut i.then_branch, env)?;
                self.resolve_block(&mut i.else_branch, env)?;
            }
            Expr::FieldAccess(f) => self.resolve_expr(&mut f.base, env)?,
            Expr::StructLiteral(s) => {
                for (_, val) in &mut s.fields {
                    self.resolve_expr(val, env)?;
                }
            }
            Expr::EnumConstruct(e) => {
                for a in &mut e.args {
                    self.resolve_expr(a, env)?;
                }
            }
            Expr::Match(m) => {
                self.resolve_expr(&mut m.scrutinee, env)?;
                for arm in &mut m.arms {
                    self.resolve_block(&mut arm.body, env)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn infer_expr_type(&self, expr: &Expr, env: &HashMap<String, Type>) -> Option<Type> {
        match expr {
            Expr::Ident(name) => env.get(name).cloned(),
            Expr::Literal(lit) => match lit {
                Literal::Int(_) => Some(Type::I32),
                Literal::Float(_) => Some(Type::F64),
                Literal::Bool(_) => Some(Type::Bool),
                Literal::String(_) => Some(Type::Str),
                Literal::Char(_) => Some(Type::Char),
            },
            Expr::StructLiteral(s) => Some(Type::Named(s.name.clone())),
            Expr::FieldAccess(fa) => {
                let base_ty = self.infer_expr_type(&fa.base, env)?;
                let s_name = match base_ty {
                    Type::Named(n) => n,
                    Type::Box(inner) => match *inner {
                        Type::Named(n) => n,
                        _ => return None,
                    },
                    _ => return None,
                };
                self.struct_fields.get(&s_name)?.get(&fa.field).cloned()
            }
            Expr::Call(c) => {
                if let Expr::Ident(name) = &*c.callee {
                    self.fn_return_types.get(name).cloned().flatten()
                } else {
                    None
                }
            }
            Expr::MethodCall(mc) => {
                let receiver_ty = self.infer_expr_type(&mc.receiver, env)?;
                let type_name = match receiver_ty {
                    Type::Named(n) => n,
                    _ => return None,
                };
                self.method_return_types
                    .get(&type_name)?
                    .get(&mc.method)?
                    .clone()
            }
            Expr::BoxAlloc(inner) => {
                let inner_ty = self.infer_expr_type(inner, env)?;
                Some(Type::Box(Box::new(inner_ty)))
            }
            _ => None,
        }
    }
}
