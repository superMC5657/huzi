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

fn builtin_fn_return_types() -> HashMap<String, Option<Type>> {
    let mut map = HashMap::new();
    let i32_fns = [
        "len", "push", "clear", "remove", "insert", "abs", "read_int", "write_file",
        "parse_int", "time", "arg_count", "exit", "sleep_ms", "map_len",
    ];
    for f in i32_fns {
        map.insert(f.to_string(), Some(Type::I32));
    }
    let str_fns = [
        "to_string", "concat", "trim", "substring", "read_line", "read_file", "arg",
    ];
    for f in str_fns {
        map.insert(f.to_string(), Some(Type::Str));
    }
    let bool_fns = ["contains", "arg_ok", "is_eof", "map_has"];
    for f in bool_fns {
        map.insert(f.to_string(), Some(Type::Bool));
    }
    let f64_fns = [
        "sqrt", "sin", "cos", "tan", "floor", "ceil", "round", "pow", "read_float", "parse_float",
    ];
    for f in f64_fns {
        map.insert(f.to_string(), Some(Type::F64));
    }
    map.insert(
        "split".to_string(),
        Some(Type::Applied("vec".to_string(), vec![Type::Str])),
    );
    for f in ["vec_map", "vec_filter", "map", "filter", "vec_take", "take", "vec_skip", "skip"] {
        map.insert(
            f.to_string(),
            Some(Type::Applied("vec".to_string(), vec![])),
        );
    }
    for f in ["vec_any", "any", "vec_all", "all"] {
        map.insert(f.to_string(), Some(Type::Bool));
    }
    for f in ["vec_count", "count", "vec_for_each", "for_each"] {
        map.insert(f.to_string(), Some(Type::I32));
    }
    for f in [
        "pop", "print", "println", "eprint", "eprintln", "panic", "assert", "vec_fold", "fold",
    ] {
        map.insert(f.to_string(), None);
    }
    map
}

impl TraitDesugarer {
    pub(super) fn new() -> Self {
        let mut known_types = HashSet::new();
        for s in &[
            "i32", "i64", "u32", "u64", "f32", "f64", "bool", "str", "char", "unit", "vec", "Box",
        ] {
            known_types.insert(s.to_string());
        }
        Self {
            traits: HashMap::new(),
            known_types,
            struct_fields: HashMap::new(),
            implemented_methods: HashMap::new(),
            method_return_types: HashMap::new(),
            fn_return_types: builtin_fn_return_types(),
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
                    rewrite_self_in_fn(&mut desugared, &i.target_type);
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
                            rewrite_self_in_fn(&mut desugared, &i.target_type);
                            fns.push((desugared, s.span));
                        }
                    }
                }
            }
        }
        Ok(fns)
    }
}

fn rewrite_self_in_fn(f: &mut FnStmt, target_type: &str) {
    if let Some(ret) = &mut f.return_type {
        *ret = ret.substitute_self(target_type);
    }
    for p in &mut f.params {
        p.param_type = p.param_type.substitute_self(target_type);
    }
    rewrite_self_in_block(&mut f.body, target_type);
}

fn rewrite_self_in_block(block: &mut Block, target_type: &str) {
    for s in &mut block.statements {
        rewrite_self_in_stmt(&mut s.node, target_type);
    }
}

fn rewrite_self_in_stmt(stmt: &mut Stmt, target_type: &str) {
    match stmt {
        Stmt::Let(l) => {
            if let Some(ann) = &mut l.type_annotation {
                *ann = ann.substitute_self(target_type);
            }
            if let Some(val) = &mut l.value {
                rewrite_self_in_expr(val, target_type);
            }
        }
        Stmt::Expr(e) => rewrite_self_in_expr(&mut e.expr, target_type),
        Stmt::Return(r) => {
            if let Some(val) = &mut r.value {
                rewrite_self_in_expr(val, target_type);
            }
        }
        Stmt::Block(b) => rewrite_self_in_block(b, target_type),
        Stmt::If(i) => {
            rewrite_self_in_expr(&mut i.condition, target_type);
            rewrite_self_in_block(&mut i.then_branch, target_type);
            for (cond, blk) in &mut i.elif_branches {
                rewrite_self_in_expr(cond, target_type);
                rewrite_self_in_block(blk, target_type);
            }
            if let Some(b) = &mut i.else_branch {
                rewrite_self_in_block(b, target_type);
            }
        }
        Stmt::While(w) => {
            rewrite_self_in_expr(&mut w.condition, target_type);
            rewrite_self_in_block(&mut w.body, target_type);
        }
        Stmt::For(f) => {
            match &mut f.source {
                ForSource::Range { start, end } => {
                    rewrite_self_in_expr(start, target_type);
                    rewrite_self_in_expr(end, target_type);
                }
                ForSource::Array(arr) => rewrite_self_in_expr(arr, target_type),
            }
            rewrite_self_in_block(&mut f.body, target_type);
        }
        Stmt::Defer(d) => rewrite_self_in_stmt(&mut d.node, target_type),
        _ => {}
    }
}

fn rewrite_self_in_expr(expr: &mut Expr, target_type: &str) {
    match expr {
        Expr::StructLiteral(s) => {
            if s.name == "Self" {
                s.name = target_type.to_string();
            }
            for (_, val) in &mut s.fields {
                rewrite_self_in_expr(val, target_type);
            }
        }
        Expr::EnumConstruct(e) => {
            if e.enum_name == "Self" {
                e.enum_name = target_type.to_string();
            }
            for a in &mut e.args {
                rewrite_self_in_expr(a, target_type);
            }
        }
        Expr::MethodCall(mc) => {
            rewrite_self_in_expr(&mut mc.receiver, target_type);
            for a in &mut mc.arguments {
                rewrite_self_in_expr(a, target_type);
            }
        }
        Expr::Call(c) => {
            rewrite_self_in_expr(&mut c.callee, target_type);
            for a in &mut c.arguments {
                rewrite_self_in_expr(a, target_type);
            }
        }
        Expr::Binary(b) => {
            rewrite_self_in_expr(&mut b.left, target_type);
            rewrite_self_in_expr(&mut b.right, target_type);
        }
        Expr::Unary(u) => rewrite_self_in_expr(&mut u.operand, target_type),
        Expr::Assign(a) => {
            rewrite_self_in_expr(&mut a.target, target_type);
            rewrite_self_in_expr(&mut a.value, target_type);
        }
        Expr::ArrayIndex(a) => {
            rewrite_self_in_expr(&mut a.array, target_type);
            rewrite_self_in_expr(&mut a.index, target_type);
        }
        Expr::ArrayLiteral(elems) | Expr::TupleLiteral(elems) => {
            for e in elems {
                rewrite_self_in_expr(e, target_type);
            }
        }
        Expr::BoxAlloc(inner) => rewrite_self_in_expr(inner, target_type),
        Expr::FieldAccess(f) => rewrite_self_in_expr(&mut f.base, target_type),
        Expr::If(i) => {
            rewrite_self_in_expr(&mut i.condition, target_type);
            rewrite_self_in_block(&mut i.then_branch, target_type);
            rewrite_self_in_block(&mut i.else_branch, target_type);
        }
        Expr::Match(m) => {
            rewrite_self_in_expr(&mut m.scrutinee, target_type);
            for arm in &mut m.arms {
                if let Some(guard) = &mut arm.guard {
                    rewrite_self_in_expr(guard, target_type);
                }
                rewrite_self_in_block(&mut arm.body, target_type);
            }
        }
        _ => {}
    }
}
