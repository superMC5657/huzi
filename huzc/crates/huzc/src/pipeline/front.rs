use huzi_ast::Program;
use huzi_lexer::Lexer;
use huzi_parser::Parser as HuziParser;
use std::fs;

use crate::die;

/// 读取待编译的 Huzi 源码文件。
pub(crate) fn read_source(input: &str) -> String {
    fs::read_to_string(input).unwrap_or_else(|e| die(format!("Error reading file: {}", e)))
}

/// [1/5] 词法分析 + [2/5] 语法分析：将源码文本转换为程序 AST。
/// 词法/语法错误包含精确行列位置，并通过 huzi-error 渲染源码片段。
pub(crate) fn parse_source(source: &str, quiet: bool) -> Program {
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
