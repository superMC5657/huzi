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
        tuple_pattern: None,
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

/// 除数为编译期非常零整数字面量时跳过除零检查（`a / 3` 直接 sdiv）。
#[test]
fn const_div_skips_zero_check() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let program = main_program(vec![
        let_stmt("a", Expr::Literal(Literal::Int(10))),
        sp(Stmt::Return(ReturnStmt {
            value: Some(Expr::Binary(BinaryExpr {
                left: Box::new(Expr::Ident("a".to_string())),
                operator: BinOp::Div,
                right: Box::new(Expr::Literal(Literal::Int(3))),
            })),
        })),
    ]);
    codegen.compile(&program).expect("compile should succeed");
    assert!(codegen.verify());
    let ir = codegen.print_llvm_ir();
    assert!(ir.contains("sdiv"), "constant division should emit sdiv");
    assert!(
        !ir.contains("rt_fail"),
        "constant nonzero divisor should skip the zero check block"
    );
    assert!(
        !ir.contains("div_nz"),
        "constant nonzero divisor should skip the zero compare"
    );
}

/// `a % 7` 直接 srem,不带检查块。
#[test]
fn const_mod_skips_zero_check() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let program = main_program(vec![
        let_stmt("a", Expr::Literal(Literal::Int(10))),
        sp(Stmt::Return(ReturnStmt {
            value: Some(Expr::Binary(BinaryExpr {
                left: Box::new(Expr::Ident("a".to_string())),
                operator: BinOp::Mod,
                right: Box::new(Expr::Literal(Literal::Int(7))),
            })),
        })),
    ]);
    codegen.compile(&program).expect("compile should succeed");
    assert!(codegen.verify());
    let ir = codegen.print_llvm_ir();
    assert!(ir.contains("srem"), "constant modulo should emit srem");
    assert!(
        !ir.contains("rt_fail"),
        "constant nonzero modulus should skip the zero check block"
    );
}

/// 变量作模数时仍保留检查块。
#[test]
fn variable_mod_keeps_zero_check() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let program = main_program(vec![
        let_stmt("a", Expr::Literal(Literal::Int(10))),
        let_stmt("b", Expr::Literal(Literal::Int(3))),
        sp(Stmt::Return(ReturnStmt {
            value: Some(Expr::Binary(BinaryExpr {
                left: Box::new(Expr::Ident("a".to_string())),
                operator: BinOp::Mod,
                right: Box::new(Expr::Ident("b".to_string())),
            })),
        })),
    ]);
    codegen.compile(&program).expect("compile should succeed");
    assert!(codegen.verify());
    let ir = codegen.print_llvm_ir();
    assert!(ir.contains("srem"), "integer modulo should emit srem");
    assert!(
        ir.contains("rt_fail"),
        "variable modulus should carry a zero check block"
    );
}

/// 常量零作除数时保留原有检查路径（行为不变）。
#[test]
fn zero_literal_div_keeps_zero_check() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let program = main_program(vec![
        let_stmt("a", Expr::Literal(Literal::Int(10))),
        sp(Stmt::Return(ReturnStmt {
            value: Some(Expr::Binary(BinaryExpr {
                left: Box::new(Expr::Ident("a".to_string())),
                operator: BinOp::Div,
                right: Box::new(Expr::Literal(Literal::Int(0))),
            })),
        })),
    ]);
    codegen.compile(&program).expect("compile should succeed");
    assert!(codegen.verify());
    let ir = codegen.print_llvm_ir();
    assert!(
        ir.contains("rt_fail"),
        "zero divisor should keep the zero check block"
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

/// range 循环 `for i in start..end { body }` 的 AST 构造。
fn range_for(var: &str, start: i64, end: i64, body: Vec<Spanned<Stmt>>) -> Spanned<Stmt> {
    sp(Stmt::For(ForStmt {
        var_name: var.to_string(),
        source: ForSource::Range {
            start: Expr::Literal(Literal::Int(start)),
            end: Expr::Literal(Literal::Int(end)),
        },
        body: Block { statements: body },
    }))
}

/// `name = value` 表达式语句的 AST 构造。
fn assign_stmt(name: &str, value: Expr) -> Spanned<Stmt> {
    sp(Stmt::Expr(ExprStmt {
        expr: Expr::Assign(AssignExpr {
            target: Box::new(Expr::Ident(name.to_string())),
            operator: AssignOp::Assign,
            value: Box::new(value),
        }),
    }))
}

fn count_occurrences(haystack: &str, needle: &str) -> usize {
    haystack.matches(needle).count()
}

/// 含双子串的行数(供 `store i32 <val>, ptr %i` 这类带值操作数的计数).
fn count_lines_with(haystack: &str, a: &str, b: &str) -> usize {
    haystack.lines().filter(|l| l.contains(a) && l.contains(b)).count()
}

/// R2:range 循环归纳变量走头部 phi,热循环内 `i` 的 load 计数下降。
/// 程序:`let mut sum = 0; for i in 0..10 { sum = sum + i }; return sum`(无 continue)
/// - 改前 O0 IR:`load i32, ptr %i` = 3(条件 1 + 尾部递增 1 + 体内 `+i` 1),
///   `store i32, ptr %i` = 2(预存 1 + 回存 1)。
/// - 改后:`load` = 2(尾部 1 + 体内 1;条件走 phi 零 load,无 continue 故无中转块),
///   `store` = 2 不变。体内变量(sum)的访问形态不动,只动归纳变量 i。
#[test]
fn range_loop_induction_uses_phi() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let sum_add = Expr::Binary(BinaryExpr {
        left: Box::new(Expr::Ident("sum".to_string())),
        operator: BinOp::Add,
        right: Box::new(Expr::Ident("i".to_string())),
    });
    let program = main_program(vec![
        sp(Stmt::Let(LetStmt {
            name: "sum".to_string(),
            mutable: true,
            tuple_pattern: None,
            type_annotation: None,
            value: Some(Expr::Literal(Literal::Int(0))),
        })),
        range_for("i", 0, 10, vec![assign_stmt("sum", sum_add)]),
        sp(Stmt::Return(ReturnStmt {
            value: Some(Expr::Ident("sum".to_string())),
        })),
    ]);
    codegen.compile(&program).expect("compile should succeed");
    assert!(codegen.verify());
    let ir = codegen.print_llvm_ir();
    assert!(ir.contains("i_phi"), "header should carry an induction phi, got:\n{ir}");
    assert!(
        !ir.contains("for_cont"),
        "loop without continue should skip the trampoline block, got:\n{ir}"
    );
    assert_eq!(
        count_occurrences(&ir, "load i32, ptr %i,"),
        2,
        "induction loads should drop 3 -> 2, got:\n{ir}"
    );
    assert_eq!(
        count_lines_with(&ir, "store i32", "ptr %i,"),
        2,
        "induction stores stay 2, got:\n{ir}"
    );
}

/// 空循环 `0..0` 与逆序 `5..0` 首轮即出:保持 SLT 条件形态,编译+校验通过。
#[test]
fn range_loop_empty_and_reversed_verify() {
    for (start, end) in [(0_i64, 0_i64), (5, 0)] {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test");
        let program = main_program(vec![
            range_for("i", start, end, vec![]),
            sp(Stmt::Return(ReturnStmt {
                value: Some(Expr::Literal(Literal::Int(0))),
            })),
        ]);
        codegen
            .compile(&program)
            .expect("empty/reversed range should compile");
        assert!(codegen.verify());
        let ir = codegen.print_llvm_ir();
        assert!(
            ir.contains("icmp slt"),
            "{start}..{end} should keep an SLT exit check, got:\n{ir}"
        );
    }
}

/// break/continue/体内改写/嵌套:中转块与回边补齐后仍可校验。
/// continue 保持"跳过自增"的既有语义;体内 `i = 5` 改写经 latch 重载
/// 被下一轮看到;内层循环遮蔽外层同名变量。
#[test]
fn range_loop_control_flow_and_nesting_verify() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let sum_add_i = || {
        assign_stmt(
            "sum",
            Expr::Binary(BinaryExpr {
                left: Box::new(Expr::Ident("sum".to_string())),
                operator: BinOp::Add,
                right: Box::new(Expr::Ident("i".to_string())),
            }),
        )
    };
    let inner = range_for(
        "j",
        1,
        4,
        vec![assign_stmt(
            "sum",
            Expr::Binary(BinaryExpr {
                left: Box::new(Expr::Ident("sum".to_string())),
                operator: BinOp::Add,
                right: Box::new(Expr::Binary(BinaryExpr {
                    left: Box::new(Expr::Ident("i".to_string())),
                    operator: BinOp::Mul,
                    right: Box::new(Expr::Ident("j".to_string())),
                })),
            }),
        )],
    );
    let program = main_program(vec![
        sp(Stmt::Let(LetStmt {
            name: "sum".to_string(),
            mutable: true,
            tuple_pattern: None,
            type_annotation: None,
            value: Some(Expr::Literal(Literal::Int(0))),
        })),
        range_for("i", 0, 5, vec![sum_add_i(), sp(Stmt::Continue)]),
        range_for("i", 0, 5, vec![sp(Stmt::Break)]),
        range_for(
            "i",
            0,
            5,
            vec![assign_stmt("i", Expr::Literal(Literal::Int(5)))],
        ),
        range_for("i", 1, 4, vec![inner]),
        sp(Stmt::Return(ReturnStmt {
            value: Some(Expr::Ident("sum".to_string())),
        })),
    ]);
    codegen.compile(&program).expect("compile should succeed");
    assert!(codegen.verify());
    let ir = codegen.print_llvm_ir();
    assert!(ir.contains("i_phi"), "nested loops should carry induction phis, got:\n{ir}");
    assert!(ir.contains("for_cont"), "the loop with continue should carry a trampoline, got:\n{ir}");
}
