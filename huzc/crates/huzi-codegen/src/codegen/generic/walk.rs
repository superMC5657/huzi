//! 泛型单态化的 AST 遍历:表达式/语句/块的递归下降,在调用点
//! (含与 `Enum::Variant` 同形的限定调用)触发推导与实例化。

use super::Monomorphizer;
use crate::codegen::qname::qualified;
use huzi_ast::*;
use huzi_error::Result;

impl Monomorphizer {
    pub(super) fn monomorphize_expr(&mut self, expr: &mut Expr) -> Result<()> {
        match expr {
            Expr::Call(c) => self.monomorphize_call_expr(c)?,
            Expr::EnumConstruct(ec) => {
                self.monomorphize_enum_construct_expr(ec)?;
                // `mod::fn(args)` 与 `Enum::Variant(args)` 同形:非已知枚举
                // 时按限定函数调用处理并参与泛型单态化(与 codegen 的判定
                // 一致);非泛型调用保持原样,由 codegen 继续分派。
                if !self.inferrer.known_enums.contains(&ec.enum_name) {
                    if let Some(mangled) = self.try_monomorphize_enum_construct(ec)? {
                        *expr = Expr::Call(CallExpr {
                            callee: Box::new(Expr::Ident(mangled)),
                            arguments: std::mem::take(&mut ec.args),
                            type_args: Vec::new(),
                        });
                    }
                }
            }
            Expr::StructLiteral(s) => self.monomorphize_struct_literal(s)?,
            Expr::Binary(b) => {
                self.monomorphize_expr(&mut b.left)?;
                self.monomorphize_expr(&mut b.right)?;
            }
            Expr::Unary(u) => self.monomorphize_expr(&mut u.operand)?,
            Expr::Assign(a) => {
                self.monomorphize_expr(&mut a.target)?;
                if let Expr::EnumConstruct(ec) = &mut *a.value {
                    if ec.type_args.is_empty() && self.enum_templates.contains_key(&ec.enum_name) {
                        if let Some(Type::Named(mangled)) = self.inferrer.infer_expr_type(&a.target) {
                            if let Some((base, args)) = self.inferrer.instantiated_enum_types.get(&mangled) {
                                if base == &ec.enum_name {
                                    ec.type_args = args.clone();
                                }
                            }
                        }
                    }
                }
                self.monomorphize_expr(&mut a.value)?;
            }
            Expr::ArrayIndex(a) => {
                self.monomorphize_expr(&mut a.array)?;
                self.monomorphize_expr(&mut a.index)?;
            }
            Expr::ArrayLiteral(elems) | Expr::TupleLiteral(elems) => {
                for e in elems {
                    self.monomorphize_expr(e)?;
                }
            }
            Expr::VecEmpty(ty) => self.monomorphize_type(ty)?,
            Expr::BoxAlloc(inner) => self.monomorphize_expr(inner)?,
            Expr::If(i) => {
                self.monomorphize_expr(&mut i.condition)?;
                self.monomorphize_block(&mut i.then_branch)?;
                self.monomorphize_block(&mut i.else_branch)?;
            }
            Expr::FieldAccess(f) => self.monomorphize_expr(&mut f.base)?,
            Expr::Try(t) => self.monomorphize_expr(&mut t.inner)?,
            Expr::Match(m) => self.monomorphize_match_expr(m)?,
            Expr::MethodCall(m) => {
                self.monomorphize_expr(&mut m.receiver)?;
                for a in &mut m.arguments {
                    self.monomorphize_expr(a)?;
                }
            }
            Expr::Closure(c) => self.monomorphize_closure_expr(c)?,
            _ => {}
        }
        Ok(())
    }

    fn monomorphize_closure_expr(&mut self, c: &mut ClosureExpr) -> Result<()> {
        for p in &mut c.params {
            if let Some(ty) = &mut p.param_type {
                self.monomorphize_type(ty)?;
            }
        }
        if let Some(ret) = &mut c.return_type {
            self.monomorphize_type(ret)?;
        }
        self.inferrer.enter_scope();
        for p in &c.params {
            if let Some(ty) = &p.param_type {
                self.inferrer.insert_var(&p.name, ty.clone());
            }
        }
        match &mut c.body {
            ClosureBody::Expr(e) => self.monomorphize_expr(e)?,
            ClosureBody::Block(b) => self.monomorphize_block(b)?,
        }
        self.inferrer.leave_scope();
        Ok(())
    }

    /// 调用表达式:先遍历被调者与实参/类型实参,再对裸名 callee
    /// 尝试泛型单态化,命中则重写 callee 并清空类型实参。
    fn monomorphize_call_expr(&mut self, c: &mut CallExpr) -> Result<()> {
        self.monomorphize_expr(&mut c.callee)?;
        for arg in &mut c.arguments {
            self.monomorphize_expr(arg)?;
        }
        for targ in &mut c.type_args {
            self.monomorphize_type(targ)?;
        }
        let callee_name = match &*c.callee {
            Expr::Ident(n) => n.clone(),
            _ => return Ok(()),
        };
        if let Some(mangled) =
            self.try_monomorphize_call(&callee_name, &mut c.type_args, &mut c.arguments)?
        {
            c.callee = Box::new(Expr::Ident(mangled));
            c.type_args.clear();
            for arg in &mut c.arguments {
                if let Expr::Closure(closure) = arg {
                    self.monomorphize_closure_expr(closure)?;
                }
            }
        }
        Ok(())
    }

    /// 非已知枚举的 `EnumConstruct` 按限定函数调用参与泛型推导与
    /// 实例化,命中则返回单态化名。
    fn try_monomorphize_enum_construct(
        &mut self,
        ec: &mut EnumConstructExpr,
    ) -> Result<Option<String>> {
        let full_name = qualified(&ec.enum_name, &ec.variant);
        let mut type_args: Vec<Type> = Vec::new();
        self.try_monomorphize_call(&full_name, &mut type_args, &mut ec.args)
    }

    /// 结构体字面量:遍历字段值;带显式类型实参时对类型做单态化并
    /// 把结构体名重写为单态化名。
    fn monomorphize_struct_literal(&mut self, s: &mut StructLiteralExpr) -> Result<()> {
        for (_, val) in &mut s.fields {
            self.monomorphize_expr(val)?;
        }
        for targ in &mut s.type_args {
            self.monomorphize_type(targ)?;
        }
        if !s.type_args.is_empty() {
            let mut applied = Type::Applied(s.name.clone(), s.type_args.clone());
            self.monomorphize_type(&mut applied)?;
            if let Type::Named(mangled) = applied {
                s.name = mangled;
            }
            s.type_args.clear();
        }
        Ok(())
    }

    fn monomorphize_enum_construct_expr(&mut self, ec: &mut EnumConstructExpr) -> Result<()> {
        for a in &mut ec.args {
            self.monomorphize_expr(a)?;
        }
        for targ in &mut ec.type_args {
            self.monomorphize_type(targ)?;
        }
        if self.enum_templates.contains_key(&ec.enum_name) {
            let template = self.enum_templates.get(&ec.enum_name).cloned().unwrap();
            let type_args = if !ec.type_args.is_empty() {
                ec.type_args.clone()
            } else {
                self.inferrer.infer_enum_variant_type_args(
                    &ec.enum_name,
                    &ec.variant,
                    &template,
                    &ec.args,
                )?
            };
            let mangled = self.monomorphize_enum_type(&ec.enum_name, &type_args)?;
            ec.enum_name = mangled;
            ec.type_args.clear();
        }
        Ok(())
    }

    fn monomorphize_match_expr(&mut self, m: &mut MatchExpr) -> Result<()> {
        self.monomorphize_expr(&mut m.scrutinee)?;
        let mut scrut_ty = self.inferrer.infer_expr_type(&m.scrutinee);
        if let Some(ty) = &mut scrut_ty {
            let _ = self.monomorphize_type(ty);
        }
        for arm in &mut m.arms {
            self.inferrer.enter_scope();
            if let Pattern::Variant { enum_name, variant, bindings } = &mut arm.pattern {
                if let Some(Type::Named(mangled)) = &scrut_ty {
                    if let Some((base, _)) = self.inferrer.instantiated_enum_types.get(mangled) {
                        if enum_name == base {
                            *enum_name = mangled.clone();
                        }
                    }
                    if let Some(spec) = self.instantiated_enums.get(mangled) {
                        if let Some(v) = spec.variants.iter().find(|v| &v.name == variant) {
                            for (binding, payload_ty) in bindings.iter().zip(v.payloads.iter()) {
                                self.inferrer.insert_var(binding, payload_ty.clone());
                            }
                        }
                    }
                }
            } else if let Pattern::Variable(name) = &arm.pattern {
                if let Some(ty) = &scrut_ty {
                    self.inferrer.insert_var(name, ty.clone());
                }
            }
            if let Some(guard) = &mut arm.guard {
                self.monomorphize_expr(guard)?;
            }
            self.monomorphize_block(&mut arm.body)?;
            self.inferrer.leave_scope();
        }
        Ok(())
    }

    pub(super) fn monomorphize_stmt(&mut self, stmt: &mut Stmt) -> Result<()> {
        match stmt {
            Stmt::Let(l) => {
                if let Some(ann) = &mut l.type_annotation {
                    self.monomorphize_type(ann)?;
                    if let (Some(items), Type::Tuple(elem_tys)) = (&l.tuple_pattern, &ann) {
                        for (it, et) in items.iter().zip(elem_tys) {
                            let mut names = Vec::new();
                            it.collect_bound_names(&mut names);
                            for (n, _) in names {
                                if n != "_" {
                                    self.inferrer.insert_var(n, et.clone());
                                }
                            }
                        }
                    } else {
                        self.inferrer.insert_var(&l.name, ann.clone());
                    }
                }
                if let Some(val) = &mut l.value {
                    if let Expr::EnumConstruct(ec) = val {
                        if ec.type_args.is_empty() && self.enum_templates.contains_key(&ec.enum_name) {
                            if let Some(Type::Named(mangled)) = &l.type_annotation {
                                if let Some((base, args)) = self.inferrer.instantiated_enum_types.get(mangled) {
                                    if base == &ec.enum_name {
                                        ec.type_args = args.clone();
                                    }
                                }
                            }
                        }
                    }
                    self.monomorphize_expr(val)?;
                    if l.type_annotation.is_none() {
                        if let Some(ty) = self.inferrer.infer_expr_type(val) {
                            if let (Some(items), Type::Tuple(elem_tys)) = (&l.tuple_pattern, &ty) {
                                for (it, et) in items.iter().zip(elem_tys) {
                                    let mut names = Vec::new();
                                    it.collect_bound_names(&mut names);
                                    for (n, _) in names {
                                        if n != "_" {
                                            self.inferrer.insert_var(n, et.clone());
                                        }
                                    }
                                }
                            } else {
                                self.inferrer.insert_var(&l.name, ty);
                            }
                        }
                    }
                }
            }
            Stmt::Expr(e) => self.monomorphize_expr(&mut e.expr)?,
            Stmt::Return(r) => {
                if let Some(v) = &mut r.value {
                    if let Expr::EnumConstruct(ec) = v {
                        if ec.type_args.is_empty() && self.enum_templates.contains_key(&ec.enum_name) {
                            if let Some(Type::Named(mangled)) = &self.current_fn_return {
                                if let Some((base, args)) = self.inferrer.instantiated_enum_types.get(mangled) {
                                    if base == &ec.enum_name {
                                        ec.type_args = args.clone();
                                    }
                                }
                            }
                        }
                    }
                    self.monomorphize_expr(v)?;
                }
            }
            Stmt::Block(b) => {
                self.inferrer.enter_scope();
                self.monomorphize_block(b)?;
                self.inferrer.leave_scope();
            }
            Stmt::If(i) => self.monomorphize_if_stmt(i)?,
            Stmt::For(f) => self.monomorphize_for_stmt(f)?,
            Stmt::While(w) => {
                self.monomorphize_expr(&mut w.condition)?;
                self.inferrer.enter_scope();
                self.monomorphize_block(&mut w.body)?;
                self.inferrer.leave_scope();
            }
            Stmt::Defer(d) => self.monomorphize_stmt(&mut d.node)?,
            Stmt::Impl(i) => {
                for m in &mut i.methods {
                    self.monomorphize_block(&mut m.body)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// if/elif/else 各分支:条件单态化,分支内独立作用域遍历。
    fn monomorphize_if_stmt(&mut self, i: &mut IfStmt) -> Result<()> {
        self.monomorphize_expr(&mut i.condition)?;
        self.inferrer.enter_scope();
        self.monomorphize_block(&mut i.then_branch)?;
        self.inferrer.leave_scope();
        for (cond, blk) in &mut i.elif_branches {
            self.monomorphize_expr(cond)?;
            self.inferrer.enter_scope();
            self.monomorphize_block(blk)?;
            self.inferrer.leave_scope();
        }
        if let Some(b) = &mut i.else_branch {
            self.inferrer.enter_scope();
            self.monomorphize_block(b)?;
            self.inferrer.leave_scope();
        }
        Ok(())
    }

    /// for-in:循环变量类型由可迭代源推导(range 为 i32,数组取元素,
    /// vec 取元素类型,str 为 char)后进入循环体遍历。
    fn monomorphize_for_stmt(&mut self, f: &mut ForStmt) -> Result<()> {
        self.inferrer.enter_scope();
        match &mut f.source {
            ForSource::Range { start, end } => {
                self.monomorphize_expr(start)?;
                self.monomorphize_expr(end)?;
                self.inferrer
                    .insert_var(&f.var_name, Type::Named("i32".to_string()));
            }
            ForSource::Array(arr) => {
                self.monomorphize_expr(arr)?;
                if let Some(arr_ty) = self.inferrer.infer_expr_type(arr) {
                    match arr_ty {
                        Type::Array(elem, _) => {
                            self.inferrer.insert_var(&f.var_name, *elem);
                        }
                        Type::Applied(name, args) if name == "vec" => {
                            if let Some(elem) = args.first() {
                                self.inferrer.insert_var(&f.var_name, elem.clone());
                            }
                        }
                        Type::Named(s) if s == "str" => {
                            self.inferrer
                                .insert_var(&f.var_name, Type::Named("char".to_string()));
                        }
                        _ => {}
                    }
                }
            }
        }
        self.monomorphize_block(&mut f.body)?;
        self.inferrer.leave_scope();
        Ok(())
    }

    pub(super) fn monomorphize_block(&mut self, block: &mut Block) -> Result<()> {
        for stmt in &mut block.statements {
            self.current_span = Some(stmt.span);
            self.monomorphize_stmt(&mut stmt.node)?;
        }
        Ok(())
    }
}
