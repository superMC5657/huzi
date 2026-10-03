//! `--dump-ast-json` 子集序列化：与 hzast `json.hz` 逐字节一致（见 `examples/hzast/RFC.md`）。
//!
//! - 紧凑、无空白、键序固定；转义仅 `"` `\` `\n` `\r` `\t`（与 `jsonq::escape_json_str` 同口径）。
//! - Box 透明、Span 丢弃；`un` 统一为 `operand` 单字段；`if` 空 else 为 `[]`。
//! - 子集外变体返回 `Err`（文件模式非零退出；向量模式未知 ID 返回 `None`）。
use huzi_ast::{BinOp, Block, Expr, FnStmt, Literal, Program, Stmt, Type, UnOp};
/// JSON 字符串转义（冻结 5 种，其余字节透传，不做 `\u`）。
pub fn escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 2);
    for c in input.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out
}
/// 带引号的 JSON 字符串。
pub(crate) fn quoted(input: &str) -> String {
    format!("\"{}\"", escape(input))
}
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
        Literal::String(s) => Ok(format!("{{\"kind\":\"str\",\"value\":{}}}", quoted(s))),
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
        quoted(&name),
        exprs_to_json(&c.arguments)?
    ))
}
/// 表达式递归（子集：Literal/Ident/Binary/Unary/Call + P3 复合 tuple/array/index/enum/try + P3b struct/field/method/fstring）。
pub fn expr_to_json(expr: &Expr) -> Result<String, String> {
    match expr {
        Expr::Literal(l) => literal_to_json(l),
        Expr::Ident(n) => Ok(format!("{{\"kind\":\"var\",\"name\":{}}}", quoted(n))),
        Expr::Binary(b) => Ok(format!(
            "{{\"kind\":\"bin\",\"op\":{},\"left\":{},\"right\":{}}}",
            quoted(bin_op_str(&b.operator)),
            expr_to_json(&b.left)?,
            expr_to_json(&b.right)?
        )),
        Expr::Unary(u) => Ok(format!(
            "{{\"kind\":\"un\",\"op\":{},\"operand\":{}}}",
            quoted(un_op_str(&u.operator)),
            expr_to_json(&u.operand)?
        )),
        Expr::Call(c) => call_to_json(c),
        Expr::TupleLiteral(es) => Ok(format!("{{\"kind\":\"tuple\",\"elems\":{}}}", exprs_to_json(es)?)),
        Expr::ArrayLiteral(es) => Ok(format!("{{\"kind\":\"array\",\"elems\":{}}}", exprs_to_json(es)?)),
        Expr::ArrayIndex(a) => Ok(format!("{{\"kind\":\"index\",\"array\":{},\"index\":{}}}", expr_to_json(&a.array)?, expr_to_json(&a.index)?)),
        Expr::EnumConstruct(c) => Ok(format!("{{\"kind\":\"enum\",\"name\":{},\"args\":{}}}", quoted(&format!("{}::{}", c.enum_name, c.variant)), exprs_to_json(&c.args)?)),
        Expr::Try(t) => Ok(format!("{{\"kind\":\"try\",\"expr\":{}}}", expr_to_json(&t.inner)?)),
        Expr::StructLiteral(s) => struct_to_json(s),
        Expr::FieldAccess(f) => Ok(format!("{{\"kind\":\"field\",\"base\":{},\"field\":{}}}", expr_to_json(&f.base)?, quoted(&f.field))),
        Expr::MethodCall(m) => Ok(format!("{{\"kind\":\"method\",\"receiver\":{},\"method\":{},\"args\":{}}}", expr_to_json(&m.receiver)?, quoted(&m.method), exprs_to_json(&m.arguments)?)),
        Expr::FString(f) => Ok(format!("{{\"kind\":\"fstring\",\"template\":{},\"args\":{}}}", quoted(&f.template), exprs_to_json(&f.args)?)),
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
        fields.push_str(&format!("{{\"name\":{},\"value\":{}}}", quoted(name), expr_to_json(value)?));
    }
    fields.push(']');
    Ok(format!("{{\"kind\":\"struct\",\"name\":{},\"fields\":{}}}", quoted(&s.name), fields))
}
/// 类型递归（P3b：仅名+参；复合 Array/Tuple/Box/Weak/Fn 为超集）。
pub fn type_to_json(ty: &Type) -> Result<String, String> {
    if let Some(n) = ty.canonical_name() {
        return Ok(format!("{{\"kind\":\"type\",\"name\":{},\"args\":[]}}", quoted(n)));
    }
    if matches!(ty, Type::Unit) {
        return Ok(format!("{{\"kind\":\"type\",\"name\":{},\"args\":[]}}", quoted("()")));
    }
    if let Type::Generic(n) = ty {
        return Ok(format!("{{\"kind\":\"type\",\"name\":{},\"args\":[]}}", quoted(n)));
    }
    if let Type::Applied(n, args) = ty {
        return Ok(format!("{{\"kind\":\"type\",\"name\":{},\"args\":{}}}", quoted(n), types_to_json(args)?));
    }
    Err("ast-json: type outside P3b subset".to_string())
}
/// 类型列表。
fn types_to_json(tys: &[Type]) -> Result<String, String> {
    let mut out = String::from("[");
    for (i, t) in tys.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&type_to_json(t)?);
    }
    out.push(']');
    Ok(out)
}
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
                expr_to_json(v)?
            ))
        }
        Stmt::If(i) => stmt_if_to_json(i),
        Stmt::While(w) => Ok(format!(
            "{{\"kind\":\"while\",\"cond\":{},\"body\":{}}}",
            expr_to_json(&w.condition)?,
            block_to_json(&w.body)?
        )),
        _ => Err("ast-json: stmt outside M1-C subset".to_string()),
    }
}
/// `Expr` 语句分发：赋值/打印折叠，其余为普通表达式语句。
fn stmt_expr_to_json(e: &huzi_ast::ExprStmt) -> Result<String, String> {
    if let Expr::Assign(a) = &e.expr {
        if a.operator == huzi_ast::AssignOp::Assign {
            if let Expr::Ident(name) = a.target.as_ref() {
                return Ok(format!(
                    "{{\"kind\":\"assign\",\"name\":{},\"expr\":{}}}",
                    quoted(name),
                    expr_to_json(&a.value)?
                ));
            }
            return Err("ast-json: non-ident assign outside subset".to_string());
        }
        return Err("ast-json: compound assign outside M1-C subset".to_string());
    }
    if let Expr::Call(c) = &e.expr {
        if c.type_args.is_empty() {
            if let Expr::Ident(name) = c.callee.as_ref() {
                if name == "print" && c.arguments.len() == 1 {
                    return Ok(format!(
                        "{{\"kind\":\"print\",\"expr\":{}}}",
                        expr_to_json(&c.arguments[0])?
                    ));
                }
            }
        }
    }
    Ok(format!(
        "{{\"kind\":\"expr\",\"expr\":{}}}",
        expr_to_json(&e.expr)?
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
        quoted(&l.name),
        expr_to_json(v)?
    ))
}
/// `if` 语句（elif 不在子集；空 else 为 `[]`）。
fn stmt_if_to_json(i: &huzi_ast::IfStmt) -> Result<String, String> {
    if !i.elif_branches.is_empty() {
        return Err("ast-json: elif outside M1-C subset".to_string());
    }
    let els = match &i.else_branch {
        Some(b) => block_to_json(b)?,
        None => "[]".to_string(),
    };
    Ok(format!(
        "{{\"kind\":\"if\",\"cond\":{},\"then\":{},\"else\":{}}}",
        expr_to_json(&i.condition)?,
        block_to_json(&i.then_branch)?,
        els
    ))
}
/// 块内逐语句数组。
pub(crate) fn block_to_json(block: &Block) -> Result<String, String> {
    let mut out = String::from("[");
    for (i, s) in block.statements.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&stmt_to_json(&s.node)?);
    }
    out.push(']');
    Ok(out)
}
/// 函数定义（忽略泛型形参/返回类型标注，只取名/参数名/体）。
pub(crate) fn fn_to_json(func: &FnStmt) -> Result<String, String> {
    let mut params = String::from("[");
    for (i, p) in func.params.iter().enumerate() {
        if i > 0 {
            params.push(',');
        }
        params.push_str(&quoted(&p.name));
    }
    params.push(']');
    Ok(format!(
        "{{\"name\":{},\"params\":{},\"body\":{}}}",
        quoted(&func.name),
        params,
        block_to_json(&func.body)?
    ))
}
/// 文件模式整程序（`fns` 为除 main 外顶层 fn；`main` 为 main 体或顶层非 fn 语句）。
pub fn program_to_json(program: &Program) -> Result<String, String> {
    let mut fns = String::from("[");
    let mut main_stmts: Vec<String> = Vec::new();
    let mut main_body: Option<String> = None;
    let mut first_fn = true;
    for s in &program.statements {
        if let Stmt::Fn(f) = &s.node {
            if f.name == "main" {
                main_body = Some(block_to_json(&f.body)?);
            } else {
                if !first_fn {
                    fns.push(',');
                }
                first_fn = false;
                fns.push_str(&fn_to_json(f)?);
            }
        } else {
            main_stmts.push(stmt_to_json(&s.node)?);
        }
    }
    fns.push(']');
    let main = match main_body {
        Some(b) => b,
        None => {
            let mut m = String::from("[");
            for (i, s) in main_stmts.iter().enumerate() {
                if i > 0 {
                    m.push(',');
                }
                m.push_str(s);
            }
            m.push(']');
            m
        }
    };
    Ok(format!("{{\"fns\":{fns},\"main\":{main}}}"))
}
fn synth_span(stmt: Stmt) -> huzi_ast::Spanned<Stmt> {
    huzi_ast::Spanned::with_span(stmt, huzi_ast::Span::new(1, 1))
}
fn synth_block(stmt: Stmt) -> Block {
    Block {
        statements: vec![synth_span(stmt)],
    }
}
fn synth_num(n: i64) -> Expr {
    Expr::Literal(Literal::Int(n))
}
fn synth_bin(op: BinOp, l: Expr, r: Expr) -> Expr {
    Expr::Binary(huzi_ast::BinaryExpr {
        left: Box::new(l),
        operator: op,
        right: Box::new(r),
    })
}
fn synth_assign(name: &str, value: Expr) -> Stmt {
    Stmt::Expr(huzi_ast::ExprStmt {
        expr: Expr::Assign(huzi_ast::AssignExpr {
            target: Box::new(Expr::Ident(name.to_string())),
            operator: huzi_ast::AssignOp::Assign,
            value: Box::new(value),
        }),
    })
}
fn synth_let(name: &str, value: Expr) -> Stmt {
    Stmt::Let(huzi_ast::LetStmt {
        name: name.to_string(),
        mutable: false,
        tuple_pattern: None,
        type_annotation: None,
        value: Some(value),
    })
}
/// 向量 ID 的表达式 JSON（8+8+6 个，与 `vectors.hz` 同构；中 8 为 P3 复合，后 6 为 P3b 结构类型）。
fn test_expr_json(id: &str) -> Option<Result<String, String>> {
    match id {
        "expr_num" => Some(expr_to_json(&synth_num(42))),
        "expr_bool" => Some(expr_to_json(&Expr::Literal(Literal::Bool(true)))),
        "expr_str" => Some(expr_to_json(&Expr::Literal(Literal::String(
            "hello huzi".to_string(),
        )))),
        "expr_str_esc" => Some(expr_to_json(&Expr::Literal(Literal::String(
            "a\"b\\c\nd\te\rf".to_string(),
        )))),
        "expr_var" => Some(expr_to_json(&Expr::Ident("x".to_string()))),
        "expr_bin" => Some(expr_to_json(&synth_bin(BinOp::Add, synth_num(3), synth_num(4)))),
        "expr_un" => Some(expr_to_json(&Expr::Unary(huzi_ast::UnaryExpr {
            operator: UnOp::Neg,
            operand: Box::new(synth_num(5)),
        }))),
        "expr_call" => Some(expr_to_json(&Expr::Call(huzi_ast::CallExpr {
            callee: Box::new(Expr::Ident("fact".to_string())),
            arguments: vec![synth_num(5)],
            type_args: vec![],
        }))),
        "expr_tuple" => Some(expr_to_json(&Expr::TupleLiteral(vec![synth_num(1), synth_num(2)]))),
        "expr_array" => Some(expr_to_json(&Expr::ArrayLiteral(vec![synth_num(1), synth_num(2)]))),
        "expr_index" => Some(expr_to_json(&Expr::ArrayIndex(huzi_ast::ArrayIndexExpr { array: Box::new(Expr::Ident("a".to_string())), index: Box::new(synth_num(0)) }))),
        "expr_enum_some" => Some(expr_to_json(&Expr::EnumConstruct(huzi_ast::EnumConstructExpr { enum_name: "Option".to_string(), variant: "Some".to_string(), args: vec![synth_num(42)], type_args: vec![] }))),
        "expr_enum_none" => Some(expr_to_json(&Expr::EnumConstruct(huzi_ast::EnumConstructExpr { enum_name: "Option".to_string(), variant: "None".to_string(), args: vec![], type_args: vec![] }))),
        "expr_enum_ok" => Some(expr_to_json(&Expr::EnumConstruct(huzi_ast::EnumConstructExpr { enum_name: "Result".to_string(), variant: "Ok".to_string(), args: vec![synth_num(99)], type_args: vec![] }))),
        "expr_enum_err" => Some(expr_to_json(&Expr::EnumConstruct(huzi_ast::EnumConstructExpr { enum_name: "Result".to_string(), variant: "Err".to_string(), args: vec![Expr::Literal(Literal::String("oops".to_string()))], type_args: vec![] }))),
        "expr_try" => Some(expr_to_json(&Expr::Try(huzi_ast::TryExpr { inner: Box::new(Expr::Ident("x".to_string())) }))),
        "expr_struct" => Some(expr_to_json(&Expr::StructLiteral(huzi_ast::StructLiteralExpr { name: "Point".to_string(), fields: vec![("x".to_string(), synth_num(10)), ("y".to_string(), synth_num(20))], type_args: vec![] }))),
        "expr_field" => Some(expr_to_json(&Expr::FieldAccess(huzi_ast::FieldAccessExpr { base: Box::new(Expr::Ident("p".to_string())), field: "x".to_string() }))),
        "expr_method" => Some(expr_to_json(&Expr::MethodCall(huzi_ast::MethodCallExpr { receiver: Box::new(Expr::Ident("p".to_string())), method: "sum".to_string(), arguments: vec![] }))),
        "expr_fstring" => Some(expr_to_json(&Expr::FString(huzi_ast::FStringExpr { template: "hi {}!".to_string(), args: vec![Expr::Ident("name".to_string())] }))),
        "type_atom" => Some(type_to_json(&Type::I32)),
        "type_applied" => Some(type_to_json(&Type::Applied("Pair".to_string(), vec![Type::I32, Type::Str]))),
        _ => None,
    }
}
/// 向量 ID 的语句 JSON（7 个，与 `vectors.hz` 同构）。
fn test_stmt_json(id: &str) -> Option<Result<String, String>> {
    match id {
        "stmt_let" => Some(stmt_to_json(&synth_let("x", synth_num(10)))),
        "stmt_assign" => Some(stmt_to_json(&synth_assign(
            "x",
            synth_bin(BinOp::Add, Expr::Ident("x".to_string()), synth_num(20)),
        ))),
        "stmt_if" => Some(stmt_to_json(&Stmt::If(huzi_ast::IfStmt {
            condition: synth_bin(BinOp::Gt, Expr::Ident("a".to_string()), synth_num(10)),
            then_branch: synth_block(synth_assign("r", synth_num(100))),
            elif_branches: vec![],
            else_branch: Some(synth_block(synth_assign("r", synth_num(200)))),
        }))),
        "stmt_while" => Some(stmt_to_json(&Stmt::While(huzi_ast::WhileStmt {
            condition: synth_bin(BinOp::Le, Expr::Ident("i".to_string()), synth_num(10)),
            body: synth_block(synth_assign(
                "i",
                synth_bin(BinOp::Add, Expr::Ident("i".to_string()), synth_num(1)),
            )),
        }))),
        "stmt_return" => Some(stmt_to_json(&Stmt::Return(huzi_ast::ReturnStmt {
            value: Some(Expr::Ident("x".to_string())),
        }))),
        "stmt_print" => Some(stmt_to_json(&Stmt::Expr(huzi_ast::ExprStmt {
            expr: Expr::Call(huzi_ast::CallExpr {
                callee: Box::new(Expr::Ident("print".to_string())),
                arguments: vec![Expr::Literal(Literal::String("hi".to_string()))],
                type_args: vec![],
            }),
        }))),
        "stmt_expr" => Some(stmt_to_json(&Stmt::Expr(huzi_ast::ExprStmt {
            expr: Expr::Call(huzi_ast::CallExpr {
                callee: Box::new(Expr::Ident("f".to_string())),
                arguments: vec![],
                type_args: vec![],
            }),
        }))),
        _ => None,
    }
}
/// fact 函数体（与 `vectors.hz` 同构：if 空 else + 递归 return）。
fn synth_fact_fn() -> FnStmt {
    let cond = synth_bin(BinOp::Le, Expr::Ident("n".to_string()), synth_num(1));
    let then_b = synth_block(Stmt::Return(huzi_ast::ReturnStmt {
        value: Some(synth_num(1)),
    }));
    let recur = Expr::Call(huzi_ast::CallExpr {
        callee: Box::new(Expr::Ident("fact".to_string())),
        arguments: vec![synth_bin(
            BinOp::Sub,
            Expr::Ident("n".to_string()),
            synth_num(1),
        )],
        type_args: vec![],
    });
    let ret = synth_bin(BinOp::Mul, Expr::Ident("n".to_string()), recur);
    FnStmt {
        name: "fact".to_string(),
        type_params: vec![],
        params: vec![huzi_ast::FnParam {
            name: "n".to_string(),
            param_type: huzi_ast::Type::I32,
        }],
        return_type: None,
        body: Block {
            statements: vec![
                synth_span(Stmt::If(huzi_ast::IfStmt {
                    condition: cond,
                    then_branch: then_b,
                    elif_branches: vec![],
                    else_branch: None,
                })),
                synth_span(Stmt::Return(huzi_ast::ReturnStmt { value: Some(ret) })),
            ],
        },
    }
}
/// 向量 ID 分发（表达式 → 语句 → 程序；未知返回 `None`）。
pub fn test_vector(id: &str) -> Option<Result<String, String>> {
    if let Some(r) = test_expr_json(id) {
        return Some(r);
    }
    if let Some(r) = test_stmt_json(id) {
        return Some(r);
    }
    if id == "prog_fact" {
        let fj = match fn_to_json(&synth_fact_fn()) {
            Ok(s) => s,
            Err(e) => return Some(Err(e)),
        };
        let main = synth_block(Stmt::Return(huzi_ast::ReturnStmt {
            value: Some(Expr::Call(huzi_ast::CallExpr {
                callee: Box::new(Expr::Ident("fact".to_string())),
                arguments: vec![synth_num(5)],
                type_args: vec![],
            })),
        }));
        let mj = match block_to_json(&main) {
            Ok(s) => s,
            Err(e) => return Some(Err(e)),
        };
        return Some(Ok(format!("{{\"fns\":[{fj}],\"main\":{mj}}}")));
    }
    None
}
