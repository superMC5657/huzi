//! Huzi code generation library
//!
//! This crate handles LLVM IR code generation from Huzi AST.
//!
//! ## Module Organization
//!
//! - **codegen::mod**: `CodeGen` struct, compile entry point, scope management, public API
//! - **codegen::types**: type registration/layout, LLVM type mapping, coercion utilities
//! - **codegen::stmt**: statement compilation (let, fn, if, for, while, ...)
//! - **codegen::expr**: expression compilation (literals, binary/unary ops, calls, assignment)
//! - **codegen::builtins**: prelude declarations and built-in functions (print, read_*, math, strings)
//! - **codegen::aggregates**: struct literals, enum construction, match, arrays

// codegen 的 emit_*/compile_* 助手常需同时传递 builder、目标 block、LLVM 类型、
// 变量槽等多个上下文参数，数量超过 clippy 默认阈值（7）属该层固有风格，统一放行。
#![allow(clippy::too_many_arguments)]

mod codegen;

pub use codegen::{CodeGen, ModuleCode};

/// 编译器内置模块:import 时不解析文件,`模块::函数` 调用走 builtin 调度。
pub const BUILTIN_MODULES: &[&str] = &["math"];
