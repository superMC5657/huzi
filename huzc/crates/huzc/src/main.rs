use huzc::cli;
use huzc::cli::Args;
mod ast_json;
mod linker;
mod parse_stats;
mod paths;

use clap::Parser;
use huzi_ast::Program;
use huzi_codegen::CodeGen;
use huzi_lexer::{Lexer, SpannedToken, Token};
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
    if let Some(ref test_id) = args.ast_json_test {
        run_dump_ast_json_test(test_id);
        return;
    }
    if args.dump_ast_json {
        run_dump_ast_json(&args);
        return;
    }
    if args.dump_tokens {
        run_dump_tokens(&args);
        return;
    }
    if args.dump_parse_stats {
        run_dump_parse_stats(&args);
        return;
    }
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
            cli::Command::Run(run_args) => {
                run_target(&run_args);
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
            cli::Command::Test(test_args) => {
                let ok = huzc::test_runner::run_tests(&test_args);
                if !ok {
                    std::process::exit(1);
                }
                return;
            }
        }
    }

    let _ = compile_source_file(&args);
}

/// 编译单个 Huzi 源码文件并生成可执行文件。
fn compile_source_file(args: &Args) -> OutputPaths {
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

/// 编排编译并直接运行目标程序。
fn run_target(run_args: &cli::RunArgs) {
    let target_path = Path::new(&run_args.target);
    let is_package = run_args.path.is_some()
        || target_path.join("huzi.toml").is_file()
        || (!target_path.is_file() && !run_args.target.ends_with(".hz"));

    let exe_path = if is_package {
        let proj_path = run_args
            .path
            .clone()
            .unwrap_or_else(|| run_args.target.clone());
        let build_args = cli::BuildArgs {
            path: proj_path,
            output: None,
            linker: run_args.linker,
            release: run_args.release,
        };
        huzc::pkg::build_and_get_output(&build_args)
    } else {
        let args = Args {
            command: None,
            input: Some(run_args.target.clone()),
            output: None,
            linker: run_args.linker,
            release: run_args.release,
            opt_level: None,
            debug: false,
            dump_tokens: false,
            dump_parse_stats: false,
            dump_ast_json: false,
            ast_json_test: None,
        };
        compile_source_file(&args).exe_path
    };

    let run_cmd_path = if exe_path.is_relative() && exe_path.parent() == Some(Path::new("")) {
        Path::new(".").join(&exe_path)
    } else {
        exe_path
    };

    let status = std::process::Command::new(&run_cmd_path)
        .args(&run_args.args)
        .status()
        .unwrap_or_else(|e| die(format!("Failed to run {}: {}", run_cmd_path.display(), e)));

    std::process::exit(status.code().unwrap_or(1));
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

/// `--dump-tokens` 的 token 种类：与 hzlex（examples/hzlex/src/lexer.hz）一致。
/// 取值仅为 kw | ident | int | float | string | char | punct | eof。
fn dump_token_kind(token: &Token) -> &'static str {
    match token {
        Token::Fn
        | Token::Struct
        | Token::Enum
        | Token::Match
        | Token::Let
        | Token::Mut
        | Token::If
        | Token::Else
        | Token::Elif
        | Token::For
        | Token::In
        | Token::While
        | Token::Return
        | Token::Import
        | Token::Export
        | Token::Break
        | Token::Continue
        | Token::Defer
        | Token::Trait
        | Token::Impl
        | Token::True
        | Token::False
        | Token::Weak => "kw",
        Token::Ident(_) => "ident",
        Token::Int(_) => "int",
        Token::Float(_) => "float",
        Token::String(_) | Token::FString(_) => "string",
        Token::Char(_) => "char",
        Token::Eof => "eof",
        _ => "punct",
    }
}

/// `--dump-tokens` 的 token 文本：与 hzlex 的 texts 平行数组一致。
/// kw/punct 为源码拼写；ident 为值显示（即源码拼写）；int/float 由调用方
/// 经源码切片取源码拼写（见 `format_tokens`），此处仅为取不到切片时的
/// 回退（值显示）；string 为解转义后原文
/// （去除首尾引号，f-string 去掉前缀 f）；char 为码点十进制；eof 为空串。
fn dump_token_text(token: &Token) -> String {
    match token {
        Token::Fn => "fn".to_string(),
        Token::Struct => "struct".to_string(),
        Token::Enum => "enum".to_string(),
        Token::Match => "match".to_string(),
        Token::Let => "let".to_string(),
        Token::Mut => "mut".to_string(),
        Token::If => "if".to_string(),
        Token::Else => "else".to_string(),
        Token::Elif => "elif".to_string(),
        Token::For => "for".to_string(),
        Token::In => "in".to_string(),
        Token::While => "while".to_string(),
        Token::Return => "return".to_string(),
        Token::Import => "import".to_string(),
        Token::Export => "export".to_string(),
        Token::Break => "break".to_string(),
        Token::Continue => "continue".to_string(),
        Token::Defer => "defer".to_string(),
        Token::Trait => "trait".to_string(),
        Token::Impl => "impl".to_string(),
        Token::True => "true".to_string(),
        Token::False => "false".to_string(),
        Token::Weak => "weak".to_string(),
        Token::Ident(s) | Token::String(s) | Token::FString(s) => s.clone(),
        Token::Int(n) => n.to_string(),
        Token::Float(n) => n.to_string(),
        Token::Char(c) => (*c as u32).to_string(),
        Token::Eof => String::new(),
        Token::Plus => "+".to_string(),
        Token::PlusEq => "+=".to_string(),
        Token::Minus => "-".to_string(),
        Token::MinusEq => "-=".to_string(),
        Token::Star => "*".to_string(),
        Token::StarEq => "*=".to_string(),
        Token::Slash => "/".to_string(),
        Token::SlashEq => "/=".to_string(),
        Token::Percent => "%".to_string(),
        Token::PercentEq => "%=".to_string(),
        Token::Equal => "=".to_string(),
        Token::EqualEqual => "==".to_string(),
        Token::Bang => "!".to_string(),
        Token::BangEqual => "!=".to_string(),
        Token::Less => "<".to_string(),
        Token::LessEqual => "<=".to_string(),
        Token::Greater => ">".to_string(),
        Token::GreaterEqual => ">=".to_string(),
        Token::AmpAmp => "&&".to_string(),
        Token::BarBar => "||".to_string(),
        Token::Pipe => "|".to_string(),
        Token::Question => "?".to_string(),
        Token::LParen => "(".to_string(),
        Token::RParen => ")".to_string(),
        Token::LBrace => "{".to_string(),
        Token::RBrace => "}".to_string(),
        Token::LBracket => "[".to_string(),
        Token::RBracket => "]".to_string(),
        Token::Comma => ",".to_string(),
        Token::Colon => ":".to_string(),
        Token::PathSep => "::".to_string(),
        Token::Semi => ";".to_string(),
        Token::Arrow => "->".to_string(),
        Token::FatArrow => "=>".to_string(),
        Token::Dot => ".".to_string(),
        Token::DotDot => "..".to_string(),
    }
}

/// 将 token 序列格式化为 hzlex 兼容的 dump 文本。
/// 每行格式为 `{line}:{col} {kind} {text}`（单空格分隔，含 eof 行；
/// eof 行 text 为空故行尾带一个空格），行尾统一 `\n`。
/// int/float 的 text 取源码拼写（逐字节一致，如 `007`/`0.0` 不归一化），
/// 经行列号反推源码切片得到；反推失败时回退为值显示。
fn format_tokens(tokens: &[SpannedToken], source: &str) -> String {
    let chars: Vec<char> = source.chars().collect();
    let line_starts = build_line_starts(&chars);
    let mut out = String::new();
    for st in tokens {
        let kind = dump_token_kind(&st.token);
        let text = match &st.token {
            Token::Int(_) => number_lexeme(&chars, &line_starts, st.line, st.column, false)
                .unwrap_or_else(|| dump_token_text(&st.token)),
            Token::Float(_) => number_lexeme(&chars, &line_starts, st.line, st.column, true)
                .unwrap_or_else(|| dump_token_text(&st.token)),
            _ => dump_token_text(&st.token),
        };
        out.push_str(&format!("{}:{} {} {}\n", st.line, st.column, kind, text));
    }
    out
}

/// 按 char 计的行首索引表：`line_starts[i]` 为第 `i+1` 行首个字符的下标。
/// 与词法器口径一致（`\n` 换行，`\r` 只占一列不换行）。
fn build_line_starts(chars: &[char]) -> Vec<usize> {
    let mut starts = vec![0];
    for (i, c) in chars.iter().enumerate() {
        if *c == '\n' {
            starts.push(i + 1);
        }
    }
    starts
}

/// 从源码切片取数字字面量的源码拼写：int 取前导 `[0-9]+`，float 取
/// `[0-9]+.[0-9]+`。行列号为词法器的 1 基 char 口径；越界或无数字时
/// 返回 `None` 由调用方回退为值显示。
fn number_lexeme(
    chars: &[char],
    line_starts: &[usize],
    line: usize,
    column: usize,
    is_float: bool,
) -> Option<String> {
    if line == 0 || column == 0 || line > line_starts.len() {
        return None;
    }
    let mut idx = line_starts[line - 1] + (column - 1);
    if idx >= chars.len() {
        return None;
    }
    let start = idx;
    while idx < chars.len() && chars[idx].is_ascii_digit() {
        idx += 1;
    }
    if is_float {
        if idx >= chars.len() || chars[idx] != '.' {
            return None;
        }
        idx += 1;
        let frac_start = idx;
        while idx < chars.len() && chars[idx].is_ascii_digit() {
            idx += 1;
        }
        if idx == frac_start {
            return None;
        }
    }
    if idx == start {
        return None;
    }
    Some(chars[start..idx].iter().collect())
}

/// 执行 `--dump-tokens -i <file>`：词法分析并逐行打印后直接返回。
/// 词法失败沿用现有错误路径（stderr 渲染 + 非零退出），不做错误恢复。
fn run_dump_tokens(args: &Args) {
    let input = match &args.input {
        Some(i) => i,
        None => {
            eprintln!("error: '--dump-tokens' requires '--input <file.hz>'");
            std::process::exit(1);
        }
    };
    let source = read_source(input);
    let tokens = Lexer::new(source.clone())
        .tokenize()
        .unwrap_or_else(|e| die(huzi_error::render(&e, &source, "Lex error")));
    print!("{}", format_tokens(&tokens, &source));
}

/// 对源码串做词法+语法两段并返回程序与表达式峰值深度。
fn lex_parse_with_depth(source: &str) -> (Program, usize) {
    let tokens = Lexer::new(source.to_string())
        .tokenize()
        .unwrap_or_else(|e| {
            eprintln!("lex-error {}:{} {}", e.line(), e.column(), e.message());
            std::process::exit(1);
        });
    let mut parser = HuziParser::new(tokens);
    let program = parser.parse().unwrap_or_else(|e| {
        eprintln!("parse-error {}:{} {}", e.line(), e.column(), e.message());
        std::process::exit(1);
    });
    let depth = parser.max_expr_depth();
    (program, depth)
}

/// 执行 `--dump-parse-stats -i <file>`：打印九维统计单行后返回。
fn run_dump_parse_stats(args: &Args) {
    let input = match &args.input {
        Some(i) => i,
        None => {
            eprintln!("error: '--dump-parse-stats' requires '--input <file.hz>'");
            std::process::exit(1);
        }
    };
    let source = read_source(input);
    let (program, depth) = lex_parse_with_depth(&source);
    let stats = parse_stats::collect(&program, depth);
    println!("{}", stats.format());
}

/// 执行 `--dump-ast-json -i <file>`：打印整程序紧凑 JSON 单行后返回。
/// 子集外节点按冻结口径报错非零退出，不做截断输出。
fn run_dump_ast_json(args: &Args) {
    let input = match &args.input {
        Some(i) => i,
        None => {
            eprintln!("error: '--dump-ast-json' requires '--input <file.hz>'");
            std::process::exit(1);
        }
    };
    let source = read_source(input);
    let (program, _) = lex_parse_with_depth(&source);
    match ast_json::program_to_json(&program) {
        Ok(s) => println!("{s}"),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}

/// 执行 `--ast-json-test <id>`：打印 C2 向量单行 JSON 后返回。
/// 未知 ID 非零退出（与 hzast `panic` 对应对拍 FAIL）。
fn run_dump_ast_json_test(id: &str) {
    match ast_json::test_vector(id) {
        Some(Ok(s)) => println!("{s}"),
        Some(Err(e)) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
        None => {
            eprintln!("error: unknown ast-json-test id '{id}'");
            std::process::exit(1);
        }
    }
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

#[cfg(test)]
mod dump_tokens_tests {
    use super::*;

    /// dump 行与 hzlex 输出一致：覆盖 kw/ident/float/转义 string/range 点。
    /// 期望行即 hzlex `lex()` 对同一源码的 kinds/texts/lines/cols
    /// 按 `行:列 类型 文本` 打印的结果（逐字节一致，eof 行尾带空格）。
    #[test]
    fn dump_matches_hzlex_format() {
        let src = "let s = \"a\\tb\"; let f = 3.14; let r = 1..2".to_string();
        let tokens = Lexer::new(src.clone())
            .tokenize()
            .expect("test snippet must lex cleanly");
        let got = format_tokens(&tokens, &src);
        let tab = '\t';
        let expected = format!(
            "1:1 kw let\n1:5 ident s\n1:7 punct =\n1:9 string a{tab}b\n1:15 punct ;\n1:17 kw let\n1:21 ident f\n1:23 punct =\n1:25 float 3.14\n1:29 punct ;\n1:31 kw let\n1:35 ident r\n1:37 punct =\n1:39 int 1\n1:40 punct ..\n1:42 int 2\n1:43 eof \n"
        );
        assert_eq!(got, expected);
    }

    /// 数字拼写逐字节一致：前导零与小数尾零不做值归一化。
    /// `007` 不得归一为 `7`，`0.0`/`144.0`/`2.0` 不得归一为 `0`/`144`/`2`。
    #[test]
    fn dump_preserves_numeric_spelling() {
        let src = "let a = 007; let b = 0.0; let c = 144.0; let d = 2.0".to_string();
        let tokens = Lexer::new(src.clone())
            .tokenize()
            .expect("test snippet must lex cleanly");
        let got = format_tokens(&tokens, &src);
        let expected = "1:1 kw let\n1:5 ident a\n1:7 punct =\n1:9 int 007\n1:12 punct ;\n1:14 kw let\n1:18 ident b\n1:20 punct =\n1:22 float 0.0\n1:25 punct ;\n1:27 kw let\n1:31 ident c\n1:33 punct =\n1:35 float 144.0\n1:40 punct ;\n1:42 kw let\n1:46 ident d\n1:48 punct =\n1:50 float 2.0\n1:53 eof \n";
        assert_eq!(got, expected);
    }
}

#[cfg(test)]
mod dump_parse_stats_tests {
    use super::*;

    fn stats_of(src: &str) -> String {
        let tokens = Lexer::new(src.to_string())
            .tokenize()
            .expect("test snippet must lex cleanly");
        let mut parser = HuziParser::new(tokens);
        let program = parser.parse().expect("test snippet must parse");
        let depth = parser.max_expr_depth();
        parse_stats::collect(&program, depth).format()
    }

    #[test]
    fn stats_counts_top_shapes() {
        let src = "import a export b fn f() -> i32 { let x = 1 return x }";
        let got = stats_of(src);
        assert_eq!(
            got,
            "fns=1 structs=0 enums=0 traits=0 impls=0 imports=1 exports=1 lets=1 depth=1"
        );
    }

    #[test]
    fn stats_counts_impl_methods_as_fns() {
        let src = "struct S { x: i32 } impl S { fn m(self: S) -> i32 { return self.x } }";
        let got = stats_of(src);
        assert_eq!(
            got,
            "fns=1 structs=1 enums=0 traits=0 impls=1 imports=0 exports=0 lets=0 depth=1"
        );
    }

    #[test]
    fn stats_depth_nests_calls() {
        let got = stats_of("fn f() -> i32 { return g(h(1)) }");
        assert_eq!(
            got,
            "fns=1 structs=0 enums=0 traits=0 impls=0 imports=0 exports=0 lets=0 depth=3"
        );
    }
}

#[cfg(test)]
mod dump_ast_json_tests {
    use super::*;

    #[test]
    fn json_escape_freezes_five() {
        assert_eq!(ast_json::escape("a\"b\\c\nd\te\rf"), "a\\\"b\\\\c\\nd\\te\\rf");
    }

    #[test]
    fn json_vector_expr_num() {
        let got = ast_json::test_vector("expr_num").expect("known id").expect("in subset");
        assert_eq!(got, "{\"kind\":\"num\",\"value\":42}");
    }

    #[test]
    fn json_vector_stmt_assign_folds() {
        let got = ast_json::test_vector("stmt_assign")
            .expect("known id")
            .expect("in subset");
        assert_eq!(
            got,
            "{\"kind\":\"assign\",\"name\":\"x\",\"expr\":{\"kind\":\"bin\",\"op\":\"+\",\"left\":{\"kind\":\"var\",\"name\":\"x\"},\"right\":{\"kind\":\"num\",\"value\":20}}}"
        );
    }

    #[test]
    fn json_vector_prog_fact_shape() {
        let got = ast_json::test_vector("prog_fact").expect("known id").expect("in subset");
        assert!(got.starts_with("{\"fns\":[{\"name\":\"fact\",\"params\":[\"n\"]"));
        assert!(got.ends_with("\"main\":[{\"kind\":\"return\",\"expr\":{\"kind\":\"call\",\"name\":\"fact\",\"args\":[{\"kind\":\"num\",\"value\":5}]}}]}"));
    }

    #[test]
    fn json_unknown_id_is_none() {
        assert!(ast_json::test_vector("no_such_id").is_none());
    }
}
