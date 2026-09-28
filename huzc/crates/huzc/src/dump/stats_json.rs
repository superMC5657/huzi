use huzc::cli::Args;
use huzi_ast::Program;
use huzi_lexer::Lexer;
use huzi_parser::Parser as HuziParser;

use crate::ast_json;
use crate::parse_stats;
use crate::pipeline::front::read_source;

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
pub(crate) fn run_dump_parse_stats(args: &Args) {
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
pub(crate) fn run_dump_ast_json(args: &Args) {
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
pub(crate) fn run_dump_ast_json_test(id: &str) {
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
