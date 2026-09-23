use super::*;
use huzi_codegen::BUILTIN_MODULES;
use huzi_lexer::Lexer;
use huzi_parser::Parser as HuziParser;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

/// 测试夹具源码走真实 Lexer + Parser,保证 import 语法有效。
fn parse_program(source: &str) -> Program {
    let tokens = Lexer::new(source.to_string())
        .tokenize()
        .expect("test fixture must lex");
    HuziParser::new(tokens)
        .parse()
        .expect("test fixture must parse")
}

/// 并行安全的唯一临时目录,避免多 checkout/多用例互相干扰。
fn unique_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir =
        std::env::temp_dir().join(format!("huzc_mod_{tag}_{}_{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

#[test]
fn bad_module_name_returns_err_without_exiting() {
    let mut program =
        parse_program("import no_such_module_xyz\nfn main() -> i32 {\n return 0\n}\n");
    let err = load_modules_result(&mut program, &unique_dir("missing"))
        .expect_err("missing module must be Err, not exit");
    assert!(
        err.contains("Cannot find module 'no_such_module_xyz'"),
        "unexpected: {err}"
    );
}

#[test]
fn circular_import_truncates() {
    let dir = unique_dir("cycle");
    std::fs::write(
        dir.join("ca.hz"),
        "import cb\nfn fa() -> i32 {\n return 1\n}\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("cb.hz"),
        "import ca\nfn fb() -> i32 {\n return 2\n}\n",
    )
    .unwrap();
    let mut program = parse_program("import ca\nfn main() -> i32 {\n return 0\n}\n");
    let modules = load_modules_result(&mut program, &dir).expect("cycle must truncate");
    let mut names: Vec<&str> = modules.iter().map(|m| m.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, vec!["ca", "cb"]);
}

#[test]
fn memory_loader_collects_two_errors() {
    let mut files = HashMap::new();
    files.insert("bad_lex".to_string(), "\"unterminated\n".to_string());
    files.insert("bad_top".to_string(), "let x = 1\n".to_string());
    let err = load_modules_from_memory(files).expect_err("two bad modules must be Err");
    assert!(err.contains("bad_lex"), "missing lex error: {err}");
    assert!(err.contains("bad_top"), "missing validate error: {err}");
}

#[test]
fn memory_loader_ok_with_url_key_and_builtin_skip() {
    let mut files = HashMap::new();
    files.insert(
        "file:///proj/mods/greeter.hz".to_string(),
        "fn greet() -> i32 {\n return 1\n}\n".to_string(),
    );
    // 内置模块直接跳过:内容为垃圾也能成功,证明未尝试解析。
    files.insert("math".to_string(), "garbage ((( ".to_string());
    files.insert(
        "app".to_string(),
        "import mods.greeter\nfn run() -> i32 {\n return 1\n}\n".to_string(),
    );
    let modules = load_modules_from_memory(files).expect("memory load must succeed");
    assert!(modules.iter().any(|m| m.name == "app" && m.program.is_some()));
    assert!(modules
        .iter()
        .any(|m| m.name == "greeter" && m.program.is_some() && m.path.is_none()));
    assert!(modules
        .iter()
        .any(|m| m.name == "math" && m.program.is_none() && m.path.is_none()));
}

#[test]
fn memory_builtin_table_matches_codegen() {
    assert_eq!(memory::MEMORY_BUILTIN_MODULES, BUILTIN_MODULES);
}

#[test]
fn std_module_resolution_works() {
    let mut program = parse_program("import core.result\nfn main() -> i32 {\n return 0\n}\n");
    let dir = unique_dir("std_test");
    let modules = load_modules_result(&mut program, &dir).expect("std core.result must resolve");
    assert!(modules.iter().any(|m| m.name == "result"));
}
