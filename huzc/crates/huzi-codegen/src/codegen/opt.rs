//! 进程内 LLVM IR 优化:替代原外部 `opt -S -O<level>` 调用。
//!
//! 流水线 `default<Olevel>` 即 `opt -O<level>` 运行的新 Pass 管理器默认流水线
//! (已含寄存器提升/内联/常量折叠/CSE),不再依赖外部 opt 二进制,
//! 缺 opt 环境下 release 照常用。llc/link 仍走外部工具链
//! (见 `huzc/src/main.rs`)。

use super::CodeGen;
use huzi_error::{HuziError, Result};
use inkwell::OptimizationLevel;
use inkwell::passes::PassBuilderOptions;
use inkwell::targets::{CodeModel, InitializationConfig, RelocMode, Target, TargetMachine};

/// 将 huzc 优化级别(0-3)映射为 LLVM TargetMachine 优化等级。
/// 0 级不做优化,由调用方提前返回,此处防御性归一为 Less。
fn llvm_opt_level(level: u8) -> OptimizationLevel {
    match level {
        0 | 1 => OptimizationLevel::Less,
        2 => OptimizationLevel::Default,
        _ => OptimizationLevel::Aggressive,
    }
}

/// 将 huzc 优化级别映射为新 Pass 管理器流水线字符串,
/// 与 `opt -O<level>` 运行的默认流水线一致。
/// 注:不可写成 `mem2reg,default<O2>`——顶层为模块流水线,裸函数 Pass 名会
/// 使解析器误入函数 Pass 上下文而报错 `unknown function pass 'default<O2>'`
/// (已用 opt-18 实测确认);默认流水线本身已含 mem2reg 等价变换。
fn pipeline_for(level: u8) -> String {
    match level {
        0 | 1 => "default<O1>".to_string(),
        2 => "default<O2>".to_string(),
        _ => "default<O3>".to_string(),
    }
}

impl<'ctx> CodeGen<'ctx> {
    /// 在内存中对当前模块运行优化流水线。
    /// `level` 为 0 时直接返回 Ok(不触碰模块);否则按级别运行
    /// `default<Olevel>`。`verify_each` 为 true 时每个 Pass 后校验
    /// (开发模式显式 `--opt-level` 用 true,release 用 false 提速)。
    pub fn optimize(&self, level: u8, verify_each: bool) -> Result<()> {
        if level == 0 {
            return Ok(());
        }
        Target::initialize_all(&InitializationConfig::default());
        let triple = TargetMachine::get_default_triple();
        let target = Target::from_triple(&triple)
            .map_err(|e| HuziError::new_global(format!("Error: no LLVM target for triple: {}", e)))?;
        let machine = target
            .create_target_machine(
                &triple,
                "",
                "",
                llvm_opt_level(level),
                RelocMode::PIC,
                CodeModel::Default,
            )
            .ok_or_else(|| HuziError::new_global("Error: failed to create LLVM target machine"))?;
        let options = PassBuilderOptions::create();
        options.set_verify_each(verify_each);
        self.module
            .run_passes(&pipeline_for(level), &machine, options)
            .map_err(|e| HuziError::new_global(format!("Error running optimization passes: {}", e)))?;
        Ok(())
    }
}
