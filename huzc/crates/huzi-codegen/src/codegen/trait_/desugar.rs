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
            Stmt::For(f) => self.resolve_for_stmt(f, env)?,
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

    fn resolve_for_stmt(&self, f: &mut ForStmt, env: &mut HashMap<String, Type>) -> Result<()> {
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
                    Some(Type::Applied(name, args)) if name == "vec" && !args.is_empty() => {
                        args[0].clone()
                    }
                    _ => Type::I32,
                };
                env.insert(f.var_name.clone(), elem_ty);
            }
        }
        self.resolve_block(&mut f.body, env)
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
                *expr = self.desugar_method_call(mc, env)?;
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
                let scrut_ty = self.infer_expr_type(&m.scrutinee, env);
                for arm in &mut m.arms {
                    let mut arm_env = env.clone();
                    if let Pattern::Variable(name) = &arm.pattern {
                        if let Some(t) = &scrut_ty {
                            arm_env.insert(name.clone(), t.clone());
                        }
                    }
                    if let Some(guard) = &mut arm.guard {
                        self.resolve_expr(guard, &mut arm_env)?;
                    }
                    self.resolve_block(&mut arm.body, &mut arm_env)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn desugar_method_call(
        &self,
        mc: &MethodCallExpr,
        env: &HashMap<String, Type>,
    ) -> Result<Expr> {
        let receiver_ty = self.infer_expr_type(&mc.receiver, env);

        // 优先 1: 具名类型的固有方法或 Trait 方法
        let base_type_name = match &receiver_ty {
            Some(Type::Named(type_name)) => Some(type_name.as_str()),
            Some(Type::Applied(type_name, _)) => Some(type_name.as_str()),
            _ => None,
        };
        if let Some(type_name) = base_type_name {
            if let Some(methods) = self.implemented_methods.get(type_name) {
                if methods.contains_key(&mc.method) {
                    let mangled = format!("{}__{}", type_name, mc.method);
                    let mut args = vec![*mc.receiver.clone()];
                    args.extend(mc.arguments.iter().cloned());
                    return Ok(Expr::Call(CallExpr {
                        callee: Box::new(Expr::Ident(mangled)),
                        arguments: args,
                        type_args: Vec::new(),
                    }));
                }
            }
        }

        // 优先 2: 向量专属前缀降解 (vec_{method})
        if receiver_ty.as_ref().is_some_and(is_vec_type) {
            let vec_fn = format!("vec_{}", mc.method);
            if self.fn_return_types.contains_key(&vec_fn) {
                let mut args = vec![*mc.receiver.clone()];
                args.extend(mc.arguments.iter().cloned());
                return Ok(Expr::Call(CallExpr {
                    callee: Box::new(Expr::Ident(vec_fn)),
                    arguments: args,
                    type_args: Vec::new(),
                }));
            }
        }

        // 优先 3: 普通全局函数 / 内置函数 UFCS 降解
        if self.fn_return_types.contains_key(&mc.method) {
            let mut args = vec![*mc.receiver.clone()];
            args.extend(mc.arguments.iter().cloned());
            return Ok(Expr::Call(CallExpr {
                callee: Box::new(Expr::Ident(mc.method.clone())),
                arguments: args,
                type_args: Vec::new(),
            }));
        }

        // 优先 4: 未匹配报错与 did_you_mean 建议
        self.method_resolution_error(mc, &receiver_ty)
    }

    fn method_resolution_error(
        &self,
        mc: &MethodCallExpr,
        receiver_ty: &Option<Type>,
    ) -> Result<Expr> {
        let base_type_name = match receiver_ty {
            Some(Type::Named(type_name)) => Some(type_name.as_str()),
            Some(Type::Applied(type_name, _)) => Some(type_name.as_str()),
            _ => None,
        };
        if let Some(type_name) = base_type_name {
            let available: Vec<&str> = self
                .implemented_methods
                .get(type_name)
                .map(|m| m.keys().map(|k| k.as_str()).collect())
                .unwrap_or_default();
            let hint = did_you_mean(&mc.method, available.iter().copied());
            let mut msg = if available.is_empty() {
                format!(
                    "类型 '{}' 没有方法 '{}':期望已实现的方法,实际该类型尚未实现任何方法,亦未找到同名全局函数",
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
                msg.push_str(&format!(";帮助:{}", h));
            }
            return Err(HuziError::new_global(msg));
        }
        match receiver_ty {
            Some(other_ty) => Err(HuziError::new_global(format!(
                "类型 '{}' 没有方法 '{}',且未找到匹配的 UFCS 同名函数;帮助:检查方法拼写或定义接受该类型为首参数的同名函数",
                other_ty, mc.method
            ))),
            None => Err(HuziError::new_global(format!(
                "无法确定方法 '{}' 的目标:接收者类型未知且未找到名为 '{}' 的全局函数;帮助:请为接收者提供类型标注或检查函数名拼写",
                mc.method, mc.method
            ))),
        }
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
            Expr::StructLiteral(s) => {
                if !s.type_args.is_empty() {
                    Some(Type::Applied(s.name.clone(), s.type_args.clone()))
                } else {
                    Some(Type::Named(s.name.clone()))
                }
            }
            Expr::EnumConstruct(e) => {
                let full_name = format!("{}::{}", e.enum_name, e.variant);
                if let Some(ret) = self.fn_return_types.get(&full_name).or_else(|| self.fn_return_types.get(&e.variant)) {
                    if let Some(r) = ret {
                        return Some(r.clone());
                    }
                }
                if self.known_types.contains(&e.enum_name) {
                    if !e.type_args.is_empty() {
                        return Some(Type::Applied(e.enum_name.clone(), e.type_args.clone()));
                    } else {
                        return Some(Type::Named(e.enum_name.clone()));
                    }
                }
                None
            }
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
                    if name == "vec" {
                        return Some(Type::Applied("vec".to_string(), c.type_args.clone()));
                    }
                    if name == "map" || name == "Map" || name == "HashMap" {
                        return Some(Type::Applied("map".to_string(), c.type_args.clone()));
                    }
                    self.fn_return_types.get(name).cloned().flatten()
                } else {
                    None
                }
            }
            Expr::MethodCall(mc) => {
                let receiver_ty = self.infer_expr_type(&mc.receiver, env)?;
                let base_name = match &receiver_ty {
                    Type::Named(type_name) => Some(type_name.as_str()),
                    Type::Applied(type_name, _) => Some(type_name.as_str()),
                    _ => None,
                };
                if let Some(type_name) = base_name {
                    if let Some(ret) = self
                        .method_return_types
                        .get(type_name)
                        .and_then(|m| m.get(&mc.method))
                    {
                        return ret.clone();
                    }
                }
                if let Some(ret) = self.fn_return_types.get(&mc.method) {
                    return ret.clone();
                }
                let vec_fn = format!("vec_{}", mc.method);
                self.fn_return_types.get(&vec_fn).cloned().flatten()
            }
            Expr::ArrayLiteral(elems) => {
                let elem_ty = elems
                    .first()
                    .and_then(|e| self.infer_expr_type(e, env))
                    .unwrap_or(Type::I32);
                Some(Type::Array(Box::new(elem_ty), elems.len()))
            }
            Expr::BoxAlloc(inner) => {
                let inner_ty = self.infer_expr_type(inner, env)?;
                Some(Type::Box(Box::new(inner_ty)))
            }
            Expr::Binary(b) => match b.operator {
                BinOp::Eq
                | BinOp::Neq
                | BinOp::Lt
                | BinOp::Le
                | BinOp::Gt
                | BinOp::Ge
                | BinOp::And
                | BinOp::Or => Some(Type::Bool),
                _ => self.infer_expr_type(&b.left, env),
            },
            Expr::Unary(u) => match u.operator {
                UnOp::Not => Some(Type::Bool),
                UnOp::Neg => self.infer_expr_type(&u.operand, env),
                UnOp::Deref => match self.infer_expr_type(&u.operand, env) {
                    Some(Type::Box(inner)) => Some(*inner),
                    other => other,
                },
            },
            Expr::Match(m) => {
                if let Some(first_arm) = m.arms.first() {
                    if let Some(stmt) = first_arm.body.statements.last() {
                        if let Stmt::Expr(e) = &stmt.node {
                            return self.infer_expr_type(&e.expr, env);
                        }
                    }
                }
                None
            }
            _ => None,
        }
    }
}

fn is_vec_type(ty: &Type) -> bool {
    match ty {
        Type::Applied(name, _) => name == "vec",
        Type::Named(name) => name == "vec",
        _ => false,
    }
}
