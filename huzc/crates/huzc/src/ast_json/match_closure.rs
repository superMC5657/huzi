//! P3c match/closure 子集序列化（字面量/枚举变体形状臂 + 守卫 + 无捕获闭包）。
//! 与 hzast `json_p3c.hz` 逐字节一致，键序固定，紧凑无空白。
//! 缺臂（空 arms）/ 非形状臂（Variable/Wildcard/Float/Char/泛型闭包/块闭包）一律 `Err` 双非零。
use huzi_ast::{ClosureBody, MatchArm, MatchExpr, Pattern};

/// 模式直映（仅 Int/Bool/String 字面量 + Option/Result 变体；其余双非零）。
fn pattern_to_json(p: &Pattern) -> Result<String, String> {
    match p {
        Pattern::Literal(lit) => match lit {
            huzi_ast::Literal::Int(n) => Ok(format!("{{\"kind\":\"num\",\"value\":{n}}}")),
            huzi_ast::Literal::Bool(b) => Ok(format!(
                "{{\"kind\":\"bool\",\"value\":{}}}",
                if *b { "true" } else { "false" }
            )),
            huzi_ast::Literal::String(s) => {
                Ok(format!("{{\"kind\":\"str\",\"value\":{}}}", super::quoted(s)))
            }
            huzi_ast::Literal::Float(_) => {
                Err("ast-json: float pattern outside P3c subset".to_string())
            }
            huzi_ast::Literal::Char(_) => {
                Err("ast-json: char pattern outside P3c subset".to_string())
            }
        },
        Pattern::Variant {
            enum_name,
            variant,
            bindings,
        } => variant_pattern_to_json(enum_name, variant, bindings),
        Pattern::Variable(_) => Err("ast-json: variable pattern outside P3c subset".to_string()),
        Pattern::Wildcard => Err("ast-json: wildcard outside P3c subset".to_string()),
    }
}

/// 变体模式（仅 Option/Result × Some/None/Ok/Err；其余枚举双非零）。
fn variant_pattern_to_json(
    enum_name: &str,
    variant: &str,
    bindings: &[String],
) -> Result<String, String> {
    let is_comp = matches!(enum_name, "Option" | "Result")
        && matches!(variant, "Some" | "None" | "Ok" | "Err");
    if !is_comp {
        return Err("ast-json: non-Option/Result pattern outside P3c subset".to_string());
    }
    let mut bs = String::from("[");
    for (i, b) in bindings.iter().enumerate() {
        if i > 0 {
            bs.push(',');
        }
        bs.push_str(&super::quoted(b));
    }
    bs.push(']');
    Ok(format!(
        "{{\"kind\":\"variant\",\"name\":{},\"bindings\":{}}}",
        super::quoted(&format!("{enum_name}::{variant}")),
        bs
    ))
}

/// 单臂（键序 pattern/guard/body；无守卫统一为 true 表达式，保证同构单形）。
fn arm_to_json(arm: &MatchArm) -> Result<String, String> {
    let pat = pattern_to_json(&arm.pattern)?;
    let guard = match &arm.guard {
        Some(g) => super::expr::expr_to_json(g)?,
        None => "{\"kind\":\"bool\",\"value\":true}".to_string(),
    };
    let body = super::block_to_json(&arm.body)?;
    Ok(format!(
        "{{\"pattern\":{pat},\"guard\":{guard},\"body\":{body}}}"
    ))
}

/// match 表达式（键序 kind/scrutinee/arms；空臂双非零）。
pub fn match_to_json(m: &MatchExpr) -> Result<String, String> {
    if m.arms.is_empty() {
        return Err("ast-json: empty match arms outside P3c subset".to_string());
    }
    let scr = super::expr::expr_to_json(&m.scrutinee)?;
    let mut arms = String::from("[");
    for (i, a) in m.arms.iter().enumerate() {
        if i > 0 {
            arms.push(',');
        }
        arms.push_str(&arm_to_json(a)?);
    }
    arms.push(']');
    Ok(format!(
        "{{\"kind\":\"match\",\"scrutinee\":{scr},\"arms\":{arms}}}"
    ))
}

/// 闭包表达式（仅无类型参 + 无返回标注 + 单表达式体；块体/类型双非零）。
pub fn closure_to_json(c: &huzi_ast::ClosureExpr) -> Result<String, String> {
    if c.return_type.is_some() {
        return Err("ast-json: closure return type outside P3c subset".to_string());
    }
    for p in &c.params {
        if p.param_type.is_some() {
            return Err("ast-json: typed closure param outside P3c subset".to_string());
        }
    }
    let body = match &c.body {
        ClosureBody::Expr(e) => super::expr::expr_to_json(e)?,
        ClosureBody::Block(_) => {
            return Err("ast-json: block closure outside P3c subset".to_string());
        }
    };
    let mut params = String::from("[");
    for (i, p) in c.params.iter().enumerate() {
        if i > 0 {
            params.push(',');
        }
        params.push_str(&super::quoted(&p.name));
    }
    params.push(']');
    Ok(format!(
        "{{\"kind\":\"closure\",\"params\":{params},\"body\":{body}}}"
    ))
}

/// P3c 合成原语（自包含，不依赖 vectors.rs，避免跨文件可见性漂移）。
fn p3c_span(stmt: huzi_ast::Stmt) -> huzi_ast::Spanned<huzi_ast::Stmt> {
    huzi_ast::Spanned::with_span(stmt, huzi_ast::Span::new(1, 1))
}

/// 单表达式体块（`=> expr` 糖的 `Block{ [ExprStmt] }` 形，与 parser::expr_block 同构）。
fn p3c_block_expr(e: huzi_ast::Expr) -> huzi_ast::Block {
    huzi_ast::Block {
        statements: vec![p3c_span(huzi_ast::Stmt::Expr(huzi_ast::ExprStmt {
            expr: e,
        }))],
    }
}

fn p3c_num(n: i64) -> huzi_ast::Expr {
    huzi_ast::Expr::Literal(huzi_ast::Literal::Int(n))
}

fn p3c_var(name: &str) -> huzi_ast::Expr {
    huzi_ast::Expr::Ident(name.to_string())
}

fn p3c_bin(op: huzi_ast::BinOp, l: huzi_ast::Expr, r: huzi_ast::Expr) -> huzi_ast::Expr {
    huzi_ast::Expr::Binary(huzi_ast::BinaryExpr {
        left: Box::new(l),
        operator: op,
        right: Box::new(r),
    })
}

/// 单臂合成（体为单表达式语句块；守卫 None 即无守卫 true 形）。
fn p3c_arm(pat: Pattern, guard: Option<huzi_ast::Expr>, body_expr: huzi_ast::Expr) -> MatchArm {
    MatchArm {
        pattern: pat,
        guard,
        body: p3c_block_expr(body_expr),
    }
}

/// 字面量 match：`match x { 1 => 10, 2 => 20 }`（双臂无守卫）。
fn synth_match_lit() -> huzi_ast::Expr {
    huzi_ast::Expr::Match(MatchExpr {
        scrutinee: Box::new(p3c_var("x")),
        arms: vec![
            p3c_arm(
                Pattern::Literal(huzi_ast::Literal::Int(1)),
                None,
                p3c_num(10),
            ),
            p3c_arm(
                Pattern::Literal(huzi_ast::Literal::Int(2)),
                None,
                p3c_num(20),
            ),
        ],
    })
}

/// 守卫 match：`match x { 1 if x > 0 => 10, 2 => 20 }`。
fn synth_match_guard() -> huzi_ast::Expr {
    let guard = p3c_bin(huzi_ast::BinOp::Gt, p3c_var("x"), p3c_num(0));
    huzi_ast::Expr::Match(MatchExpr {
        scrutinee: Box::new(p3c_var("x")),
        arms: vec![
            p3c_arm(
                Pattern::Literal(huzi_ast::Literal::Int(1)),
                Some(guard),
                p3c_num(10),
            ),
            p3c_arm(
                Pattern::Literal(huzi_ast::Literal::Int(2)),
                None,
                p3c_num(20),
            ),
        ],
    })
}

/// 枚举变体 match：`match opt { Some(v) => 1, None => 0 }`。
fn synth_match_enum() -> huzi_ast::Expr {
    huzi_ast::Expr::Match(MatchExpr {
        scrutinee: Box::new(p3c_var("opt")),
        arms: vec![
            p3c_arm(
                Pattern::Variant {
                    enum_name: "Option".to_string(),
                    variant: "Some".to_string(),
                    bindings: vec!["v".to_string()],
                },
                None,
                p3c_num(1),
            ),
            p3c_arm(
                Pattern::Variant {
                    enum_name: "Option".to_string(),
                    variant: "None".to_string(),
                    bindings: vec![],
                },
                None,
                p3c_num(0),
            ),
        ],
    })
}

/// 变体+守卫 match：`match opt { Some(v) if flag => 1, None => 0 }`（守卫引外层 flag）。
fn synth_match_enum_guard() -> huzi_ast::Expr {
    huzi_ast::Expr::Match(MatchExpr {
        scrutinee: Box::new(p3c_var("opt")),
        arms: vec![
            p3c_arm(
                Pattern::Variant {
                    enum_name: "Option".to_string(),
                    variant: "Some".to_string(),
                    bindings: vec!["v".to_string()],
                },
                Some(p3c_var("flag")),
                p3c_num(1),
            ),
            p3c_arm(
                Pattern::Variant {
                    enum_name: "Option".to_string(),
                    variant: "None".to_string(),
                    bindings: vec![],
                },
                None,
                p3c_num(0),
            ),
        ],
    })
}

/// 无捕获闭包：`|x| x + 1`（单参无类型单表达式体）。
fn synth_closure() -> huzi_ast::Expr {
    huzi_ast::Expr::Closure(huzi_ast::ClosureExpr {
        params: vec![huzi_ast::ClosureParam {
            name: "x".to_string(),
            param_type: None,
        }],
        return_type: None,
        body: ClosureBody::Expr(Box::new(p3c_bin(
            huzi_ast::BinOp::Add,
            p3c_var("x"),
            p3c_num(1),
        ))),
    })
}

/// 零参闭包：`|| 42`（MPEM 零参形）。
fn synth_closure_zero() -> huzi_ast::Expr {
    huzi_ast::Expr::Closure(huzi_ast::ClosureExpr {
        params: vec![],
        return_type: None,
        body: ClosureBody::Expr(Box::new(p3c_num(42))),
    })
}

/// P3c 向量 ID 分发（每形状一 ID：lit/guard/enum/enum_guard/closure/closure_zero，共 6）。
pub(crate) fn test_p3c_json(id: &str) -> Option<Result<String, String>> {
    match id {
        "expr_match_lit" => Some(super::expr::expr_to_json(&synth_match_lit())),
        "expr_match_guard" => Some(super::expr::expr_to_json(&synth_match_guard())),
        "expr_match_enum" => Some(super::expr::expr_to_json(&synth_match_enum())),
        "expr_match_enum_guard" => Some(super::expr::expr_to_json(&synth_match_enum_guard())),
        "expr_closure" => Some(super::expr::expr_to_json(&synth_closure())),
        "expr_closure_zero" => Some(super::expr::expr_to_json(&synth_closure_zero())),
        _ => None,
    }
}
