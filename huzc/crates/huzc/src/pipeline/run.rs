use huzc::cli;
use huzc::cli::Args;
use std::path::Path;

use super::emit::compile_source_file;
use crate::die;

/// 编排编译并直接运行目标程序。
pub(crate) fn run_target(run_args: &cli::RunArgs) {
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
