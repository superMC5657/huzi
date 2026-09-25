//! 各上下文补全项组装:空白/词内、`.` 成员、`::` 模块。

use super::{GENERIC_MEMBERS, KEYWORDS, MATH_FNS};
use huzi_ast::symbols::SymbolKind as HuziSymbolKind;
use tower_lsp_server::ls_types::{CompletionItem, CompletionItemKind};

use crate::analysis::parse_and_collect;

/// 空白/词内上下文:关键字(前缀匹配) + 同文件符号(前缀匹配)。
pub(super) fn normal_items(
    symbols: &[huzi_ast::symbols::Symbol],
    prefix: &str,
) -> Vec<CompletionItem> {
    let mut out = Vec::new();
    for kw in KEYWORDS.iter().filter(|kw| kw.starts_with(prefix)) {
        out.push(CompletionItem {
            label: (*kw).to_string(),
            kind: Some(CompletionItemKind::KEYWORD),
            detail: Some("keyword".to_string()),
            ..Default::default()
        });
    }
    for sym in symbols.iter().filter(|s| s.name.starts_with(prefix)) {
        out.push(CompletionItem {
            label: sym.name.clone(),
            kind: Some(to_completion_kind(sym.kind)),
            detail: Some(sym.detail.clone()),
            ..Default::default()
        });
    }
    out
}

/// `.` 上下文:类型名/变量名补字段+impl 方法,枚举补变体,否则兜底。
pub(super) fn dot_items(text: &str, base: &str, prefix: &str) -> Vec<CompletionItem> {
    let (program, _) = parse_and_collect(text);
    if let Some(items) = struct_items(&program, base, prefix) {
        return items;
    }
    for stmt in &program.statements {
        if let huzi_ast::Stmt::Enum(d) = &stmt.node {
            if d.name == base {
                return variant_items(&d.variants, base, prefix);
            }
        }
    }
    if let Some(ty) = let_ty(&program, base) {
        if let Some(items) = struct_items(&program, &ty, prefix) {
            return items;
        }
    }
    generic_member_items(prefix)
}
/// 结构体字段+同文件 impl 方法(命中才 Some,供类型名/变量类型共用)。
fn struct_items(p: &huzi_ast::Program, ty: &str, prefix: &str) -> Option<Vec<CompletionItem>> {
    for stmt in &p.statements {
        if let huzi_ast::Stmt::Struct(d) = &stmt.node {
            if d.name == ty {
                let mut items: Vec<CompletionItem> = d.fields.iter().filter(|f| f.name.starts_with(prefix)).map(|f| CompletionItem { label: f.name.clone(), kind: Some(CompletionItemKind::FIELD), detail: Some(format!("{ty}::{}: {}", f.name, f.field_type)), ..Default::default() }).collect();
                items.extend(crate::stditems::impl_method_items(p, ty, prefix));
                return Some(items);
            }
        }
    }
    None
}
/// 变量名->结构体名(`let p: Point`注解优先,`let p = Point{}`按字面量)。
fn let_ty(p: &huzi_ast::Program, var: &str) -> Option<String> {
    let_ty_in(&p.statements, var)
}
fn let_ty_in(stmts: &[huzi_ast::Spanned<huzi_ast::Stmt>], var: &str) -> Option<String> {
    let mut hit: Option<String> = None;
    for s in stmts {
        match &s.node {
            huzi_ast::Stmt::Let(l) if l.name == var => {
                if let Some(huzi_ast::Type::Named(t)) = &l.type_annotation {
                    hit = Some(t.clone());
                } else if let Some(huzi_ast::Expr::StructLiteral(sl)) = &l.value {
                    hit = Some(sl.name.clone());
                }
            }
            huzi_ast::Stmt::Fn(f) => { if let Some(t) = let_ty_in(&f.body.statements, var) { hit = Some(t); } }
            huzi_ast::Stmt::Block(b) => { if let Some(t) = let_ty_in(&b.statements, var) { hit = Some(t); } }
            huzi_ast::Stmt::If(v) => { for b in std::iter::once(&v.then_branch).chain(v.elif_branches.iter().map(|(_, b)| b)).chain(v.else_branch.iter()) { if let Some(t) = let_ty_in(&b.statements, var) { hit = Some(t); } } }
            huzi_ast::Stmt::For(v) => { if let Some(t) = let_ty_in(&v.body.statements, var) { hit = Some(t); } }
            huzi_ast::Stmt::While(v) => { if let Some(t) = let_ty_in(&v.body.statements, var) { hit = Some(t); } }
            _ => {}
        }
    }
    hit
}

/// `::` 上下文:`math` 补数学函数,枚举名补变体,trait 名补 trait 方法,
/// 类型名补 impl 方法,`std` 补 export 表,import 绑定读模块文件补符号。
pub(super) fn colon_items(
    text: &str,
    module: &str,
    prefix: &str,
) -> Vec<CompletionItem> {
    if module == "math" {
        return MATH_FNS
            .iter()
            .filter(|name| name.starts_with(prefix))
            .map(|name| CompletionItem {
                label: (*name).to_string(),
                kind: Some(CompletionItemKind::FUNCTION),
                detail: Some(format!("math::{name}")),
                ..Default::default()
            })
            .collect();
    }
    let (program, _symbols) = parse_and_collect(text);
    for stmt in &program.statements {
        if let huzi_ast::Stmt::Enum(d) = &stmt.node {
            if d.name == module {
                return variant_items(&d.variants, module, prefix);
            }
        }
    }
    let mut items =
        crate::stditems::trait_method_items(&program, module, prefix);
    items.extend(crate::stditems::impl_method_items(
        &program, module, prefix,
    ));
    if !items.is_empty() {
        return items;
    }
    if module == "std" {
        return crate::stditems::std_prefix_items(prefix);
    }
    crate::stditems::bind_items(text, module, prefix)
}

/// 枚举变体列表(前缀匹配,供 `.` 与 `::` 共用)。
fn variant_items(
    variants: &[huzi_ast::EnumVariant],
    enum_name: &str,
    prefix: &str,
) -> Vec<CompletionItem> {
    variants
        .iter()
        .filter(|v| v.name.starts_with(prefix))
        .map(|v| CompletionItem {
            label: v.name.clone(),
            kind: Some(CompletionItemKind::ENUM_MEMBER),
            detail: Some(format!("{enum_name}::{}", v.name)),
            ..Default::default()
        })
        .collect()
}

/// 通用成员列表(前缀匹配)。
fn generic_member_items(prefix: &str) -> Vec<CompletionItem> {
    GENERIC_MEMBERS
        .iter()
        .filter(|name| name.starts_with(prefix))
        .map(|name| CompletionItem {
            label: (*name).to_string(),
            kind: Some(CompletionItemKind::FIELD),
            detail: Some("member".to_string()),
            ..Default::default()
        })
        .collect()
}

/// Huzi 符号种类 -> 补全项种类。
fn to_completion_kind(kind: HuziSymbolKind) -> CompletionItemKind {
    match kind {
        HuziSymbolKind::Function => CompletionItemKind::FUNCTION,
        HuziSymbolKind::Struct => CompletionItemKind::STRUCT,
        HuziSymbolKind::Enum => CompletionItemKind::ENUM,
        HuziSymbolKind::Variant => CompletionItemKind::ENUM_MEMBER,
        HuziSymbolKind::Module => CompletionItemKind::MODULE,
        HuziSymbolKind::Trait => CompletionItemKind::INTERFACE,
        HuziSymbolKind::Variable | HuziSymbolKind::Param => {
            CompletionItemKind::VARIABLE
        }
    }
}
