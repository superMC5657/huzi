use super::*;
use crate::CodeGen;

/// 单元测试不关心位置,统一用 (1,1) 包裹。
fn sp(stmt: Stmt) -> Spanned<Stmt> {
    Spanned::new(stmt, 1, 1)
}

/// 构建一个仅包含 `fn main() -> i32` 且以 `body` 为函数体的测试程序。
fn main_program(body: Vec<Spanned<Stmt>>) -> Program {
    Program {
        statements: vec![sp(Stmt::Fn(FnStmt {
            name: "main".to_string(),
            type_params: vec![],
            params: vec![],
            return_type: Some(Type::I32),
            body: Block { statements: body },
        }))],
    }
}

fn let_stmt(name: &str, value: Expr) -> Spanned<Stmt> {
    sp(Stmt::Let(LetStmt {
        name: name.to_string(),
        mutable: false,
        type_annotation: None,
        value: Some(value),
    }))
}

#[test]
fn minimal_program_verifies() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let program = main_program(vec![sp(Stmt::Return(ReturnStmt {
        value: Some(Expr::Literal(Literal::Int(0))),
    }))]);
    codegen.compile(&program).expect("compile should succeed");
    assert!(codegen.verify());
}

#[test]
fn scalar_type_mapping() {
    let context = Context::create();
    let codegen = CodeGen::new(&context, "test");

    let mapped = |ty: &Type| codegen.type_to_llvm(ty).unwrap();
    match mapped(&Type::I32) {
        inkwell::types::BasicTypeEnum::IntType(t) => assert_eq!(t.get_bit_width(), 32),
        other => panic!("i32 should map to an int type, got {:?}", other),
    }
    match mapped(&Type::Bool) {
        inkwell::types::BasicTypeEnum::IntType(t) => assert_eq!(t.get_bit_width(), 1),
        other => panic!("bool should map to i1, got {:?}", other),
    }
    assert_eq!(mapped(&Type::F64), context.f64_type().into());
    assert_eq!(mapped(&Type::F32), context.f32_type().into());
    assert!(matches!(
        mapped(&Type::Str),
        inkwell::types::BasicTypeEnum::PointerType(_)
    ));
    // 数组退化为裸指针；元素类型保存在 VarSlot 中。
    assert!(matches!(
        mapped(&Type::Array(Box::new(Type::I32), 4)),
        inkwell::types::BasicTypeEnum::PointerType(_)
    ));
}

#[test]
fn tuple_type_maps_to_struct() {
    let context = Context::create();
    let codegen = CodeGen::new(&context, "test");
    let ty = Type::Tuple(vec![Type::I32, Type::Str]);
    match codegen.type_to_llvm(&ty).unwrap() {
        inkwell::types::BasicTypeEnum::StructType(st) => assert_eq!(st.count_fields(), 2),
        other => panic!("tuple should map to a struct type, got {:?}", other),
    }
}

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
            type_annotation: Some(Type::I32),
            value: Some(Expr::EnumConstruct(EnumConstructExpr {
                enum_name: "my_math".to_string(),
                variant: "add".to_string(),
                args: vec![Expr::Literal(Literal::Int(10)), Expr::Literal(Literal::Int(20))],
            })),
        })),
        sp(Stmt::Return(ReturnStmt {
            value: Some(Expr::EnumConstruct(EnumConstructExpr {
                enum_name: "my_math".to_string(),
                variant: "helpers::add".to_string(),
                args: vec![Expr::Ident("x".to_string()), Expr::Literal(Literal::Int(5))],
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
fn debug_info_emits_metadata_and_variables() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    codegen.enable_debug_info("src/demo.hz");
    let program = main_program(vec![
        let_stmt("count", Expr::Literal(Literal::Int(1))),
        sp(Stmt::Return(ReturnStmt {
            value: Some(Expr::Ident("count".to_string())),
        })),
    ]);
    codegen.compile(&program).expect("compile should succeed");
    assert!(codegen.verify());

    let ir = codegen.print_llvm_ir();
    assert!(ir.contains("!dbg"), "IR should carry dbg attachments");
    assert!(
        ir.contains("DICompileUnit"),
        "IR should contain a compile unit"
    );
    assert!(
        ir.contains("DISubprogram"),
        "IR should contain a subprogram"
    );
    assert!(ir.contains("DIFile"), "IR should contain a file entry");
}

#[test]
fn no_debug_info_keeps_ir_clean() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let program = main_program(vec![sp(Stmt::Return(ReturnStmt {
        value: Some(Expr::Literal(Literal::Int(0))),
    }))]);
    codegen.compile(&program).expect("compile should succeed");
    assert!(codegen.verify());
    assert!(
        !codegen.print_llvm_ir().contains("!dbg"),
        "plain build must not emit debug metadata"
    );
}

/// `a / b` 应生成除零运行时检查块(rt_fail),除法本身为有符号除法。
#[test]
fn int_division_carries_zero_check() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let program = main_program(vec![
        let_stmt("a", Expr::Literal(Literal::Int(10))),
        let_stmt("b", Expr::Literal(Literal::Int(3))),
        sp(Stmt::Return(ReturnStmt {
            value: Some(Expr::Binary(BinaryExpr {
                left: Box::new(Expr::Ident("a".to_string())),
                operator: BinOp::Div,
                right: Box::new(Expr::Ident("b".to_string())),
            })),
        })),
    ]);
    codegen.compile(&program).expect("compile should succeed");
    assert!(codegen.verify());
    let ir = codegen.print_llvm_ir();
    assert!(ir.contains("sdiv"), "integer division should emit sdiv");
    assert!(
        ir.contains("rt_fail"),
        "division should carry a zero check block"
    );
}

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
