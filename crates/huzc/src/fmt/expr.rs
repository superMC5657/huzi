use huzi_ast::*;

pub(super) fn format_expr(expr: &Expr) -> String {
    match expr {
        Expr::Literal(lit) => format_literal(lit),
        Expr::Ident(s) => s.clone(),
        Expr::Binary(b) => format_binary(b),
        Expr::Unary(u) => format_unary(u),
        Expr::Call(c) => format_call(c),
        Expr::Assign(a) => format!("{} = {}", format_expr(&a.target), format_expr(&a.value)),
        Expr::ArrayIndex(a) => format_array_index(a),
        Expr::ArrayLiteral(elems) => format_array_literal(elems),
        Expr::TupleLiteral(elems) => format_tuple_literal(elems),
        Expr::VecEmpty(ty) => format!("vec<{}>()", ty),
        Expr::BoxAlloc(inner) => format!("box({})", format_expr(inner)),
        Expr::Null => "null".to_string(),
        Expr::If(i) => format_if_expr(i),
        Expr::FieldAccess(f) => format_field_access(f),
        Expr::StructLiteral(s) => format_struct_literal(s),
        Expr::EnumConstruct(e) => format_enum_construct(e),
        Expr::Match(m) => format_match_expr(m),
        Expr::MethodCall(m) => format_method_call(m),
    }
}

fn format_literal(lit: &Literal) -> String {
    match lit {
        Literal::Int(n) => n.to_string(),
        Literal::Float(f) => {
            let s = f.to_string();
            if !s.contains('.') && !s.contains('e') && !s.contains('E') {
                format!("{}.0", s)
            } else {
                s
            }
        }
        Literal::Bool(b) => if *b { "true".to_string() } else { "false".to_string() },
        Literal::String(s) => format_string_lit(s),
        Literal::Char(c) => format_char_lit(*c),
    }
}

fn format_string_lit(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\0' => out.push_str("\\0"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

fn format_char_lit(c: char) -> String {
    let mut out = String::new();
    out.push('\'');
    match c {
        '\n' => out.push_str("\\n"),
        '\t' => out.push_str("\\t"),
        '\r' => out.push_str("\\r"),
        '\\' => out.push_str("\\\\"),
        '\'' => out.push_str("\\'"),
        '\0' => out.push_str("\\0"),
        other => out.push(other),
    }
    out.push('\'');
    out
}

pub(super) fn op_precedence(op: &BinOp) -> u8 {
    match op {
        BinOp::Or => 1,
        BinOp::And => 2,
        BinOp::Eq | BinOp::Neq => 3,
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => 4,
        BinOp::Add | BinOp::Sub => 5,
        BinOp::Mul | BinOp::Div | BinOp::Mod => 6,
    }
}

pub(super) fn expr_precedence(expr: &Expr) -> u8 {
    match expr {
        Expr::Binary(b) => op_precedence(&b.operator),
        Expr::Assign(_) => 0,
        Expr::Unary(_) => 7,
        _ => 8,
    }
}

fn format_binary(b: &BinaryExpr) -> String {
    let parent_prec = op_precedence(&b.operator);
    let left_str = format_expr(&b.left);
    let left = if expr_precedence(&b.left) < parent_prec {
        format!("({})", left_str)
    } else {
        left_str
    };
    let right_str = format_expr(&b.right);
    let right = if expr_precedence(&b.right) <= parent_prec {
        format!("({})", right_str)
    } else {
        right_str
    };
    let op_str = match b.operator {
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
    };
    format!("{} {} {}", left, op_str, right)
}

fn format_unary(u: &UnaryExpr) -> String {
    let op_str = match u.operator {
        UnOp::Neg => "-",
        UnOp::Not => "!",
    };
    let inner_str = format_expr(&u.operand);
    if expr_precedence(&u.operand) < 7 {
        format!("{}({})", op_str, inner_str)
    } else {
        format!("{}{}", op_str, inner_str)
    }
}

fn format_call(c: &CallExpr) -> String {
    let callee = format_expr(&c.callee);
    let type_args = if c.type_args.is_empty() {
        String::new()
    } else {
        let ts: Vec<_> = c.type_args.iter().map(|t| t.to_string()).collect();
        format!("<{}>", ts.join(", "))
    };
    let args = c
        .arguments
        .iter()
        .map(format_expr)
        .collect::<Vec<_>>()
        .join(", ");
    format!("{}{}({})", callee, type_args, args)
}

fn format_array_index(a: &ArrayIndexExpr) -> String {
    let base = format_expr(&a.array);
    let base = if expr_precedence(&a.array) < 8 {
        format!("({})", base)
    } else {
        base
    };
    format!("{}[{}]", base, format_expr(&a.index))
}

fn format_field_access(f: &FieldAccessExpr) -> String {
    let base = format_expr(&f.base);
    let base = if expr_precedence(&f.base) < 8 {
        format!("({})", base)
    } else {
        base
    };
    format!("{}.{}", base, f.field)
}

fn format_array_literal(elems: &[Expr]) -> String {
    let formatted: Vec<_> = elems.iter().map(format_expr).collect();
    format!("[{}]", formatted.join(", "))
}

fn format_tuple_literal(elems: &[Expr]) -> String {
    if elems.len() == 1 {
        format!("({},)", format_expr(&elems[0]))
    } else {
        let formatted: Vec<_> = elems.iter().map(format_expr).collect();
        format!("({})", formatted.join(", "))
    }
}

fn format_struct_literal(s: &StructLiteralExpr) -> String {
    let type_args = if s.type_args.is_empty() {
        String::new()
    } else {
        let ts: Vec<_> = s.type_args.iter().map(|t| t.to_string()).collect();
        format!("<{}>", ts.join(", "))
    };
    let fields: Vec<_> = s
        .fields
        .iter()
        .map(|(name, val)| format!("{}: {}", name, format_expr(val)))
        .collect();
    format!("{}{} {{ {} }}", s.name, type_args, fields.join(", "))
}

fn format_enum_construct(e: &EnumConstructExpr) -> String {
    if e.args.is_empty() {
        format!("{}::{}", e.enum_name, e.variant)
    } else {
        let args: Vec<_> = e.args.iter().map(format_expr).collect();
        format!("{}::{}({})", e.enum_name, e.variant, args.join(", "))
    }
}

fn format_inline_block(b: &Block) -> String {
    if b.statements.len() == 1 {
        if let Stmt::Expr(e) = &b.statements[0].node {
            return format_expr(&e.expr);
        }
    }
    let stmts: Vec<_> = b
        .statements
        .iter()
        .map(|s| format_stmt_inline(&s.node))
        .collect();
    stmts.join("; ")
}

pub(super) fn format_stmt_inline(stmt: &Stmt) -> String {
    match stmt {
        Stmt::Let(l) => {
            let mut s = String::from("let ");
            if l.mutable {
                s.push_str("mut ");
            }
            s.push_str(&l.name);
            if let Some(t) = &l.type_annotation {
                s.push_str(&format!(": {}", t));
            }
            if let Some(v) = &l.value {
                s.push_str(&format!(" = {}", format_expr(v)));
            }
            s
        }
        Stmt::Expr(e) => format_expr(&e.expr),
        Stmt::Return(r) => match &r.value {
            Some(v) => format!("return {}", format_expr(v)),
            None => "return".to_string(),
        },
        Stmt::Break => "break".to_string(),
        Stmt::Continue => "continue".to_string(),
        Stmt::Defer(inner) => format!("defer {}", format_stmt_inline(&inner.node)),
        _ => "<complex_stmt>".to_string(),
    }
}

fn extract_if_expr(b: &Block) -> Option<&IfExpr> {
    if b.statements.len() == 1 {
        if let Stmt::Expr(ExprStmt {
            expr: Expr::If(ref nested),
        }) = b.statements[0].node
        {
            return Some(nested);
        }
    }
    None
}

fn format_if_expr(i: &IfExpr) -> String {
    let mut out = format!(
        "if {} {{ {} }}",
        format_expr(&i.condition),
        format_inline_block(&i.then_branch)
    );
    let mut cur = i;
    while let Some(nested) = extract_if_expr(&cur.else_branch) {
        out.push_str(&format!(
            " elif {} {{ {} }}",
            format_expr(&nested.condition),
            format_inline_block(&nested.then_branch)
        ));
        cur = nested;
    }
    out.push_str(&format!(
        " else {{ {} }}",
        format_inline_block(&cur.else_branch)
    ));
    out
}

fn format_pattern(p: &Pattern) -> String {
    match p {
        Pattern::Wildcard => "_".to_string(),
        Pattern::Variant {
            enum_name,
            variant,
            bindings,
        } => {
            if bindings.is_empty() {
                format!("{}::{}", enum_name, variant)
            } else {
                format!("{}::{}({})", enum_name, variant, bindings.join(", "))
            }
        }
    }
}

fn format_match_expr(m: &MatchExpr) -> String {
    let mut out = format!("match {} {{\n", format_expr(&m.scrutinee));
    for arm in &m.arms {
        let pat_str = format_pattern(&arm.pattern);
        if arm.body.statements.len() == 1 {
            if let Stmt::Expr(e) = &arm.body.statements[0].node {
                out.push_str(&format!("    {} => {},\n", pat_str, format_expr(&e.expr)));
                continue;
            }
        }
        out.push_str(&format!("    {} => {{\n", pat_str));
        for s in &arm.body.statements {
            out.push_str(&format!("        {}\n", format_stmt_inline(&s.node)));
        }
        out.push_str("    },\n");
    }
    out.push('}');
    out
}

fn format_method_call(m: &MethodCallExpr) -> String {
    let receiver = format_expr(&m.receiver);
    let args: Vec<_> = m.arguments.iter().map(format_expr).collect();
    format!("{}.{}({})", receiver, m.method, args.join(", "))
}
