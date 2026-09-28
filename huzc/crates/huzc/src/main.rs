use huzc::cli;
use huzc::cli::Args;
mod ast_json;
mod dump;
mod linker;
mod parse_stats;
mod paths;
mod pipeline;

use clap::Parser;

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
        dump::stats_json::run_dump_ast_json_test(test_id);
        return;
    }
    if args.dump_ast_json {
        dump::stats_json::run_dump_ast_json(&args);
        return;
    }
    if args.dump_tokens {
        dump::tokens::run_dump_tokens(&args);
        return;
    }
    if args.dump_parse_stats {
        dump::stats_json::run_dump_parse_stats(&args);
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
                pipeline::run::run_target(&run_args);
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

    let _ = pipeline::emit::compile_source_file(&args);
}
