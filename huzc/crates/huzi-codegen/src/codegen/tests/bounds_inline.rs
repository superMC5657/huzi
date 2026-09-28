use super::helpers::*;
use super::super::*;

/// 数组下标读取应生成越界检查块(rt_fail)。
#[test]
fn array_indexing_carries_bounds_check() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let program = main_program(vec![
        let_stmt(
            "arr",
            Expr::ArrayLiteral(vec![
                Expr::Literal(Literal::Int(1)),
                Expr::Literal(Literal::Int(2)),
                Expr::Literal(Literal::Int(3)),
            ]),
        ),
        sp(Stmt::Return(ReturnStmt {
            value: Some(Expr::ArrayIndex(ArrayIndexExpr {
                array: Box::new(Expr::Ident("arr".to_string())),
                index: Box::new(Expr::Literal(Literal::Int(1))),
            })),
        })),
    ]);
    codegen.compile(&program).expect("compile should succeed");
    assert!(codegen.verify());
    let ir = codegen.print_llvm_ir();
    assert!(
        ir.contains("rt_fail"),
        "array indexing should carry a bounds check block"
    );
}

/// R3:小叶函数挂 `inlinehint`(仅提示,不强制内联);大函数与 main 不挂。
#[test]
fn small_function_gets_inlinehint() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let add = sp(Stmt::Fn(FnStmt {
        name: "add".to_string(),
        type_params: vec![],
        params: vec![
            FnParam { name: "a".to_string(), param_type: Type::I32 },
            FnParam { name: "b".to_string(), param_type: Type::I32 },
        ],
        return_type: Some(Type::I32),
        body: Block {
            statements: vec![sp(Stmt::Return(ReturnStmt {
                value: Some(Expr::Binary(BinaryExpr {
                    left: Box::new(Expr::Ident("a".to_string())),
                    operator: BinOp::Add,
                    right: Box::new(Expr::Ident("b".to_string())),
                })),
            }))],
        },
    }));
    // 10 条顶层语句(>8 阈值):不应挂提示,作负例。
    let mut big_body: Vec<Spanned<Stmt>> = (0..9)
        .map(|i| {
            let_stmt(
                &format!("t{i}"),
                Expr::Binary(BinaryExpr {
                    left: Box::new(Expr::Literal(Literal::Int(i))),
                    operator: BinOp::Add,
                    right: Box::new(Expr::Ident("x".to_string())),
                }),
            )
        })
        .collect();
    big_body.push(sp(Stmt::Return(ReturnStmt {
        value: Some(Expr::Ident("x".to_string())),
    })));
    let big = sp(Stmt::Fn(FnStmt {
        name: "big".to_string(),
        type_params: vec![],
        params: vec![FnParam { name: "x".to_string(), param_type: Type::I32 }],
        return_type: Some(Type::I32),
        body: Block { statements: big_body },
    }));
    let main = sp(Stmt::Fn(FnStmt {
        name: "main".to_string(),
        type_params: vec![],
        params: vec![],
        return_type: Some(Type::I32),
        body: Block {
            statements: vec![sp(Stmt::Return(ReturnStmt {
                value: Some(Expr::Call(CallExpr {
                    callee: Box::new(Expr::Ident("add".to_string())),
                    arguments: vec![
                        Expr::Literal(Literal::Int(1)),
                        Expr::Literal(Literal::Int(2)),
                    ],
                    type_args: vec![],
                })),
            }))],
        },
    }));
    let program = Program { statements: vec![add, big, main] };
    codegen.compile(&program).expect("compile should succeed");
    assert!(codegen.verify());
    let ir = codegen.print_llvm_ir();
    // 仅小函数 `add` 贡献 inlinehint(`big`/main 均不挂):LLVM 打印两处
    // (`; Function Attrs: inlinehint` 注释行 + `attributes #0 = { inlinehint }`)。
    assert!(
        ir.contains("define i32 @add(i32 %0, i32 %1) #0"),
        "add must carry attr set"
    );
    assert!(
        ir.contains("attributes #0 = { inlinehint }"),
        "attr set must hold inlinehint"
    );
    assert_eq!(
        ir.matches("inlinehint").count(),
        2,
        "only add may be hinted"
    );
}
