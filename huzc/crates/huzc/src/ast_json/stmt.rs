//! 语句递归（子集：Let/Expr/Return/If/While；Fn 由 `fn_to_json` 另行处理）。
//! 纯搬移自 `ast_json.rs`，逻辑逐行原样，仅跨模块引用加 `super::` 前缀。
use huzi_ast::Stmt;

/// 语句递归（子集：Let/Expr/Return/If/While；Fn 由 `fn_to_json` 另行处理）。
/// `x = v`（AssignExpr）与 `print(e)`（单参 print 调用）折叠为 Huzi 侧
/// `assign`/`print` 语句形，保证两侧逐字节一致；其余为 `expr` 语句形。
pub fn stmt_to_json(stmt: &Stmt) -> Result<String, String> {
    match stmt {
        Stmt::Let(l) => stmt_let_to_json(l),
        Stmt::Expr(e) => stmt_expr_to_json(e),
        Stmt::Return(r) => {
            let v = r
                .value
                .as_ref()
                .ok_or_else(|| "ast-json: bare return outside subset".to_string())?;
            Ok(format!(
                "{{\"kind\":\"return\",\"expr\":{}}}",
                super::expr::expr_to_json(v)?
            ))
        }
        Stmt::If(i) => stmt_if_to_json(i),
        Stmt::While(w) => Ok(format!(
            "{{\"kind\":\"while\",\"cond\":{},\"body\":{}}}",
            super::expr::expr_to_json(&w.condition)?,
            super::block_to_json(&w.body)?
        )),
        _ => Err("ast-json: stmt outside M1-C subset".to_string()),
    }
}

/// `Expr` 语句分发：赋值/打印折叠，其余为普通表达式语句。
fn stmt_expr_to_json(e: &huzi_ast::ExprStmt) -> Result<String, String> {
    if let huzi_ast::Expr::Assign(a) = &e.expr {
        if a.operator == huzi_ast::AssignOp::Assign {
            if let huzi_ast::Expr::Ident(name) = a.target.as_ref() {
                return Ok(format!(
                    "{{\"kind\":\"assign\",\"name\":{},\"expr\":{}}}",
                    super::quoted(name),
                    super::expr::expr_to_json(&a.value)?
                ));
            }
            return Err("ast-json: non-ident assign outside subset".to_string());
        }
        return Err("ast-json: compound assign outside M1-C subset".to_string());
    }
    if let huzi_ast::Expr::Call(c) = &e.expr {
        if c.type_args.is_empty() {
            if let huzi_ast::Expr::Ident(name) = c.callee.as_ref() {
                if name == "print" && c.arguments.len() == 1 {
                    return Ok(format!(
                        "{{\"kind\":\"print\",\"expr\":{}}}",
                        super::expr::expr_to_json(&c.arguments[0])?
                    ));
                }
            }
        }
    }
    Ok(format!(
        "{{\"kind\":\"expr\",\"expr\":{}}}",
        super::expr::expr_to_json(&e.expr)?
    ))
}

/// `let` 语句（元组模式/无初值/类型标注不在子集）。
fn stmt_let_to_json(l: &huzi_ast::LetStmt) -> Result<String, String> {
    if l.tuple_pattern.is_some() {
        return Err("ast-json: tuple let outside M1-C subset".to_string());
    }
    let v = l
        .value
        .as_ref()
        .ok_or_else(|| "ast-json: let without value outside subset".to_string())?;
    Ok(format!(
        "{{\"kind\":\"let\",\"name\":{},\"expr\":{}}}",
        super::quoted(&l.name),
        super::expr::expr_to_json(v)?
    ))
}

/// `if` 语句（elif 不在子集；空 else 为 `[]`）。
fn stmt_if_to_json(i: &huzi_ast::IfStmt) -> Result<String, String> {
    if !i.elif_branches.is_empty() {
        return Err("ast-json: elif outside M1-C subset".to_string());
    }
    let els = match &i.else_branch {
        Some(b) => super::block_to_json(b)?,
        None => "[]".to_string(),
    };
    Ok(format!(
        "{{\"kind\":\"if\",\"cond\":{},\"then\":{},\"else\":{}}}",
        super::expr::expr_to_json(&i.condition)?,
        super::block_to_json(&i.then_branch)?,
        els
    ))
}
