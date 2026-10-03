//! `--dump-ast-json` 子集序列化：与 hzast `json.hz` 逐字节一致（见 `examples/hzast/RFC.md`）。
//!
//! - 紧凑、无空白、键序固定；转义仅 `"` `\` `\n` `\r` `\t`（与 `jsonq::escape_json_str` 同口径）。
//! - Box 透明、Span 丢弃；`un` 统一为 `operand` 单字段；`if` 空 else 为 `[]`。
//! - 子集外变体返回 `Err`（文件模式非零退出；向量模式未知 ID 返回 `None`）。
//!
//! 拆分说明（纯搬移）：类型定义与入口留本文件，子实现按组拆分
//! `expr.rs`（表达式）/`stmt.rs`（语句）/`types.rs`（类型）/`vectors.rs`（向量合成）。
//! 逻辑逐行原样搬移，仅可见性由私有提升为 `pub(super)` 以供跨子模块调用。
pub(crate) mod expr;
pub(crate) mod match_closure;
pub(crate) mod stmt;
pub(crate) mod types;
pub(crate) mod vectors;

use huzi_ast::{Block, Expr, FnStmt, Program, Stmt};

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

/// 块内逐语句数组。
pub(crate) fn block_to_json(block: &Block) -> Result<String, String> {
    let mut out = String::from("[");
    for (i, s) in block.statements.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&stmt::stmt_to_json(&s.node)?);
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
            main_stmts.push(stmt::stmt_to_json(&s.node)?);
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

/// 向量 ID 分发（表达式 → P3c → 语句 → 程序；未知返回 `None`）。
pub fn test_vector(id: &str) -> Option<Result<String, String>> {
    if let Some(r) = vectors::test_expr_json(id) {
        return Some(r);
    }
    if let Some(r) = match_closure::test_p3c_json(id) {
        return Some(r);
    }
    if let Some(r) = vectors::test_stmt_json(id) {
        return Some(r);
    }
    if id == "prog_fact" {
        let fj = match fn_to_json(&vectors::synth_fact_fn()) {
            Ok(s) => s,
            Err(e) => return Some(Err(e)),
        };
        let main = vectors::synth_block(Stmt::Return(huzi_ast::ReturnStmt {
            value: Some(Expr::Call(huzi_ast::CallExpr {
                callee: Box::new(Expr::Ident("fact".to_string())),
                arguments: vec![vectors::synth_num(5)],
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
