//! 目标文件方法与 `std::` 前缀/impl/trait 方法补全(失败保空表,不抛错)。

use std::path::Path;

use huzi_ast::{Program, Stmt, SymbolKind as HuziSymbolKind};
use tower_lsp_server::ls_types::{CompletionItem, CompletionItemKind};

use super::roots::{find_std_lib, module_file_roots};

/// 目标文件内全部 impl/trait 方法(前缀匹配,供 `<bind>::` 深度补齐)。
pub(super) fn target_method_items(program: &Program, prefix: &str) -> Vec<CompletionItem> {
    let mut out = Vec::new();
    for stmt in &program.statements {
        match &stmt.node {
            Stmt::Impl(b) => for m in &b.methods {
                if m.name.starts_with(prefix) {
                    out.push(method_item(&m.name, &b.target_type));
                }
            },
            Stmt::Trait(d) => for m in &d.methods {
                if m.name.starts_with(prefix) {
                    out.push(method_item(&m.name, &d.name));
                }
            },
            _ => {}
        }
    }
    out
}

/// 方法补全项(供目标文件深度补齐,种类与同文件 impl/trait 对齐)。
fn method_item(name: &str, owner: &str) -> CompletionItem {
    CompletionItem {
        label: name.to_string(),
        kind: Some(CompletionItemKind::METHOD),
        detail: Some(format!("{owner}::{name}")),
        ..Default::default()
    }
}

/// `std::` 补全:首个命中的 `std/lib.hz` 的 export 首段(前缀匹配)。
pub(crate) fn std_prefix_items(prefix: &str) -> Vec<CompletionItem> {
    let Some(lib) = find_std_lib(&module_file_roots(None)) else {
        return Vec::new();
    };
    std_prefix_items_with_lib(&lib, prefix)
}

/// 同上,lib.hz 路径由调用方注入(供单测)。
pub(crate) fn std_prefix_items_with_lib(lib: &Path, prefix: &str) -> Vec<CompletionItem> {
    let Ok(content) = std::fs::read_to_string(lib) else {
        return Vec::new();
    };
    export_module_names(&content)
        .into_iter()
        .filter(|name| name.starts_with(prefix))
        .map(|name| CompletionItem {
            label: name.clone(),
            kind: Some(CompletionItemKind::MODULE),
            detail: Some(format!("std::{name}")),
            ..Default::default()
        })
        .collect()
}

/// 同文件 `impl ... for Target` 的方法补全(前缀匹配,供 `.` 与 `::` 共用)。
pub(crate) fn impl_method_items(program: &Program, target: &str, prefix: &str) -> Vec<CompletionItem> {
    let mut out = Vec::new();
    for stmt in &program.statements {
        if let Stmt::Impl(block) = &stmt.node {
            if block.target_type != target {
                continue;
            }
            for m in &block.methods {
                if m.name.starts_with(prefix) {
                    out.push(method_item(&m.name, target));
                }
            }
        }
    }
    out
}

/// 同文件 `trait Name` 的方法补全(前缀匹配,供 `Trait::` 使用)。
pub(crate) fn trait_method_items(program: &Program, name: &str, prefix: &str) -> Vec<CompletionItem> {
    for stmt in &program.statements {
        if let Stmt::Trait(def) = &stmt.node {
            if def.name == name {
                return def
                    .methods
                    .iter()
                    .filter(|m| m.name.starts_with(prefix))
                    .map(|m| CompletionItem {
                        label: m.name.clone(),
                        kind: Some(CompletionItemKind::METHOD),
                        detail: Some(format!("{name}::{}", m.name)),
                        ..Default::default()
                    })
                    .collect();
            }
        }
    }
    Vec::new()
}

/// `lib.hz` 文本 -> export 首段名(去重保序,非法行跳过)。
fn export_module_names(content: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix("export") else {
            continue;
        };
        let Some(path) = rest.strip_prefix(|c: char| c.is_whitespace()) else {
            continue;
        };
        let name = path.split_whitespace().next().unwrap_or("");
        let first = name.split("::").next().unwrap_or("");
        if !first.is_empty() && first.chars().all(|c| c.is_alphanumeric() || c == '_') && !out.contains(&first.to_string()) {
            out.push(first.to_string());
        }
    }
    out
}

/// 跨模块补全只收录可导出符号(函数与具名类型)。
pub(super) fn is_exportable(kind: HuziSymbolKind) -> bool {
    matches!(kind, HuziSymbolKind::Function | HuziSymbolKind::Struct | HuziSymbolKind::Enum | HuziSymbolKind::Trait)
}

/// Huzi 符号种类 -> 补全项种类(与 `completion.rs` 对齐)。
pub(super) fn to_completion_kind(kind: HuziSymbolKind) -> CompletionItemKind {
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
