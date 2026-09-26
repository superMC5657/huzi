//! L3/L4 级能力:std import 感知与 trait/impl 成员(补全与跳转共用)。
//!
//! - L3:按 `huzc modules.rs probe_entry_file` 语义解析模块文件
//!   (直连 `<root>/<a/b.hz>` → `<stem>/huzi.toml` 的 `lib_entry` →
//!   `<stem>/{src/lib.hz, lib.hz, src/mod.hz, mod.hz}`);读
//!   `std/lib.hz` export 表做 `std::` 补全,`<bind>::` 读目标文件
//!   符号表补全。磁盘失败一律空表,不抛错。
//! - L4:同文件 trait/impl 表转补全项(`Point.` 补 impl 方法,
//!   `Trait::` 补 trait 方法),AST 形状与 `symbols.rs` 对齐。
//!
//! 子模块:`roots`(模块入口探查与候选根)、`binds`(`<bind>::` 与文件符号补全)、
//! `members`(目标文件方法与 std 前缀/impl/trait 补全)。

mod binds;
mod members;
mod roots;

/// `<stem>` 下的入口候选(与 `modules.rs` 顺序一致)。
const STEM_CANDIDATES: &[&str] = &["src/lib.hz", "lib.hz", "src/mod.hz", "mod.hz"];

/// `huzi-src` 相对探查后缀(与 `imports.rs` 存量顺序一致)。
const HUZI_SRC_SUBS: &[&str] =
    &["huzi-src", "../huzi-src", "../../huzi-src", "../../../huzi-src"];

pub(crate) use binds::bind_items;
pub(crate) use members::{impl_method_items, std_prefix_items, trait_method_items};
pub(crate) use roots::{module_file_roots, probe_entry_file};

/// 单测注入 helpers:仅测试构建重导出(生产构建无外部调用,避免未使用重导出警告)。
#[cfg(test)]
pub(crate) use binds::bind_items_with_roots;
#[cfg(test)]
pub(crate) use members::std_prefix_items_with_lib;
#[cfg(test)]
pub(crate) use roots::resolve_in_roots;

#[cfg(test)]
mod tests;
