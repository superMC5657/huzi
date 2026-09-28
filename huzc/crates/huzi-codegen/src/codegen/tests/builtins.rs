use super::helpers::*;
use super::super::*;

/// 系统类内置函数(rand/srand/time)生成合法 IR 并正确声明符号。
#[test]
fn system_builtins_verify() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let call = |name: &str, args: Vec<Expr>| Expr::Call(CallExpr {
        callee: Box::new(Expr::Ident(name.to_string())),
        arguments: args,
        type_args: vec![],
    });
    let program = main_program(vec![
        sp(Stmt::Expr(ExprStmt {
            expr: call("srand", vec![Expr::Literal(Literal::Int(42))]),
        })),
        let_stmt("r", call("rand", vec![])),
        let_stmt("t", call("time", vec![])),
        sp(Stmt::Expr(ExprStmt {
            expr: call("sleep_ms", vec![Expr::Literal(Literal::Int(1))]),
        })),
        sp(Stmt::Return(ReturnStmt {
            value: Some(Expr::Literal(Literal::Int(0))),
        })),
    ]);
    codegen.compile(&program).expect("compile should succeed");
    assert!(codegen.verify());
    let ir = codegen.print_llvm_ir();
    for symbol in ["declare i32 @rand", "declare void @srand", "declare i64 @time"] {
        assert!(ir.contains(symbol), "IR should declare {symbol}");
    }
}

/// vec 构造/push/下标/for-in 生成合法 IR,扩容走 realloc。
#[test]
fn vec_push_grows_and_verifies() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let call = |name: &str, args: Vec<Expr>| Expr::Call(CallExpr {
        callee: Box::new(Expr::Ident(name.to_string())),
        arguments: args,
        type_args: vec![],
    });
    let vec_ident = || Expr::Ident("v".to_string());
    let program = main_program(vec![
        sp(Stmt::Let(LetStmt {
            name: "v".to_string(),
            mutable: true,
            tuple_pattern: None,
            type_annotation: None,
            value: Some(call("vec", vec![Expr::Literal(Literal::Int(1))])),
        })),
        sp(Stmt::Expr(ExprStmt {
            expr: call("push", vec![vec_ident(), Expr::Literal(Literal::Int(2))]),
        })),
        sp(Stmt::For(ForStmt {
            var_name: "x".to_string(),
            source: ForSource::Array(vec_ident()),
            body: Block { statements: vec![] },
        })),
        sp(Stmt::Return(ReturnStmt {
            value: Some(Expr::ArrayIndex(ArrayIndexExpr {
                array: Box::new(vec_ident()),
                index: Box::new(Expr::Literal(Literal::Int(1))),
            })),
        })),
    ]);
    codegen.compile(&program).expect("compile should succeed");
    assert!(codegen.verify());
    let ir = codegen.print_llvm_ir();
    for symbol in ["declare ptr @realloc", "vec_realloc", "vec_grow", "vec_for_loop"] {
        assert!(ir.contains(symbol), "IR should contain {symbol}");
    }
}

/// 文件 I/O 内置函数生成合法 IR,声明 stdio 符号。
#[test]
fn file_io_builtins_verify() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let call = |name: &str, args: Vec<Expr>| Expr::Call(CallExpr {
        callee: Box::new(Expr::Ident(name.to_string())),
        arguments: args,
        type_args: vec![],
    });
    let program = main_program(vec![
        let_stmt("content", Expr::Literal(Literal::String("data".to_string()))),
        let_stmt("ok", call("write_file", vec![
            Expr::Literal(Literal::String("out.txt".to_string())),
            Expr::Ident("content".to_string()),
        ])),
        let_stmt("data", call("read_file", vec![
            Expr::Literal(Literal::String("out.txt".to_string())),
        ])),
        sp(Stmt::Return(ReturnStmt {
            value: Some(Expr::Literal(Literal::Int(0))),
        })),
    ]);
    codegen.compile(&program).expect("compile should succeed");
    assert!(codegen.verify());
    let ir = codegen.print_llvm_ir();
    for symbol in ["declare ptr @fopen", "declare i64 @fread", "declare i64 @fwrite"] {
        assert!(ir.contains(symbol), "IR should declare {symbol}");
    }
}
