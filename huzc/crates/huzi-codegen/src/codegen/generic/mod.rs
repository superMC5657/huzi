//! 泛型单态化 (Monomorphization):在代码生成前将泛型函数与泛型结构体特化为具体 AST。
//!
//! 流程:
//! 1. 收集泛型函数与结构体模板,校验类型变量合法性;
//! 2. 遍历 AST,遇到显式实参调用/构造/类型注解或推导调用时触发按需单态化;
//! 3. 深拷贝模板 AST 并替换类型实参,修饰名称(如 `id__i32`, `Pair__i32_str`);
//! 4. 消除所有模板定义,将特化后的具体定义追加到 AST 中供后续管线编译。
//!
//! 子模块:`dispatch`(类型遍历与调用点分派)、`instantiate`(模板特化与产物登记)、
//! `walk`(AST 遍历与按需单态化触发)。

mod dispatch;
mod infer;
mod instantiate;
pub mod mangle;
pub mod subst;
mod validate;
mod walk;

pub use mangle::mangle_name;
pub use subst::{substitute_block, substitute_type};

use super::ModuleCode;
use huzi_ast::*;
use huzi_error::{HuziError, Result};
use std::collections::{HashMap, HashSet};

/// 单态化器状态。
struct Monomorphizer {
    struct_templates: HashMap<String, StructDef>,
    enum_templates: HashMap<String, EnumDef>,
    fn_templates: HashMap<String, (FnStmt, Span)>,
    known_types: HashSet<String>,
    instantiated_structs: HashMap<String, StructDef>,
    instantiated_enums: HashMap<String, EnumDef>,
    instantiated_fns: HashMap<String, (FnStmt, Span)>,
    inferrer: infer::TypeInferrer,
    /// 当前正在遍历的语句位置(调用点诊断用):进入每条语句前更新,
    /// 诊断构造时回填行列,保证错误含 span 位置。
    current_span: Option<Span>,
    /// 当前正在遍历的函数返回类型(供 return 表达式推导泛型枚举用)。
    current_fn_return: Option<Type>,
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
            enum_templates: HashMap::new(),
            fn_templates: HashMap::new(),
            known_types,
            instantiated_structs: HashMap::new(),
            instantiated_enums: HashMap::new(),
            instantiated_fns: HashMap::new(),
            inferrer: infer::TypeInferrer::new(),
            current_span: None,
            current_fn_return: None,
        }
    }

    /// 以当前语句位置构造诊断:有位置则带行列,无则退回全局错误。
    fn diag(&self, message: String) -> HuziError {
        match self.current_span {
            Some(span) => HuziError::new(message, span.line, span.column),
            None => HuziError::new_global(message),
        }
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
            mono.collect_module_templates(&m.name, prog)?;
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
        Stmt::Enum(d) => d.type_params.is_empty(),
        Stmt::Fn(f) => f.type_params.is_empty(),
        _ => true,
    });
    for m in modules.iter_mut() {
        if let Some(prog) = &mut m.program {
            prog.statements.retain(|s| match &s.node {
                Stmt::Struct(d) => d.type_params.is_empty(),
                Stmt::Enum(d) => d.type_params.is_empty(),
                Stmt::Fn(f) => f.type_params.is_empty(),
                _ => true,
            });
        }
    }

    for def in mono.instantiated_structs.into_values() {
        new_program.statements.push(Spanned::new(Stmt::Struct(def), 1, 1));
    }
    for def in mono.instantiated_enums.into_values() {
        new_program.statements.push(Spanned::new(Stmt::Enum(def), 1, 1));
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
                mono.inferrer.fn_signatures.insert(
                    f.name.clone(),
                    (
                        f.params.iter().map(|p| p.param_type.clone()).collect(),
                        f.return_type.clone(),
                    ),
                );
                mono.current_fn_return = f.return_type.clone();
                mono.inferrer.enter_scope();
                for p in &f.params {
                    mono.inferrer.insert_var(&p.name, p.param_type.clone());
                }
                mono.monomorphize_block(&mut f.body)?;
                mono.inferrer.leave_scope();
                mono.current_fn_return = None;
            }
        } else {
            mono.monomorphize_stmt(&mut s.node)?;
        }
    }
    Ok(())
}
