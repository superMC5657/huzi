//! 轻量符号表:从 [`Program`] 提取顶层与函数内符号,供后续 LSP hover/跳转使用。
//!
//! 只做单 [`Program`] 内收集,不做跨文件 import 解析,不依赖 parser/lexer。

use crate::ast::{Program, Span, Stmt};

mod collect;

use self::collect::{
    collect_block, collect_enum, collect_fn, collect_let_symbols, collect_struct,
};

/// 符号种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    Function,
    Struct,
    Enum,
    Variant,
    Variable,
    Param,
    Module,
    Trait,
}

/// 单个符号:名字、种类、源码区间与签名串。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub span: Span,
    pub detail: String,
}

/// 从整个 [`Program`] 收集符号。
///
/// 覆盖顶层 `fn`/`struct`/`enum`(含变体)/`let`/`import`,
/// 以及每个 `fn` 体内递归的 `let` 与 `for` 绑定变量。
/// 返回顺序即源码出现顺序。
pub fn collect_symbols(program: &Program) -> Vec<Symbol> {
    let mut out = Vec::new();
    for stmt in &program.statements {
        collect_top_level(stmt, &mut out);
    }
    out
}

/// 顶层语句分发:只处理符号定义类语句,表达式等直接跳过。
fn collect_top_level(stmt: &crate::ast::Spanned<Stmt>, out: &mut Vec<Symbol>) {
    match &stmt.node {
        Stmt::Fn(f) => collect_fn(stmt.span, f, out),
        Stmt::Struct(d) => collect_struct(stmt.span, d, out),
        Stmt::Enum(d) => collect_enum(stmt.span, d, out),
        Stmt::Trait(t) => {
            out.push(Symbol {
                name: t.name.clone(),
                kind: SymbolKind::Trait,
                span: stmt.span,
                detail: format!("trait {}", t.name),
            });
            for m in &t.methods {
                out.push(Symbol {
                    name: m.name.clone(),
                    kind: SymbolKind::Function,
                    span: stmt.span,
                    detail: format!("fn {}(...)", m.name),
                });
            }
        }
        Stmt::Impl(i) => {
            for m in &i.methods {
                collect_fn(stmt.span, m, out);
            }
        }
        Stmt::Let(l) => collect_let_symbols(stmt.span, l, out),
        Stmt::Import(i) => out.push(Symbol {
            name: i.name.clone(),
            kind: SymbolKind::Module,
            span: stmt.span,
            detail: format!("import {}", i.name),
        }),
        Stmt::Export(e) => out.push(Symbol {
            name: e.path.clone(),
            kind: SymbolKind::Module,
            span: stmt.span,
            detail: format!("export {}", if e.is_wildcard { format!("{}::*", e.path) } else { e.path.clone() }),
        }),
        Stmt::Block(b) => collect_block(b, out),
        Stmt::If(i) => {
            collect_block(&i.then_branch, out);
            for (_, b) in &i.elif_branches {
                collect_block(b, out);
            }
            if let Some(b) = &i.else_branch {
                collect_block(b, out);
            }
        }
        Stmt::For(f) => {
            out.push(Symbol {
                name: f.var_name.clone(),
                kind: SymbolKind::Variable,
                span: stmt.span,
                detail: format!("for {}", f.var_name),
            });
            collect_block(&f.body, out);
        }
        Stmt::While(w) => collect_block(&w.body, out),
        Stmt::Expr(_) | Stmt::Return(_) | Stmt::Break | Stmt::Continue | Stmt::Defer(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{
        Block, EnumDef, EnumVariant, FnParam, FnStmt, LetStmt, Spanned,
        StructDef, Type,
    };

    fn spanned_fn(name: &str, line: usize) -> Spanned<Stmt> {
        Spanned::with_range(
            Stmt::Fn(FnStmt {
                name: name.to_string(),
                type_params: vec![],
                params: vec![FnParam {
                    name: "a".to_string(),
                    param_type: Type::I32,
                }],
                return_type: Some(Type::I32),
                body: Block { statements: vec![] },
            }),
            line,
            1,
            line,
            10,
        )
    }

    fn spanned_struct(name: &str, line: usize) -> Spanned<Stmt> {
        Spanned::with_range(
            Stmt::Struct(StructDef {
                name: name.to_string(),
                type_params: vec![],
                fields: vec![],
            }),
            line,
            1,
            line,
            12,
        )
    }

    fn spanned_let(name: &str, line: usize) -> Spanned<Stmt> {
        Spanned::with_range(
            Stmt::Let(LetStmt {
                name: name.to_string(),
                mutable: false,
                tuple_pattern: None,
                type_annotation: Some(Type::I32),
                value: None,
            }),
            line,
            1,
            line,
            8,
        )
    }

    #[test]
    fn fn_struct_let_with_same_name_all_collected() {
        let program = Program {
            statements: vec![
                spanned_fn("item", 1),
                spanned_struct("item", 2),
                spanned_let("item", 3),
            ],
        };
        let syms = collect_symbols(&program);
        let kinds: Vec<SymbolKind> = syms
            .iter()
            .filter(|s| s.name == "item")
            .map(|s| s.kind)
            .collect();
        assert!(kinds.contains(&SymbolKind::Function));
        assert!(kinds.contains(&SymbolKind::Struct));
        assert!(kinds.contains(&SymbolKind::Variable));
        assert_eq!(kinds.len(), 3);
    }

    #[test]
    fn spans_are_never_inverted() {
        let program = Program {
            statements: vec![
                spanned_fn("add", 1),
                Spanned::with_range(
                    Stmt::Enum(EnumDef {
                        name: "Color".to_string(),
                        type_params: vec![],
                        variants: vec![EnumVariant {
                            name: "Red".to_string(),
                            payloads: vec![],
                        }],
                    }),
                    2,
                    1,
                    2,
                    9,
                ),
                spanned_let("x", 3),
            ],
        };
        for s in collect_symbols(&program) {
            assert!(
                (s.span.end_line, s.span.end_column)
                    >= (s.span.line, s.span.column),
                "inverted span for {}",
                s.name
            );
        }
    }

    #[test]
    fn details_are_non_empty_with_signature_shape() {
        let program = Program {
            statements: vec![
                spanned_fn("add", 1),
                spanned_struct("Point", 2),
                spanned_let("x", 3),
            ],
        };
        let syms = collect_symbols(&program);
        assert!(!syms.is_empty());
        for s in &syms {
            assert!(!s.detail.is_empty(), "empty detail for {}", s.name);
        }
        let fun = syms
            .iter()
            .find(|s| s.kind == SymbolKind::Function)
            .expect("fn symbol");
        assert_eq!(fun.detail, "fn add(a: i32): i32");
        let st = syms
            .iter()
            .find(|s| s.kind == SymbolKind::Struct)
            .expect("struct symbol");
        assert_eq!(st.detail, "struct Point");
        assert!(syms.iter().any(|s| s.kind == SymbolKind::Param));
    }

    #[test]
    fn fn_body_let_and_for_bindings_collected() {
        let body = Block {
            statements: vec![
                spanned_let("local", 2),
                Spanned::with_range(
                    Stmt::For(crate::ast::ForStmt {
                        var_name: "i".to_string(),
                        source: crate::ast::ForSource::Range {
                            start: crate::ast::Expr::Literal(
                                crate::ast::Literal::Int(0),
                            ),
                            end: crate::ast::Expr::Literal(
                                crate::ast::Literal::Int(10),
                            ),
                        },
                        body: Block { statements: vec![] },
                    }),
                    3,
                    1,
                    3,
                    6,
                ),
            ],
        };
        let program = Program {
            statements: vec![Spanned::with_range(
                Stmt::Fn(FnStmt {
                    name: "run".to_string(),
                    type_params: vec![],
                    params: vec![],
                    return_type: None,
                    body,
                }),
                1,
                1,
                1,
                8,
            )],
        };
        let syms = collect_symbols(&program);
        let names: Vec<&str> =
            syms.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"local"));
        assert!(names.contains(&"i"));
    }
}
