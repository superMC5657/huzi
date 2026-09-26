//! 左值与赋值:变量/字段/下标赋值、取址、字段 GEP、结构体定义查询。
//!
//! 子模块:`assign`(赋值三路分发)、`addr`(取址与字段定位)。
//! (自 `expr_place.rs` 纯搬移,零逻辑变化。)

mod addr;
mod assign;
