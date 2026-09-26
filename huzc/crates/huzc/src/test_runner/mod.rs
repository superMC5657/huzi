//! 测试运行器入口:用例类型定义与 `run_tests` 主流程。
//!
//! 子模块:`discover`(用例发现)、`exec`(编译/执行/断言)。

mod discover;
mod exec;

use crate::cli::TestArgs;
use discover::discover_tests;
use exec::{resolve_output_dir, run_single_test};
use std::fs;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TestCaseKind {
    Regular,
    CompileFail,
    RuntimeFail,
}

#[derive(Clone, Debug)]
pub struct TestCase {
    pub path: PathBuf,
    pub name: String,
    pub kind: TestCaseKind,
    pub expected_stdout: Option<PathBuf>,
}

pub fn run_tests(args: &TestArgs) -> bool {
    let cases = discover_tests(&args.path, args.filter.as_deref());
    if cases.is_empty() {
        println!("未找到匹配的测试用例。路径: {}", args.path);
        return true;
    }

    // Ensure common test/out directory exists for cases that perform file I/O
    let _ = fs::create_dir_all("test/out");
    let _ = fs::create_dir_all("huzc/test/out");

    let mut pass = 0;
    let mut fail = 0;
    let mut skip = 0;

    for case in &cases {
        if case.name == "10_guess_number_game" {
            skip += 1;
            println!("SKIP(interactive): {}", case.name);
            continue;
        }

        let out_dir = resolve_output_dir(case);
        match run_single_test(case, &out_dir, args) {
            Ok(()) => pass += 1,
            Err(e) => {
                fail += 1;
                eprintln!("{}", e);
            }
        }
    }

    println!("-----------------------------");
    println!("{} passed, {} failed, {} skipped", pass, fail, skip);
    fail == 0
}
