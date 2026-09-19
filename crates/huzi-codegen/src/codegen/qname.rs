//! 限定名助手。Huzi 的 `mod::fn(args)` 限定调用与 `Enum::Variant(args)`
//! 枚举构造在 AST 中同形(`EnumConstruct`),签名收录、类型推导与
//! 代码生成分派都必须按同一形状拼键与回退,集中在此防止各处拼写漂移。

/// `prefix::name` 键拼接(模块限定与枚举构造共用)。
pub(super) fn qualified(prefix: &str, name: &str) -> String {
    format!("{}::{}", prefix, name)
}

/// 限定名的末段裸名:`result::is_ok` → `is_ok`;无 `::` 时原样返回。
pub(super) fn bare_name(name: &str) -> &str {
    name.rsplit("::").next().unwrap_or(name)
}
