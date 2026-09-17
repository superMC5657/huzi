use huzi_ast::*;
use std::collections::HashMap;

/// 替换类型中的泛型参数。
pub fn substitute_type(ty: &Type, mapping: &HashMap<String, Type>) -> Type {
    match ty {
        Type::Generic(name) | Type::Named(name) => {
            if let Some(sub) = mapping.get(name) {
                sub.clone()
            } else {
                ty.clone()
            }
        }
        Type::Box(inner) => Type::Box(Box::new(substitute_type(inner, mapping))),
        Type::Applied(name, args) => Type::Applied(
            name.clone(),
            args.iter().map(|a| substitute_type(a, mapping)).collect(),
        ),
        Type::Array(elem, size) => {
            Type::Array(Box::new(substitute_type(elem, mapping)), *size)
        }
        Type::Tuple(elems) => {
            Type::Tuple(elems.iter().map(|e| substitute_type(e, mapping)).collect())
        }
        other => other.clone(),
    }
}

/// 替换表达式中的类型参数。
pub fn substitute_expr(expr: &mut Expr, mapping: &HashMap<String, Type>) {
    match expr {
        Expr::Literal(_) | Expr::Ident(_) | Expr::Null => {}
        Expr::Binary(b) => {
            substitute_expr(&mut b.left, mapping);
            substitute_expr(&mut b.right, mapping);
        }
        Expr::Unary(u) => substitute_expr(&mut u.operand, mapping),
        Expr::Call(c) => {
            substitute_expr(&mut c.callee, mapping);
            for arg in &mut c.arguments {
                substitute_expr(arg, mapping);
            }
            for targ in &mut c.type_args {
                *targ = substitute_type(targ, mapping);
            }
        }
        Expr::Assign(a) => {
            substitute_expr(&mut a.target, mapping);
            substitute_expr(&mut a.value, mapping);
        }
        Expr::ArrayIndex(a) => {
            substitute_expr(&mut a.array, mapping);
            substitute_expr(&mut a.index, mapping);
        }
        Expr::ArrayLiteral(elems) | Expr::TupleLiteral(elems) => {
            for elem in elems {
                substitute_expr(elem, mapping);
            }
        }
        Expr::VecEmpty(ty) => *ty = substitute_type(ty, mapping),
        Expr::BoxAlloc(inner) => substitute_expr(inner, mapping),
        Expr::If(i) => {
            substitute_expr(&mut i.condition, mapping);
            substitute_block(&mut i.then_branch, mapping);
            substitute_block(&mut i.else_branch, mapping);
        }
        Expr::FieldAccess(f) => substitute_expr(&mut f.base, mapping),
        Expr::StructLiteral(s) => {
            for (_, val) in &mut s.fields {
                substitute_expr(val, mapping);
            }
            for targ in &mut s.type_args {
                *targ = substitute_type(targ, mapping);
            }
        }
        Expr::EnumConstruct(e) => {
            for arg in &mut e.args {
                substitute_expr(arg, mapping);
            }
        }
        Expr::Match(m) => {
            substitute_expr(&mut m.scrutinee, mapping);
            for arm in &mut m.arms {
                substitute_block(&mut arm.body, mapping);
            }
        }
    }
}

/// 替换语句中的类型参数。
pub fn substitute_stmt(stmt: &mut Stmt, mapping: &HashMap<String, Type>) {
    match stmt {
        Stmt::Let(l) => {
            if let Some(ann) = &mut l.type_annotation {
                *ann = substitute_type(ann, mapping);
            }
            if let Some(val) = &mut l.value {
                substitute_expr(val, mapping);
            }
        }
        Stmt::Expr(e) => substitute_expr(&mut e.expr, mapping),
        Stmt::Return(r) => {
            if let Some(val) = &mut r.value {
                substitute_expr(val, mapping);
            }
        }
        Stmt::Block(b) => substitute_block(b, mapping),
        Stmt::If(i) => {
            substitute_expr(&mut i.condition, mapping);
            substitute_block(&mut i.then_branch, mapping);
            for (cond, blk) in &mut i.elif_branches {
                substitute_expr(cond, mapping);
                substitute_block(blk, mapping);
            }
            if let Some(b) = &mut i.else_branch {
                substitute_block(b, mapping);
            }
        }
        Stmt::For(f) => {
            match &mut f.source {
                ForSource::Range { start, end } => {
                    substitute_expr(start, mapping);
                    substitute_expr(end, mapping);
                }
                ForSource::Array(arr) => substitute_expr(arr, mapping),
            }
            substitute_block(&mut f.body, mapping);
        }
        Stmt::While(w) => {
            substitute_expr(&mut w.condition, mapping);
            substitute_block(&mut w.body, mapping);
        }
        Stmt::Defer(d) => substitute_stmt(&mut d.node, mapping),
        Stmt::Struct(_) | Stmt::Enum(_) | Stmt::Fn(_) | Stmt::Import(_) | Stmt::Break | Stmt::Continue => {}
    }
}

/// 替换语句块中的类型参数。
pub fn substitute_block(block: &mut Block, mapping: &HashMap<String, Type>) {
    for stmt in &mut block.statements {
        substitute_stmt(&mut stmt.node, mapping);
    }
}
