//! 向量合成（表达式/语句/程序合成 + ID 分发子函数）。
//! 纯搬移自 `ast_json.rs`，逻辑逐行原样，仅可见性提升为 `pub(crate)`，跨模块引用加前缀。
use huzi_ast::{BinOp, Block, Expr, FnStmt, Literal, Stmt, Type, UnOp};

pub(crate) fn synth_span(stmt: Stmt) -> huzi_ast::Spanned<Stmt> {
    huzi_ast::Spanned::with_span(stmt, huzi_ast::Span::new(1, 1))
}

pub(crate) fn synth_block(stmt: Stmt) -> Block {
    Block {
        statements: vec![synth_span(stmt)],
    }
}

pub(crate) fn synth_num(n: i64) -> Expr {
    Expr::Literal(Literal::Int(n))
}

fn synth_bin(op: BinOp, l: Expr, r: Expr) -> Expr {
    Expr::Binary(huzi_ast::BinaryExpr {
        left: Box::new(l),
        operator: op,
        right: Box::new(r),
    })
}

fn synth_assign(name: &str, value: Expr) -> Stmt {
    Stmt::Expr(huzi_ast::ExprStmt {
        expr: Expr::Assign(huzi_ast::AssignExpr {
            target: Box::new(Expr::Ident(name.to_string())),
            operator: huzi_ast::AssignOp::Assign,
            value: Box::new(value),
        }),
    })
}

fn synth_let(name: &str, value: Expr) -> Stmt {
    Stmt::Let(huzi_ast::LetStmt {
        name: name.to_string(),
        mutable: false,
        tuple_pattern: None,
        type_annotation: None,
        value: Some(value),
    })
}

/// 向量 ID 的表达式 JSON（8+8+6 个，与 `vectors.hz` 同构；中 8 为 P3 复合，后 6 为 P3b 结构类型）。
pub(crate) fn test_expr_json(id: &str) -> Option<Result<String, String>> {
    match id {
        "expr_num" => Some(super::expr::expr_to_json(&synth_num(42))),
        "expr_bool" => Some(super::expr::expr_to_json(&Expr::Literal(Literal::Bool(
            true,
        )))),
        "expr_str" => Some(super::expr::expr_to_json(&Expr::Literal(
            Literal::String("hello huzi".to_string()),
        ))),
        "expr_str_esc" => Some(super::expr::expr_to_json(&Expr::Literal(
            Literal::String("a\"b\\c\nd\te\rf".to_string()),
        ))),
        "expr_var" => Some(super::expr::expr_to_json(&Expr::Ident("x".to_string()))),
        "expr_bin" => Some(super::expr::expr_to_json(&synth_bin(
            BinOp::Add,
            synth_num(3),
            synth_num(4),
        ))),
        "expr_un" => Some(super::expr::expr_to_json(&Expr::Unary(
            huzi_ast::UnaryExpr {
                operator: UnOp::Neg,
                operand: Box::new(synth_num(5)),
            },
        ))),
        "expr_call" => Some(super::expr::expr_to_json(&Expr::Call(
            huzi_ast::CallExpr {
                callee: Box::new(Expr::Ident("fact".to_string())),
                arguments: vec![synth_num(5)],
                type_args: vec![],
            },
        ))),
        "expr_tuple" => Some(super::expr::expr_to_json(&Expr::TupleLiteral(vec![
            synth_num(1),
            synth_num(2),
        ]))),
        "expr_array" => Some(super::expr::expr_to_json(&Expr::ArrayLiteral(vec![
            synth_num(1),
            synth_num(2),
        ]))),
        "expr_index" => Some(super::expr::expr_to_json(&Expr::ArrayIndex(
            huzi_ast::ArrayIndexExpr {
                array: Box::new(Expr::Ident("a".to_string())),
                index: Box::new(synth_num(0)),
            },
        ))),
        "expr_enum_some" => Some(super::expr::expr_to_json(&Expr::EnumConstruct(
            huzi_ast::EnumConstructExpr {
                enum_name: "Option".to_string(),
                variant: "Some".to_string(),
                args: vec![synth_num(42)],
                type_args: vec![],
            },
        ))),
        "expr_enum_none" => Some(super::expr::expr_to_json(&Expr::EnumConstruct(
            huzi_ast::EnumConstructExpr {
                enum_name: "Option".to_string(),
                variant: "None".to_string(),
                args: vec![],
                type_args: vec![],
            },
        ))),
        "expr_enum_ok" => Some(super::expr::expr_to_json(&Expr::EnumConstruct(
            huzi_ast::EnumConstructExpr {
                enum_name: "Result".to_string(),
                variant: "Ok".to_string(),
                args: vec![synth_num(99)],
                type_args: vec![],
            },
        ))),
        "expr_enum_err" => Some(super::expr::expr_to_json(&Expr::EnumConstruct(
            huzi_ast::EnumConstructExpr {
                enum_name: "Result".to_string(),
                variant: "Err".to_string(),
                args: vec![Expr::Literal(Literal::String("oops".to_string()))],
                type_args: vec![],
            },
        ))),
        "expr_try" => Some(super::expr::expr_to_json(&Expr::Try(
            huzi_ast::TryExpr {
                inner: Box::new(Expr::Ident("x".to_string())),
            },
        ))),
        "expr_struct" => Some(super::expr::expr_to_json(&Expr::StructLiteral(
            huzi_ast::StructLiteralExpr {
                name: "Point".to_string(),
                fields: vec![
                    ("x".to_string(), synth_num(10)),
                    ("y".to_string(), synth_num(20)),
                ],
                type_args: vec![],
            },
        ))),
        "expr_field" => Some(super::expr::expr_to_json(&Expr::FieldAccess(
            huzi_ast::FieldAccessExpr {
                base: Box::new(Expr::Ident("p".to_string())),
                field: "x".to_string(),
            },
        ))),
        "expr_method" => Some(super::expr::expr_to_json(&Expr::MethodCall(
            huzi_ast::MethodCallExpr {
                receiver: Box::new(Expr::Ident("p".to_string())),
                method: "sum".to_string(),
                arguments: vec![],
            },
        ))),
        "expr_fstring" => Some(super::expr::expr_to_json(&Expr::FString(
            huzi_ast::FStringExpr {
                template: "hi {}!".to_string(),
                args: vec![Expr::Ident("name".to_string())],
            },
        ))),
        "type_atom" => Some(super::types::type_to_json(&Type::I32)),
        "type_applied" => Some(super::types::type_to_json(&Type::Applied(
            "Pair".to_string(),
            vec![Type::I32, Type::Str],
        ))),
        _ => None,
    }
}

/// 向量 ID 的语句 JSON（7 个，与 `vectors.hz` 同构）。
pub(crate) fn test_stmt_json(id: &str) -> Option<Result<String, String>> {
    match id {
        "stmt_let" => Some(super::stmt::stmt_to_json(&synth_let("x", synth_num(10)))),
        "stmt_assign" => Some(super::stmt::stmt_to_json(&synth_assign(
            "x",
            synth_bin(BinOp::Add, Expr::Ident("x".to_string()), synth_num(20)),
        ))),
        "stmt_if" => Some(super::stmt::stmt_to_json(&Stmt::If(huzi_ast::IfStmt {
            condition: synth_bin(BinOp::Gt, Expr::Ident("a".to_string()), synth_num(10)),
            then_branch: synth_block(synth_assign("r", synth_num(100))),
            elif_branches: vec![],
            else_branch: Some(synth_block(synth_assign("r", synth_num(200)))),
        }))),
        "stmt_while" => Some(super::stmt::stmt_to_json(&Stmt::While(
            huzi_ast::WhileStmt {
                condition: synth_bin(BinOp::Le, Expr::Ident("i".to_string()), synth_num(10)),
                body: synth_block(synth_assign(
                    "i",
                    synth_bin(BinOp::Add, Expr::Ident("i".to_string()), synth_num(1)),
                )),
            },
        ))),
        "stmt_return" => Some(super::stmt::stmt_to_json(&Stmt::Return(
            huzi_ast::ReturnStmt {
                value: Some(Expr::Ident("x".to_string())),
            },
        ))),
        "stmt_print" => Some(super::stmt::stmt_to_json(&Stmt::Expr(
            huzi_ast::ExprStmt {
                expr: Expr::Call(huzi_ast::CallExpr {
                    callee: Box::new(Expr::Ident("print".to_string())),
                    arguments: vec![Expr::Literal(Literal::String("hi".to_string()))],
                    type_args: vec![],
                }),
            },
        ))),
        "stmt_expr" => Some(super::stmt::stmt_to_json(&Stmt::Expr(
            huzi_ast::ExprStmt {
                expr: Expr::Call(huzi_ast::CallExpr {
                    callee: Box::new(Expr::Ident("f".to_string())),
                    arguments: vec![],
                    type_args: vec![],
                }),
            },
        ))),
        _ => None,
    }
}

/// fact 函数体（与 `vectors.hz` 同构：if 空 else + 递归 return）。
pub(crate) fn synth_fact_fn() -> FnStmt {
    let cond = synth_bin(BinOp::Le, Expr::Ident("n".to_string()), synth_num(1));
    let then_b = synth_block(Stmt::Return(huzi_ast::ReturnStmt {
        value: Some(synth_num(1)),
    }));
    let recur = Expr::Call(huzi_ast::CallExpr {
        callee: Box::new(Expr::Ident("fact".to_string())),
        arguments: vec![synth_bin(
            BinOp::Sub,
            Expr::Ident("n".to_string()),
            synth_num(1),
        )],
        type_args: vec![],
    });
    let ret = synth_bin(BinOp::Mul, Expr::Ident("n".to_string()), recur);
    FnStmt {
        name: "fact".to_string(),
        type_params: vec![],
        params: vec![huzi_ast::FnParam {
            name: "n".to_string(),
            param_type: huzi_ast::Type::I32,
        }],
        return_type: None,
        body: Block {
            statements: vec![
                synth_span(Stmt::If(huzi_ast::IfStmt {
                    condition: cond,
                    then_branch: then_b,
                    elif_branches: vec![],
                    else_branch: None,
                })),
                synth_span(Stmt::Return(huzi_ast::ReturnStmt { value: Some(ret) })),
            ],
        },
    }
}
