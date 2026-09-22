//! Trait 静态分发与方法调用脱糖 (Trait static dispatch and desugaring)。
//!
//! 流程:
//! 1. 收集所有 `trait` 声明与 `impl` 实现块;
//! 2. 校验 trait 方法完备性、签名匹配与同类型多 trait 方法冲突;
//! 3. 将 `impl` 方法转换为顶层函数 (修饰为 `TargetType__method`);
//! 4. 遍历并推导接收者类型,将 `receiver.method(args)` 静态重写为 `TargetType__method(receiver, args)`;
//! 5. 输出脱糖后的 Program。

mod desugar;
mod validate;

use super::ModuleCode;
use huzi_ast::*;
use huzi_error::Result;
use std::collections::{HashMap, HashSet};

pub fn desugar_traits(program: &Program, modules: &mut [ModuleCode]) -> Result<Program> {
    let mut desugarer = TraitDesugarer::new();
    desugarer.collect_types_and_traits(program, modules)?;
    desugarer.validate_and_collect_impls(program, modules)?;

    let mut new_statements = Vec::new();

    // 1. 生成所有 impl 脱糖出的顶层函数并对其方法体执行脱糖
    let mut generated_fns = desugarer.generate_impl_functions(program, modules)?;
    for (f, span) in &mut generated_fns {
        let span_copy = *span;
        let mut fn_stmt = Stmt::Fn(f.clone());
        desugarer
            .resolve_stmt(&mut fn_stmt)
            .map_err(|e| e.with_position(span_copy.line, span_copy.column))?;
        if let Stmt::Fn(resolved) = fn_stmt {
            *f = resolved;
        }
        new_statements.push(Spanned::with_span(Stmt::Fn(f.clone()), *span));
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
                        let span = s.span;
                        desugarer
                            .resolve_stmt(&mut cloned.node)
                            .map_err(|e| e.with_position(span.line, span.column))?;
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
                let span = s.span;
                desugarer
                    .resolve_stmt(&mut cloned.node)
                    .map_err(|e| e.with_position(span.line, span.column))?;
                new_statements.push(cloned);
            }
        }
    }

    Ok(Program {
        statements: new_statements,
    })
}

pub(super) struct TraitDesugarer {
    traits: HashMap<String, TraitDef>,
    known_types: HashSet<String>,
    struct_fields: HashMap<String, HashMap<String, Type>>,
    /// 目标类型 -> (方法名 -> trait 名)
    implemented_methods: HashMap<String, HashMap<String, String>>,
    /// 目标类型 -> (方法名 -> 返回类型)
    method_return_types: HashMap<String, HashMap<String, Option<Type>>>,
    /// 函数名 -> 返回类型
    fn_return_types: HashMap<String, Option<Type>>,
}

impl TraitDesugarer {
    pub(super) fn new() -> Self {
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

    pub(super) fn generate_impl_functions(
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
}
