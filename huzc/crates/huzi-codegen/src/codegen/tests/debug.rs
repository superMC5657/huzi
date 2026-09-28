use super::helpers::*;
use super::super::*;

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
