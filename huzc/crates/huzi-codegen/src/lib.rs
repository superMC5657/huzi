//! Huzi 代码生成库
//!
//! 本 crate 负责将 Huzi AST 生成为 LLVM IR。
//!
//! ## 模块组织
//!
//! - **codegen::mod**: `CodeGen` 结构体、编译入口、作用域管理、公开 API
//! - **codegen::types**: 类型注册/布局、LLVM 类型映射、类型强制转换工具
//! - **codegen::stmt**: 语句编译（let、fn、if、for、while...）
//! - **codegen::expr**: 表达式编译（字面量、二元/一元运算、调用、赋值）
//! - **codegen::builtins**: prelude 声明与内置函数（print、read_*、数学、字符串）
//! - **codegen::aggregates**: 结构体字面量、枚举构造、match、数组

// codegen 的 emit_*/compile_* 助手常需同时传递 builder、目标 block、LLVM 类型、
// 变量槽等多个上下文参数，数量超过 clippy 默认阈值（7）属该层固有风格，统一放行。
#![allow(clippy::too_many_arguments)]

mod codegen;

pub use codegen::{CodeGen, ModuleCode};

/// 编译器内置模块:import 时不解析文件,`模块::函数` 调用走 builtin 调度。
pub const BUILTIN_MODULES: &[&str] = &["math"];
