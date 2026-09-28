use super::helpers::*;
use super::super::*;

#[test]
fn unknown_variable_error_suggests_close_name() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let program = main_program(vec![
        let_stmt("count", Expr::Literal(Literal::Int(1))),
        sp(Stmt::Return(ReturnStmt {
            value: Some(Expr::Binary(BinaryExpr {
                left: Box::new(Expr::Ident("cont".to_string())),
                operator: BinOp::Add,
                right: Box::new(Expr::Literal(Literal::Int(1))),
            })),
        })),
    ]);
    let err = codegen.compile(&program).expect_err("typo must fail");
    let message = err.message();
    assert!(message.contains("Unknown variable: cont"), "got: {}", message);
    assert!(message.contains("did you mean `count`?"), "got: {}", message);
}

#[test]
fn unknown_function_typo_suggests_builtin_with_position() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    // `prnt` 与 `print` 编辑距离为 1(len4 阈值 1,可建议);`pritn` 距离为 2
    // 超出阈值,按现行算法无建议,故用 `prnt` 覆盖 builtin 候选路径。
    let program = main_program(vec![sp(Stmt::Return(ReturnStmt {
        value: Some(Expr::Call(CallExpr {
            callee: Box::new(Expr::Ident("prnt".to_string())),
            arguments: vec![Expr::Literal(Literal::Int(1))],
            type_args: vec![],
        })),
    }))]);
    let err = codegen.compile(&program).expect_err("builtin typo must fail");
    let message = err.message();
    assert!(message.contains("Unknown function: prnt"), "got: {}", message);
    assert!(message.contains("did you mean `print`?"), "got: {}", message);
    assert!(err.line() > 0, "error must carry position, got line 0");
    let rendered = huzi_error::render_with_color(
        &err,
        "fn main() -> i32 {\n    return prnt(1)\n}",
        "Compile error",
        false,
    );
    assert!(rendered.contains("^"), "render must contain caret, got: {}", rendered);
    assert!(rendered.contains("help:"), "render must keep help text, got: {}", rendered);
}
