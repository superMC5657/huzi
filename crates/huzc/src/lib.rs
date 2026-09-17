//! huzc 库入口:对外只暴露纯函数模块加载器(无 codegen 类型进出签名),
//! 供 huzi-lsp 等常驻进程复用;CLI 流程仍以 `src/main.rs` 为入口。
//!
//! - `load_modules`:CLI 兼容入口,出错 `die`,成功语义与旧版一致。
//! - `load_modules_result`:可恢复的磁盘版,返回 `Err` 不退出进程。
//! - `load_modules_from_memory`:纯内存版,不读盘,多错收集。

pub mod fmt;
mod modules;

pub use fmt::{format_program, format_source};
pub use modules::{
    LoadedModule, load_modules, load_modules_from_memory, load_modules_result,
};

/// 与 `main.rs::die` 同文的错误出口:库内兼容入口 `load_modules`
/// 出错时沿用 CLI 的“打印并退出码 1”行为。
pub(crate) fn die(msg: String) -> ! {
    eprintln!("{}", msg);
    std::process::exit(1);
}
