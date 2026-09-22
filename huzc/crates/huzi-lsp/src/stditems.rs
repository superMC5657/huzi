//! L3/L4 级能力:std import 感知与 trait/impl 成员(补全与跳转共用)。
//!
//! - L3:按 `huzc modules.rs probe_entry_file` 语义解析模块文件
//!   (直连 `<root>/<a/b.hz>` → `<stem>/huzi.toml` 的 `lib_entry` →
//!   `<stem>/{src/lib.hz, lib.hz, src/mod.hz, mod.hz}`);读
//!   `std/lib.hz` export 表做 `std::` 补全,`<bind>::` 读目标文件
//!   符号表补全。磁盘失败一律空表,不抛错。
//! - L4:同文件 trait/impl 表转补全项(`Point.` 补 impl 方法,
//!   `Trait::` 补 trait 方法),AST 形状与 `symbols.rs` 对齐。

use std::path::{Path, PathBuf};

use huzi_ast::{Program, Stmt, SymbolKind as HuziSymbolKind};
use tower_lsp_server::ls_types::{
    CompletionItem, CompletionItemKind, Uri,
};

use crate::analysis::parse_and_collect;

/// `<stem>` 下的入口候选(与 `modules.rs` 顺序一致)。
const STEM_CANDIDATES: &[&str] =
    &["src/lib.hz", "lib.hz", "src/mod.hz", "mod.hz"];

/// `huzi-src` 相对探查后缀(与 `imports.rs` 存量顺序一致)。
const HUZI_SRC_SUBS: &[&str] = &[
    "huzi-src",
    "../huzi-src",
    "../../huzi-src",
    "../../../huzi-src",
];

/// 指定根集合内解析模块(供单测注入临时目录,纯磁盘探查)。
pub(crate) fn resolve_in_roots(
    import_name: &str,
    roots: &[PathBuf],
) -> Option<PathBuf> {
    let rel: PathBuf = import_name.split('.').collect();
    let file = rel.with_extension("hz");
    for root in roots {
        if let Some(hit) = probe_entry_file(root, &file) {
            return Some(hit);
        }
    }
    None
}

/// 探查单个根下的模块入口文件(直连 → `huzi.toml lib_entry` → 常规候选)。
pub(crate) fn probe_entry_file(
    root: &Path,
    file: &Path,
) -> Option<PathBuf> {
    let direct = root.join(file);
    if direct.is_file() {
        return Some(direct);
    }
    let stem_dir = root.join(file.with_extension(""));
    if stem_dir.is_dir() {
        let toml_path = stem_dir.join("huzi.toml");
        if toml_path.is_file() {
            if let Ok(content) = std::fs::read_to_string(&toml_path) {
                if let Some(entry) = parse_lib_entry(&content) {
                    let candidate = stem_dir.join(entry);
                    if candidate.is_file() {
                        return Some(candidate);
                    }
                }
            }
        }
        for candidate in STEM_CANDIDATES {
            let p = stem_dir.join(candidate);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// 候选根集合:当前文件目录 → 进程 cwd → 标准库根(环境/可执行文件旁/家目录)。
pub(crate) fn module_file_roots(
    current_uri: Option<&Uri>,
) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(uri) = current_uri {
        if let Some(path) = uri.to_file_path() {
            if let Some(dir) = path.parent() {
                roots.push(dir.to_path_buf());
            }
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd);
    }
    roots.extend(std_roots());
    let snapshot = roots.clone();
    for base in snapshot {
        for sub in HUZI_SRC_SUBS {
            roots.push(base.join(sub));
        }
    }
    roots
}

/// `<bind>::` 补全:经 import 表找点分名,读盘解析后转符号项。
pub(crate) fn bind_items(
    text: &str,
    bind: &str,
    prefix: &str,
) -> Vec<CompletionItem> {
    bind_items_with_roots(text, bind, prefix, &module_file_roots(None))
}

/// 同上,根集合由调用方注入(供单测,纯磁盘探查)。
pub(crate) fn bind_items_with_roots(
    text: &str,
    bind: &str,
    prefix: &str,
    roots: &[PathBuf],
) -> Vec<CompletionItem> {
    let Some(import_name) = crate::imports::find_import_for_bind(text, bind)
    else {
        return Vec::new();
    };
    let Some(path) = resolve_in_roots(&import_name, roots) else {
        return Vec::new();
    };
    file_symbol_items(&path, prefix)
}

/// 目标模块文件 -> 符号补全项(fn/类型,前缀匹配,读盘失败为空表)。
pub(crate) fn file_symbol_items(
    path: &Path,
    prefix: &str,
) -> Vec<CompletionItem> {
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

/// `std::` 补全:首个命中的 `std/lib.hz` 的 export 首段(前缀匹配)。
pub(crate) fn std_prefix_items(prefix: &str) -> Vec<CompletionItem> {
    let Some(lib) = find_std_lib(&module_file_roots(None)) else {
        return Vec::new();
    };
    std_prefix_items_with_lib(&lib, prefix)
}

/// 同上,lib.hz 路径由调用方注入(供单测)。
pub(crate) fn std_prefix_items_with_lib(
    lib: &Path,
    prefix: &str,
) -> Vec<CompletionItem> {
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
pub(crate) fn impl_method_items(
    program: &Program,
    target: &str,
    prefix: &str,
) -> Vec<CompletionItem> {
    let mut out = Vec::new();
    for stmt in &program.statements {
        if let Stmt::Impl(block) = &stmt.node {
            if block.target_type != target {
                continue;
            }
            for m in &block.methods {
                if m.name.starts_with(prefix) {
                    out.push(CompletionItem {
                        label: m.name.clone(),
                        kind: Some(CompletionItemKind::METHOD),
                        detail: Some(format!(
                            "{}::{}",
                            target, m.name
                        )),
                        ..Default::default()
                    });
                }
            }
        }
    }
    out
}

/// 同文件 `trait Name` 的方法补全(前缀匹配,供 `Trait::` 使用)。
pub(crate) fn trait_method_items(
    program: &Program,
    name: &str,
    prefix: &str,
) -> Vec<CompletionItem> {
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

/// 标准库根:HUZI_LIB → 可执行文件旁 → 家目录(与存量跳转顺序一致)。
fn std_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(lib) = std::env::var("HUZI_LIB") {
        roots.push(PathBuf::from(lib));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            for sub in [
                "../huzi-src",
                "../../huzi-src",
                "../../../huzi-src",
                "../lib/huzi-src",
                "huzi-src",
            ] {
                roots.push(exe_dir.join(sub));
            }
        }
    }
    if let Ok(home) =
        std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME"))
    {
        roots.push(PathBuf::from(&home).join(".huzi").join("huzi-src"));
        roots.push(PathBuf::from(&home).join(".huzi").join("std"));
    }
    roots
}

/// 根集合内找首个 `std/lib.hz`。
fn find_std_lib(roots: &[PathBuf]) -> Option<PathBuf> {
    for root in roots {
        for rel in ["std/lib.hz", "huzi-src/std/lib.hz"] {
            let p = root.join(rel);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// `huzi.toml` 文本 -> `lib_entry`/`lib` 值(无依赖的最小解析,失败为 `None`)。
fn parse_lib_entry(content: &str) -> Option<String> {
    for line in content.lines() {
        let line = line.trim();
        let Some(rest) = line
            .strip_prefix("lib_entry")
            .or_else(|| line.strip_prefix("lib"))
        else {
            continue;
        };
        let Some(value) = rest.trim_start().strip_prefix('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"').trim_matches('\'');
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

/// `lib.hz` 文本 -> export 首段名(去重保序,非法行跳过)。
fn export_module_names(content: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix("export") else {
            continue;
        };
        let Some(path) =
            rest.strip_prefix(|c: char| c.is_whitespace())
        else {
            continue;
        };
        let name = path.split_whitespace().next().unwrap_or("");
        let first = name.split("::").next().unwrap_or("");
        if !first.is_empty()
            && first
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_')
            && !out.contains(&first.to_string())
        {
            out.push(first.to_string());
        }
    }
    out
}

/// 跨模块补全只收录可导出符号(函数与具名类型)。
fn is_exportable(kind: HuziSymbolKind) -> bool {
    matches!(
        kind,
        HuziSymbolKind::Function
            | HuziSymbolKind::Struct
            | HuziSymbolKind::Enum
            | HuziSymbolKind::Trait
    )
}

/// Huzi 符号种类 -> 补全项种类(与 `completion.rs` 对齐)。
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// 并行安全的唯一临时目录。
    fn unique_dir(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "huzi_lsp_std_{tag}_{}_{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(dir.join("mods")).expect("create temp dirs");
        dir
    }

    #[test]
    fn lib_entry_wins_over_conventional_candidates() {
        // Given: stem 目录下 huzi.toml 指定 custom_entry.hz,且另有 lib.hz
        let dir = unique_dir("libentry");
        let stem = dir.join("mods").join("greeter");
        std::fs::create_dir_all(&stem).unwrap();
        std::fs::write(
            stem.join("huzi.toml"),
            "[package]\nname = \"greeter\"\nlib_entry = \"custom_entry.hz\"\n",
        )
        .unwrap();
        std::fs::write(stem.join("custom_entry.hz"), "fn hi() -> i32 {\n return 1\n}\n")
            .unwrap();
        std::fs::write(stem.join("lib.hz"), "fn other() -> i32 {\n return 2\n}\n").unwrap();
        // When: 解析 mods.greeter
        let hit = resolve_in_roots("mods.greeter", &[dir.clone()])
            .expect("lib_entry hit");
        // Then: 命中 custom_entry.hz 而非 lib.hz
        assert_eq!(hit, stem.join("custom_entry.hz"));
    }

    #[test]
    fn stem_lib_fallback_without_toml() {        // Given: 无 toml 的 stem 目录,仅有 lib.hz
        let dir = unique_dir("stemlib");
        let stem = dir.join("mods").join("plain");
        std::fs::create_dir_all(&stem).unwrap();
        std::fs::write(stem.join("lib.hz"), "fn go() -> i32 {\n return 1\n}\n").unwrap();
        // When: 解析 mods.plain
        let hit =
            resolve_in_roots("mods.plain", &[dir.clone()]).expect("stem hit");
        // Then: 命中 lib.hz
        assert_eq!(hit, stem.join("lib.hz"));
    }

    #[test]
    fn current_file_dir_root_resolves_sibling_module() {
        // Given: main.hz 与 mods/helpers.hz 并存
        let dir = unique_dir("curdir");
        let main = dir.join("main.hz");
        std::fs::write(&main, "import mods.helpers\n").unwrap();
        std::fs::write(
            dir.join("mods/helpers.hz"),
            "fn add(a: i32, b: i32) -> i32 {\n return a + b\n}\n",
        )
        .unwrap();
        let uri = Uri::from_file_path(&main).expect("file uri");
        // When: 经当前文件目录根解析
        let hit = resolve_in_roots(
            "mods.helpers",
            &module_file_roots(Some(&uri)),
        )
        .expect("hit");
        // Then: 命中同目录模块文件
        assert_eq!(hit, dir.join("mods/helpers.hz"));
    }

    #[test]
    fn bind_completion_reads_module_symbols() {
        // Given: 主文件 import mods.helpers,模块含 add/Point
        let dir = unique_dir("bind");
        std::fs::write(
            dir.join("mods/helpers.hz"),
            "struct Point { x: i32 }\nfn add(a: i32, b: i32) -> i32 {\n return a + b\n}\n",
        )
        .unwrap();
        let text = "import mods.helpers\nhelpers::";
        // When: 取 helpers:: 补全(注入临时根)
        let items = bind_items_with_roots(text, "helpers", "", &[dir]);
        let labels: Vec<&str> =
            items.iter().map(|i| i.label.as_str()).collect();
        // Then: 含 add 与 Point,不含局部变量
        assert!(labels.contains(&"add"), "{labels:?}");
        assert!(labels.contains(&"Point"), "{labels:?}");
    }

    #[test]
    fn bind_without_import_returns_empty() {
        // Given: 无 import 的文件
        // When: 取未知绑定补全
        let items = bind_items_with_roots("let x = 1\nfoo::", "foo", "", &[]);
        // Then: 空表,不抛错
        assert!(items.is_empty());
    }

    #[test]
    fn std_prefix_lists_exported_modules() {
        // Given: 仿 std/lib.hz 的 export 表
        let dir = unique_dir("stdlib");
        let lib = dir.join("lib.hz");
        std::fs::write(
            &lib,
            "export json\nexport json::*\nexport csv\nexport csv::*\n",
        )
        .unwrap();
        // When: 取空前缀与 `j` 前缀
        let all = std_prefix_items_with_lib(&lib, "");
        let filtered = std_prefix_items_with_lib(&lib, "j");
        // Then: 去重含 json/csv;j 前缀仅 json
        let labels: Vec<&str> =
            all.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"json"), "{labels:?}");
        assert!(labels.contains(&"csv"), "{labels:?}");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].label, "json");
    }

    #[test]
    fn impl_and_trait_tables_complete_members() {
        // Given: 含 Point/impl 与 Printable trait 的程序
        let text = "struct Point { x: i32, y: i32 }\ntrait Printable {\n fn show(self) -> str\n}\nimpl Printable for Point {\n fn show(self: Point) -> str {\n return \"p\"\n }\n}\n";
        let (program, _symbols) = parse_and_collect(text);
        // When: 取 Point 的 impl 方法与 Printable 的 trait 方法
        let impl_items = impl_method_items(&program, "Point", "");
        let trait_items = trait_method_items(&program, "Printable", "");
        // Then: 均含 show;无关类型为空
        assert!(impl_items.iter().any(|i| i.label == "show"));
        assert!(trait_items.iter().any(|i| i.label == "show"));
        assert!(impl_method_items(&program, "Other", "").is_empty());
        assert!(trait_method_items(&program, "Missing", "").is_empty());
    }
}
