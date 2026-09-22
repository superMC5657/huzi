//! 泛型单态化 (Monomorphization):在代码生成前将泛型函数与泛型结构体特化为具体 AST。
//!
//! 流程:
//! 1. 收集泛型函数与结构体模板,校验类型变量合法性;
//! 2. 遍历 AST,遇到显式实参调用/构造/类型注解或推导调用时触发按需单态化;
//! 3. 深拷贝模板 AST 并替换类型实参,修饰名称(如 `id__i32`, `Pair__i32_str`);
//! 4. 消除所有模板定义,将特化后的具体定义追加到 AST 中供后续管线编译。
//!
//! 子模块:`walk`(AST 遍历与按需单态化触发)。

mod infer;
pub mod mangle;
pub mod subst;
mod validate;
mod walk;

pub use mangle::mangle_name;
pub use subst::{substitute_block, substitute_type};

use super::ModuleCode;
use crate::codegen::qname::bare_name;
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
    /// 当前正在遍历的语句位置(调用点诊断用):进入每条语句前更新,
    /// 诊断构造时回填行列,保证错误含 span 位置。
    current_span: Option<Span>,
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
            current_span: None,
        }
    }

    /// 以当前语句位置构造诊断:有位置则带行列,无则退回全局错误。
    fn diag(&self, message: String) -> HuziError {
        match self.current_span {
            Some(span) => HuziError::new(message, span.line, span.column),
            None => HuziError::new_global(message),
        }
    }

    fn monomorphize_type(&mut self, ty: &mut Type) -> Result<()> {
        match ty {
            Type::Applied(name, args) => {
                for a in args.iter_mut() {
                    self.monomorphize_type(a)?;
                }
                // 内置容器不做单态化:`vec<T>` 与 `Map<K,V>` 直接放行。
                if name == "vec" || name == "map" || name == "Map" || name == "HashMap" {
                    return Ok(());
                }
                let template = self.struct_templates.get(name).cloned().ok_or_else(|| {
                    let hint = did_you_mean(name, self.struct_templates.keys().map(|s| s.as_str()));
                    let mut message = format!(
                        "未知泛型结构体 '{}':期望已定义的泛型结构体,实际未找到",
                        name
                    );
                    if let Some(h) = hint {
                        message.push_str(&format!(";帮助:{}", h));
                    }
                    self.diag(message)
                })?;
                if args.len() != template.type_params.len() {
                    return Err(self.diag(format!(
                        "泛型结构体 '{}' 类型实参数量不匹配:期望 {} 个,实际 {} 个",
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
        let bare = bare_name(callee_name);
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
                let inferred = self
                    .inferrer
                    .infer_call_type_args(callee_name, template, arguments)
                    .map_err(|e| match self.current_span {
                        Some(span) => e.with_position(span.line, span.column),
                        None => e,
                    })?;
                *type_args = inferred;
            }
        }
        if type_args.is_empty() {
            return Ok(None);
        }
        // 模板按定义名(不带模块前缀)收录:限定调用(result::is_ok)取
        // 末段查找,单态化产物也以裸名注册进主程序。
        let bare_callee = bare_name(callee_name);
        let (template, span) = self
            .fn_templates
            .get(bare_callee)
            .cloned()
            .ok_or_else(|| {
                let hint = did_you_mean(callee_name, self.fn_templates.keys().map(|s| s.as_str()));
                let mut message = format!(
                    "未知泛型函数 '{}':期望已定义的泛型函数,实际未找到",
                    callee_name
                );
                if let Some(h) = hint {
                    message.push_str(&format!(";帮助:{}", h));
                }
                self.diag(message)
            })?;
        if type_args.len() != template.type_params.len() {
            return Err(self.diag(format!(
                "泛型函数 '{}' 类型实参数量不匹配:期望 {} 个,实际 {} 个;请写成 `{}<{}>(...)`",
                callee_name,
                template.type_params.len(),
                type_args.len(),
                bare_callee,
                template.type_params.join(", ")
            )));
        }
        for a in type_args.iter() {
            self.validate_type_arg(a)?;
        }
        let mangled = mangle_name(bare_callee, type_args);
        if !self.instantiated_fns.contains_key(&mangled) {
            self.instantiate_fn(&template, span, type_args, &mangled)?;
        }
        Ok(Some(mangled))
    }

    /// 实例化泛型函数模板:按实参映射替换签名与函数体,登记单态化
    /// 产物并补齐签名,最后对函数体做嵌套单态化遍历(模板内的 `?`、
    /// 限定调用等在此展开)。
    fn instantiate_fn(
        &mut self,
        template: &FnStmt,
        span: Span,
        type_args: &[Type],
        mangled: &str,
    ) -> Result<()> {
        let mapping: HashMap<String, Type> = template
            .type_params
            .iter()
            .cloned()
            .zip(type_args.iter().cloned())
            .collect();
        let mut spec = template.clone();
        spec.name = mangled.to_string();
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
            .insert(mangled.to_string(), (spec.clone(), span));
        self.inferrer.fn_signatures.insert(
            mangled.to_string(),
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
        self.instantiated_fns.insert(mangled.to_string(), (spec, span));
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
            monomorphize_statements(&mut mono, &mut prog.statements)?;
        }
    }

    // 阶段 3: 单态化主程序代码
    let mut new_program = program.clone();
    monomorphize_statements(&mut mono, &mut new_program.statements)?;

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

/// 对一份语句表做单态化遍历(阶段 2/3 共用):非泛型顶层函数做
/// 形参/返回类型替换与函数体遍历,其余语句直接遍历。
fn monomorphize_statements(
    mono: &mut Monomorphizer,
    statements: &mut [Spanned<Stmt>],
) -> Result<()> {
    for s in statements {
        mono.current_span = Some(s.span);
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
    Ok(())
}
