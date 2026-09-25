use super::{Symbol, SymbolKind};
use crate::ast::{Block, EnumDef, FnStmt, LetStmt, Span, Stmt, StructDef};

/// 登记一个 `fn`:函数符号 + 参数符号 + 体内递归收集。
pub(super) fn collect_fn(span: Span, f: &FnStmt, out: &mut Vec<Symbol>) {
    out.push(Symbol {
        name: f.name.clone(),
        kind: SymbolKind::Function,
        span,
        detail: fn_signature(f),
    });
    for p in &f.params {
        out.push(Symbol {
            name: p.name.clone(),
            kind: SymbolKind::Param,
            span,
            detail: format!("{}: {}", p.name, p.param_type),
        });
    }
    collect_block(&f.body, out);
}

/// 登记一个 `struct` 定义。
pub(super) fn collect_struct(span: Span, d: &StructDef, out: &mut Vec<Symbol>) {
    let type_params = if d.type_params.is_empty() {
        String::new()
    } else {
        format!("<{}>", d.type_params.join(", "))
    };
    out.push(Symbol {
        name: d.name.clone(),
        kind: SymbolKind::Struct,
        span,
        detail: format!("struct {}{}", d.name, type_params),
    });
}

/// 登记一个 `enum` 定义及其全部变体。
pub(super) fn collect_enum(span: Span, d: &EnumDef, out: &mut Vec<Symbol>) {
    let type_params_str = if d.type_params.is_empty() {
        String::new()
    } else {
        format!("<{}>", d.type_params.join(", "))
    };
    out.push(Symbol {
        name: d.name.clone(),
        kind: SymbolKind::Enum,
        span,
        detail: format!("enum {}{}", d.name, type_params_str),
    });
    for v in &d.variants {
        let detail = if v.payloads.is_empty() {
            format!("{}::{}", d.name, v.name)
        } else {
            let pays: Vec<String> =
                v.payloads.iter().map(|t| t.to_string()).collect();
            format!("{}::{}({})", d.name, v.name, pays.join(", "))
        };
        out.push(Symbol {
            name: v.name.clone(),
            kind: SymbolKind::Variant,
            span,
            detail,
        });
    }
}

/// 递归收集块内 `let` / `for` 绑定(含嵌套块/if/while 与块内 fn)。
pub(super) fn collect_block(block: &Block, out: &mut Vec<Symbol>) {
    for stmt in &block.statements {
        match &stmt.node {
            Stmt::Let(l) => collect_let_symbols(stmt.span, l, out),
            Stmt::For(f) => {
                out.push(Symbol {
                    name: f.var_name.clone(),
                    kind: SymbolKind::Variable,
                    span: stmt.span,
                    detail: format!("for {}", f.var_name),
                });
                collect_block(&f.body, out);
            }
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
            Stmt::While(w) => collect_block(&w.body, out),
            Stmt::Fn(f) => collect_fn(stmt.span, f, out),
            Stmt::Struct(d) => collect_struct(stmt.span, d, out),
            Stmt::Enum(d) => collect_enum(stmt.span, d, out),
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
            Stmt::Defer(inner) => {
                let synthetic_block = Block {
                    statements: vec![(**inner).clone()],
                };
                collect_block(&synthetic_block, out);
            }
            Stmt::Trait(_) | Stmt::Impl(_) | Stmt::Expr(_) | Stmt::Return(_) | Stmt::Break | Stmt::Continue => {}
        }
    }
}

/// 收集 `let` 语句定义的局部变量符号。
pub(super) fn collect_let_symbols(span: Span, l: &LetStmt, out: &mut Vec<Symbol>) {
    for (name, mutable) in l.bound_names() {
        if name == "_" {
            continue;
        }
        let mut detail = String::from("let ");
        if mutable {
            detail.push_str("mut ");
        }
        detail.push_str(name);
        if let Some(t) = &l.type_annotation {
            detail.push_str(&format!(": {}", t));
        }
        out.push(Symbol {
            name: name.to_string(),
            kind: SymbolKind::Variable,
            span,
            detail,
        });
    }
}

/// `fn` 签名串,如 `fn add(a: i32): i32`。
fn fn_signature(f: &FnStmt) -> String {
    let type_params = if f.type_params.is_empty() {
        String::new()
    } else {
        format!("<{}>", f.type_params.join(", "))
    };
    let params: Vec<String> = f
        .params
        .iter()
        .map(|p| format!("{}: {}", p.name, p.param_type))
        .collect();
    let mut s = format!("fn {}{}({})", f.name, type_params, params.join(", "));
    if let Some(ret) = &f.return_type {
        s.push_str(&format!(": {}", ret));
    }
    s
}
