//! 泛型单态化 (Monomorphization):在代码生成前将泛型函数与泛型结构体特化为具体 AST。
//!
//! 流程:
//! 1. 收集泛型函数与结构体模板,校验类型变量合法性;
//! 2. 遍历 AST,遇到显式实参调用/构造/类型注解或推导调用时触发按需单态化;
//! 3. 深拷贝模板 AST 并替换类型实参,修饰名称(如 `id__i32`, `Pair__i32_str`);
//! 4. 消除所有模板定义,将特化后的具体定义追加到 AST 中供后续管线编译。

mod infer;
pub mod mangle;
pub mod subst;
mod validate;

pub use mangle::mangle_name;
pub use subst::{substitute_block, substitute_type};

use super::ModuleCode;
use huzi_ast::*;
use huzi_error::{HuziError, Result, did_you_mean};
use std::collections::{HashMap, HashSet};

/// 单态化器状态。
struct Monomorphizer {
    struct_templates: HashMap<String, StructDef>,
    fn_templates: HashMap<String, (FnStmt, Span)>,
    known_types: HashSet<String>,
    instantiated_structs: HashMap<String, StructDef>,
    instantiated_fns: HashMap<String, (FnStmt, Span)>,
    inferrer: infer::TypeInferrer,
}

impl Monomorphizer {
    fn new() -> Self {
        let mut known_types = HashSet::new();
        for s in &[
            "i32", "i64", "u32", "u64", "f32", "f64", "bool", "str", "char", "unit", "vec", "Box",
            "map", "Map", "HashMap",
        ] {
            known_types.insert(s.to_string());
        }
        Self {
            struct_templates: HashMap::new(),
            fn_templates: HashMap::new(),
            known_types,
            instantiated_structs: HashMap::new(),
            instantiated_fns: HashMap::new(),
            inferrer: infer::TypeInferrer::new(),
        }
    }

    fn monomorphize_type(&mut self, ty: &mut Type) -> Result<()> {
        match ty {
            Type::Applied(name, args) => {
                for a in args.iter_mut() {
                    self.monomorphize_type(a)?;
                }
                if name == "vec" {
                    return Ok(());
                }
                let template = self.struct_templates.get(name).cloned().ok_or_else(|| {
                    let hint = did_you_mean(name, self.struct_templates.keys().map(|s| s.as_str()));
                    match hint {
                        Some(h) => HuziError::new_global(format!(
                            "Unknown generic struct '{}', did you mean '{}'?",
                            name, h
                        )),
                        None => HuziError::new_global(format!("Unknown generic struct '{}'", name)),
                    }
                })?;
                if args.len() != template.type_params.len() {
                    return Err(HuziError::new_global(format!(
                        "Generic struct '{}' expects {} type argument(s), got {}",
                        name,
                        template.type_params.len(),
                        args.len()
                    )));
                }
                for a in args.iter() {
                    self.validate_type_arg(a)?;
                }
                let mangled = mangle_name(name, args);
                if !self.instantiated_structs.contains_key(&mangled) {
                    let mapping: HashMap<String, Type> = template
                        .type_params
                        .iter()
                        .cloned()
                        .zip(args.iter().cloned())
                        .collect();
                    let mut spec = template.clone();
                    spec.name = mangled.clone();
                    spec.type_params.clear();
                    for field in &mut spec.fields {
                        field.field_type = substitute_type(&field.field_type, &mapping);
                    }
                    self.instantiated_structs
                        .insert(mangled.clone(), spec.clone());
                    for field in &mut spec.fields {
                        self.monomorphize_type(&mut field.field_type)?;
                    }
                    self.instantiated_structs
                        .insert(mangled.clone(), spec.clone());
                    self.inferrer.struct_defs.insert(mangled.clone(), spec);
                    self.inferrer.instantiated_struct_types.insert(
                        mangled.clone(),
                        (name.clone(), args.clone()),
                    );
                }
                *ty = Type::Named(mangled);
            }
            Type::Box(inner) => self.monomorphize_type(inner)?,
            Type::Array(elem, _) => self.monomorphize_type(elem)?,
            Type::Tuple(elems) => {
                for elem in elems {
                    self.monomorphize_type(elem)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// 泛型函数模板查找:先按调用名原样查找,再按末段回退——限定调用
    /// (`result::is_ok`)的模板按定义名 `is_ok` 收录。
    fn fn_template_for(&self, callee_name: &str) -> Option<&(FnStmt, Span)> {
        if let Some(t) = self.fn_templates.get(callee_name) {
            return Some(t);
        }
        let bare = callee_name.rsplit("::").next()?;
        self.fn_templates.get(bare)
    }

    /// 对调用点做泛型推导与单态化(实参须已完成表达式级单态化)。
    /// 返回 Some(单态化名) 时调用方把 callee 重写为该裸名并清空
    /// type_args;返回 None 表示非泛型调用,保持原样由 codegen 分派。
    fn try_monomorphize_call(
        &mut self,
        callee_name: &str,
        type_args: &mut Vec<Type>,
        arguments: &mut [Expr],
    ) -> Result<Option<String>> {
        // 实参类型推导：未提供显式类型实参时尝试推导
        if type_args.is_empty() {
            if let Some((template, _)) = self.fn_template_for(callee_name) {
                let inferred =
                    self.inferrer
                        .infer_call_type_args(callee_name, template, arguments)?;
                *type_args = inferred;
            }
        }
        if type_args.is_empty() {
            return Ok(None);
        }
        // 模板按定义名(不带模块前缀)收录:限定调用(result::is_ok)取
        // 末段查找,单态化产物也以裸名注册进主程序。
        let bare_callee = callee_name.rsplit("::").next().unwrap_or(callee_name);
        let (template, span) = self
            .fn_templates
            .get(bare_callee)
            .cloned()
            .ok_or_else(|| {
                let hint = did_you_mean(callee_name, self.fn_templates.keys().map(|s| s.as_str()));
                match hint {
                    Some(h) => HuziError::new_global(format!(
                        "Unknown generic function '{}', did you mean '{}'?",
                        callee_name, h
                    )),
                    None => HuziError::new_global(format!(
                        "Unknown generic function '{}'",
                        callee_name
                    )),
                }
            })?;
        if type_args.len() != template.type_params.len() {
            return Err(HuziError::new_global(format!(
                "Generic function '{}' expects {} type argument(s), got {}",
                callee_name,
                template.type_params.len(),
                type_args.len()
            )));
        }
        for a in type_args.iter() {
            self.validate_type_arg(a)?;
        }
        let mangled = mangle_name(bare_callee, type_args);
        if !self.instantiated_fns.contains_key(&mangled) {
            let mapping: HashMap<String, Type> = template
                .type_params
                .iter()
                .cloned()
                .zip(type_args.iter().cloned())
                .collect();
            let mut spec = template.clone();
            spec.name = mangled.clone();
            spec.type_params.clear();
            for p in &mut spec.params {
                p.param_type = substitute_type(&p.param_type, &mapping);
                self.monomorphize_type(&mut p.param_type)?;
            }
            if let Some(ret) = &mut spec.return_type {
                *ret = substitute_type(ret, &mapping);
                self.monomorphize_type(ret)?;
            }
            substitute_block(&mut spec.body, &mapping);
            self.instantiated_fns
                .insert(mangled.clone(), (spec.clone(), span));
            self.inferrer.fn_signatures.insert(
                mangled.clone(),
                (
                    spec.params.iter().map(|p| p.param_type.clone()).collect(),
                    spec.return_type.clone(),
                ),
            );
            self.inferrer.enter_scope();
            for p in &spec.params {
                self.inferrer.insert_var(&p.name, p.param_type.clone());
            }
            self.monomorphize_block(&mut spec.body)?;
            self.inferrer.leave_scope();
            self.instantiated_fns.insert(mangled.clone(), (spec, span));
        }
        Ok(Some(mangled))
    }

    fn monomorphize_expr(&mut self, expr: &mut Expr) -> Result<()> {
        match expr {
            Expr::Call(c) => {
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
                }
            }
            Expr::EnumConstruct(ec) => {
                for a in &mut ec.args {
                    self.monomorphize_expr(a)?;
                }
                // `mod::fn(args)` 与 `Enum::Variant(args)` 同形:非已知枚举
                // 时按限定函数调用处理并参与泛型单态化(与 codegen 的判定
                // 一致);非泛型调用保持原样,由 codegen 继续分派。
                if !self.inferrer.known_enums.contains(&ec.enum_name) {
                    let full_name = format!("{}::{}", ec.enum_name, ec.variant);
                    let mut type_args: Vec<Type> = Vec::new();
                    if let Some(mangled) =
                        self.try_monomorphize_call(&full_name, &mut type_args, &mut ec.args)?
                    {
                        *expr = Expr::Call(CallExpr {
                            callee: Box::new(Expr::Ident(mangled)),
                            arguments: std::mem::take(&mut ec.args),
                            type_args: Vec::new(),
                        });
                    }
                }
            }
            Expr::StructLiteral(s) => {
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
            }
            Expr::Binary(b) => {
                self.monomorphize_expr(&mut b.left)?;
                self.monomorphize_expr(&mut b.right)?;
            }
            Expr::Unary(u) => self.monomorphize_expr(&mut u.operand)?,
            Expr::Assign(a) => {
                self.monomorphize_expr(&mut a.target)?;
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
            Expr::Match(m) => {
                self.monomorphize_expr(&mut m.scrutinee)?;
                for arm in &mut m.arms {
                    self.monomorphize_block(&mut arm.body)?;
                }
            }
            Expr::MethodCall(m) => {
                self.monomorphize_expr(&mut m.receiver)?;
                for a in &mut m.arguments {
                    self.monomorphize_expr(a)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn monomorphize_stmt(&mut self, stmt: &mut Stmt) -> Result<()> {
        match stmt {
            Stmt::Let(l) => {
                if let Some(ann) = &mut l.type_annotation {
                    self.monomorphize_type(ann)?;
                    self.inferrer.insert_var(&l.name, ann.clone());
                }
                if let Some(val) = &mut l.value {
                    self.monomorphize_expr(val)?;
                    if l.type_annotation.is_none() {
                        if let Some(ty) = self.inferrer.infer_expr_type(val) {
                            self.inferrer.insert_var(&l.name, ty);
                        }
                    }
                }
            }
            Stmt::Expr(e) => self.monomorphize_expr(&mut e.expr)?,
            Stmt::Return(r) => {
                if let Some(v) = &mut r.value {
                    self.monomorphize_expr(v)?;
                }
            }
            Stmt::Block(b) => {
                self.inferrer.enter_scope();
                self.monomorphize_block(b)?;
                self.inferrer.leave_scope();
            }
            Stmt::If(i) => {
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
            }
            Stmt::For(f) => {
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
                                    self.inferrer.insert_var(
                                        &f.var_name,
                                        Type::Named("char".to_string()),
                                    );
                                }
                                _ => {}
                            }
                        }
                    }
                }
                self.monomorphize_block(&mut f.body)?;
                self.inferrer.leave_scope();
            }
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

    fn monomorphize_block(&mut self, block: &mut Block) -> Result<()> {
        for stmt in &mut block.statements {
            self.monomorphize_stmt(&mut stmt.node)?;
        }
        Ok(())
    }
}

/// 对主程序及所有导入模块执行单态化变换。
pub(super) fn monomorphize_all(
    program: &Program,
    modules: &mut [ModuleCode],
) -> Result<Program> {
    let mut mono = Monomorphizer::new();

    // 阶段 1: 从主程序与所有模块中收集泛型模板及已知类型
    for m in modules.iter() {
        if let Some(prog) = &m.program {
            mono.collect_templates(prog)?;
        }
    }
    mono.collect_templates(program)?;

    // 阶段 2: 单态化各模块内部代码
    for m in modules.iter_mut() {
        if let Some(prog) = &mut m.program {
            for s in &mut prog.statements {
                if let Stmt::Fn(f) = &mut s.node {
                    if f.type_params.is_empty() {
                        for p in &mut f.params {
                            mono.monomorphize_type(&mut p.param_type)?;
                        }
                        if let Some(ret) = &mut f.return_type {
                            mono.monomorphize_type(ret)?;
                        }
                        mono.inferrer.enter_scope();
                        for p in &f.params {
                            mono.inferrer.insert_var(&p.name, p.param_type.clone());
                        }
                        mono.monomorphize_block(&mut f.body)?;
                        mono.inferrer.leave_scope();
                    }
                } else {
                    mono.monomorphize_stmt(&mut s.node)?;
                }
            }
        }
    }

    // 阶段 3: 单态化主程序代码
    let mut new_program = program.clone();
    for s in &mut new_program.statements {
        if let Stmt::Fn(f) = &mut s.node {
            if f.type_params.is_empty() {
                for p in &mut f.params {
                    mono.monomorphize_type(&mut p.param_type)?;
                }
                if let Some(ret) = &mut f.return_type {
                    mono.monomorphize_type(ret)?;
                }
                mono.inferrer.enter_scope();
                for p in &f.params {
                    mono.inferrer.insert_var(&p.name, p.param_type.clone());
                }
                mono.monomorphize_block(&mut f.body)?;
                mono.inferrer.leave_scope();
            }
        } else {
            mono.monomorphize_stmt(&mut s.node)?;
        }
    }

    // 阶段 4: 移除纯模板定义,追加单态化生成的实体
    new_program.statements.retain(|s| match &s.node {
        Stmt::Struct(d) => d.type_params.is_empty(),
        Stmt::Fn(f) => f.type_params.is_empty(),
        _ => true,
    });

    for def in mono.instantiated_structs.into_values() {
        new_program.statements.push(Spanned::new(Stmt::Struct(def), 1, 1));
    }
    for (f, span) in mono.instantiated_fns.into_values() {
        new_program.statements.push(Spanned::with_span(Stmt::Fn(f), span));
    }

    Ok(new_program)
}
