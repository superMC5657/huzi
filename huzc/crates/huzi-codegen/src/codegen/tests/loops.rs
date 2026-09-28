use super::helpers::*;
use super::super::*;

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
