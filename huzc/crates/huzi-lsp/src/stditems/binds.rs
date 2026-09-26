//! `<bind>::` 补全与目标模块文件符号补全(读盘失败为空表,不抛错)。

use std::path::{Path, PathBuf};

use tower_lsp_server::ls_types::CompletionItem;

use crate::analysis::parse_and_collect;

use super::members::{is_exportable, target_method_items, to_completion_kind};
use super::roots::{module_file_roots, resolve_in_roots};

/// `<bind>::` 补全:经 import 表找点分名,读盘解析后转符号项。
pub(crate) fn bind_items(text: &str, bind: &str, prefix: &str) -> Vec<CompletionItem> {
    bind_items_with_roots(text, bind, prefix, &module_file_roots(None))
}

/// 同上,根集合由调用方注入(供单测,纯磁盘探查)。
/// 除 fn/类型外追加目标文件内全部 impl 与 trait 方法(深度补齐,失败保空表)。
pub(crate) fn bind_items_with_roots(
    text: &str, bind: &str, prefix: &str, roots: &[PathBuf],
) -> Vec<CompletionItem> {
    let Some(import_name) = crate::imports::find_import_for_bind(text, bind)
    else {
        return Vec::new();
    };
    let Some(path) = resolve_in_roots(&import_name, roots) else {
        return Vec::new();
    };
    let mut out = file_symbol_items(&path, prefix);
    let Ok(content) = std::fs::read_to_string(&path) else {
        return out;
    };
    let (program, _) = parse_and_collect(&content);
    out.extend(target_method_items(&program, prefix));
    out
}

/// 目标模块文件 -> 符号补全项(fn/类型,前缀匹配,读盘失败为空表)。
pub(crate) fn file_symbol_items(path: &Path, prefix: &str) -> Vec<CompletionItem> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let (_program, symbols) = parse_and_collect(&content);
    symbols
        .iter()
        .filter(|s| s.name.starts_with(prefix) && is_exportable(s.kind))
        .map(|s| CompletionItem {
            label: s.name.clone(),
            kind: Some(to_completion_kind(s.kind)),
            detail: Some(s.detail.clone()),
            ..Default::default()
        })
        .collect()
}
