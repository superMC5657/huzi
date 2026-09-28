use huzc::cli::Args;
use huzc::load_modules;
use huzi_codegen::CodeGen;
use inkwell::context::Context;
use std::fs;
use std::path::Path;

use crate::die;
use crate::linker::{link, run_command};
use crate::paths::OutputPaths;

use super::front::{parse_source, read_source};

/// 编译单个 Huzi 源码文件并生成可执行文件。
pub(crate) fn compile_source_file(args: &Args) -> OutputPaths {
    let input = match &args.input {
        Some(i) => i,
        None => {
            eprintln!("error: either a subcommand (e.g. 'run', 'fmt') or '--input <file.hz>' is required");
            std::process::exit(1);
        }
    };

    // 发布模式静默运行：不输出过程日志，错误仍输出至 stderr。
    let quiet = args.release;

    let source = read_source(input);
    let mut program = parse_source(&source, quiet);

    // 解析 import:内置模块直接注册,文件模块递归加载(去重 + 防循环)。
    let base_dir = Path::new(input)
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf();
    let imported = load_modules(&mut program, &base_dir);

    // [3/5] Compiling
    if !quiet {
        println!("[3/5] Compiling...");
    }
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "huzi");
    if args.debug {
        // 调试模式:以规范化的绝对路径作为编译单元源文件。
        // 去掉 canonicalize 产生的 `\\?\` 前缀,否则 gdb/lldb 按此路径
        // 找不到源文件。
        let source_path = fs::canonicalize(input)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| input.clone());
        let source_path = source_path
            .strip_prefix(r"\\?\")
            .map(|s| s.to_string())
            .unwrap_or(source_path);
        codegen.enable_debug_info(&source_path);
    }
    for module in &imported {
        let module_path = module.path.as_deref().map(|p| p.to_string_lossy().into_owned());
        let module_path = module_path
            .as_deref()
            .map(|p| p.strip_prefix(r"\\?\").map(|s| s.to_string()).unwrap_or_else(|| p.to_string()));
        codegen.add_module(&module.name, module.program.as_ref(), module_path.as_deref());
    }
    if let Err(e) = codegen.compile(&program) {
        die(huzi_error::render(&e, &source, "Compile error"));
    }

    let paths = OutputPaths::new(&args.effective_output());
    emit_and_link(&paths, &codegen, args, quiet);

    if !quiet {
        // 纯 ASCII 前缀:避免 Windows 控制台 GBK/CP936 代码页下 `✓` 显示为乱码。
        println!("[ok] {} generated successfully!", paths.exe_path.display());
    }

    paths
}

/// 执行校验、优化、目标文件生成与链接流程。
fn emit_and_link(paths: &OutputPaths, codegen: &CodeGen, args: &Args, quiet: bool) {
    // [4/5] Verifying
    if !quiet {
        println!("[4/5] Verifying...");
    }
    if let Err(e) = codegen.verify_detailed() {
        die(format!("Error: LLVM module verification failed (this is a compiler bug)\n{}", e));
    }

    // 优化阶段：当有效优化级别大于 0 时在内存中运行 LLVM 优化流水线
    // （--release 映射至级别 2）。级别 0（开发模式）直接将原始 inkwell IR 传入 llc。
    let opt_level = args.effective_opt_level();
    if opt_level > 0 {
        optimize_ir(codegen, opt_level, args.release, quiet);
    }

    // 优化后的 IR 落盘,以便失败时排查调试。
    write_ir(codegen, &paths.ll_path);

    // 阶段 5/5: 生成可执行文件
    if !quiet {
        println!("[5/5] Generating executable...");
    }
    compile_ir_to_object(paths, opt_level, args.debug);
    if !quiet {
        println!("  Linking to executable...");
    }
    link(paths, args.linker, args.debug, quiet);

    // 清理中间产物文件
    let _ = fs::remove_file(&paths.ll_path);
    let _ = fs::remove_file(&paths.obj_path);
}

/// 在校验前将 LLVM IR 写盘，以便失败时排查调试。
fn write_ir(codegen: &CodeGen, ll_path: &Path) {
    if let Err(e) = codegen.write_ir_to_file(ll_path.to_str().unwrap()) {
        die(format!("Error writing IR: {}", e));
    }
}

/// 使用 llc 将 LLVM IR 编译为平台目标文件 (.obj/.o)。
/// 后端优化等级与 IR 的 `opt -O<level>` 对齐：dev（0 级）用 `-O0` 快速出包，
/// release（N 级）用 `-O<N>` 做后端优化；优化档额外加 `--mcpu=native` 复用
/// 主机 CPU 特性（SIMD/流水线）。默认不碰 fast-math
///（不传 `--enable-unsafe-fp-math`），保持 IEEE 浮点语义。
/// 调试模式下将调试器格式调整为 DWARF (gdb/lldb)，而非平台默认格式（如 windows-msvc 目标的 CodeView）。
fn compile_ir_to_object(paths: &OutputPaths, opt_level: u8, debug: bool) {
    let llc_opt: &str = match opt_level {
        0 => "-O0",
        1 => "-O1",
        2 => "-O2",
        _ => "-O3",
    };
    let mut llc_args: Vec<&str> = vec![
        "--relocation-model=pic",
        "--filetype=obj",
        llc_opt,
    ];
    if opt_level > 0 {
        llc_args.push("--mcpu=native");
    }
    if debug {
        llc_args.push("-debugger-tune=gdb");
    }
    llc_args.extend(["-o", paths.obj_path.to_str().unwrap()]);
    llc_args.push(paths.ll_path.to_str().unwrap());
    run_command("llc", &llc_args).unwrap_or_else(|e| die(e));
}

/// 在内存中运行 LLVM 新 Pass 管理器优化流水线（仅在 level > 0 时调用）。
/// 流水线 `default<Olevel>` 即原 `opt -S -O<level>` 运行的默认流水线，不再依赖
/// 外部 opt 二进制（缺 opt 环境下 release 照常用）；llc/link 仍走外部工具链。
/// `verify_each` 仅开发模式显式 `--opt-level` 时开启（release 关闭以提速）。
fn optimize_ir(codegen: &CodeGen, level: u8, release: bool, quiet: bool) {
    if let Err(e) = codegen.optimize(level, !release) {
        die(format!("Error running optimization passes: {}", e));
    }
    if !quiet {
        println!("  [opt] -O{} optimization applied", level);
    }
}
