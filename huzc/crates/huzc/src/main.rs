use huzc::cli;
use huzc::cli::Args;
mod linker;
mod paths;

use clap::Parser;
use huzi_ast::Program;
use huzi_codegen::CodeGen;
use huzi_lexer::Lexer;
use huzi_parser::Parser as HuziParser;
use huzc::load_modules;
use inkwell::context::Context;
use linker::{link, run_command};
use paths::OutputPaths;
use std::fs;
use std::path::Path;

/// 将错误打印到 stderr 并以状态码 1 退出。
pub(crate) fn die(msg: String) -> ! {
    eprintln!("{}", msg);
    std::process::exit(1);
}

fn main() {
    // 内部 panic(多为 codegen 对 inkwell builder 的 .unwrap() 在非法 IR 下失败)
    // 统一转为清晰的“内部编译器错误”提示,避免向用户暴露裸 Rust backtrace。
    // 消息用纯英文以规避 Windows GBK 控制台对 UTF-8 的乱码(与成功提示同理)。
    std::panic::set_hook(Box::new(|info| {
        eprintln!();
        eprintln!("error: internal compiler error (a bug in huzc, not your code)");
        eprintln!("  {}", info);
        eprintln!("  please report it together with the .hz source that triggered it");
    }));

    let args = Args::parse();
    if let Some(cmd) = args.command {
        match cmd {
            cli::Command::Fmt(fmt_args) => {
                let ok = huzc::fmt::run_fmt(&fmt_args.path, fmt_args.check);
                if !ok {
                    std::process::exit(1);
                }
                return;
            }
            cli::Command::Build(build_args) => {
                huzc::pkg::run_build(&build_args);
                return;
            }
            cli::Command::Add(add_args) => {
                huzc::pkg::run_add(&add_args);
                return;
            }
            cli::Command::Fetch(fetch_args) => {
                huzc::pkg::run_fetch(&fetch_args);
                return;
            }
        }
    }

    let input = match &args.input {
        Some(i) => i,
        None => {
            eprintln!("error: either a subcommand (e.g. 'fmt') or '--input <file.hz>' is required");
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

    // [4/5] Verifying
    if !quiet {
        println!("[4/5] Verifying...");
    }
    let paths = OutputPaths::new(&args.effective_output());
    write_ir(&codegen, &paths.ll_path);
    if let Err(e) = codegen.verify_detailed() {
        die(format!("Error: LLVM module verification failed (this is a compiler bug)\n{}", e));
    }

    // 优化阶段：当有效优化级别大于 0 时在代码生成前运行 LLVM IR 优化器（--release 映射至级别 2）。
    // 级别 0（开发模式）直接将原始 inkwell IR 传入 llc。
    let opt_level = args.effective_opt_level();
    if opt_level > 0 {
        optimize_ir(&paths, opt_level, quiet);
    }

    // 阶段 5/5: 生成可执行文件
    if !quiet {
        println!("[5/5] Generating executable...");
    }
    compile_ir_to_object(&paths, args.debug);
    if !quiet {
        println!("  Linking to executable...");
    }
    link(&paths, args.linker, args.debug, quiet);

    // 清理中间产物文件
    let _ = fs::remove_file(&paths.ll_path);
    let _ = fs::remove_file(&paths.obj_path);

    if !quiet {
        // 纯 ASCII 前缀:避免 Windows 控制台 GBK/CP936 代码页下 `✓` 显示为乱码。
        println!("[ok] {} generated successfully!", paths.exe_path.display());
    }
}

/// 读取待编译的 Huzi 源码文件。
fn read_source(input: &str) -> String {
    fs::read_to_string(input).unwrap_or_else(|e| die(format!("Error reading file: {}", e)))
}

/// [1/5] 词法分析 + [2/5] 语法分析：将源码文本转换为程序 AST。
/// 词法/语法错误包含精确行列位置，并通过 huzi-error 渲染源码片段。
fn parse_source(source: &str, quiet: bool) -> Program {
    if !quiet {
        println!("[1/5] Lexing...");
    }
    let tokens = Lexer::new(source.to_string())
        .tokenize()
        .unwrap_or_else(|e| die(huzi_error::render(&e, source, "Lex error")));

    if !quiet {
        println!("[2/5] Parsing...");
    }
    HuziParser::new(tokens)
        .parse()
        .unwrap_or_else(|e| die(huzi_error::render(&e, source, "Parse error")))
}

/// 在校验前将 LLVM IR 写盘，以便失败时排查调试。
fn write_ir(codegen: &CodeGen, ll_path: &Path) {
    if let Err(e) = codegen.write_ir_to_file(ll_path.to_str().unwrap()) {
        die(format!("Error writing IR: {}", e));
    }
}

/// 使用 llc 将 LLVM IR 编译为平台目标文件 (.obj/.o)。
/// 调试模式下将调试器格式调整为 DWARF (gdb/lldb)，而非平台默认格式（如 windows-msvc 目标的 CodeView）。
fn compile_ir_to_object(paths: &OutputPaths, debug: bool) {
    let mut llc_args: Vec<&str> = vec![
        "--relocation-model=pic",
        "--filetype=obj",
    ];
    if debug {
        llc_args.push("-debugger-tune=gdb");
    }
    llc_args.extend(["-o", paths.obj_path.to_str().unwrap()]);
    llc_args.push(paths.ll_path.to_str().unwrap());
    run_command("llc", &llc_args).unwrap_or_else(|e| die(e));
}

/// 使用 opt -O<level> 原地优化 LLVM IR（仅在 level > 0 时调用）。
/// opt 与 llc 均随 LLVM 提供，无需额外工具链。该级别的 Pass 流水线涵盖函数内联、常量折叠与公共子表达式消除。
fn optimize_ir(paths: &OutputPaths, level: u8, quiet: bool) {
    let ll_path = paths.ll_path.to_str().unwrap().to_string();
    let opt_args: Vec<String> = vec![
        "-S".to_string(),
        format!("-O{}", level),
        "-o".to_string(),
        ll_path.clone(),
        ll_path,
    ];
    let opt_args_ref: Vec<&str> = opt_args.iter().map(|s| s.as_str()).collect();
    run_command("opt", &opt_args_ref).unwrap_or_else(|e| die(e));
    if !quiet {
        println!("  [opt] -O{} optimization applied", level);
    }
}
