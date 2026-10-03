//! 表达式递归（子集：Literal/Ident/Binary/Unary/Call + P3 复合 tuple/array/index/enum/try + P3b struct/field/method/fstring + P3c match/closure）。
//! 纯搬移自 `ast_json.rs`，逻辑逐行原样，仅跨模块引用加 `super::` 前缀。
use huzi_ast::{BinOp, Expr, Literal, UnOp};

/// 二元运算符 spelling（与 hzast `op` 字符串一致）。
fn bin_op_str(op: &BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Mod => "%",
        BinOp::Eq => "==",
        BinOp::Neq => "!=",
        BinOp::Lt => "<",
        BinOp::Le => "<=",
        BinOp::Gt => ">",
        BinOp::Ge => ">=",
        BinOp::And => "&&",
        BinOp::Or => "||",
    }
}

/// 一元运算符 spelling。
fn un_op_str(op: &UnOp) -> &'static str {
    match op {
        UnOp::Neg => "-",
        UnOp::Not => "!",
        UnOp::Deref => "*",
    }
}

/// 字面量直映（仅 Int/Bool/String；Float/Char 不在 M1-C 子集）。
fn literal_to_json(lit: &Literal) -> Result<String, String> {
    match lit {
        Literal::Int(n) => Ok(format!("{{\"kind\":\"num\",\"value\":{n}}}")),
        Literal::Bool(b) => Ok(format!(
            "{{\"kind\":\"bool\",\"value\":{}}}",
            if *b { "true" } else { "false" }
        )),
        Literal::String(s) => Ok(format!(
            "{{\"kind\":\"str\",\"value\":{}}}",
            super::quoted(s)
        )),
        Literal::Float(_) => Err("ast-json: float outside M1-C subset".to_string()),
        Literal::Char(_) => Err("ast-json: char outside M1-C subset".to_string()),
    }
}

/// 调用形（被调须为裸 Ident，泛型实参须空）。
fn call_to_json(c: &huzi_ast::CallExpr) -> Result<String, String> {
    if !c.type_args.is_empty() {
        return Err("ast-json: generic call outside M1-C subset".to_string());
    }
    let name = match c.callee.as_ref() {
        Expr::Ident(n) => n.clone(),
        _ => return Err("ast-json: non-ident callee outside M1-C subset".to_string()),
    };
    Ok(format!(
        "{{\"kind\":\"call\",\"name\":{},\"args\":{}}}",
        super::quoted(&name),
        exprs_to_json(&c.arguments)?
    ))
}

/// 表达式递归（子集见模块头）。
pub fn expr_to_json(expr: &Expr) -> Result<String, String> {
    match expr {
        Expr::Literal(l) => literal_to_json(l),
        Expr::Ident(n) => Ok(format!(
            "{{\"kind\":\"var\",\"name\":{}}}",
            super::quoted(n)
        )),
        Expr::Binary(b) => Ok(format!(
            "{{\"kind\":\"bin\",\"op\":{},\"left\":{},\"right\":{}}}",
            super::quoted(bin_op_str(&b.operator)),
            expr_to_json(&b.left)?,
            expr_to_json(&b.right)?
        )),
        Expr::Unary(u) => Ok(format!(
            "{{\"kind\":\"un\",\"op\":{},\"operand\":{}}}",
            super::quoted(un_op_str(&u.operator)),
            expr_to_json(&u.operand)?
        )),
        Expr::Call(c) => call_to_json(c),
        Expr::TupleLiteral(es) => Ok(format!(
            "{{\"kind\":\"tuple\",\"elems\":{}}}",
            exprs_to_json(es)?
        )),
        Expr::ArrayLiteral(es) => Ok(format!(
            "{{\"kind\":\"array\",\"elems\":{}}}",
            exprs_to_json(es)?
        )),
        Expr::ArrayIndex(a) => Ok(format!(
            "{{\"kind\":\"index\",\"array\":{},\"index\":{}}}",
            expr_to_json(&a.array)?,
            expr_to_json(&a.index)?
        )),
        Expr::EnumConstruct(c) => Ok(format!(
            "{{\"kind\":\"enum\",\"name\":{},\"args\":{}}}",
            super::quoted(&format!("{}::{}", c.enum_name, c.variant)),
            exprs_to_json(&c.args)?
        )),
        Expr::Try(t) => Ok(format!(
            "{{\"kind\":\"try\",\"expr\":{}}}",
            expr_to_json(&t.inner)?
        )),
        Expr::StructLiteral(s) => struct_to_json(s),
        Expr::FieldAccess(f) => Ok(format!(
            "{{\"kind\":\"field\",\"base\":{},\"field\":{}}}",
            expr_to_json(&f.base)?,
            super::quoted(&f.field)
        )),
        Expr::MethodCall(m) => Ok(format!(
            "{{\"kind\":\"method\",\"receiver\":{},\"method\":{},\"args\":{}}}",
            expr_to_json(&m.receiver)?,
            super::quoted(&m.method),
            exprs_to_json(&m.arguments)?
        )),
        Expr::FString(f) => Ok(format!(
            "{{\"kind\":\"fstring\",\"template\":{},\"args\":{}}}",
            super::quoted(&f.template),
            exprs_to_json(&f.args)?
        )),
        Expr::Match(m) => super::match_closure::match_to_json(m),
        Expr::Closure(c) => super::match_closure::closure_to_json(c),
        _ => Err("ast-json: expr outside subset".to_string()),
    }
}

/// 表达式列表。
fn exprs_to_json(es: &[Expr]) -> Result<String, String> {
    let mut out = String::from("[");
    for (i, e) in es.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&expr_to_json(e)?);
    }
    out.push(']');
    Ok(out)
}

/// 结构体字面量（泛型实参须空；字段为 `[{"name":..,"value":..}]`，键序固定）。
fn struct_to_json(s: &huzi_ast::StructLiteralExpr) -> Result<String, String> {
    if !s.type_args.is_empty() {
        return Err("ast-json: generic struct outside P3b subset".to_string());
    }
    let mut fields = String::from("[");
    for (i, (name, value)) in s.fields.iter().enumerate() {
        if i > 0 {
            fields.push(',');
        }
        fields.push_str(&format!(
            "{{\"name\":{},\"value\":{}}}",
            super::quoted(name),
            expr_to_json(value)?
        ));
    }
    fields.push(']');
    Ok(format!(
        "{{\"kind\":\"struct\",\"name\":{},\"fields\":{}}}",
        super::quoted(&s.name),
        fields
    ))
}
