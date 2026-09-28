use super::helpers::*;
use super::super::*;

#[test]
fn module_function_callable_via_qualified_name() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");

    // 模块 helpers 定义 fn add(a: i32, b: i32) -> i32
    let helpers = Program {
        statements: vec![sp(Stmt::Fn(FnStmt {
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
        }))],
    };
    codegen.add_module("helpers", Some(&helpers), None);

    // 主程序 return helpers::add(1, 2) —— 解析为 EnumConstruct 形式
    let program = main_program(vec![sp(Stmt::Return(ReturnStmt {
        value: Some(Expr::EnumConstruct(EnumConstructExpr {
            enum_name: "helpers".to_string(),
            variant: "add".to_string(),
            args: vec![
                Expr::Literal(Literal::Int(1)),
                Expr::Literal(Literal::Int(2)),
            ],
            type_args: Vec::new(),
        })),
    }))]);
    codegen.compile(&program).expect("compile should succeed");
    assert!(codegen.verify());
}

#[test]
fn reexport_module_functions_callable_without_wrapper() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");

    // 1. 底层实现模块 helpers: fn add(a, b) -> i32
    let helpers = Program {
        statements: vec![sp(Stmt::Fn(FnStmt {
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
        }))],
    };
    codegen.add_module("helpers", Some(&helpers), None);

    // 2. 门面入口模块 my_math:
    // 导出模块：export helpers
    // 导出通配符：export helpers::*
    // 完全没有任何手写函数体！
    let my_math = Program {
        statements: vec![
            sp(Stmt::Export(ExportStmt { path: "helpers".to_string(), is_wildcard: false })),
            sp(Stmt::Export(ExportStmt { path: "helpers".to_string(), is_wildcard: true })),
        ],
    };
    codegen.add_module("my_math", Some(&my_math), None);

    // 3. 主程序: 分别通过 my_math::add (拍平重导出) 与 my_math::helpers::add (层级导出) 调用
    let program = main_program(vec![
        sp(Stmt::Let(LetStmt {
            name: "x".to_string(),
            mutable: false,
            tuple_pattern: None,
            type_annotation: Some(Type::I32),
            value: Some(Expr::EnumConstruct(EnumConstructExpr {
                enum_name: "my_math".to_string(),
                variant: "add".to_string(),
                args: vec![Expr::Literal(Literal::Int(10)), Expr::Literal(Literal::Int(20))],
                type_args: Vec::new(),
            })),
        })),
        sp(Stmt::Return(ReturnStmt {
            value: Some(Expr::EnumConstruct(EnumConstructExpr {
                enum_name: "my_math".to_string(),
                variant: "helpers::add".to_string(),
                args: vec![Expr::Ident("x".to_string()), Expr::Literal(Literal::Int(5))],
                type_args: Vec::new(),
            })),
        })),
    ]);

    codegen.compile(&program).expect("compile should succeed");
    assert!(codegen.verify());
}

#[test]
fn unknown_module_function_reports_error() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let program = main_program(vec![sp(Stmt::Return(ReturnStmt {
        value: Some(Expr::EnumConstruct(EnumConstructExpr {
            enum_name: "nomod".to_string(),
            variant: "add".to_string(),
            args: vec![],
            type_args: Vec::new(),
        })),
    }))]);
    let err = codegen.compile(&program).expect_err("unknown module must fail");
    let message = err.message();
    assert!(
        message.contains("Unknown enum: nomod"),
        "got: {}",
        message
    );
}
