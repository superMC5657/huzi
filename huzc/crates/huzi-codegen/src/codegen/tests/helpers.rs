use super::super::*;

/// 单元测试不关心位置,统一用 (1,1) 包裹。
pub(super) fn sp(stmt: Stmt) -> Spanned<Stmt> {
    Spanned::new(stmt, 1, 1)
}

/// 构建一个仅包含 `fn main() -> i32` 且以 `body` 为函数体的测试程序。
pub(super) fn main_program(body: Vec<Spanned<Stmt>>) -> Program {
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

pub(super) fn let_stmt(name: &str, value: Expr) -> Spanned<Stmt> {
    sp(Stmt::Let(LetStmt {
        name: name.to_string(),
        mutable: false,
        tuple_pattern: None,
        type_annotation: None,
        value: Some(value),
    }))
}
