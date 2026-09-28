use super::helpers::*;
use super::super::*;

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
