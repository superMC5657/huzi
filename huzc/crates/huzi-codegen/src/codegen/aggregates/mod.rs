//! 聚合值构造:结构体字面量、枚举构造、数组字面量/下标、`if` 表达式值。
//!
//! 子模块:`struct_lit`(结构体字面量)、`enum_ctor`(枚举构造与变体解析)、
//! `array_if`(数组下标/字面量与 `if` 表达式)。(自 `aggregates.rs` 纯搬移,零逻辑变化。)

mod array_if;
mod enum_ctor;
mod struct_lit;
