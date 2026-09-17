//! Trait 静态分发与方法调用脱糖 (Trait static dispatch and desugaring)。
//!
//! 流程:
//! 1. 收集所有 `trait` 声明与 `impl` 实现块;
//! 2. 校验 trait 方法完备性、签名匹配与同类型多 trait 方法冲突;
//! 3. 将 `impl` 方法转换为顶层函数 (修饰为 `TargetType__method`);
//! 4. 遍历并推导接收者类型,将 `receiver.method(args)` 静态重写为 `TargetType__method(receiver, args)`;
//! 5. 输出脱糖后的 Program。

use super::ModuleCode;
use huzi_ast::*;
use huzi_error::{HuziError, Result};
use std::collections::{HashMap, HashSet};

pub fn desugar_traits(program: &Program, modules: &mut [ModuleCode]) -> Result<Program> {
    let mut desugarer = TraitDesugarer::new();
    desugarer.collect_types_and_traits(program, modules)?;
    desugarer.validate_and_collect_impls(program, modules)?;

    let mut new_statements = Vec::new();

    // 1. 生成所有 impl 脱糖出的顶层函数
    let generated_fns = desugarer.generate_impl_functions(program, modules)?;
    for (f, span) in generated_fns {
        new_statements.push(Spanned::with_span(Stmt::Fn(f), span));
    }

    // 2. 收集模块内的 impl 方法并脱糖模块
    for m in modules.iter_mut() {
        if let Some(prog) = &mut m.program {
            let mut mod_stmts = Vec::new();
            for s in &prog.statements {
                match &s.node {
                    Stmt::Trait(_) | Stmt::Impl(_) => {}
                    _ => {
                        let mut cloned = s.clone();
                        desugarer.resolve_stmt(&mut cloned.node)?;
                        mod_stmts.push(cloned);
                    }
                }
            }
            prog.statements = mod_stmts;
        }
    }

    // 3. 处理主程序中的非 trait/impl 语句
    for s in &program.statements {
        match &s.node {
            Stmt::Trait(_) | Stmt::Impl(_) => {}
            _ => {
                let mut cloned = s.clone();
                desugarer.resolve_stmt(&mut cloned.node)?;
                new_statements.push(cloned);
            }
        }
    }

    Ok(Program {
        statements: new_statements,
    })
}

struct TraitDesugarer {
    traits: HashMap<String, TraitDef>,
    known_types: HashSet<String>,
    struct_fields: HashMap<String, HashMap<String, Type>>,
    /// target_type -> (method_name -> trait_name)
    implemented_methods: HashMap<String, HashMap<String, String>>,
    /// target_type -> (method_name -> return_type)
    method_return_types: HashMap<String, HashMap<String, Option<Type>>>,
    /// function_name -> return_type
    fn_return_types: HashMap<String, Option<Type>>,
}

impl TraitDesugarer {
    fn new() -> Self {
        let mut known_types = HashSet::new();
        for s in &[
            "i32", "i64", "u32", "u64", "f32", "f64", "bool", "str", "char", "unit", "vec", "Box",
        ] {
            known_types.insert(s.to_string());
        }
        let mut fn_return_types = HashMap::new();
        fn_return_types.insert("len".to_string(), Some(Type::I32));
        fn_return_types.insert("to_string".to_string(), Some(Type::Str));
        fn_return_types.insert("concat".to_string(), Some(Type::Str));
        fn_return_types.insert("abs".to_string(), Some(Type::I32));
        Self {
            traits: HashMap::new(),
            known_types,
            struct_fields: HashMap::new(),
            implemented_methods: HashMap::new(),
            method_return_types: HashMap::new(),
            fn_return_types,
        }
    }

    fn collect_types_and_traits(&mut self, program: &Program, modules: &[ModuleCode]) -> Result<()> {
        self.collect_from_stmts(&program.statements)?;
        for m in modules {
            if let Some(prog) = &m.program {
                self.collect_from_stmts(&prog.statements)?;
            }
        }
        Ok(())
    }

    fn collect_from_stmts(&mut self, stmts: &[Spanned<Stmt>]) -> Result<()> {
        for s in stmts {
            match &s.node {
                Stmt::Struct(d) => {
                    self.known_types.insert(d.name.clone());
                    let mut fields = HashMap::new();
                    for f in &d.fields {
                        fields.insert(f.name.clone(), f.field_type.clone());
                    }
                    self.struct_fields.insert(d.name.clone(), fields);
                }
                Stmt::Enum(d) => {
                    self.known_types.insert(d.name.clone());
                }
                Stmt::Trait(t) => {
                    if self.traits.contains_key(&t.name) {
                        return Err(HuziError::new_global(format!(
                            "Duplicate trait definition: {}",
                            t.name
                        )));
                    }
                    let mut mnames = HashSet::new();
                    for m in &t.methods {
                        if !mnames.insert(&m.name) {
                            return Err(HuziError::new_global(format!(
                                "Duplicate method '{}' in trait '{}'",
                                m.name, t.name
                            )));
                        }
                    }
                    self.traits.insert(t.name.clone(), t.clone());
                }
                Stmt::Fn(f) => {
                    self.fn_return_types
                        .insert(f.name.clone(), f.return_type.clone());
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn validate_and_collect_impls(&mut self, program: &Program, modules: &[ModuleCode]) -> Result<()> {
        for s in &program.statements {
            if let Stmt::Impl(i) = &s.node {
                self.validate_impl(i)?;
            }
        }
        for m in modules {
            if let Some(prog) = &m.program {
                for s in &prog.statements {
                    if let Stmt::Impl(i) = &s.node {
                        self.validate_impl(i)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn validate_impl(&mut self, i: &ImplBlock) -> Result<()> {
        let trait_def = self
            .traits
            .get(&i.trait_name)
            .ok_or_else(|| HuziError::new_global(format!("Unknown trait '{}'", i.trait_name)))?
            .clone();

        if !self.known_types.contains(&i.target_type) {
            return Err(HuziError::new_global(format!(
                "Unknown type '{}' in implementation of trait '{}'",
                i.target_type, i.trait_name
            )));
        }

        // 检查缺漏方法
        for tm in &trait_def.methods {
            if !i.methods.iter().any(|m| m.name == tm.name) {
                return Err(HuziError::new_global(format!(
                    "Missing method '{}' in implementation of trait '{}' for '{}'",
                    tm.name, trait_def.name, i.target_type
                )));
            }
        }

        // 检查多余方法与签名
        for m in &i.methods {
            let tm = trait_def
                .methods
                .iter()
                .find(|tm| tm.name == m.name)
                .ok_or_else(|| {
                    HuziError::new_global(format!(
                        "Method '{}' is not a member of trait '{}'",
                        m.name, trait_def.name
                    ))
                })?;

            let expected_params = tm.params.len() + if tm.has_self { 1 } else { 0 };
            if m.params.len() != expected_params {
                return Err(HuziError::new_global(format!(
                    "Method '{}' expects {} parameter(s), got {}",
                    m.name,
                    expected_params,
                    m.params.len()
                )));
            }

            if m.return_type != tm.return_type {
                return Err(HuziError::new_global(format!(
                    "Method '{}' return type does not match trait '{}'",
                    m.name, trait_def.name
                )));
            }

            // 冲突检查 (多 trait 冲突直接报错)
            let type_methods = self
                .implemented_methods
                .entry(i.target_type.clone())
                .or_default();
            if let Some(existing_trait) = type_methods.get(&m.name) {
                return Err(HuziError::new_global(format!(
                    "Conflict: method '{}' already implemented for type '{}' by trait '{}'",
                    m.name, i.target_type, existing_trait
                )));
            }
            type_methods.insert(m.name.clone(), i.trait_name.clone());

            self.method_return_types
                .entry(i.target_type.clone())
                .or_default()
                .insert(m.name.clone(), m.return_type.clone());

            let mangled = format!("{}__{}", i.target_type, m.name);
            self.fn_return_types.insert(mangled, m.return_type.clone());
        }

        Ok(())
    }

    fn generate_impl_functions(
        &self,
        program: &Program,
        modules: &[ModuleCode],
    ) -> Result<Vec<(FnStmt, Span)>> {
        let mut fns = Vec::new();
        for s in &program.statements {
            if let Stmt::Impl(i) = &s.node {
                for m in &i.methods {
                    let mut desugared = m.clone();
                    desugared.name = format!("{}__{}", i.target_type, m.name);
                    fns.push((desugared, s.span));
                }
            }
        }
        for m in modules {
            if let Some(prog) = &m.program {
                for s in &prog.statements {
                    if let Stmt::Impl(i) = &s.node {
                        for m_fn in &i.methods {
                            let mut desugared = m_fn.clone();
                            desugared.name = format!("{}__{}", i.target_type, m_fn.name);
                            fns.push((desugared, s.span));
                        }
                    }
                }
            }
        }
        Ok(fns)
    }

    fn resolve_stmt(&self, stmt: &mut Stmt) -> Result<()> {
        let mut local_env = HashMap::new();
        self.resolve_stmt_scoped(stmt, &mut local_env)
    }

    fn resolve_stmt_scoped(&self, stmt: &mut Stmt, env: &mut HashMap<String, Type>) -> Result<()> {
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
                            Some(Type::Applied(name, args)) if name == "vec" && !args.is_empty() => {
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
                self.resolve_block(&mut f.body, &mut fn_env)?;
            }
            _ => {}
        }
        Ok(())
    }

    fn resolve_block(&self, block: &mut Block, env: &HashMap<String, Type>) -> Result<()> {
        let mut scope_env = env.clone();
        for stmt in &mut block.statements {
            self.resolve_stmt_scoped(&mut stmt.node, &mut scope_env)?;
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
                        "Cannot resolve receiver type for method call '{}'",
                        mc.method
                    ))
                })?;

                let type_name = match &receiver_ty {
                    Type::Named(n) => n.clone(),
                    _ => {
                        return Err(HuziError::new_global(format!(
                            "Method call '{}' requires a named struct type (got '{}')",
                            mc.method, receiver_ty
                        )));
                    }
                };

                let has_method = self
                    .implemented_methods
                    .get(&type_name)
                    .and_then(|m| m.get(&mc.method))
                    .is_some();

                if !has_method {
                    return Err(HuziError::new_global(format!(
                        "Type '{}' has no method '{}'",
                        type_name, mc.method
                    )));
                }

                let mangled_callee = format!("{}__{}", type_name, mc.method);
                let mut args = vec![*mc.receiver.clone()];
                args.extend(mc.arguments.drain(..));

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
