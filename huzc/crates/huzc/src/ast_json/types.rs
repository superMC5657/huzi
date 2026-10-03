//! 类型递归（P3b：仅名+参；复合 Array/Tuple/Box/Weak/Fn 为超集）。
//! 纯搬移自 `ast_json.rs`，逻辑逐行原样，仅跨模块引用加 `super::` 前缀。
use huzi_ast::Type;

/// 类型递归（P3b：仅名+参；复合 Array/Tuple/Box/Weak/Fn 为超集）。
pub fn type_to_json(ty: &Type) -> Result<String, String> {
    if let Some(n) = ty.canonical_name() {
        return Ok(format!(
            "{{\"kind\":\"type\",\"name\":{},\"args\":[]}}",
            super::quoted(n)
        ));
    }
    if matches!(ty, Type::Unit) {
        return Ok(format!(
            "{{\"kind\":\"type\",\"name\":{},\"args\":[]}}",
            super::quoted("()")
        ));
    }
    if let Type::Generic(n) = ty {
        return Ok(format!(
            "{{\"kind\":\"type\",\"name\":{},\"args\":[]}}",
            super::quoted(n)
        ));
    }
    if let Type::Applied(n, args) = ty {
        return Ok(format!(
            "{{\"kind\":\"type\",\"name\":{},\"args\":{}}}",
            super::quoted(n),
            types_to_json(args)?
        ));
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
