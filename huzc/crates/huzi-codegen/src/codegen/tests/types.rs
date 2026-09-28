use super::helpers::*;
use super::super::*;

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
