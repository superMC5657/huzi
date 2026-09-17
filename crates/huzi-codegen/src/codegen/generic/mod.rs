//! 泛型单态化 (Monomorphization):在代码生成前将泛型函数与泛型结构体特化为具体 AST。
//!
//! 流程:
//! 1. 收集泛型函数与结构体模板,校验类型变量合法性;
//! 2. 遍历 AST,遇到显式实参调用/构造/类型注解时触发按需单态化;
//! 3. 深拷贝模板 AST 并替换类型实参,修饰名称(如 `id__i32`, `Pair__i32_str`);
//! 4. 消除所有模板定义,将特化后的具体定义追加到 AST 中供后续管线编译。

pub mod mangle;
pub mod subst;

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
}

impl Monomorphizer {
    fn new() -> Self {
        let mut known_types = HashSet::new();
        for s in &[
            "i32", "i64", "u32", "u64", "f32", "f64", "bool", "str", "char", "unit", "vec", "Box",
        ] {
            known_types.insert(s.to_string());
        }
        Self {
            struct_templates: HashMap::new(),
            fn_templates: HashMap::new(),
            known_types,
            instantiated_structs: HashMap::new(),
            instantiated_fns: HashMap::new(),
        }
    }

    fn collect_templates(&mut self, program: &Program) -> Result<()> {
        for s in &program.statements {
            match &s.node {
                Stmt::Struct(d) => {
                    self.known_types.insert(d.name.clone());
                    if !d.type_params.is_empty() {
                        self.validate_struct_template(d)?;
                        self.struct_templates.insert(d.name.clone(), d.clone());
                    }
                }
                Stmt::Enum(d) => {
                    self.known_types.insert(d.name.clone());
                }
                Stmt::Fn(f) => {
                    if !f.type_params.is_empty() {
                        self.validate_fn_template(f)?;
                        self.fn_templates.insert(f.name.clone(), (f.clone(), s.span));
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn validate_struct_template(&self, d: &StructDef) -> Result<()> {
        for field in &d.fields {
            self.validate_type_params(&field.field_type, &d.type_params, &d.name)?;
        }
        Ok(())
    }

    fn validate_fn_template(&self, f: &FnStmt) -> Result<()> {
        for p in &f.params {
            self.validate_type_params(&p.param_type, &f.type_params, &f.name)?;
        }
        if let Some(ret) = &f.return_type {
            self.validate_type_params(ret, &f.type_params, &f.name)?;
        }
        Ok(())
    }

    fn validate_type_params(&self, ty: &Type, in_scope: &[String], def_name: &str) -> Result<()> {
        match ty {
            Type::Generic(n) | Type::Named(n) => {
                if !in_scope.contains(n) && !self.known_types.contains(n) {
                    let candidates = in_scope.iter().map(|s| s.as_str()).chain(self.known_types.iter().map(|s| s.as_str()));
                    let hint = did_you_mean(n, candidates);
                    let msg = match hint {
                        Some(h) => format!("Undefined type variable '{}' in '{}', did you mean '{}'?", n, def_name, h),
                        None => format!("Undefined type variable '{}' in '{}'", n, def_name),
                    };
                    return Err(HuziError::new_global(msg));
                }
            }
            Type::Box(inner) => self.validate_type_params(inner, in_scope, def_name)?,
            Type::Applied(_, args) => {
                for arg in args {
                    self.validate_type_params(arg, in_scope, def_name)?;
                }
            }
            Type::Array(elem, _) => self.validate_type_params(elem, in_scope, def_name)?,
            Type::Tuple(elems) => {
                for elem in elems {
                    self.validate_type_params(elem, in_scope, def_name)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn validate_type_arg(&self, ty: &Type) -> Result<()> {
        match ty {
            Type::Named(n) => {
                if !self.known_types.contains(n) && !self.instantiated_structs.contains_key(n) {
                    let hint = did_you_mean(n, self.known_types.iter().map(|s| s.as_str()));
                    let msg = match hint {
                        Some(h) => format!("Unknown type '{}', did you mean '{}'?", n, h),
                        None => format!("Unknown type '{}'", n),
                    };
                    return Err(HuziError::new_global(msg));
                }
            }
            Type::Box(inner) => self.validate_type_arg(inner)?,
            Type::Applied(_, args) => {
                for a in args {
                    self.validate_type_arg(a)?;
                }
            }
            Type::Array(elem, _) => self.validate_type_arg(elem)?,
            Type::Tuple(elems) => {
                for elem in elems {
                    self.validate_type_arg(elem)?;
                }
            }
            _ => {}
        }
        Ok(())
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
                        Some(h) => HuziError::new_global(format!("Unknown generic struct '{}', did you mean '{}'?", name, h)),
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
                    self.instantiated_structs.insert(mangled.clone(), spec.clone());
                    for field in &mut spec.fields {
                        self.monomorphize_type(&mut field.field_type)?;
                    }
                    self.instantiated_structs.insert(mangled.clone(), spec);
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
                if !c.type_args.is_empty() {
                    let callee_name = match &*c.callee {
                        Expr::Ident(n) => n.clone(),
                        _ => return Ok(()),
                    };
                    let (template, span) = self.fn_templates.get(&callee_name).cloned().ok_or_else(|| {
                        let hint = did_you_mean(&callee_name, self.fn_templates.keys().map(|s| s.as_str()));
                        match hint {
                            Some(h) => HuziError::new_global(format!("Unknown generic function '{}', did you mean '{}'?", callee_name, h)),
                            None => HuziError::new_global(format!("Unknown generic function '{}'", callee_name)),
                        }
                    })?;
                    if c.type_args.len() != template.type_params.len() {
                        return Err(HuziError::new_global(format!(
                            "Generic function '{}' expects {} type argument(s), got {}",
                            callee_name,
                            template.type_params.len(),
                            c.type_args.len()
                        )));
                    }
                    for a in &c.type_args {
                        self.validate_type_arg(a)?;
                    }
                    let mangled = mangle_name(&callee_name, &c.type_args);
                    if !self.instantiated_fns.contains_key(&mangled) {
                        let mapping: HashMap<String, Type> = template
                            .type_params
                            .iter()
                            .cloned()
                            .zip(c.type_args.iter().cloned())
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
                        self.instantiated_fns.insert(mangled.clone(), (spec.clone(), span));
                        self.monomorphize_block(&mut spec.body)?;
                        self.instantiated_fns.insert(mangled.clone(), (spec, span));
                    }
                    c.callee = Box::new(Expr::Ident(mangled));
                    c.type_args.clear();
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
            Expr::EnumConstruct(e) => {
                for a in &mut e.args {
                    self.monomorphize_expr(a)?;
                }
            }
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
                }
                if let Some(val) = &mut l.value {
                    self.monomorphize_expr(val)?;
                }
            }
            Stmt::Expr(e) => self.monomorphize_expr(&mut e.expr)?,
            Stmt::Return(r) => {
                if let Some(v) = &mut r.value {
                    self.monomorphize_expr(v)?;
                }
            }
            Stmt::Block(b) => self.monomorphize_block(b)?,
            Stmt::If(i) => {
                self.monomorphize_expr(&mut i.condition)?;
                self.monomorphize_block(&mut i.then_branch)?;
                for (cond, blk) in &mut i.elif_branches {
                    self.monomorphize_expr(cond)?;
                    self.monomorphize_block(blk)?;
                }
                if let Some(b) = &mut i.else_branch {
                    self.monomorphize_block(b)?;
                }
            }
            Stmt::For(f) => {
                match &mut f.source {
                    ForSource::Range { start, end } => {
                        self.monomorphize_expr(start)?;
                        self.monomorphize_expr(end)?;
                    }
                    ForSource::Array(arr) => self.monomorphize_expr(arr)?,
                }
                self.monomorphize_block(&mut f.body)?;
            }
            Stmt::While(w) => {
                self.monomorphize_expr(&mut w.condition)?;
                self.monomorphize_block(&mut w.body)?;
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
                        mono.monomorphize_block(&mut f.body)?;
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
                mono.monomorphize_block(&mut f.body)?;
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
