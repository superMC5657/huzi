use super::*;
use huzi_ast::*;
use huzi_error::HuziError;
use huzi_lexer::Lexer;

fn parse(src: &str) -> Program {
    let tokens = Lexer::new(src.to_string())
        .tokenize()
        .unwrap_or_else(|e| panic!("unexpected lex error: {}", e));
    Parser::new(tokens)
        .parse()
        .unwrap_or_else(|e| panic!("unexpected parse error: {}", e))
}

fn parse_recoverable(src: &str) -> (Program, Vec<HuziError>) {
    let tokens = Lexer::new(src.to_string())
        .tokenize()
        .unwrap_or_else(|e| panic!("unexpected lex error: {}", e));
    Parser::new(tokens).parse_recoverable()
}

#[test]
fn multiplication_binds_tighter_than_addition() {
    let program = parse("let r = 1 + 2 * 3");
    let Stmt::Let(let_stmt) = &program.statements[0].node else {
        panic!("expected a let statement");
    };
    let Some(Expr::Binary(add)) = &let_stmt.value else {
        panic!("expected a binary expression");
    };
    assert!(matches!(add.operator, BinOp::Add));
    let Expr::Literal(Literal::Int(1)) = &*add.left else {
        panic!("expected literal 1 as the left operand");
    };
    let Expr::Binary(mul) = &*add.right else {
        panic!("expected `2 * 3` as the right operand");
    };
    assert!(matches!(mul.operator, BinOp::Mul));
}

#[test]
fn let_mut_is_parsed_as_mutable() {
    let program = parse("let mut x = 1
let y = 2");
    let Stmt::Let(mutable) = &program.statements[0].node else {
        panic!("expected a let statement");
    };
    assert!(mutable.mutable);
    let Stmt::Let(immutable) = &program.statements[1].node else {
        panic!("expected a let statement");
    };
    assert!(!immutable.mutable);
}

#[test]
fn if_elif_else_structure() {
    let program = parse(
        "if x > 0 { let a = 1 } elif x < 0 { let b = 2 } else { let c = 3 }",
    );
    let Stmt::If(if_stmt) = &program.statements[0].node else {
        panic!("expected an if statement");
    };
    assert_eq!(if_stmt.then_branch.statements.len(), 1);
    assert_eq!(if_stmt.elif_branches.len(), 1);
    assert!(if_stmt.else_branch.is_some());
}

#[test]
fn import_parses_dotted_name() {
    let program = parse("import mods.helpers");
    let Stmt::Import(imp) = &program.statements[0].node else {
        panic!("expected an import statement");
    };
    assert_eq!(imp.name, "mods.helpers");
}

#[test]
fn export_parses_module_wildcard_and_item() {
    let p1 = parse("export calc");
    let Stmt::Export(exp1) = &p1.statements[0].node else { panic!("expected export"); };
    assert_eq!(exp1.path, "calc");
    assert!(!exp1.is_wildcard);

    let p2 = parse("export calc::*");
    let Stmt::Export(exp2) = &p2.statements[0].node else { panic!("expected export"); };
    assert_eq!(exp2.path, "calc");
    assert!(exp2.is_wildcard);

    let p3 = parse("export calc::add");
    let Stmt::Export(exp3) = &p3.statements[0].node else { panic!("expected export"); };
    assert_eq!(exp3.path, "calc::add");
    assert!(!exp3.is_wildcard);
}

#[test]
fn parse_error_reports_real_position() {
    let tokens = Lexer::new("let x = ;".to_string())
        .tokenize()
        .expect("lexing must succeed");
    let err = Parser::new(tokens)
        .parse()
        .expect_err("`;` is not a valid expression");
    assert_eq!((err.line(), err.column()), (1, 9));
}

#[test]
fn statements_carry_source_span() {
    let program = parse("let x = 1\nlet y = 2");
    assert_eq!(program.statements[0].span.line, 1);
    assert_eq!(program.statements[0].span.column, 1);
    assert_eq!(program.statements[1].span.line, 2);
    assert_eq!(program.statements[1].span.column, 1);
}

/// `for x in arr` 应解析为 ForSource::Array。
#[test]
fn parses_for_in_array() {    let program = parse("fn main() -> i32 {\n    for x in nums {\n        print(x)\n    }\n    return 0\n}");
    let Stmt::Fn(fn_stmt) = &program.statements[0].node else {
        panic!("expected fn main");
    };
    let mut found_array = false;
    for s in &fn_stmt.body.statements {
        if let Stmt::For(for_stmt) = &s.node {
            assert!(
                matches!(&for_stmt.source, ForSource::Array(_)),
                "for x in nums should parse as array iteration"
            );
            found_array = true;
        }
    }
    assert!(found_array, "for statement should be present");
}

/// 双错同报:一次返回 2 个带真实行列的错误,失败语句不入 Program。
#[test]
fn recoverable_reports_both_errors_with_positions() {
    let (program, errors) = parse_recoverable("let x = ; let y = ;");
    assert_eq!(errors.len(), 2);
    assert_eq!((errors[0].line(), errors[0].column()), (1, 9));
    assert_eq!((errors[1].line(), errors[1].column()), (1, 19));
    assert!(program.statements.is_empty());
}

/// 错后好语句仍保留,且区间为 `with_range` 起止(第 2 行起始)。
#[test]
fn recoverable_keeps_good_statement_after_error() {
    let (program, errors) = parse_recoverable("let x = ;\nlet y = 2");
    assert_eq!(errors.len(), 1);
    assert_eq!(program.statements.len(), 1);
    let Stmt::Let(let_stmt) = &program.statements[0].node else {
        panic!("expected the good let statement to survive");
    };
    assert_eq!(let_stmt.name, "y");
    assert_eq!(program.statements[0].span.line, 2);
    assert_eq!(program.statements[0].span.column, 1);
}

/// 错误上限截断:40 个坏语句只收集 32 个错后停。
#[test]
fn recoverable_caps_errors_at_32() {
    let src = "let x = ;\n".repeat(40);
    let (program, errors) = parse_recoverable(&src);
    assert_eq!(errors.len(), 32);
    assert!(program.statements.is_empty());
}

/// 兼容入口仍首错即停:双错下 `parse` 返回第一个错。
#[test]
fn parse_still_returns_first_error() {
    let tokens = Lexer::new("let x = ; let y = ;".to_string())
        .tokenize()
        .expect("lexing must succeed");
    let err = Parser::new(tokens)
        .parse()
        .expect_err("double fault must still fail fast");
    assert_eq!((err.line(), err.column()), (1, 9));
}
